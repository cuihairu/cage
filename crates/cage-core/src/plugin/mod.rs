//! Host-side plugin worker driver (P2-2): runs a validation plugin as an
//! *independent process* over the versioned JSON IPC protocol
//! (`cage-plugin-protocol`), never embedding an interpreter.
//!
//! This is the failure-behaviour table of review §四.2 made concrete. Each
//! of the seven failure classes is a [`WorkerFailure`] variant, mapped onto
//! the `gamerule` error-code family so a plugin problem surfaces exactly
//! like any other L7 finding:
//!
//! | Failure class | `WorkerFailure` variant | Cage code |
//! |---|---|
//! | 插件缺失 | [`WorkerFailure::Missing`] | E1602 |
//! | 协议版本不符 | [`WorkerFailure::ProtocolVersionMismatch`] | E1603 |
//! | Python 版本不符 | [`WorkerFailure::PythonVersionMismatch`] | E1603 |
//! | 插件超时 | [`WorkerFailure::Timeout`] | E1603 |
//! | 插件崩溃 | [`WorkerFailure::Crash`] | E1603 |
//! | 插件输出非法结果 | [`WorkerFailure::InvalidOutput`] | E1603 |
//! | 插件抛异常 | [`WorkerFailure::PluginError`] | E1603 |
//!
//! Plugin *findings* (the diagnostics a plugin chooses to report) are not a
//! failure class — they map to E1601 and carry the table/row/field back to a
//! concrete source location (the "自动定位到表、行、字段" row).
//!
//! ## Not a sandbox
//!
//! An independent process improves crash isolation but is not a security
//! boundary. The plugin runs with the author's own privileges; untrusted
//! plugins need an OS-level container (design §17 / review §四.2).

use crate::diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics};
use crate::error::codes::gamerule;
use crate::schema::Schema;
use crate::value::Document;
use cage_plugin_protocol::{
    ProtocolError, Request, Response, RowPayload, TablePayload, WireDiagnostic, PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub mod attestation;

pub use attestation::{
    AttestationOutcome, PluginAttestation, ValidationAttestation, ValidationEnvironment,
};

/// Default plugin timeout: a validation plugin is expected to finish fast.
pub const DEFAULT_PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Cap on stderr carried into a crash diagnostic (400 bytes, char boundary).
pub const MAX_PLUGIN_STDERR_BYTES: usize = 400;

/// The only runtime the MVP worker drives.
pub const RUNTIME_PYTHON: &str = "python";

/// The interpreter the MVP worker drives.
const PYTHON_BIN: &str = "python3";

/// A Python interpreter version constraint from `plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonConstraint {
    /// Lower bound (inclusive), e.g. `3.9`.
    pub minimum: (u32, u32),
    /// Exact version when `Some`, e.g. `3.12.1`.
    pub exact: Option<(u32, u32, u32)>,
}

impl PythonConstraint {
    /// Parse a constraint string the way `plugin.toml` writes it.
    ///
    /// Accepted forms: `>=X.Y`, `>=X.Y.Z`, `X.Y[.Z]` (exact), `==X.Y[.Z]`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty python constraint".to_string());
        }
        let (exact, version_text) = if let Some(rest) = text.strip_prefix(">=") {
            (false, rest)
        } else if let Some(rest) = text.strip_prefix("==") {
            (true, rest)
        } else {
            (true, text)
        };
        let version_text = version_text.trim();
        let parts: Vec<&str> = version_text.split('.').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(format!(
                "invalid python constraint '{text}' (need X.Y or X.Y.Z)"
            ));
        }
        let major: u32 = parts[0]
            .trim()
            .parse()
            .map_err(|_| format!("invalid python constraint '{text}'"))?;
        let minor: u32 = parts[1]
            .trim()
            .parse()
            .map_err(|_| format!("invalid python constraint '{text}'"))?;
        let patch: Option<u32> = if parts.len() == 3 {
            Some(
                parts[2]
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid python constraint '{text}'"))?,
            )
        } else {
            None
        };
        if exact {
            Ok(Self {
                minimum: (major, minor),
                exact: Some((major, minor, patch.unwrap_or(0))),
            })
        } else {
            Ok(Self {
                minimum: (major, minor),
                exact: None,
            })
        }
    }

    /// Whether an interpreter version `(major, minor, patch)` satisfies this.
    pub fn matches(&self, version: (u32, u32, u32)) -> bool {
        if let Some(exact) = self.exact {
            return version == exact;
        }
        version.0 > self.minimum.0 || (version.0 == self.minimum.0 && version.1 >= self.minimum.1)
    }

    /// Human-readable form for diagnostics.
    pub fn display(&self) -> String {
        if let Some(exact) = self.exact {
            format!("{}.{}.{}", exact.0, exact.1, exact.2)
        } else {
            format!(">={}.{}", self.minimum.0, self.minimum.1)
        }
    }
}

/// A parsed `plugins/<id>/plugin.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginManifest {
    /// Stable plugin id (matches the directory name by convention).
    pub id: String,
    /// Plugin version declared by its author.
    pub version: String,
    /// Runtime; the MVP accepts only [`RUNTIME_PYTHON`].
    pub runtime: String,
    /// Entry script, relative to the manifest directory.
    pub entry: PathBuf,
    /// Optional interpreter constraint (pre-checked before every run).
    pub python: Option<PythonConstraint>,
    /// Worker timeout.
    pub timeout: Duration,
    /// Directory holding `plugin.toml` (the entry resolves against this).
    pub dir: PathBuf,
}

/// The plugin `plugin.toml` wire shape (before validation).
#[derive(Debug, Deserialize)]
struct RawManifest {
    id: String,
    version: String,
    #[serde(default = "default_runtime")]
    runtime: String,
    entry: String,
    #[serde(default)]
    python: Option<String>,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
}

fn default_runtime() -> String {
    RUNTIME_PYTHON.to_string()
}

fn default_timeout_secs() -> u64 {
    DEFAULT_PLUGIN_TIMEOUT.as_secs()
}

impl PluginManifest {
    /// Load and validate `plugins/<id>/plugin.toml` from `dir`.
    pub fn load(dir: &Path) -> Result<Self, Diagnostics> {
        let manifest_path = dir.join("plugin.toml");
        if !manifest_path.is_file() {
            return Err(plugin_missing(
                &dir.display().to_string(),
                "plugin.toml not found",
            ));
        }
        let text = match std::fs::read_to_string(&manifest_path) {
            Ok(t) => t,
            Err(e) => {
                return Err(plugin_missing(
                    &manifest_path.display().to_string(),
                    &format!("cannot read: {e}"),
                ))
            }
        };
        let raw: RawManifest = toml::from_str(&text).map_err(|e| {
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                gamerule::E1602,
                format!("invalid plugin manifest {}: {e}", manifest_path.display()),
            ));
            diags
        })?;
        if raw.id.trim().is_empty() {
            return Err(plugin_missing(
                &manifest_path.display().to_string(),
                "manifest has an empty 'id'",
            ));
        }
        if raw.runtime != RUNTIME_PYTHON {
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                gamerule::E1602,
                format!(
                    "plugin {} declares unsupported runtime '{}' (this build drives '{}')",
                    raw.id, raw.runtime, RUNTIME_PYTHON
                ),
            ));
            return Err(diags);
        }
        if raw.entry.trim().is_empty() {
            return Err(plugin_missing(
                &manifest_path.display().to_string(),
                "manifest has an empty 'entry'",
            ));
        }
        let python = match &raw.python {
            Some(text) => match PythonConstraint::parse(text) {
                Ok(c) => Some(c),
                Err(e) => {
                    let mut diags = Diagnostics::new();
                    diags.add(Diagnostic::error(
                        gamerule::E1602,
                        format!("plugin {}: {e}", raw.id),
                    ));
                    return Err(diags);
                }
            },
            None => None,
        };
        let timeout = if raw.timeout_secs == 0 {
            DEFAULT_PLUGIN_TIMEOUT
        } else {
            Duration::from_secs(raw.timeout_secs)
        };
        Ok(Self {
            id: raw.id,
            version: raw.version,
            runtime: raw.runtime,
            entry: PathBuf::from(raw.entry),
            python,
            timeout,
            dir: dir.to_path_buf(),
        })
    }

    /// The entry script path, resolved and guarded against path escape.
    pub fn entry_path(&self) -> Result<PathBuf, Diagnostics> {
        resolve_entry(&self.dir, &self.entry)
    }
}

/// Guard the entry path: reject absolute paths and `..` escapes so a manifest
/// cannot point the worker at an arbitrary executable.
fn resolve_entry(dir: &Path, entry: &Path) -> Result<PathBuf, Diagnostics> {
    if entry.is_absolute() {
        return Err(plugin_missing(
            &entry.display().to_string(),
            "entry must be relative to the manifest directory",
        ));
    }
    let mut path = dir.to_path_buf();
    for comp in entry.components() {
        match comp {
            Component::Normal(c) => path.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(plugin_missing(
                    &entry.display().to_string(),
                    "entry must not escape the manifest directory",
                ))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(plugin_missing(
                    &entry.display().to_string(),
                    "entry must be a relative path",
                ))
            }
        }
    }
    if !path.is_file() {
        return Err(plugin_missing(
            &path.display().to_string(),
            "entry script not found",
        ));
    }
    Ok(path)
}

/// Inputs a worker run needs, besides the manifest.
pub struct WorkerRun<'a> {
    /// Correlation id echoed on the wire and reported on failure.
    pub request_id: String,
    /// The canonical schema.
    pub schema: &'a Schema,
    /// The normalized document (already validated through L6).
    pub document: &'a Document,
    /// Project-configuration options for this plugin (read-only for it).
    pub options: serde_json::Value,
}

/// The outcome of one worker run.
///
/// `Diagnostics` has no `PartialEq`, so this is compared by pattern matching
/// in tests rather than with `==`.
#[derive(Debug, Clone)]
pub enum WorkerOutcome {
    /// The plugin ran to completion; findings are mapped to L7 diagnostics.
    Ok(Diagnostics),
    /// The plugin did not complete; one of the seven failure classes.
    Failed(WorkerFailure),
}

/// The seven-class failure table of review §四.2.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkerFailure {
    /// 插件缺失: no manifest, or the entry script is not a readable file.
    Missing {
        /// The plugin id that could not be found.
        plugin_id: String,
        /// Human-readable reason (missing manifest, unreadable entry, ...).
        detail: String,
    },
    /// 版本不符 (protocol): the worker refuses the protocol we speak.
    ProtocolVersionMismatch {
        /// The plugin id.
        plugin_id: String,
        /// The protocol version the worker declared.
        request_protocol: u32,
        /// The protocol version this host speaks.
        supported: u32,
    },
    /// Python 版本不符: the interpreter does not satisfy the manifest's
    /// constraint (pre-checked before spawn, with a readable hint).
    PythonVersionMismatch {
        /// The plugin id.
        plugin_id: String,
        /// The constraint from `plugin.toml`, e.g. `>=3.9`.
        expected: String,
        /// The interpreter version actually found, e.g. `3.14.4`.
        found: String,
    },
    /// 插件超时: the worker was terminated after the manifest timeout.
    Timeout {
        /// The plugin id.
        plugin_id: String,
        /// The timeout the worker was subject to.
        timeout: Duration,
    },
    /// 插件崩溃: non-zero exit (or spawn failure) with no usable response.
    Crash {
        /// The plugin id.
        plugin_id: String,
        /// Exit code / spawn error plus a stderr tail.
        detail: String,
    },
    /// 插件输出非法结果: stdout is not a parseable protocol response.
    InvalidOutput {
        /// The plugin id.
        plugin_id: String,
        /// Why the output was not a valid response.
        detail: String,
    },
    /// 插件抛异常: the worker replied with `PLUGIN_ERROR`.
    PluginError {
        /// The plugin id.
        plugin_id: String,
        /// The exception/error text the worker reported.
        detail: String,
    },
}

impl WorkerFailure {
    /// The plugin id this failure concerns.
    pub fn plugin_id(&self) -> &str {
        match self {
            WorkerFailure::Missing { plugin_id, .. }
            | WorkerFailure::ProtocolVersionMismatch { plugin_id, .. }
            | WorkerFailure::PythonVersionMismatch { plugin_id, .. }
            | WorkerFailure::Timeout { plugin_id, .. }
            | WorkerFailure::Crash { plugin_id, .. }
            | WorkerFailure::InvalidOutput { plugin_id, .. }
            | WorkerFailure::PluginError { plugin_id, .. } => plugin_id,
        }
    }

    /// Stable machine name of the failure class (one of the seven). Used by
    /// the [`crate::plugin::attestation`] record and any downstream audit.
    pub fn class(&self) -> &'static str {
        match self {
            WorkerFailure::Missing { .. } => "missing",
            WorkerFailure::ProtocolVersionMismatch { .. } => "protocol_version_mismatch",
            WorkerFailure::PythonVersionMismatch { .. } => "python_version_mismatch",
            WorkerFailure::Timeout { .. } => "timeout",
            WorkerFailure::Crash { .. } => "crash",
            WorkerFailure::InvalidOutput { .. } => "invalid_output",
            WorkerFailure::PluginError { .. } => "plugin_error",
        }
    }

    /// Map the failure onto a Cage diagnostic in the `gamerule` code family.
    pub fn to_diagnostic(&self) -> Diagnostic {
        match self {
            WorkerFailure::Missing { detail, .. } => Diagnostic::error(
                gamerule::E1602,
                format!("validator plugin not found: {detail}"),
            ),
            WorkerFailure::ProtocolVersionMismatch {
                request_protocol,
                supported,
                plugin_id,
            } => Diagnostic::error(
                gamerule::E1603,
                format!(
                    "plugin {plugin_id} spoke protocol {request_protocol}, cage speaks {supported}; \
                     rebuild the plugin against the current protocol version"
                ),
            ),
            WorkerFailure::PythonVersionMismatch {
                plugin_id,
                expected,
                found,
            } => Diagnostic::error(
                gamerule::E1603,
                format!(
                    "plugin {plugin_id} requires python {expected}, but the interpreter is {found}; \
                     install a matching python or adjust the 'python' constraint in plugin.toml"
                ),
            ),
            WorkerFailure::Timeout { plugin_id, timeout } => Diagnostic::error(
                gamerule::E1603,
                format!(
                    "plugin {plugin_id} timed out after {}s and was terminated",
                    timeout.as_secs_f64()
                ),
            ),
            WorkerFailure::Crash { plugin_id, detail } => Diagnostic::error(
                gamerule::E1603,
                format!("plugin {plugin_id} crashed: {detail}"),
            ),
            WorkerFailure::InvalidOutput { plugin_id, detail } => Diagnostic::error(
                gamerule::E1603,
                format!("plugin {plugin_id} produced invalid output: {detail}"),
            ),
            WorkerFailure::PluginError { plugin_id, detail } => Diagnostic::error(
                gamerule::E1603,
                format!("plugin {plugin_id} failed: {detail}"),
            ),
        }
    }
}

/// Run a plugin once: write one request, read one response, map the outcome.
///
/// The worker is spawned as `python3 <entry>` with the parent environment
/// (so a project can make `cage_sdk` importable via `PYTHONPATH` or install).
/// The plugin never runs in-process and never embeds an interpreter.
pub fn run_plugin(root: &Path, manifest: &PluginManifest, run: &WorkerRun) -> WorkerOutcome {
    // Pre-checks: entry presence, then interpreter version. Both before spawn.
    let entry = match manifest.entry_path() {
        Ok(p) => p,
        Err(diags) => {
            return WorkerOutcome::Failed(WorkerFailure::Missing {
                plugin_id: manifest.id.clone(),
                detail: first_message(&diags),
            })
        }
    };
    if let Some(constraint) = &manifest.python {
        match interpreter_version(root) {
            Ok(found) if !constraint.matches(found) => {
                return WorkerOutcome::Failed(WorkerFailure::PythonVersionMismatch {
                    plugin_id: manifest.id.clone(),
                    expected: constraint.display(),
                    found: format_version(found),
                });
            }
            Ok(_) => {}
            Err(detail) => {
                return WorkerOutcome::Failed(WorkerFailure::Crash {
                    plugin_id: manifest.id.clone(),
                    detail,
                });
            }
        }
    }

    let request = build_request(manifest, run);
    let mut child = match std::process::Command::new(PYTHON_BIN)
        .arg(&entry)
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return WorkerOutcome::Failed(WorkerFailure::Crash {
                plugin_id: manifest.id.clone(),
                detail: format!("failed to start python3: {e}"),
            })
        }
    };

    // The request goes out before we wait; a worker that exits early gets a
    // broken pipe and is reported as a crash (it chose to stop reading).
    let request_bytes = request.to_bytes();
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        if let Err(e) = stdin.write_all(&request_bytes) {
            let _ = child.kill();
            return WorkerOutcome::Failed(WorkerFailure::Crash {
                plugin_id: manifest.id.clone(),
                detail: format!("worker closed stdin before the request was written: {e}"),
            });
        }
    } // dropping stdin signals EOF

    let output = collect_with_timeout(child, manifest.timeout);
    let Some(output) = output else {
        return WorkerOutcome::Failed(WorkerFailure::Timeout {
            plugin_id: manifest.id.clone(),
            timeout: manifest.timeout,
        });
    };

    if !output.status.success() && output.stdout.iter().all(u8::is_ascii_whitespace) {
        return WorkerOutcome::Failed(WorkerFailure::Crash {
            plugin_id: manifest.id.clone(),
            detail: format!(
                "exit code {:?}: {}",
                output.status.code(),
                truncate_stderr(&String::from_utf8_lossy(&output.stderr))
            ),
        });
    }

    let response = match serde_json::from_slice::<Response>(trim_stdout(&output.stdout)) {
        Ok(r) => r,
        Err(e) => {
            return WorkerOutcome::Failed(WorkerFailure::InvalidOutput {
                plugin_id: manifest.id.clone(),
                detail: format!(
                    "{e} (stdout: {}, stderr: {})",
                    truncate_stdout(&output.stdout),
                    truncate_stderr(&String::from_utf8_lossy(&output.stderr))
                ),
            })
        }
    };

    match response {
        Response {
            ok: true,
            diagnostics: Some(diags),
            ..
        } => WorkerOutcome::Ok(map_findings(&diags, run.document)),
        Response {
            ok: true,
            diagnostics: None,
            ..
        } => WorkerOutcome::Failed(WorkerFailure::InvalidOutput {
            plugin_id: manifest.id.clone(),
            detail: "ok response without a diagnostics array".to_string(),
        }),
        Response {
            ok: false,
            error: Some(error),
            ..
        } => failure_from_protocol_error(&manifest.id, &error),
        Response {
            ok: false,
            error: None,
            ..
        } => WorkerOutcome::Failed(WorkerFailure::InvalidOutput {
            plugin_id: manifest.id.clone(),
            detail: "error response without an error object".to_string(),
        }),
    }
}

/// Wait for the child up to `timeout`, killing it on the deadline.
fn collect_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Option<std::process::Output> {
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<std::process::Output>>();
    let (abort_tx, abort_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                // `wait_with_output` drains the pipes while the process is still
                // running, so a plugin emitting more than a pipe buffer of
                // diagnostics cannot deadlock on a full pipe and be misreported
                // as a timeout.
                Ok(Some(_status)) => {
                    let _ = tx.send(child.wait_with_output());
                    return;
                }
                Ok(None) => {
                    if abort_rx.try_recv().is_ok() || Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = tx.send(Err(std::io::Error::other("timed out")));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        }
    });

    match rx.recv_timeout(Duration::from_secs(2).max(timeout)) {
        Ok(Ok(output)) => Some(output),
        // A child error and a disconnected channel both mean "no output".
        Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let _ = abort_tx.send(());
            None
        }
    }
}

/// Map a protocol error onto the failure table.
fn failure_from_protocol_error(plugin_id: &str, error: &ProtocolError) -> WorkerOutcome {
    use cage_plugin_protocol as proto;
    match error.code.as_str() {
        proto::ERR_PROTOCOL_VERSION_MISMATCH => {
            // The worker tells us which version it speaks; surface both.
            let request_protocol = extract_protocol(&error.message);
            WorkerOutcome::Failed(WorkerFailure::ProtocolVersionMismatch {
                plugin_id: plugin_id.to_string(),
                request_protocol: request_protocol.unwrap_or(PROTOCOL_VERSION),
                supported: PROTOCOL_VERSION,
            })
        }
        proto::ERR_PLUGIN_ERROR => WorkerOutcome::Failed(WorkerFailure::PluginError {
            plugin_id: plugin_id.to_string(),
            detail: error.message.clone(),
        }),
        proto::ERR_PLUGIN_INVALID_RESULT => WorkerOutcome::Failed(WorkerFailure::InvalidOutput {
            plugin_id: plugin_id.to_string(),
            detail: error.message.clone(),
        }),
        _ => WorkerOutcome::Failed(WorkerFailure::InvalidOutput {
            plugin_id: plugin_id.to_string(),
            detail: format!("{}: {}", error.code, error.message),
        }),
    }
}

/// Best-effort extraction of the protocol number from a version-mismatch
/// message (the worker includes the requested version).
fn extract_protocol(message: &str) -> Option<u32> {
    message
        .split_whitespace()
        .find_map(|tok| tok.trim_end_matches(',').parse::<u32>().ok())
}

/// Build the wire `run` request from the schema, document, and options.
fn build_request(manifest: &PluginManifest, run: &WorkerRun) -> Request {
    use cage_plugin_protocol::{PluginInfo, RunParams};
    let tables: Vec<TablePayload> = run
        .document
        .tables
        .iter()
        .map(|(name, table)| TablePayload {
            name: name.clone(),
            primary_key: table.primary_key_fields.clone(),
            rows: table
                .rows
                .iter()
                .map(|row| RowPayload {
                    index: row.index as u64,
                    location: Some(row.location.to_string()),
                    fields: row
                        .fields
                        .iter()
                        .map(|(k, tv)| (k.clone(), tv.value.to_json()))
                        .collect(),
                })
                .collect(),
        })
        .collect();
    let params = RunParams {
        plugin: PluginInfo {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
        },
        schema: serde_json::to_value(run.schema).unwrap_or(serde_json::Value::Null),
        tables,
        options: run.options.clone(),
    };
    Request::run(run.request_id.clone(), params)
}

/// Convert plugin findings into L7 diagnostics, resolving table/row/field
/// back to a concrete source location.
fn map_findings(diags: &[WireDiagnostic], document: &Document) -> Diagnostics {
    let mut out = Diagnostics::new();
    for wire in diags {
        let mut builder = match wire.severity {
            cage_plugin_protocol::Severity::Error => {
                DiagnosticBuilder::error(wire.code.clone(), wire.message.clone())
            }
            cage_plugin_protocol::Severity::Warning => {
                DiagnosticBuilder::warning(wire.code.clone(), wire.message.clone())
            }
            cage_plugin_protocol::Severity::Info => {
                DiagnosticBuilder::info(wire.code.clone(), wire.message.clone())
            }
        };
        if let Some(table) = &wire.table {
            builder = builder.table(table.clone());
        }
        if let Some(field) = &wire.field {
            builder = builder.field(field.clone());
        }
        if let Some(row_index) = wire.row {
            builder = builder.row(row_index.to_string());
        }
        if let Some(hint) = &wire.hint {
            builder = builder.hint(hint.clone());
        }
        // Resolve the concrete source location from the document when the
        // finding names a table and row.
        if let (Some(table_name), Some(row_index)) = (&wire.table, wire.row) {
            if let Some(table) = document.tables.get(table_name) {
                if let Some(row) = table.rows.get(row_index as usize) {
                    let mut loc = row.location.clone();
                    if let Some(field) = &wire.field {
                        loc.field = Some(field.clone());
                        loc.column = None;
                    }
                    builder = builder.location(loc);
                    builder = builder.source(table.source_file.clone());
                }
            }
        }
        out.add(builder.build());
    }
    out
}

/// Read the interpreter version as `(major, minor, patch)`.
fn interpreter_version(root: &Path) -> Result<(u32, u32, u32), String> {
    let output = std::process::Command::new(PYTHON_BIN)
        .current_dir(root)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {PYTHON_BIN} --version: {e}"))?;
    if !output.status.success() {
        return Err(format!("{PYTHON_BIN} --version failed"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    // "Python 3.14.4" (stdout) or "Python 3.14.4" (stderr on some builds).
    let text = if text.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    } else {
        text.to_string()
    };
    parse_python_version(&text)
}

fn parse_python_version(text: &str) -> Result<(u32, u32, u32), String> {
    let text = text.trim();
    let rest = text.strip_prefix("Python ").unwrap_or(text);
    let version = rest.split_whitespace().next().unwrap_or("");
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 {
        return Err(format!("unrecognized interpreter version '{text}'"));
    }
    let major = parts[0]
        .parse()
        .map_err(|_| format!("unrecognized interpreter version '{text}'"))?;
    let minor = parts[1]
        .parse()
        .map_err(|_| format!("unrecognized interpreter version '{text}'"))?;
    let patch = parts
        .get(2)
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    Ok((major, minor, patch))
}

fn format_version(v: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

fn plugin_missing(target: &str, detail: &str) -> Diagnostics {
    let mut diags = Diagnostics::new();
    diags.add(Diagnostic::error(
        gamerule::E1602,
        format!("validator plugin not found: {target}: {detail}"),
    ));
    diags
}

fn first_message(diags: &Diagnostics) -> String {
    diags
        .errors()
        .first()
        .map_or_else(|| "unknown".to_string(), |d| d.message.clone())
}

/// Strip a single trailing newline the worker appended to its stdout.
fn trim_stdout(stdout: &[u8]) -> &[u8] {
    let mut end = stdout.len();
    while end > 0 && stdout[end - 1] == b'\n' {
        end -= 1;
    }
    &stdout[..end]
}

/// Truncate stdout for a diagnostic to a bounded, printable snippet.
fn truncate_stdout(stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(stdout);
    truncate_chars(&text, 200)
}

/// Truncate stderr to [`MAX_PLUGIN_STDERR_BYTES`] bytes on a char boundary.
fn truncate_stderr(stderr: &str) -> String {
    if stderr.len() <= MAX_PLUGIN_STDERR_BYTES {
        return stderr.trim().to_string();
    }
    let mut end = MAX_PLUGIN_STDERR_BYTES;
    while !stderr.is_char_boundary(end) {
        end -= 1;
    }
    stderr[..end].to_string()
}

/// Truncate to `chars` characters (for the stdout snippet).
fn truncate_chars(text: &str, chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(chars).collect()
}

#[cfg(test)]
mod tests;
