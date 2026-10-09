//! Configuration Snapshot（八个概念 #7）: self-verifying build artifact.
//!
//! A snapshot packages ONE profile view of a build — the build manifest, the
//! canonical (profile-projected) schema, the generated artifacts partitioned
//! into data/ (json/csv/msgpack) and generated/ (code targets), and a per-file
//! hash ledger — into a directory a server can load without the build machine:
//!
//! ```text
//! snapshot/<profile>-<build_id[..12]>/
//! ├── manifest.json   # the build manifest (authoritative 账本)
//! ├── schema.json     # canonical schema of the profile view
//! ├── data/…          # data artifacts (json/csv/msgpack)
//! ├── generated/…     # code artifacts
//! └── HASHES.json     # per-file blake3 ledger + build_id / content_hash
//! ```
//!
//! The directory name derives from `profile` + `build_id` — a deterministic
//! fingerprint, deliberately NOT wall-clock (the review's snapshot sketch
//! suggested a date stamp; that would break the determinism contract: same
//! inputs → byte-identical snapshot). Re-running the same build overwrites
//! the same directory with byte-identical content.

use crate::manifest::BuildManifest;
use crate::schema::Schema;
use indexmap::IndexMap;
use serde_json::Value;
use std::path::Path;

const LEDGER_FILE: &str = "HASHES.json";

/// Snapshot-relative path of one artifact record: `output_dir` (the
/// project-relative build root) is stripped, so the snapshot carries only
/// the profile view — build's "build/client/Item.json" → "data/client/Item.json".
fn snapshot_artifact_path(rel: &str, format: &str, output_dir: &str) -> String {
    let kind = if matches!(format, "json" | "csv" | "msgpack") {
        "data"
    } else {
        "generated"
    };
    let stripped = rel.strip_prefix(output_dir).unwrap_or(rel);
    let stripped = stripped.strip_prefix('/').unwrap_or(stripped);
    format!("{kind}/{stripped}")
}

/// Pack one profile build into the snapshot file map (deterministic order).
/// The ledger covers every packed file; `HASHES.json` itself is the trust
/// root and is never self-hashed.
pub fn snapshot_files(
    manifest_json: &[u8],
    schema_json: &[u8],
    artifacts: &[(String, Vec<u8>, String, Option<String>)],
    output_dir: &str,
    build_id: &str,
    content_hash: &str,
) -> IndexMap<String, Vec<u8>> {
    let mut files = IndexMap::new();
    files.insert("manifest.json".to_string(), manifest_json.to_vec());
    files.insert("schema.json".to_string(), schema_json.to_vec());
    for (rel, bytes, format, _) in artifacts {
        files.insert(
            snapshot_artifact_path(rel, format, output_dir),
            bytes.clone(),
        );
    }

    let mut ledger = serde_json::Map::new();
    ledger.insert("build_id".to_string(), Value::String(build_id.to_string()));
    ledger.insert(
        "content_hash".to_string(),
        Value::String(content_hash.to_string()),
    );
    let mut file_hashes = serde_json::Map::new();
    for (path, bytes) in &files {
        file_hashes.insert(
            path.clone(),
            Value::String(blake3::hash(bytes).to_hex().to_string()),
        );
    }
    ledger.insert("files".to_string(), Value::Object(file_hashes));
    files.insert(
        LEDGER_FILE.to_string(),
        serde_json::to_vec_pretty(&Value::Object(ledger)).expect("ledger serializes"),
    );
    files
}

/// Result of a load-time snapshot verification.
pub struct SnapshotReport {
    /// Whether every ledger entry re-hashed to the recorded value and no
    /// unexpected file appeared
    pub ok: bool,
    /// Number of files actually re-hashed
    pub files_checked: usize,
    /// Per-file problems ("path: reason"), empty when `ok`
    pub mismatches: Vec<String>,
}

/// Load-time verification entry（服务器启动加载校验）: re-hashes every file
/// the ledger lists, flags tampered / missing / unreadable ones, and flags
/// unexpected files (snapshot growth or tampering).
pub fn verify_snapshot(dir: &Path) -> Result<SnapshotReport, String> {
    let ledger_raw = std::fs::read(dir.join(LEDGER_FILE))
        .map_err(|e| format!("cannot read {}/{}: {e}", dir.display(), LEDGER_FILE))?;
    let ledger: Value =
        serde_json::from_slice(&ledger_raw).map_err(|e| format!("{LEDGER_FILE}: parse: {e}"))?;
    let expected = ledger
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{LEDGER_FILE}: missing 'files' object"))?;

    let mut mismatches = Vec::new();
    let mut checked = 0usize;
    for (rel, want) in expected {
        let want = want
            .as_str()
            .ok_or_else(|| format!("{LEDGER_FILE}: non-string hash for '{rel}'"))?;
        match std::fs::read(dir.join(rel)) {
            Err(e) => mismatches.push(format!("{rel}: unreadable ({e})")),
            Ok(bytes) => {
                checked += 1;
                let got = blake3::hash(&bytes).to_hex().to_string();
                if got != want {
                    mismatches.push(format!(
                        "{rel}: hash mismatch (recorded {want}, actual {got})"
                    ));
                }
            }
        }
    }

    for extra in files_on_disk(dir)? {
        if !expected.contains_key(&extra) {
            mismatches.push(format!("{extra}: not in snapshot ledger"));
        }
    }

    Ok(SnapshotReport {
        ok: mismatches.is_empty(),
        files_checked: checked,
        mismatches,
    })
}

/// What the server needs after verification: the build manifest, the
/// canonical profile-view schema and every packed artifact
/// (data/ + generated/ by snapshot-relative path).
pub type SnapshotData = (BuildManifest, Schema, IndexMap<String, Vec<u8>>);

/// Verify + load — the server-side entry point.
pub fn load(dir: &Path) -> Result<SnapshotData, String> {
    let report = verify_snapshot(dir)?;
    if !report.ok {
        return Err(format!(
            "snapshot verification failed ({} problem(s)): {}",
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    let manifest: BuildManifest = serde_json::from_slice(
        &std::fs::read(dir.join("manifest.json")).map_err(|e| format!("manifest read: {e}"))?,
    )
    .map_err(|e| format!("manifest parse: {e}"))?;
    let schema: Schema = serde_json::from_slice(
        &std::fs::read(dir.join("schema.json")).map_err(|e| format!("schema read: {e}"))?,
    )
    .map_err(|e| format!("schema parse: {e}"))?;
    let mut artifacts = IndexMap::new();
    for rel in files_on_disk(dir)? {
        if rel.starts_with("data/") || rel.starts_with("generated/") {
            let bytes = std::fs::read(dir.join(&rel)).map_err(|e| format!("{rel} read: {e}"))?;
            artifacts.insert(rel, bytes);
        }
    }
    Ok((manifest, schema, artifacts))
}

/// Sorted snapshot-relative paths of every file on disk under `dir`
/// (excluding the ledger itself — it is the trust root).
fn files_on_disk(dir: &Path) -> Result<Vec<String>, String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("read_dir: {e}"))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .map_err(|_| "path escape".to_string())?
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                walk(&path, base, out)?;
            } else if rel != LEDGER_FILE {
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> IndexMap<String, Vec<u8>> {
        let schema_json = r#"{"tables":{},"enums":{}}"#;
        snapshot_files(
            br#"{"project":"demo"}"#,
            schema_json.as_bytes(),
            &[
                (
                    "build/client/Item.json".to_string(),
                    br#"{"id":1}"#.to_vec(),
                    "json".to_string(),
                    Some("Item".to_string()),
                ),
                (
                    "build/client/Item.msgpack".to_string(),
                    [0x91_u8, 0x01].to_vec(),
                    "msgpack".to_string(),
                    Some("Item".to_string()),
                ),
                (
                    "build/client/client.json".to_string(),
                    br"struct Item{}".to_vec(),
                    "cs".to_string(),
                    None,
                ),
            ],
            "build",
            "b1d_012",
            "c_h_0123",
        )
    }

    #[test]
    fn snapshot_files_partition_and_ledger_are_deterministic() {
        let a = pack();
        let b = pack();
        assert_eq!(a, b, "same inputs → same snapshot file map");

        // Partitioning: json/msgpack → data/, code → generated/, output_dir stripped.
        assert!(a.contains_key("data/client/Item.json"));
        assert!(a.contains_key("data/client/Item.msgpack"));
        assert!(a.contains_key("generated/client/client.json"));
        assert!(a.contains_key("manifest.json") && a.contains_key("schema.json"));

        // Ledger covers every packed file with the correct blake3.
        let ledger: Value = serde_json::from_slice(&a["HASHES.json"]).unwrap();
        assert_eq!(ledger["build_id"], "b1d_012");
        assert_eq!(ledger["content_hash"], "c_h_0123");
        assert_eq!(ledger["files"].as_object().unwrap().len(), a.len() - 1);
        for (path, bytes) in &a {
            if path == LEDGER_FILE {
                continue;
            }
            assert_eq!(
                ledger["files"][path.as_str()],
                Value::String(blake3::hash(bytes).to_hex().to_string()),
                "ledger hash for {path}"
            );
        }
    }

    #[test]
    fn verify_accepts_clean_and_flags_tamper_growth_and_loss() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for (rel, bytes) in pack() {
            let abs = dir.join(&rel);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, bytes).unwrap();
        }

        let clean = verify_snapshot(dir).unwrap();
        assert!(clean.ok);
        assert_eq!(clean.files_checked, 5); // manifest + schema + 3 artifacts

        // Tamper with an artifact.
        std::fs::write(dir.join("data/client/Item.json"), b"{\"id\":9}").unwrap();
        let tampered = verify_snapshot(dir).unwrap();
        assert!(!tampered.ok);
        assert!(
            tampered
                .mismatches
                .iter()
                .any(|m| m.contains("data/client/Item.json")),
            "{:?}",
            tampered.mismatches
        );

        // Grown snapshot (unexpected file) is flagged too.
        std::fs::write(dir.join("data/client/rogue.json"), b"x").unwrap();
        let grown = verify_snapshot(dir).unwrap();
        assert!(!grown.ok);
        assert!(
            grown.mismatches.iter().any(|m| m.contains("rogue.json")),
            "{:?}",
            grown.mismatches
        );

        // Missing ledger → error.
        std::fs::remove_file(dir.join(LEDGER_FILE)).unwrap();
        assert!(verify_snapshot(dir).is_err());
    }

    #[test]
    fn load_round_trips_manifest_schema_and_artifacts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut files = pack();
        // A realistic manifest for deserialization.
        files.insert(
            "manifest.json".to_string(),
            serde_json::to_vec(&crate::manifest::BuildManifest {
                project: "demo".to_string(),
                profile: "client".to_string(),
                cage_version: "0.1.0".to_string(),
                generator_version: "1.0.0".to_string(),
                build_id: "b1d_012".to_string(),
                schema_hash: "abc".to_string(),
                source_hash: "def".to_string(),
                content_hash: "c_h_0123".to_string(),
                dependencies: IndexMap::new(),
                table_hashes: IndexMap::new(),
                targets: Vec::new(),
                artifacts: IndexMap::new(),
            })
            .unwrap(),
        );
        // Re-ledger after swapping the manifest (blake3 changes).
        let ledger_json = {
            let mut ledger = serde_json::Map::new();
            ledger.insert("build_id".to_string(), Value::String("b1d_012".to_string()));
            ledger.insert(
                "content_hash".to_string(),
                Value::String("c_h_0123".to_string()),
            );
            let mut file_hashes = serde_json::Map::new();
            // The ledger never self-hashes (see snapshot_files).
            for (path, bytes) in &files {
                if path == LEDGER_FILE {
                    continue;
                }
                file_hashes.insert(
                    path.clone(),
                    Value::String(blake3::hash(bytes).to_hex().to_string()),
                );
            }
            ledger.insert("files".to_string(), Value::Object(file_hashes));
            serde_json::to_vec_pretty(&Value::Object(ledger)).unwrap()
        };
        files.insert(LEDGER_FILE.to_string(), ledger_json);

        for (rel, bytes) in &files {
            let abs = dir.join(rel);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, bytes).unwrap();
        }

        let (manifest, schema, artifacts) = load(dir).unwrap();
        assert_eq!(manifest.profile, "client");
        assert_eq!(schema.tables.len(), 0);
        assert_eq!(artifacts.len(), 3);
        assert_eq!(artifacts["data/client/Item.json"], br#"{"id":1}"#);
        assert_eq!(artifacts["data/client/Item.msgpack"], [0x91_u8, 0x01]);

        // Corrupted snapshot → load refuses.
        std::fs::write(dir.join("data/client/Item.json"), b"evil").unwrap();
        assert!(load(dir).is_err());
    }
}
