//! Whole-registry audit, garbage collection, and entry removal — every
//! path re-verifies ledgers before touching bytes. Split out of the
//! registry module (LAY-002).

use super::{packages, read_index, valid_component, write_index, E1801, E1802, E1803, LEDGER_FILE};
use crate::snapshot;
use std::fs;
use std::path::Path;

/// Audit report of a full-registry verification (`cage registry verify`,
/// R4). `problems` are the complete trail — one string per finding, each
/// carrying its registry error code (E1803 ledger/structure; E1802
/// unreadable index) and `package/version` — in deterministic order. An
/// empty report is a clean registry.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RegistryReport {
    /// Number of packages with an index.json
    pub packages: usize,
    /// Number of index-recorded entries actually re-hashed
    pub entries_checked: usize,
    /// Findings, deterministic order, empty when the registry is clean
    pub problems: Vec<String>,
}

impl RegistryReport {
    /// Whether every entry verified clean with a consistent index.
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Verify every package of a registry (R4): each index-recorded version
/// must exist and self-verify, its ledger's `build_id`/`content_hash` must
/// match the index record, and no entry directory may sit on disk without
/// an index record (orphan of an interrupted remove/gc or a hand-edit).
/// Read-only — never rewrites anything.
pub fn verify_registry(root: &Path) -> Result<RegistryReport, String> {
    let mut report = RegistryReport::default();
    for index in packages(root)? {
        report.packages += 1;
        let pkg_root = root.join(&index.package);
        let mut indexed: Vec<String> = Vec::new();
        for entry in &index.entries {
            indexed.push(entry.version.clone());
            let ver_dir = pkg_root.join(&entry.version);
            let rep = match snapshot::verify_snapshot(&ver_dir) {
                Ok(rep) if rep.ok => {
                    report.entries_checked += 1;
                    let ledger: serde_json::Value = serde_json::from_slice(
                        &fs::read(ver_dir.join(LEDGER_FILE))
                            .map_err(|e| format!("{E1802} ledger read: {e}"))?,
                    )
                    .map_err(|e| format!("{E1802} ledger parse: {e}"))?;
                    let build_id = ledger
                        .get("build_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let content_hash = ledger
                        .get("content_hash")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if build_id == entry.build_id && content_hash == entry.content_hash {
                        continue;
                    }
                    report.problems.push(format!(
                        "{E1803} {}/{}: index record does not match ledger \
                         (build_id {build_id}, content_hash {content_hash})",
                        index.package, entry.version
                    ));
                    continue;
                }
                Ok(rep) => format!(
                    "{E1803} {}/{}: entry verification failed ({} problem(s)): {}",
                    index.package,
                    entry.version,
                    rep.mismatches.len(),
                    rep.mismatches.join("; ")
                ),
                Err(e) => format!("{E1803} {}/{}: {e}", index.package, entry.version),
            };
            report.problems.push(rep);
        }
        // Entry directories on disk that no index record names.
        let mut orphans: Vec<String> = Vec::new();
        let dirs = fs::read_dir(&pkg_root)
            .map_err(|e| format!("{E1802} cannot read {}: {e}", pkg_root.display()))?;
        for entry in dirs {
            let entry = entry.map_err(|e| format!("{E1802} dir entry error: {e}"))?;
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(name) = p.file_name().and_then(std::ffi::OsStr::to_str) {
                if !indexed.contains(&name.to_string()) {
                    orphans.push(name.to_string());
                }
            }
        }
        orphans.sort();
        for name in orphans {
            report.problems.push(format!(
                "{E1803} {}/{}: entry directory without index record",
                index.package, name
            ));
        }
    }
    Ok(report)
}

/// What one garbage-collection pass removed (R4), deterministic order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Removed entry dirs and index records as `package/version` — plus
    /// orphaned dirs as `package/version (orphan)` — in registry order
    pub removed: Vec<String>,
    /// Packages whose index.json was rewritten
    pub rewritten: usize,
}

/// Garbage-collect a registry (R4): per package, keep the newest `keep`
/// versions (dotted-numeric, never fewer than one — a version window is
/// the rollback surface: consumers pin an old `@version` and keep
/// resolving), delete the older entries and their index records, and sweep
/// orphaned entry directories no index records. Indexes are rewritten
/// deterministically through the same writer as publish. With `dry_run`
/// the exact same report is computed but nothing is deleted or rewritten.
pub fn gc_registry(root: &Path, keep: usize, dry_run: bool) -> Result<GcReport, String> {
    let keep = keep.max(1);
    let mut report = GcReport::default();
    for mut index in packages(root)? {
        let pkg_root = root.join(&index.package);
        let mut changed = false;
        let drop_count = index.entries.len().saturating_sub(keep);
        let dropped: Vec<String> = index
            .entries
            .drain(..drop_count)
            .map(|entry| entry.version)
            .collect();
        for v in &dropped {
            changed = true;
            let ver_dir = pkg_root.join(v);
            if ver_dir.exists() && !dry_run {
                fs::remove_dir_all(&ver_dir)
                    .map_err(|e| format!("{E1801} cannot remove {}: {e}", ver_dir.display()))?;
            }
            report.removed.push(format!("{}/{}", index.package, v));
        }
        // Orphaned entry dirs (no index record) are garbage by definition.
        // Orphans are judged against the index as it is on disk (before
        // the drain above) — in a dry run the dropped dirs are still there
        // and must not be reported twice.
        let indexed: Vec<String> = index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .chain(dropped.iter().cloned())
            .collect();
        let mut dirs: Vec<String> = Vec::new();
        for entry in fs::read_dir(&pkg_root)
            .map_err(|e| format!("{E1802} cannot read {}: {e}", pkg_root.display()))?
        {
            let entry = entry.map_err(|e| format!("{E1802} dir entry error: {e}"))?;
            let p = entry.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(std::ffi::OsStr::to_str) {
                    dirs.push(name.to_string());
                }
            }
        }
        dirs.sort();
        for name in dirs {
            if !indexed.contains(&name) {
                changed = true;
                let ver_dir = pkg_root.join(&name);
                if !dry_run {
                    fs::remove_dir_all(&ver_dir)
                        .map_err(|e| format!("{E1801} cannot remove {}: {e}", ver_dir.display()))?;
                }
                report
                    .removed
                    .push(format!("{}/{} (orphan)", index.package, name));
            }
        }
        if changed {
            if !dry_run {
                write_index(root, &index)?;
            }
            report.rewritten += 1;
        }
    }
    Ok(report)
}

/// Remove one entry — version directory and index record — the explicit
/// administration path (R4). The version must be recorded (E1802
/// otherwise). The index is rewritten even when the package becomes empty
/// (the package dir stays, so a later publish recreates the version
/// cleanly — history removed explicitly is reusable; anything else is an
/// E1801 conflict). With `dry_run` the removal is validated (name rules,
/// existence) but nothing is deleted or rewritten.
pub fn remove_entry(
    root: &Path,
    package: &str,
    version: &str,
    dry_run: bool,
) -> Result<(), String> {
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
    let mut index = read_index(root, package)?;
    let before = index.entries.len();
    index.entries.retain(|e| e.version != version);
    if index.entries.len() == before {
        return Err(format!(
            "{E1802} registry version not found: {package}@{version} (registry {})",
            root.display()
        ));
    }
    if !dry_run {
        let ver_dir = root.join(package).join(version);
        if ver_dir.exists() {
            fs::remove_dir_all(&ver_dir)
                .map_err(|e| format!("{E1801} cannot remove {}: {e}", ver_dir.display()))?;
        }
        write_index(root, &index)?;
    }
    Ok(())
}
