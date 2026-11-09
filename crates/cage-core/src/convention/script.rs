use super::{has_sub_columns, is_a1};
use super::{ColumnMeta, ExcelConvention, TableMeta, CONTRACT_VERSION};
use crate::diagnostics::{Diagnostic, Diagnostics};
use crate::error::codes::source_convention::{E2202, E2203, E2204, E2205, E2206, E2207};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The source fingerprint input for a script adapter (design §51 Q2):
/// blake3 of the script's bytes + resolved path + canonical `script_config`
/// + contract version. Editing the script or its config changes the string,
///   so `source_hash` / `build_id` rotate even when the parsed model is byte-identical.
pub fn script_fingerprint(
    root: &Path,
    convention: &ExcelConvention,
) -> Result<Option<String>, Diagnostics> {
    let Some(rel) = convention.script.as_deref() else {
        return Ok(None);
    };
    let script = resolve_script(root, rel)?;
    let bytes = std::fs::read(&script).map_err(|e| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            crate::error::codes::source_convention::E2203,
            format!("failed to read script {}: {e}", script.display()),
        ));
        diags
    })?;
    let script_hash = blake3::hash(&bytes).to_hex().to_string();
    // Canonical `script_config`: serialize with object keys sorted so that
    // semantically equal configs (differing only in key order) hash the
    // same, matching the "canonical form" determinism contract.
    let config = canonical_json(&convention.script_config);
    Ok(Some(format!(
        "{CONTRACT_VERSION}|{script_hash}|{}|{config}",
        script.to_string_lossy()
    )))
}

/// Serialize a JSON value with every object's keys sorted lexicographically
/// (recursively), so the bytes are order-independent — the "canonical form"
/// the deterministic-build contract requires. Arrays keep their order (order
/// is meaningful there); only object key order is normalized.
pub(crate) fn canonical_json(value: &JsonValue) -> String {
    fn sort(value: &JsonValue) -> JsonValue {
        match value {
            JsonValue::Object(map) => {
                let mut entries: Vec<(String, JsonValue)> =
                    map.iter().map(|(k, v)| (k.clone(), sort(v))).collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                JsonValue::Object(entries.into_iter().collect())
            }
            JsonValue::Array(items) => JsonValue::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(value)).unwrap_or_else(|_| value.to_string())
}

/// Resolve a script path declared in cage.toml against the project root
/// (design §51 Q2: repo-relative explicit path, no `$PATH` lookup, no `..`
/// escape, symlinks may not leave the root).
pub fn resolve_script(root: &Path, rel: &str) -> Result<PathBuf, Diagnostics> {
    let fail = |message: String| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(E2203, message));
        Err(diags)
    };
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return fail(format!(
            "{E2203} script must be relative to the project root, got {rel:?}"
        ));
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return fail(format!(
            "{E2203} script path must not contain '.' or '..' segments, got {rel:?}"
        ));
    }
    let joined = root.join(rel_path);
    let (Ok(root_canon), Ok(joined_canon)) = (root.canonicalize(), joined.canonicalize()) else {
        return fail(format!(
            "{E2203} script not found: {} (relative to the project root)",
            joined.display()
        ));
    };
    if !joined_canon.starts_with(&root_canon) {
        return fail(format!(
            "{E2203} script escapes the project root through a symlink: {rel:?}"
        ));
    }
    if !joined_canon.is_file() {
        return fail(format!(
            "{E2203} script is not a file: {}",
            joined.display()
        ));
    }
    // Return the canonical path: `root` may itself be a relative path (e.g.
    // `.` from the CLI), so the raw `root.join(rel)` spelling would make the
    // same project hash differently depending on how the root was spelled.
    // The canonical path is the same bytes for every spelling of the same
    // project, preserving the "same input → same build_id" contract.
    Ok(joined_canon)
}

/// Run the script adapter once: the request JSON on stdin, the response on
/// stdout (design §51 Q2). E2203 will not start, E2204 timeout, E2205
/// non-zero exit, E2206 unusable output. The script runs with the author's
/// own privileges — cage sandboxes nothing and claims nothing.
pub fn run_script(
    root: &Path,
    rel: &str,
    args: &[String],
    timeout: Duration,
    request: &JsonValue,
) -> Result<JsonValue, Diagnostics> {
    let script = resolve_script(root, rel)?;
    let mut child = std::process::Command::new(&script)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                E2203,
                format!("failed to start script {}: {e}", script.display()),
            ));
            diags
        })?;

    // The request goes out before we wait: a script that exits early gets a
    // broken pipe, reported as E2205 (it chose to stop reading).
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(request.to_string().as_bytes()) {
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                E2205,
                format!(
                    "script {} closed stdin before the request was written: {e}",
                    script.display()
                ),
            ));
            let _ = child.kill();
            return Err(diags);
        }
    } // dropping stdin signals EOF

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    let (abort_tx, abort_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut child = child;
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    let _ = tx.send(child.wait_with_output());
                    return;
                }
                Ok(None) => {
                    if abort_rx.try_recv().is_ok() {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = tx.send(Err(std::io::Error::other("timed out")));
                        return;
                    }
                    if Instant::now() >= deadline {
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

    let output = match rx.recv_timeout(Duration::from_secs(2).max(timeout)) {
        Ok(Ok(output)) => Some(output),
        Ok(Err(_timed_out)) => None,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = abort_tx.send(());
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    };
    let Some(output) = output else {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2204,
            format!(
                "script {} timed out after {}s without producing a response",
                script.display(),
                timeout.as_secs_f64()
            ),
        ));
        return Err(diags);
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        // Truncate to 400 bytes at a char boundary: `&str[..400]` would panic
        // when byte 400 falls inside a multi-byte character (e.g. a script
        // emitting >400 bytes of CJK stderr), turning a diagnostics path into
        // a crash.
        let stderr = if stderr.len() > 400 {
            let mut end = 400;
            while !stderr.is_char_boundary(end) {
                end -= 1;
            }
            &stderr[..end]
        } else {
            stderr
        };
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2205,
            format!(
                "script {} exited with {}: {stderr}",
                script.display(),
                output.status
            ),
        ));
        return Err(diags);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let value: JsonValue = serde_json::from_str(text.trim()).map_err(|e| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2206,
            format!(
                "script {} did not produce valid JSON: {e}",
                script.display()
            ),
        ));
        diags
    })?;
    if value.get("contract").and_then(|c| c.as_str()) != Some(CONTRACT_VERSION) {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2206,
            format!(
                "script {} answered contract {:?}, cage speaks {CONTRACT_VERSION:?}",
                script.display(),
                value.get("contract").cloned().unwrap_or(JsonValue::Null)
            ),
        ));
        return Err(diags);
    }
    Ok(value)
}

/// Parse and gate the script's `columns` against the data header row. The
/// vocabulary is enforced by deserialization (`deny_unknown_fields` on
/// [`ColumnMeta`]); a `type` cage cannot parse is E2202, any other shape
/// violation is E2206, a column name outside the header row or a wrong
/// `table` is E2207 (design §51 Q3).
pub fn parse_response(
    value: &JsonValue,
    expected_table: &str,
    header_names: &[String],
    source_file: &str,
    sheet: &str,
) -> Result<TableMeta, Diagnostics> {
    let fail = |code: &str, message: String| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(code, message));
        Err(diags)
    };
    let Some(obj) = value.as_object() else {
        return fail(E2206, format!("{E2206} response is not an object"));
    };
    let table = match obj.get("table").and_then(serde_json::Value::as_str) {
        Some(t) => t.to_string(),
        None => return fail(E2206, format!("{E2206} response carries no table name")),
    };
    if table != expected_table {
        return fail(
            E2207,
            format!(
                "{E2207} script answered table {table:?}, this sheet resolves to \
                 {expected_table:?}"
            ),
        );
    }
    let header_row = match obj.get("header_row").and_then(serde_json::Value::as_u64) {
        Some(h) => h as usize,
        None => {
            return fail(
                E2206,
                format!("{E2206} response carries no usable header_row"),
            )
        }
    };
    let Some(columns_json) = obj.get("columns").and_then(serde_json::Value::as_object) else {
        return fail(E2206, format!("{E2206} response carries no columns object"));
    };
    if columns_json.is_empty() {
        return fail(E2206, format!("{E2206} response declares no columns"));
    }

    let mut columns = IndexMap::new();
    for (name, col_json) in columns_json {
        let meta: ColumnMeta = serde_json::from_value(col_json.clone()).map_err(|e| {
            let message = e.to_string();
            let is_vocabulary =
                message.contains("unknown variant") || message.contains("unknown field");
            let code = if is_vocabulary { E2202 } else { E2206 };
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                code,
                format!("{code} column {name:?}: {e} (source {source_file}, sheet {sheet})"),
            ));
            diags
        })?;
        if meta.cell.is_empty() || !is_a1(&meta.cell) {
            return fail(
                E2206,
                format!(
                    "{E2206} column {name:?} carries cell anchor {:?}, expected an A1 \
                     address like \"B2\" or \"G27\"",
                    meta.cell
                ),
            );
        }
        if has_sub_columns(&meta) {
            return fail(
                E2206,
                format!(
                    "{E2206} column {name:?} declares sub_columns; script-declared nesting \
                     is a later slice (design §51 Q6)"
                ),
            );
        }
        if meta.multi_row {
            return fail(
                E2206,
                format!(
                    "{E2206} column {name:?} declares multi_row; script-declared multi-row \
                     records are a later slice (design §51 Q6)"
                ),
            );
        }
        if !header_names.iter().any(|h| h == name) {
            return fail(
                E2207,
                format!("{E2207} script declares column {name:?}, which is not in the header row"),
            );
        }
        columns.insert(name.clone(), meta);
    }
    Ok(TableMeta {
        header_row,
        columns,
    })
}
