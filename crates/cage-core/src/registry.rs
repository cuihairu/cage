//! Configuration Registry（R1）: local registry + source resolution.
//!
//! A registry is a root directory versioning published configuration
//! packages (design §29). Each published entry is a **self-verifying
//! snapshot** — the exact `cage snapshot` payload (manifest.json /
//! schema.json / data/… / generated/… / HASHES.json ledger) — so publish
//! and resolve both gate on ledger verification: nothing enters the
//! registry unverified, nothing leaves it unverified.
//!
//! Layout (all bytes deterministic — same inputs → same entry → same index):
//!
//! ```text
//! <root>/
//!   <package>/                       # package name: [A-Za-z0-9._-]+
//!     index.json                     # deterministic index (sorted versions)
//!     <version>/                     # version: [A-Za-z0-9._-]+, no path segs
//!       manifest.json schema.json HASHES.json data/… generated/…
//! ```
//!
//! Version ordering is dotted-numeric (1.9 < 1.10); a resolve spec without a
//! version picks the highest. Consumers declare the registry root in their
//! project config (`[registry] path = "…"`) and reference entries as source
//! roots with the `registry:<package>[@<version>]` syntax — resolution
//! verifies the entry's ledger, then yields its `data/` directory as the
//! source root.

use crate::error::codes::registry::{E1801, E1802, E1803};
use crate::snapshot;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Ledger file name inside a published entry (the snapshot trust root).
const LEDGER_FILE: &str = "HASHES.json";
/// Registry index file name inside a package directory.
const INDEX_FILE: &str = "index.json";

/// Valid package/version character set: alphanumerics, dot, dash, underscore.
/// `..` / `/` / `\` are rejected so names can never escape the registry root.
fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// One published version of a package, as recorded in `index.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    /// Published version (dotted-numeric order used for sorting)
    pub version: String,
    /// Deterministic build fingerprint of the entry (from its ledger)
    pub build_id: String,
    /// Deterministic content fingerprint of the entry (from its ledger)
    pub content_hash: String,
    /// Number of files the ledger covers
    pub files: usize,
}

/// Per-package registry index. JSON is written pretty-printed, entries
/// sorted by dotted-numeric version order — byte-deterministic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RegistryIndex {
    /// Package name this file indexes
    pub package: String,
    /// Published versions, ascending
    pub entries: Vec<IndexEntry>,
}

/// Result of `publish`: what changed in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    /// Destination entry directory (registry-relative from publish root)
    pub entry_path: String,
    /// `true` when a byte-identical version already existed and nothing
    /// was rewritten (idempotent re-publish), `false` when newly written
    pub already_identical: bool,
}

/// Dotted-numeric version component: either a number (compared
/// numerically — 1.9 < 1.10) or an arbitrary label (compared as text,
/// sorted after numbers).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum VersionComponent {
    Number(u64),
    Label(String),
}

/// Split a version into sortable components: dots/hyphens separate
/// (1.0.0-beta → [1, 0, 0, beta]). Shorter vectors sort first when all
/// shared components tie ("1.0" < "1.0.1", bare "1.0.0" < "1.0.0-beta").
fn version_key(version: &str) -> Vec<VersionComponent> {
    version
        .split(['.', '-'])
        .map(|part| match part.parse::<u64>() {
            Ok(n) => VersionComponent::Number(n),
            Err(_) => VersionComponent::Label(part.to_string()),
        })
        .collect()
}

/// Highest version by dotted-numeric order; ties (same numeric key, e.g.
/// "1.0" vs "1.0.0") break on the raw string for determinism.
fn latest_version(versions: &[String]) -> Option<&str> {
    versions
        .iter()
        .max_by(|a, b| version_key(a).cmp(&version_key(b)).then_with(|| a.cmp(b)))
        .map(String::as_str)
}

/// Parse a `registry:<package>[@<version>]` source-root spec.
/// A missing version means "latest".
pub fn parse_spec(spec: &str) -> Result<(String, Option<String>), String> {
    let rest = spec
        .strip_prefix("registry:")
        .ok_or_else(|| format!("registry spec must start with 'registry:': {spec}"))?;
    let (package, version) = match rest.split_once('@') {
        Some((p, v)) => (p, Some(v.to_string())),
        None => (rest, None),
    };
    if !valid_component(package) {
        return Err(format!(
            "{E1802} invalid registry package name '{package}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }
    if let Some(v) = &version {
        if !valid_component(v) {
            return Err(format!(
                "{E1802} invalid registry version '{v}' (allowed: letters, digits, '.', '-', '_')"
            ));
        }
    }
    Ok((package.to_string(), version))
}

/// Read (or initialize) a package's index. A missing index is an empty one;
/// a corrupt index is an error (never silently rewritten by resolve).
fn read_index(root: &Path, package: &str) -> Result<RegistryIndex, String> {
    let index_path = root.join(package).join(INDEX_FILE);
    match fs::read(&index_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RegistryIndex {
            package: package.to_string(),
            entries: Vec::new(),
        }),
        Err(e) => Err(format!(
            "{E1802} cannot read registry index {}: {e}",
            index_path.display()
        )),
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "{E1802} corrupt registry index {}: {e}",
                index_path.display()
            )
        }),
    }
}

fn write_index(root: &Path, index: &RegistryIndex) -> Result<(), String> {
    let index_path = root.join(&index.package).join(INDEX_FILE);
    let mut bytes = serde_json::to_vec_pretty(index)
        .map_err(|e| format!("{E1801} cannot serialize registry index: {e}"))?;
    bytes.push(b'\n');
    fs::create_dir_all(index_path.parent().expect("index parent")).map_err(|e| {
        format!(
            "{E1801} cannot create {}: {e}",
            index_path.parent().unwrap().display()
        )
    })?;
    fs::write(&index_path, bytes).map_err(|e| {
        format!(
            "{E1801} cannot write registry index {}: {e}",
            index_path.display()
        )
    })
}

/// Recursively collect the paths of every file under `dir`, relative to
/// `dir` (so a plain `target.join(rel)` yields the entry copy path) and
/// sorted for determinism.
fn files_under(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let entries = fs::read_dir(&cur)
            .map_err(|e| format!("{E1801} cannot read dir {}: {e}", cur.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{E1801} dir entry error: {e}"))?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                files.push(p.strip_prefix(dir).expect("under dir").to_path_buf());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Publish a verified snapshot directory into the registry as
/// `<package>/<version>`. The snapshot must verify clean before anything
/// is written (E1803 otherwise). Re-publishing a byte-identical version is
/// an idempotent no-op; the same version with different bytes is an E1801
/// version conflict. The package index is rewritten deterministically.
pub fn publish(
    root: &Path,
    package: &str,
    version: &str,
    snapshot_dir: &Path,
) -> Result<PublishReport, String> {
    if !valid_component(package) {
        return Err(format!(
            "{E1801} invalid registry package name '{package}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }
    if !valid_component(version) {
        return Err(format!(
            "{E1801} invalid registry version '{version}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }

    // The trust gate: only a self-verifying snapshot enters the registry.
    // verify_snapshot's own errors (unreadable/corrupt ledger) are tagged
    // with E1803 here — the gate, not the snapshot's internal reason, is
    // what the consumer reports.
    let report = snapshot::verify_snapshot(snapshot_dir).map_err(|e| format!("{E1803} {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E1803} refusing to publish unverifiable snapshot {} ({} problem(s)): {}",
            snapshot_dir.display(),
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(snapshot_dir.join(LEDGER_FILE))
            .map_err(|e| format!("{E1801} cannot read ledger: {e}"))?,
    )
    .map_err(|e| format!("{E1801} ledger parse: {e}"))?;
    let build_id = ledger
        .get("build_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "{E1801} ledger has no build_id".to_string())?
        .to_string();
    let content_hash = ledger
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "{E1801} ledger has no content_hash".to_string())?;

    let mut index = read_index(root, package)?;
    let existing = index.entries.iter().find(|e| e.version == version);
    if let Some(existing) = existing {
        if existing.content_hash == content_hash {
            return Ok(PublishReport {
                entry_path: format!("{package}/{version}"),
                already_identical: true,
            });
        }
        return Err(format!(
            "{E1801} registry version conflict: {package}/{version} already published with \
             content_hash {} — republish under a new version",
            existing.content_hash
        ));
    }

    // Copy every snapshot file into the entry (deterministic byte copy —
    // the snapshot is already a fixed point, so the entry matches a fresh
    // snapshot of the same inputs byte for byte).
    let target = root.join(package).join(version);
    let source_files = files_under(snapshot_dir)?;
    let file_count = source_files.len();
    for rel in &source_files {
        let abs = target.join(rel);
        fs::create_dir_all(abs.parent().expect("entry parent")).map_err(|e| {
            format!(
                "{E1801} cannot create {}: {e}",
                abs.parent().unwrap().display()
            )
        })?;
        fs::copy(snapshot_dir.join(rel), &abs)
            .map_err(|e| format!("{E1801} cannot copy {}: {e}", abs.display()))?;
    }

    index.entries.push(IndexEntry {
        version: version.to_string(),
        build_id,
        content_hash: content_hash.to_string(),
        files: file_count,
    });
    index.entries.sort_by(|a, b| {
        version_key(&a.version)
            .cmp(&version_key(&b.version))
            .then_with(|| a.version.cmp(&b.version))
    });
    write_index(root, &index)?;

    Ok(PublishReport {
        entry_path: format!("{package}/{version}"),
        already_identical: false,
    })
}

/// List every package in the registry (deterministic name order) with its
/// index — the `cage registry list` backing. A directory without an
/// `index.json` is not a package and is skipped.
pub fn packages(root: &Path) -> Result<Vec<RegistryIndex>, String> {
    let mut names = Vec::new();
    let entries =
        fs::read_dir(root).map_err(|e| format!("{E1802} cannot read {}: {e}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{E1802} dir entry error: {e}"))?;
        let p = entry.path();
        if p.is_dir() && p.join(INDEX_FILE).is_file() {
            if let Some(name) = p.file_name().and_then(std::ffi::OsStr::to_str) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.iter().map(|n| read_index(root, n)).collect()
}

/// Resolve a registry reference to an entry directory. `version` omitted
/// means the highest published version (dotted-numeric order). The entry's
/// ledger is verified before it is handed out — resolving a tampered entry
/// is an E1803 error, missing package/version an E1802 error.
pub fn resolve(root: &Path, package: &str, version: Option<&str>) -> Result<PathBuf, String> {
    let index = read_index(root, package)?;
    if index.entries.is_empty() {
        return Err(format!(
            "{E1802} registry package not found: {package} (registry {})",
            root.display()
        ));
    }
    let versions: Vec<String> = index
        .entries
        .iter()
        .map(|e| e.version.clone())
        .collect::<Vec<_>>();
    let version = match version {
        Some(v) => {
            if !versions.iter().any(|e| e == v) {
                return Err(format!(
                    "{E1802} registry version not found: {package}@{v} (registry {}; published: {})",
                    root.display(),
                    versions.join(", ")
                ));
            }
            v
        }
        None => latest_version(&versions).expect("non-empty entries"),
    };

    let entry = root.join(package).join(version);
    let report = snapshot::verify_snapshot(&entry).map_err(|e| format!("{E1803} {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E1803} registry entry verification failed: {package}/{version} ({} problem(s)): {}",
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build a tiny self-verifying snapshot directory on the fly. The
    /// ledger's `content_hash` derives from `content`, so different contents
    /// really conflict and equal contents really dedupe.
    fn make_snapshot(dir: &Path, content: &str) {
        let artifacts = vec![(
            "build/client/json/Item.json".to_string(),
            content.as_bytes().to_vec(),
            "json".to_string(),
            Some("Item".to_string()),
        )];
        let content_hash = blake3::hash(content.as_bytes()).to_hex()[..24].to_string();
        let files = crate::snapshot::snapshot_files(
            br#"{"generator_version":"1.0.0"}"#,
            br#"{"tables":{},"enums":{},"metadata":null}"#,
            &artifacts,
            "build",
            "b111111111111111111111111",
            &content_hash,
        );
        for (rel, bytes) in &files {
            let abs = dir.join(rel);
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(abs, bytes).unwrap();
        }
    }

    #[test]
    fn version_order_is_dotted_numeric() {
        assert!(version_key("1.9") < version_key("1.10"));
        assert!(version_key("1.0.0") > version_key("1.0"));
        // A label suffix sorts after the bare release (shorter-first rule).
        assert!(version_key("1.0.0") < version_key("1.0.0-beta"));
        assert!(version_key("1.0.0-beta2") > version_key("1.0.0-beta"));
        assert_eq!(
            latest_version(&[
                "1.0.0".to_string(),
                "1.9.0".to_string(),
                "1.10.0".to_string()
            ]),
            Some("1.10.0")
        );
        // same-key tie ("1.0" vs "1.0.0") breaks on the raw string
        assert_eq!(
            latest_version(&["1.0.0".to_string(), "1.0".to_string()]),
            Some("1.0.0")
        );
    }

    #[test]
    fn parse_spec_accepts_bare_and_pinned_versions() {
        let (p, v) = parse_spec("registry:common").unwrap();
        assert_eq!(p, "common");
        assert_eq!(v, None);
        let (p, v) = parse_spec("registry:common@1.2.0").unwrap();
        assert_eq!(p, "common");
        assert_eq!(v.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn parse_spec_rejects_path_escape_attempts() {
        // Only the registry core enforces this — consumers render these as
        // E1802 resolution errors, never as paths.
        assert!(parse_spec("registry:../secret").is_err());
        assert!(parse_spec("registry:a/b").is_err());
        assert!(parse_spec("registry:ok@1.0/../../x").is_err());
        assert!(parse_spec("registry:ok@..").is_err());
        assert!(parse_spec("registry:").is_err());
        assert!(parse_spec("plainroot").is_err());
    }

    #[test]
    fn publish_resolves_latest_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");

        let r = publish(&root, "common", "1.0.0", &snap).unwrap();
        assert!(!r.already_identical);
        assert_eq!(r.entry_path, "common/1.0.0");
        assert!(root.join("common/1.0.0/HASHES.json").is_file());
        assert!(root
            .join("common/1.0.0/data/client/json/Item.json")
            .is_file());

        // Re-publish identical bytes (same version = same content_hash) → no-op.
        let r2 = publish(&root, "common", "1.0.0", &snap).unwrap();
        assert!(r2.already_identical);

        // Publish 1.9.0 and 1.10.0 with different content → latest is 1.10.0.
        let snap2 = tmp.path().join("snap2");
        make_snapshot(&snap2, "two");
        publish(&root, "common", "1.9.0", &snap2).unwrap();
        publish(&root, "common", "1.10.0", &snap2).unwrap();

        let dir = resolve(&root, "common", None).unwrap();
        assert!(dir.ends_with("1.10.0"));
        let dir = resolve(&root, "common", Some("1.0.0")).unwrap();
        assert!(dir.ends_with("1.0.0"));

        // Index is deterministic: sorted ascending, stable parse.
        let index = read_index(&root, "common").unwrap();
        assert_eq!(
            index
                .entries
                .iter()
                .map(|e| e.version.clone())
                .collect::<Vec<_>>(),
            ["1.0.0", "1.9.0", "1.10.0"]
        );
    }

    #[test]
    fn publish_same_version_different_bytes_is_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&root, "common", "1.0.0", &snap).unwrap();

        let snap2 = tmp.path().join("snap2");
        make_snapshot(&snap2, "two");
        let err = publish(&root, "common", "1.0.0", &snap2).unwrap_err();
        assert!(err.contains("E1801"), "{err}");
        assert!(err.contains("conflict"), "{err}");
    }

    #[test]
    fn resolve_missing_package_or_version_is_e1802() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&root, "common", "1.0.0", &snap).unwrap();

        let err = resolve(&root, "ghost", None).unwrap_err();
        assert!(err.contains("E1802"), "{err}");
        assert!(err.contains("not found"), "{err}");

        let err = resolve(&root, "common", Some("99.0.0")).unwrap_err();
        assert!(err.contains("E1802"), "{err}");
        assert!(err.contains("99.0.0"), "{err}");
    }

    #[test]
    fn resolve_tampered_entry_is_e1803() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&root, "common", "1.0.0", &snap).unwrap();

        // Tamper with the published data file — the ledger catches it.
        let data = root.join("common/1.0.0/data/client/json/Item.json");
        fs::write(&data, "tampered").unwrap();
        let err = resolve(&root, "common", None).unwrap_err();
        assert!(err.contains("E1803"), "{err}");
        assert!(err.contains("hash mismatch"), "{err}");
    }

    #[test]
    fn publish_refuses_unverifiable_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        // Break the ledger: the snapshot is no longer self-verifying.
        fs::write(snap.join("HASHES.json"), "{}").unwrap();
        let err = publish(&root, "common", "1.0.0", &snap).unwrap_err();
        assert!(err.contains("E1803"), "{err}");
        assert!(!root.join("common").exists(), "nothing may be written");
    }
}
