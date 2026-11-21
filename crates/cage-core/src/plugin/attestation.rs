//! [`ValidationAttestation`] (review P2-4): the auditable record of what a
//! validation ran on, under what environment, and with which plugins.
//!
//! The failure table of review §四.2 demands that a dependency or parameter
//! change that alters a validation result stays traceable — "same config,
//! different plugin environment, different results" must be attributable
//! after the fact. The existing build `Manifest` keeps its structure; this
//! record is produced beside it and can be attached to artifacts via
//! [`ValidationAttestation::digest`].
//!
//! Everything here is deterministic: the same schema, data, plugins, options
//! and environment produce the same record and the same digest. No
//! timestamps — reproducibility is the point; build identity already lives
//! in the build `Manifest`.

use super::interpreter_version;
use cage_plugin_protocol::PROTOCOL_VERSION;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The cage-core version that produced this record (part of the fingerprint).
pub const CAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Outcome of one plugin run, as attested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttestationOutcome {
    /// The plugin ran to completion; findings counted by severity.
    Ran {
        /// Number of error-severity findings the plugin reported.
        errors: usize,
        /// Number of warning-severity findings the plugin reported.
        warnings: usize,
        /// Number of info-severity findings the plugin reported.
        infos: usize,
    },
    /// The plugin did not complete (one of the seven failure classes).
    Failed {
        /// Stable class name from [`super::WorkerFailure::class`].
        class: String,
        /// Human-readable failure detail.
        detail: String,
    },
}

/// One plugin's attested execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginAttestation {
    /// The plugin id from its manifest (or directory name when load failed).
    pub id: String,
    /// The plugin version declared by its author (empty when load failed).
    pub version: String,
    /// blake3 over the canonical options JSON the worker received — the
    /// configuration digest (插件参数变化 → 验证结果记录变化).
    pub options_digest: String,
    /// blake3 over every file in the plugin directory, sorted by relative
    /// path — the dependency lock (依赖锁定): a changed dependency, vendored
    /// code or `requirements.txt` changes this digest.
    pub content_digest: String,
    /// Whether the plugin completed, and what it reported.
    pub outcome: AttestationOutcome,
}

/// The validation environment fingerprint (验证环境指纹).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationEnvironment {
    /// cage-core version that ran the validation.
    pub cage_version: String,
    /// Plugin protocol version spoken by this host.
    pub protocol_version: u32,
    /// Python interpreter version workers ran under (`None` when no worker
    /// plugin was involved, or the interpreter could not be probed).
    pub python: Option<String>,
    /// OS the validation ran on.
    pub os: String,
}

impl ValidationEnvironment {
    /// Probe the fingerprint for a run with worker plugins; `root` is the
    /// project root the workers spawn in.
    pub fn probe(root: &Path) -> Self {
        Self {
            cage_version: CAGE_VERSION.to_string(),
            protocol_version: PROTOCOL_VERSION,
            python: interpreter_version(root)
                .ok()
                .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}")),
            os: std::env::consts::OS.to_string(),
        }
    }

    /// The fingerprint for a run without worker plugins (no interpreter probe).
    pub fn without_python() -> Self {
        Self {
            cage_version: CAGE_VERSION.to_string(),
            protocol_version: PROTOCOL_VERSION,
            python: None,
            os: std::env::consts::OS.to_string(),
        }
    }
}

/// The whole-run record: what was validated, under what environment, with
/// which plugins, and what they reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationAttestation {
    /// blake3 over the canonical schema JSON.
    pub schema_digest: String,
    /// blake3 over the canonical document (data) JSON.
    pub document_digest: String,
    /// The environment fingerprint.
    pub environment: ValidationEnvironment,
    /// One entry per plugin that was (or tried to be) executed.
    pub plugins: Vec<PluginAttestation>,
    /// True only when every plugin completed. A failing run is still
    /// attested — the record is about *what ran*, not a pass stamp.
    pub passed: bool,
}

impl ValidationAttestation {
    /// Assemble a run record from its parts; `passed` is derived.
    pub fn new(
        schema_digest: String,
        document_digest: String,
        environment: ValidationEnvironment,
        plugins: Vec<PluginAttestation>,
    ) -> Self {
        let passed = plugins
            .iter()
            .all(|p| matches!(p.outcome, AttestationOutcome::Ran { .. }));
        Self {
            schema_digest,
            document_digest,
            environment,
            plugins,
            passed,
        }
    }

    /// The audit digest: blake3 over the canonical serialization. Two runs
    /// over the same inputs produce the same digest; any change to the
    /// schema, data, plugin code, options or environment changes it.
    pub fn digest(&self) -> String {
        blake3::hash(&serde_json::to_vec(self).expect("attestation is always serializable"))
            .to_hex()
            .to_string()
    }
}

/// blake3 over the canonical JSON of a value (field order fixed by the type).
pub fn value_digest(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).expect("options are always serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

/// blake3 over every file in `dir`, fed as (relative path, NUL, bytes) with
/// paths sorted — a content/dependency lock for a plugin directory.
pub fn dir_digest(dir: &Path) -> String {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_files(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (name, bytes) in &files {
        hasher.update(name.as_bytes());
        hasher.update(&[0]);
        hasher.update(bytes);
    }
    hasher.finalize().to_hex().to_string()
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.push((rel.display().to_string(), bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_digest_is_deterministic_and_sensitive() {
        let a = serde_json::json!({"cap": 10, "mode": "strict"});
        let b = serde_json::json!({"cap": 10, "mode": "strict"});
        let c = serde_json::json!({"cap": 20, "mode": "strict"});
        assert_eq!(value_digest(&a), value_digest(&b));
        assert_ne!(value_digest(&a), value_digest(&c));
        assert_eq!(value_digest(&a).len(), 64); // blake3 hex
    }

    #[test]
    fn dir_digest_is_deterministic_and_tracks_file_changes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("plugin.toml"), "id = \"d\"\n").expect("write");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src").join("main.py"), "print(1)\n").expect("write");

        let first = dir_digest(tmp.path());
        // Same content, same digest.
        assert_eq!(dir_digest(tmp.path()), first);
        // A changed dependency file changes the lock.
        std::fs::write(tmp.path().join("src").join("main.py"), "print(2)\n").expect("write");
        assert_ne!(dir_digest(tmp.path()), first);
    }

    #[test]
    fn dir_digest_covers_subdirectories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let nested = tmp.path().join("src").join("deep");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(tmp.path().join("plugin.toml"), "id = \"d\"\n").expect("write");
        std::fs::write(nested.join("main.py"), "print(1)\n").expect("write");
        let before = dir_digest(tmp.path());
        std::fs::write(nested.join("main.py"), "print(2)\n").expect("write");
        // A deeply nested dependency change changes the lock.
        assert_ne!(dir_digest(tmp.path()), before);
    }

    #[test]
    fn environment_without_python_is_protocol_and_os() {
        let env = ValidationEnvironment::without_python();
        assert_eq!(env.protocol_version, PROTOCOL_VERSION);
        assert_eq!(env.cage_version, CAGE_VERSION);
        assert!(env.python.is_none());
        assert_eq!(env.os, std::env::consts::OS);
    }

    #[test]
    fn attestation_derives_passed_and_digests() {
        let env = ValidationEnvironment::without_python();
        let ok = PluginAttestation {
            id: "p".to_string(),
            version: "1.0.0".to_string(),
            options_digest: value_digest(&serde_json::json!({})),
            content_digest: "c".to_string(),
            outcome: AttestationOutcome::Ran {
                errors: 1,
                warnings: 0,
                infos: 0,
            },
        };
        let failed = PluginAttestation {
            id: "q".to_string(),
            version: String::new(),
            options_digest: "o".to_string(),
            content_digest: "c".to_string(),
            outcome: AttestationOutcome::Failed {
                class: "missing".to_string(),
                detail: "no plugin.toml".to_string(),
            },
        };
        let pass = ValidationAttestation::new(
            "s".to_string(),
            "d".to_string(),
            env.clone(),
            vec![ok.clone()],
        );
        assert!(pass.passed);
        let fail = ValidationAttestation::new(
            "s".to_string(),
            "d".to_string(),
            env.clone(),
            vec![ok, failed],
        );
        assert!(!fail.passed);
        // Deterministic digest over the same record.
        assert_eq!(pass.digest(), pass.digest());
        assert_ne!(pass.digest(), fail.digest());
        // The record serializes (audit export) and round-trips.
        let json = serde_json::to_string(&pass).expect("serialize");
        let back: ValidationAttestation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, pass);
    }

    #[test]
    fn outcome_serializes_as_tagged_json() {
        let outcome = AttestationOutcome::Ran {
            errors: 2,
            warnings: 1,
            infos: 0,
        };
        let json = serde_json::to_value(&outcome).expect("serialize");
        assert_eq!(json["kind"], "ran");
        assert_eq!(json["errors"], 2);
        let failed = AttestationOutcome::Failed {
            class: "timeout".to_string(),
            detail: "30s".to_string(),
        };
        let failed_json = serde_json::to_value(&failed).expect("serialize");
        assert_eq!(failed_json["kind"], "failed");
        assert_eq!(failed_json["class"], "timeout");
    }
}
