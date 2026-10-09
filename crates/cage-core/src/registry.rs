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

use crate::error::codes::distribution::{E2101, E2102, E2103, E2104, E2105};
use crate::error::codes::registry::{E1801, E1802, E1803};
use crate::snapshot;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

/// Ledger file name inside a published entry (the snapshot trust root).
const LEDGER_FILE: &str = "HASHES.json";
/// Registry index file name inside a package directory.
const INDEX_FILE: &str = "index.json";

/// Fixed zstd compression level for `--compress zstd` bundles (A1).
///
/// Level 19 — the top of zstd's standard (documented 1–19) range. The
/// compression-time cost is paid once per export, while bundles are small
/// configuration artifacts where the ratio wins on every wire/store hop —
/// the same trade as the release profile's `opt-level = "z"`. The ultra
/// levels above 19 cost extreme time and window memory for marginal gain,
/// so they stay out of the contract. The level is pinned in code (not
/// user-selectable) so a given input always compresses to identical bytes
/// within a pinned zstd library version, matching the bundle's determinism
/// contract.
pub const BUNDLE_ZSTD_LEVEL: i32 = 19;

/// The 4-byte zstd frame magic `0x28 0xB5 0x2F 0xFD` (little-endian
/// `0xFD2FB528`), sniffed at the head of an import file to tell a
/// zstd-wrapped bundle from a plain tar.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

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

/// Container format of an exported bundle (A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleCompression {
    /// The deterministic tar bytes, unwrapped — the default and the only
    /// format `export_bundle` wrote before compression landed.
    Plain,
    /// The deterministic tar wrapped in a single zstd frame at the fixed
    /// level [`BUNDLE_ZSTD_LEVEL`]. `import_bundle` sniffs the frame magic,
    /// so both forms load identically; the trust ledger hashes uncompressed
    /// content, so a wrapped bundle verifies exactly like its plain form.
    Zstd,
}

/// Result of `export_bundle`: what was packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Package the bundle was exported from
    pub package: String,
    /// Entry version packed (explicitly requested, or dotted-numeric latest
    /// when the version was omitted)
    pub version: String,
    /// Entry files packed (the bundle also carries the package index excerpt)
    pub files: usize,
}

/// Result of `import_bundle`: what entered (or would enter) the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// Package imported from the bundle's index excerpt
    pub package: String,
    /// Entry version imported
    pub version: String,
    /// Entry files staged and ledger-verified
    pub files: usize,
    /// `true` when the exact bytes were already in the registry (idempotent
    /// re-import, nothing rewritten), `false` when newly written
    pub already_identical: bool,
    /// `true` when the import ran as a report-only dry run (nothing written)
    pub dry_run: bool,
}

/// One comparator of a version requirement (R2 dependency pin): `op` plus a
/// dotted version whose missing components compare as zero (`>=1.2` ≡
/// `>=1.2.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparator {
    /// Comparison operator
    pub op: CmpOp,
    /// Dotted version the operand compares against
    pub version: String,
}

/// Comparison operators of a requirement comparator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`
    Eq,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
}

/// A parsed version requirement (R2): the AND of comparators, e.g.
/// `>=1.0, <2.0`. Syntax: comma-separated comparators; operators `=` `>`
/// `>=` `<` `<=` (a bare `1.2.3` is exact); `^` expands caret
/// (`^1.2` → `>=1.2.0, <2.0.0`; `^0.2.3` → `>=0.2.3, <0.3.0`), `~` expands
/// tilde (`~1.2` → `>=1.2.0, <1.3.0`; `~1` → `>=1.0.0, <2.0.0`). All
/// comparisons use the registry's dotted-numeric order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionReq {
    /// All comparators must hold
    pub comparators: Vec<Comparator>,
}

/// Parse a version requirement string (`E1802` on invalid syntax).
pub fn parse_version_req(spec: &str) -> Result<VersionReq, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("E1802 empty version requirement".to_string());
    }
    let mut comparators = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!(
                "E1802 invalid version requirement '{spec}': empty clause"
            ));
        }
        let (op, version) = match part.chars().next() {
            Some('^') => {
                // caret: bump the left-most non-zero component
                let v = part[1..].trim();
                let base = version_parts(v)
                    .map_err(|e| format!("E1802 invalid caret version '{v}': {e}"))?;
                let (lower, upper) = caret_range(&base, spec, v)?;
                comparators.push(Comparator {
                    op: CmpOp::Ge,
                    version: lower,
                });
                comparators.push(Comparator {
                    op: CmpOp::Lt,
                    version: upper,
                });
                continue;
            }
            Some('~') => {
                // tilde: lock every component but the second-to-last given
                let v = part[1..].trim();
                let base = version_parts(v)
                    .map_err(|e| format!("E1802 invalid tilde version '{v}': {e}"))?;
                let (lower, upper) = tilde_range(&base, spec, v)?;
                comparators.push(Comparator {
                    op: CmpOp::Ge,
                    version: lower,
                });
                comparators.push(Comparator {
                    op: CmpOp::Lt,
                    version: upper,
                });
                continue;
            }
            Some('=') => (CmpOp::Eq, &part[1..]),
            Some('>') => {
                if part[1..].starts_with('=') {
                    (CmpOp::Ge, &part[2..])
                } else {
                    (CmpOp::Gt, &part[1..])
                }
            }
            Some('<') => {
                if part[1..].starts_with('=') {
                    (CmpOp::Le, &part[2..])
                } else {
                    (CmpOp::Lt, &part[1..])
                }
            }
            Some(c) if c.is_ascii_digit() => (CmpOp::Eq, part),
            _ => {
                return Err(format!(
                    "E1802 invalid version requirement '{spec}': bad comparator '{part}'"
                ))
            }
        };
        let version = version.trim();
        version_parts(version)
            .map_err(|e| format!("E1802 invalid version requirement '{spec}': {e}"))?;
        comparators.push(Comparator {
            op,
            version: version.to_string(),
        });
    }
    Ok(VersionReq { comparators })
}

/// Parse a dotted version into parts, validating each component.
fn version_parts(v: &str) -> Result<Vec<VersionComponent>, String> {
    if v.is_empty() {
        return Err("missing version".to_string());
    }
    let parts = version_key(v);
    if parts.is_empty()
        || parts
            .iter()
            .any(|p| matches!(p, VersionComponent::Label(l) if l.is_empty()))
    {
        return Err(format!("invalid version '{v}'"));
    }
    Ok(parts)
}

/// Render `parts` back to a dotted string (caret/tilde expansion output).
fn render_parts(parts: &[VersionComponent]) -> String {
    parts
        .iter()
        .map(|p| match p {
            VersionComponent::Number(n) => n.to_string(),
            VersionComponent::Label(l) => l.clone(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Zero-pad `parts` to `len` numeric components (missing = 0).
fn pad_zeros(parts: &[VersionComponent], len: usize) -> Vec<VersionComponent> {
    let mut out = parts.to_vec();
    while out.len() < len {
        out.push(VersionComponent::Number(0));
    }
    out
}

/// Caret range: bump the left-most non-zero component (all-zero base bumps
/// the last given one). `^1.2` → [1.2.0, 2.0.0); `^0.2.3` → [0.2.3, 0.3.0).
fn caret_range(
    base: &[VersionComponent],
    spec: &str,
    raw: &str,
) -> Result<(String, String), String> {
    let width = base.len();
    let mut upper = pad_zeros(base, width);
    let bump_at = base
        .iter()
        .position(|p| matches!(p, VersionComponent::Number(n) if *n > 0))
        .or_else(|| match base.last() {
            Some(VersionComponent::Number(_)) => Some(base.len() - 1),
            _ => None,
        });
    let Some(i) = bump_at else {
        return Err(format!(
            "E1802 invalid version requirement '{spec}': bad caret version '{raw}'"
        ));
    };
    let Some(VersionComponent::Number(n)) = upper.get_mut(i) else {
        return Err(format!(
            "E1802 invalid version requirement '{spec}': bad caret version '{raw}'"
        ));
    };
    *n += 1;
    // Everything after the bumped component drops to zero (^1.2 → <2.0.0,
    // not <2.2); trailing zeros are popped for a tidy upper bound ("2").
    for p in upper.iter_mut().skip(i + 1) {
        *p = VersionComponent::Number(0);
    }
    while matches!(upper.last(), Some(VersionComponent::Number(0))) && upper.len() > 1 {
        upper.pop();
    }
    Ok((render_parts(&pad_zeros(base, width)), render_parts(&upper)))
}

/// Tilde range: lock everything but the second-to-last given component
/// (`~1.2` → [1.2.0, 1.3.0); `~1` → [1.0.0, 2.0.0); `~1.2.3` →
/// [1.2.3, 1.3.0)).
fn tilde_range(
    base: &[VersionComponent],
    spec: &str,
    raw: &str,
) -> Result<(String, String), String> {
    let width = base.len().max(2);
    let lower = pad_zeros(base, width);
    let mut upper = lower.clone();
    let bump_at = usize::from(base.len() >= 2);
    let Some(VersionComponent::Number(n)) = upper.get_mut(bump_at) else {
        return Err(format!(
            "E1802 invalid version requirement '{spec}': bad tilde version '{raw}'"
        ));
    };
    *n += 1;
    // upper: zero out everything after the bumped component
    for p in upper.iter_mut().skip(bump_at + 1) {
        *p = VersionComponent::Number(0);
    }
    while matches!(upper.last(), Some(VersionComponent::Number(0))) && upper.len() > 1 {
        upper.pop();
    }
    Ok((render_parts(&lower), render_parts(&upper)))
}

/// Compare two dotted versions component-wise, treating missing trailing
/// components as zero (`1.2` ≡ `1.2.0`); labels sort after numbers.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let ka = version_key(a);
    let kb = version_key(b);
    let len = ka.len().max(kb.len());
    for i in 0..len {
        let pa = ka.get(i).cloned().unwrap_or(VersionComponent::Number(0));
        let pb = kb.get(i).cloned().unwrap_or(VersionComponent::Number(0));
        match pa.cmp(&pb) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Whether `version` satisfies every comparator of `req`.
pub fn satisfies(version: &str, req: &VersionReq) -> bool {
    use std::cmp::Ordering;
    req.comparators.iter().all(|c| {
        let ord = compare_versions(version, &c.version);
        match c.op {
            CmpOp::Eq => ord == Ordering::Equal,
            CmpOp::Gt => ord == Ordering::Greater,
            CmpOp::Ge => ord != Ordering::Less,
            CmpOp::Lt => ord == Ordering::Less,
            CmpOp::Le => ord != Ordering::Greater,
        }
    })
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

/// Export a published entry as a deterministic tar bundle (A1, design §47):
/// every entry file — artifacts, `manifest.json`, `schema.json`, the
/// `HASHES.json` ledger — plus the package index excerpt, at member paths
/// `<package>/<version>/<file>` with the excerpt as `index.json`. Same
/// entry, same bytes: members are written in name order with zeroed
/// mtime/uid/gid and a fixed 0o644 mode, so the bundle is byte-reproducible
/// from the registry alone and travels over offline / audit / object-storage
/// channels; `import_bundle` (A2) re-verifies the riding ledger before
/// anything enters a registry. A missing entry (package, version, directory
/// or ledger) and any bundle write failure are E2101. `version = None`
/// exports the latest entry in dotted-numeric order.
/// Locate a published entry in a local registry for the distribution path
/// (A1/A3): names validated against the registry character set, the package
/// index read, the version resolved (explicit must exist, `None` = latest
/// in dotted-numeric order) and the entry directory plus its ledger
/// confirmed on disk. Every failure is E2101 — the entry cannot be read
/// for distribution. A package dir without an index reads as empty and
/// lands in the not-found paths.
fn resolve_entry(
    root: &Path,
    package: &str,
    version: Option<&str>,
) -> Result<(String, PathBuf, Vec<PathBuf>), String> {
    if !valid_component(package) {
        return Err(format!(
            "{E2101} invalid registry package name '{package}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }
    if let Some(v) = version {
        if !valid_component(v) {
            return Err(format!(
                "{E2101} invalid registry version '{v}' (allowed: letters, digits, '.', '-', '_')"
            ));
        }
    }
    let index = read_index(root, package)?;
    let version = match version {
        Some(v) => {
            if !index.entries.iter().any(|e| e.version == v) {
                return Err(format!(
                    "{E2101} registry entry not found: {package}/{v} (registry {})",
                    root.display()
                ));
            }
            v.to_string()
        }
        None => latest_version(
            &index
                .entries
                .iter()
                .map(|e| e.version.clone())
                .collect::<Vec<_>>(),
        )
        .ok_or_else(|| {
            format!(
                "{E2101} registry package not found: {package} (registry {})",
                root.display()
            )
        })?
        .to_string(),
    };
    let entry_dir = root.join(package).join(&version);
    if !entry_dir.is_dir() {
        return Err(format!(
            "{E2101} registry entry directory missing: {} (registry {})",
            entry_dir.display(),
            root.display()
        ));
    }
    let rels = files_under(&entry_dir)?;
    if !rels
        .iter()
        .any(|rel| rel == std::path::Path::new(LEDGER_FILE))
    {
        return Err(format!(
            "{E2101} registry entry has no {LEDGER_FILE} ledger: {}",
            entry_dir.display()
        ));
    }
    Ok((version, entry_dir, rels))
}

/// Rebuild the entry's index record from its riding ledger (the same
/// fields publish writes): the distribution path needs `build_id` /
/// `content_hash` to merge indexes and detect conflicts without trusting
/// any second copy of the record.
fn entry_record(entry_dir: &Path, version: &str, files: usize) -> Result<IndexEntry, String> {
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(entry_dir.join(LEDGER_FILE))
            .map_err(|e| format!("{E2101} cannot read ledger: {e}"))?,
    )
    .map_err(|e| format!("{E2101} ledger parse: {e}"))?;
    let build_id = ledger
        .get("build_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{E2101} ledger has no build_id"))?
        .to_string();
    let content_hash = ledger
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "{E2101} ledger has no content_hash".to_string())?;
    Ok(IndexEntry {
        version: version.to_string(),
        build_id,
        content_hash: content_hash.to_string(),
        files,
    })
}

/// Export a registry entry as a deterministic, self-verifying bundle (A1).
///
/// The bundle is a tar archive containing:
/// - `index.json` — an excerpt of the registry index covering only the exported entry.
/// - All artifact files under `package/version/...` as recorded in the entry's `files` manifest.
///
/// The tar is deterministic: all header fields (mtime, uid, gid, mode) are zeroed or fixed,
/// and members are sorted lexicographically by name. The resulting bytes are byte-for-byte
/// identical for the same source entry regardless of build environment.
///
/// With [`BundleCompression::Zstd`] the finished tar is wrapped in a single
/// zstd frame at the fixed level [`BUNDLE_ZSTD_LEVEL`]: same tar, same
/// level, same zstd library version → the same container bytes. The
/// container is transparent to [`import_bundle`], which sniffs the frame
/// magic instead of trusting extensions — and irrelevant to the trust gate,
/// because the `HASHES.json` ledger hashes uncompressed content, so a
/// zstd-wrapped bundle verifies identically to its plain form.
///
/// Returns an `ExportReport` with the bundle path, size, and content hash.
pub fn export_bundle(
    root: &Path,
    package: &str,
    version: Option<&str>,
    compression: BundleCompression,
    out: &Path,
) -> Result<ExportReport, String> {
    let (version, entry_dir, rels) = resolve_entry(root, package, version)?;

    // The excerpt carries just the exported entry — the receiving side
    // re-derives everything else from the ledger (A2).
    let excerpt = RegistryIndex {
        package: package.to_string(),
        entries: vec![entry_record(&entry_dir, &version, rels.len())?],
    };
    let mut excerpt_bytes = serde_json::to_vec_pretty(&excerpt)
        .map_err(|e| format!("{E2101} cannot serialize index excerpt: {e}"))?;
    excerpt_bytes.push(b'\n');

    let mut members: Vec<(String, Vec<u8>)> = Vec::with_capacity(rels.len() + 1);
    members.push(("index.json".to_string(), excerpt_bytes));
    for rel in &rels {
        let source = entry_dir.join(rel);
        let bytes = fs::read(&source)
            .map_err(|e| format!("{E2101} cannot read entry file {}: {e}", source.display()))?;
        members.push((
            format!("{package}/{version}/{}", rel.to_string_lossy()),
            bytes,
        ));
    }
    members.sort_by(|a, b| a.0.cmp(&b.0));

    // The deterministic tar is built in memory first, so whichever container
    // follows wraps exactly the bytes a plain export would carry.
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in &members {
        // Determinism contract: header fields we set ourselves — the tar
        // layer never sees a filesystem stat, so nothing environment-sourced
        // (mtime, uid/gid, mode) can leak into the bytes.
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|e| format!("{E2101} cannot pack bundle member '{name}': {e}"))?;
    }
    let tar_bytes = builder
        .into_inner()
        .map_err(|e| format!("{E2101} cannot finalize bundle {}: {e}", out.display()))?;

    match compression {
        BundleCompression::Plain => {
            fs::write(out, &tar_bytes)
                .map_err(|e| format!("{E2101} cannot write bundle {}: {e}", out.display()))?;
        }
        BundleCompression::Zstd => {
            // Bulk single-threaded encode at the pinned [`BUNDLE_ZSTD_LEVEL`]:
            // deterministic for a given zstd library version, keeping the
            // bundle byte-reproducible like its plain-tar form.
            let wrapped = zstd::stream::encode_all(&tar_bytes[..], BUNDLE_ZSTD_LEVEL)
                .map_err(|e| format!("{E2101} cannot compress bundle {}: {e}", out.display()))?;
            fs::write(out, &wrapped)
                .map_err(|e| format!("{E2101} cannot write bundle {}: {e}", out.display()))?;
        }
    }

    Ok(ExportReport {
        package: package.to_string(),
        version,
        files: rels.len(),
    })
}

/// Strip a bundle's compression container, if it carries one: a zstd frame
/// magic at the head selects the zstd form, anything else passes through as
/// the plain tar. The sniff is content-based, never extension-based —
/// bundles travel renamed across offline and object-storage channels, so a
/// `.tar`/`.tar.zst` suffix carries no format truth. A head that claims the
/// frame but fails to decode is a corrupt container; the caller reports it
/// against the bundle file. Compression is container-level only: the
/// `HASHES.json` ledger hashes uncompressed content, so the trust gate runs
/// on the decompressed entries exactly as it does for a plain bundle.
fn decompress_bundle(raw: Vec<u8>) -> Result<Vec<u8>, String> {
    if !raw.starts_with(&ZSTD_MAGIC) {
        return Ok(raw);
    }
    zstd::stream::decode_all(Cursor::new(&raw[..]))
        .map_err(|e| format!("zstd container decode failed: {e}"))
}

/// Import a bundle produced by `export_bundle` (A2, design §47): the riding
/// ledger must pass `verify_snapshot` and match the bundle's index excerpt
/// before anything enters the registry — unverifiable bytes never touch the
/// target (staging is a temp dir, discarded on every refusal path). A
/// byte-identical re-import is an idempotent no-op; the same version with
/// different bytes is the usual E1801 conflict (the registry never rewrites
/// history, not even through distribution). Structural problems — unsafe
/// member paths, malformed index, members outside the entry, missing
/// ledger — and any trust-gate refusal are E2103. The container form is
/// sniffed from the file head, not the name: plain tars load unchanged and
/// a `--compress zstd` frame decompresses first, so both forms import
/// identically under any extension. With `dry_run` the full gate runs and
/// the report comes back without writing anything.
pub fn import_bundle(root: &Path, file: &Path, dry_run: bool) -> Result<ImportReport, String> {
    let raw = fs::read(file)
        .map_err(|e| format!("{E2101} cannot read bundle {}: {e}", file.display()))?;
    let bytes = decompress_bundle(raw)
        .map_err(|e| format!("{E2103} malformed bundle {}: {e}", file.display()))?;
    let mut archive = tar::Archive::new(&bytes[..]);

    // Members are untrusted input: only plain relative paths survive the
    // component check (no absolute, no `..`, no prefix segments).
    let mut excerpt: Option<RegistryIndex> = None;
    let mut members: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in archive
        .entries()
        .map_err(|e| format!("{E2103} malformed bundle {}: {e}", file.display()))?
    {
        let mut entry = entry.map_err(|e| format!("{E2103} malformed bundle entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("{E2103} malformed bundle entry path: {e}"))?
            .into_owned();
        if path.is_absolute()
            || !path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "{E2103} unsafe bundle member path '{}'",
                path.display()
            ));
        }
        let name = path.to_string_lossy().into_owned();
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .map_err(|e| format!("{E2103} cannot read bundle member '{name}': {e}"))?;
        if name == INDEX_FILE {
            if excerpt.is_some() {
                return Err(format!("{E2103} duplicate {INDEX_FILE} in bundle"));
            }
            let parsed: RegistryIndex = serde_json::from_slice(&content)
                .map_err(|e| format!("{E2103} malformed {INDEX_FILE} in bundle: {e}"))?;
            excerpt = Some(parsed);
        } else {
            members.push((name, content));
        }
    }

    // A1 bundles carry exactly one entry: the excerpt names it, the members
    // realize it.
    let index = excerpt.ok_or_else(|| format!("{E2103} bundle has no {INDEX_FILE}"))?;
    if index.entries.len() != 1 {
        return Err(format!(
            "{E2103} bundle {INDEX_FILE} must carry exactly one entry, got {}",
            index.entries.len()
        ));
    }
    let excerpt_entry = &index.entries[0];
    let package = index.package;
    let version = excerpt_entry.version.clone();
    let prefix = format!("{package}/{version}/");
    let mut has_ledger = false;
    for (name, _) in &members {
        if !name.starts_with(&prefix) {
            return Err(format!(
                "{E2103} bundle member '{name}' is outside {prefix}"
            ));
        }
        if *name == format!("{prefix}{LEDGER_FILE}") {
            has_ledger = true;
        }
    }
    if !has_ledger {
        return Err(format!("{E2103} bundle has no {LEDGER_FILE} ledger"));
    }

    // Stage into a temp dir — the registry target stays untouched until the
    // trust gate has passed.
    let staging =
        tempfile::tempdir().map_err(|e| format!("{E2103} cannot create staging directory: {e}"))?;
    for (name, content) in &members {
        let abs = staging.path().join(&name[prefix.len()..]);
        fs::create_dir_all(abs.parent().expect("entry parent"))
            .map_err(|e| format!("{E2103} cannot stage {}: {e}", abs.display()))?;
        fs::write(&abs, content)
            .map_err(|e| format!("{E2103} cannot stage {}: {e}", abs.display()))?;
    }

    // The trust gate: same gate as publish, refused as a bundle problem.
    let report = snapshot::verify_snapshot(staging.path())
        .map_err(|e| format!("{E2103} bundle failed its trust gate: {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E2103} refusing to import unverifiable bundle {} ({} problem(s)): {}",
            file.display(),
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(staging.path().join(LEDGER_FILE))
            .map_err(|e| format!("{E2103} bundle ledger read: {e}"))?,
    )
    .map_err(|e| format!("{E2103} bundle ledger parse: {e}"))?;
    let build_id = ledger
        .get("build_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let content_hash = ledger
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if build_id != excerpt_entry.build_id || content_hash != excerpt_entry.content_hash {
        return Err(format!(
            "{E2103} bundle {INDEX_FILE} record does not match its ledger (build_id {build_id}, content_hash {content_hash})"
        ));
    }
    let staged_files = files_under(staging.path())?;
    if staged_files.len() != excerpt_entry.files {
        return Err(format!(
            "{E2103} bundle {INDEX_FILE} record claims {} files, ledger covers {}",
            excerpt_entry.files,
            staged_files.len()
        ));
    }

    // Conflict pre-check shares publish's rule so dry runs report the exact
    // outcome a real import would produce.
    let existing_index = read_index(root, &package)?;
    let mut already_identical = false;
    if let Some(existing) = existing_index.entries.iter().find(|e| e.version == version) {
        if existing.content_hash != content_hash {
            return Err(format!(
                "{E1801} registry version conflict: {package}/{version} already published with \
                 content_hash {} — import refused",
                existing.content_hash
            ));
        }
        already_identical = true;
    }
    if dry_run {
        return Ok(ImportReport {
            package,
            version,
            files: staged_files.len(),
            already_identical,
            dry_run: true,
        });
    }

    let report = publish(root, &package, &version, staging.path())?;
    Ok(ImportReport {
        package,
        version,
        files: staged_files.len(),
        already_identical: report.already_identical,
        dry_run: false,
    })
}

/// Result of `push_entry`: what was (or would be) uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    /// Package pushed
    pub package: String,
    /// Entry version pushed (explicitly requested, or dotted-numeric latest
    /// when the version was omitted)
    pub version: String,
    /// Entry files uploaded (the package index PUT is counted separately —
    /// it always lands last)
    pub files: usize,
    /// `true` when the remote already held the exact bytes (idempotent
    /// no-op — zero PUTs), `false` when files were uploaded
    pub already_identical: bool,
    /// `true` when the push ran as a report-only dry run (zero PUTs)
    pub dry_run: bool,
}

/// Push a published entry from a local registry to a remote http(s)
/// registry root (A3, design §47): one PUT per entry file
/// (`<root>/<package>/<version>/<file>`), the package `index.json` written
/// last — merged over the remote's existing entries, so a push never
/// rewrites remote history. Credentials: `auth_env` names the environment
/// variable carrying the bearer token; it must resolve before any network
/// contact (E2105) and the value travels only in the `Authorization`
/// header — never logged, never persisted. `auth_env = None` pushes
/// anonymously (unauthenticated deployments). Failure mapping: the local
/// entry unreadable → E2101; the remote index unreadable or a PUT failing
/// transport after retries → E2101; HTTP 401/403 → E2102; any other
/// refusing status (405/501/…) → E2104 — the server has no write channel.
/// The remote already holding the same version with different bytes is an
/// E1801 conflict (remote history is never rewritten); identical bytes are
/// an idempotent no-op with zero PUTs. With `dry_run` the local read,
/// credential resolution and remote-state check all run, but nothing is
/// uploaded.
pub fn push_entry(
    source_root: &Path,
    remote_root: &str,
    package: &str,
    version: Option<&str>,
    auth_env: Option<&str>,
    dry_run: bool,
) -> Result<PushReport, String> {
    let base = remote_root.trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(format!(
            "{E2101} push targets a remote http(s) registry root, got '{remote_root}'"
        ));
    }

    // Local side first: the entry must read cleanly before anything remote
    // is contacted.
    let (version, entry_dir, rels) = resolve_entry(source_root, package, version)?;
    let record = entry_record(&entry_dir, &version, rels.len())?;

    // Credential resolution precedes any network contact (E2105); only the
    // env var NAME may appear in errors — never the token itself.
    let token = match auth_env {
        Some(name) => {
            let value = std::env::var(name).map_err(|_| {
                format!(
                    "{E2105} push credential missing: env '{name}' is not set (declared via \
                     --auth-env or [registry].auth_env)"
                )
            })?;
            if value.is_empty() {
                return Err(format!(
                    "{E2105} push credential missing: env '{name}' is empty"
                ));
            }
            Some(value)
        }
        None => None,
    };

    // Remote state: the package index decides merge vs no-op vs conflict.
    let index_url = format!("{base}/{package}/index.json");
    let remote_entries: Vec<IndexEntry> = match crate::remote::http_get(&index_url) {
        Ok(bytes) => {
            let remote: RegistryIndex = serde_json::from_slice(&bytes)
                .map_err(|e| format!("{E2101} cannot parse remote index {index_url}: {e}"))?;
            if let Some(existing) = remote.entries.iter().find(|e| e.version == version) {
                if existing.content_hash == record.content_hash {
                    return Ok(PushReport {
                        package: package.to_string(),
                        version,
                        files: rels.len(),
                        already_identical: true,
                        dry_run,
                    });
                }
                return Err(format!(
                    "{E1801} registry version conflict: {package}/{version} already on the remote \
                     with content_hash {} — push refused (remote history is never rewritten)",
                    existing.content_hash
                ));
            }
            remote.entries
        }
        Err(crate::remote::FetchFailure::NotFound(_)) => Vec::new(),
        // An auth rejection on the state probe is an auth rejection, full
        // stop — mapped before the generic unreachable-index case.
        Err(crate::remote::FetchFailure::Status(code)) if code == 401 || code == 403 => {
            return Err(format!(
                "{E2102} push rejected at {index_url}: http status {code}"
            ));
        }
        Err(e) => return Err(format!("{E2101} cannot read remote index {index_url}: {e}")),
    };

    if dry_run {
        return Ok(PushReport {
            package: package.to_string(),
            version,
            files: rels.len(),
            already_identical: false,
            dry_run: true,
        });
    }

    let put = |rel_path: &str, body: &[u8]| -> Result<(), String> {
        crate::remote::http_put(&format!("{base}{rel_path}"), token.as_deref(), body).map_err(|e| {
            match e {
                crate::remote::PutFailure::Transport(detail) => {
                    format!("{E2101} push transport failed for {rel_path}: {detail}")
                }
                crate::remote::PutFailure::AuthRejected(code) => {
                    format!("{E2102} push rejected for {rel_path}: http status {code}")
                }
                crate::remote::PutFailure::WriteRefused(code) => {
                    format!(
                        "{E2104} push refused for {rel_path}: http status {code} (the server has \
                         no write channel — publish locally and host statically)"
                    )
                }
            }
        })
    };

    for rel in &rels {
        let source = entry_dir.join(rel);
        let body = fs::read(&source)
            .map_err(|e| format!("{E2101} cannot read entry file {}: {e}", source.display()))?;
        put(
            &format!("/{package}/{version}/{}", rel.to_string_lossy()),
            &body,
        )?;
    }

    // The package index lands last (design §47): only after every entry
    // file is in place, merged over the remote's existing entries.
    let mut merged = remote_entries;
    merged.retain(|e| e.version != version);
    merged.push(record);
    merged.sort_by(|a, b| {
        version_key(&a.version)
            .cmp(&version_key(&b.version))
            .then_with(|| a.version.cmp(&b.version))
    });
    let merged_index = RegistryIndex {
        package: package.to_string(),
        entries: merged,
    };
    let mut index_bytes = serde_json::to_vec_pretty(&merged_index)
        .map_err(|e| format!("{E2101} cannot serialize merged index: {e}"))?;
    index_bytes.push(b'\n');
    put(&format!("/{package}/index.json"), &index_bytes)?;

    Ok(PushReport {
        package: package.to_string(),
        version,
        files: rels.len(),
        already_identical: false,
        dry_run: false,
    })
}

/// Render a requirement back to a compact comparator list (diagnostics).
fn render_req(req: &VersionReq) -> String {
    req.comparators
        .iter()
        .map(|c| {
            let op = match c.op {
                CmpOp::Eq => "=",
                CmpOp::Gt => ">",
                CmpOp::Ge => ">=",
                CmpOp::Lt => "<",
                CmpOp::Le => "<=",
            };
            format!("{op}{}", c.version)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Deterministic local cache key for a remote registry root (R3): the first
/// 12 hex chars of the URL's blake3, one key scheme shared with the Remote
/// Source cache (§45). Delegates to `crate::remote::cache_key`.
pub fn cache_key(root_url: &str) -> String {
    crate::remote::cache_key(root_url)
}

/// Pick the entry version for a resolved index: an explicit `version` must
/// exist and satisfy `requirement`; with only a requirement, the highest
/// published version satisfying it wins; with neither, the latest
/// (dotted-numeric order). `registry_display` appears in the error text
/// (local roots show the path, remote roots the URL). Pure — shared by the
/// local resolver and the remote (R3) fetcher.
pub fn select_version(
    index: &RegistryIndex,
    version: Option<&str>,
    requirement: Option<&VersionReq>,
    registry_display: &str,
) -> Result<String, String> {
    let package = &index.package;
    if index.entries.is_empty() {
        return Err(format!(
            "{E1802} registry package not found: {package} (registry {registry_display})"
        ));
    }
    let versions: Vec<String> = index
        .entries
        .iter()
        .map(|e| e.version.clone())
        .collect::<Vec<_>>();
    match (version, requirement) {
        (Some(v), _) => {
            if !versions.iter().any(|e| e == v) {
                return Err(format!(
                    "{E1802} registry version not found: {package}@{v} (registry {registry_display}; published: {})",
                    versions.join(", ")
                ));
            }
            if let Some(req) = requirement {
                if !satisfies(v, req) {
                    return Err(format!(
                        "{E1802} registry version {package}@{v} does not satisfy requirement \
                         '{}' (registry {registry_display})",
                        render_req(req)
                    ));
                }
            }
            Ok(v.to_string())
        }
        (None, Some(req)) => {
            let req_text = render_req(req);
            let best = versions
                .iter()
                .filter(|v| satisfies(v, req))
                .max_by(|a, b| version_key(a).cmp(&version_key(b)).then_with(|| a.cmp(b)));
            let Some(best) = best else {
                return Err(format!(
                    "{E1802} no published version of {package} satisfies requirement \
                     '{req_text}' (registry {registry_display}; published: {})",
                    versions.join(", ")
                ));
            };
            Ok(best.clone())
        }
        (None, None) => Ok(latest_version(&versions)
            .expect("non-empty entries")
            .to_string()),
    }
}

/// Resolve a registry reference to an entry directory. `version` omitted
/// means the highest published version (dotted-numeric order). The entry's
/// ledger is verified before it is handed out — resolving a tampered entry
/// is an E1803 error, missing package/version an E1802 error.
pub fn resolve(root: &Path, package: &str, version: Option<&str>) -> Result<PathBuf, String> {
    resolve_pinned(root, package, version, None)
}

/// Resolve a registry reference under a dependency requirement (R2): an
/// explicit `version` must satisfy `requirement` (E1802 otherwise); with
/// only a requirement, the highest published version satisfying it wins.
pub fn resolve_pinned(
    root: &Path,
    package: &str,
    version: Option<&str>,
    requirement: Option<&VersionReq>,
) -> Result<PathBuf, String> {
    let index = read_index(root, package)?;
    let display = root.display().to_string();
    let version = select_version(&index, version, requirement, &display)?;

    let entry = root.join(package).join(&version);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

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
    fn export_bundle_is_deterministic_tar_with_entry_files_and_excerpt() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&root, "common", "1.0.0", &snap).unwrap();
        let snap2 = tmp.path().join("snap2");
        make_snapshot(&snap2, "two");
        publish(&root, "common", "1.9.0", &snap2).unwrap();

        // Two exports of the same entry are byte-identical.
        let out1 = tmp.path().join("b1.tar");
        let report = export_bundle(
            &root,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &out1,
        )
        .unwrap();
        assert_eq!(report.package, "common");
        assert_eq!(report.version, "1.0.0");
        let out2 = tmp.path().join("b2.tar");
        export_bundle(
            &root,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &out2,
        )
        .unwrap();
        assert_eq!(fs::read(&out1).unwrap(), fs::read(&out2).unwrap());

        // A missing version picks the dotted-numeric latest (1.9.0).
        let out3 = tmp.path().join("b3.tar");
        let latest = export_bundle(&root, "common", None, BundleCompression::Plain, &out3).unwrap();
        assert_eq!(latest.version, "1.9.0");

        // The bundle parses as a tar carrying entry files + index excerpt,
        // with the determinism contract visible in the headers.
        let mut archive = tar::Archive::new(fs::File::open(&out1).unwrap());
        let mut names = Vec::new();
        for entry in archive.entries().unwrap() {
            let entry = entry.unwrap();
            names.push(entry.path().unwrap().to_string_lossy().into_owned());
            assert_eq!(entry.header().mtime().unwrap(), 0, "mtime must be zero");
            assert_eq!(entry.header().uid().unwrap(), 0);
            assert_eq!(entry.header().gid().unwrap(), 0);
            assert_eq!(entry.header().mode().unwrap(), 0o644);
        }
        names.sort();
        assert_eq!(
            names,
            vec![
                "common/1.0.0/HASHES.json".to_string(),
                "common/1.0.0/data/client/json/Item.json".to_string(),
                "common/1.0.0/manifest.json".to_string(),
                "common/1.0.0/schema.json".to_string(),
                "index.json".to_string(),
            ]
        );
        // The excerpt records exactly the exported entry.
        let mut ar2 = tar::Archive::new(fs::File::open(&out1).unwrap());
        for entry in ar2.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap() == std::path::Path::new("index.json") {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                let excerpt: RegistryIndex = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(excerpt.package, "common");
                assert_eq!(excerpt.entries.len(), 1);
                assert_eq!(excerpt.entries[0].version, "1.0.0");
            }
        }
    }

    #[test]
    fn export_bundle_zstd_is_byte_deterministic_and_wraps_the_plain_tar() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&root, "common", "1.0.0", &snap).unwrap();

        let plain = tmp.path().join("plain.tar");
        export_bundle(
            &root,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &plain,
        )
        .unwrap();
        let z1 = tmp.path().join("z1.tar.zst");
        let z2 = tmp.path().join("z2.tar.zst");
        export_bundle(&root, "common", Some("1.0.0"), BundleCompression::Zstd, &z1).unwrap();
        export_bundle(&root, "common", Some("1.0.0"), BundleCompression::Zstd, &z2).unwrap();

        // Fixed-level determinism: two exports of the same entry land
        // byte-identical container bytes (within the pinned zstd library —
        // the same scope as the plain-tar golden contract).
        assert_eq!(
            fs::read(&z1).unwrap(),
            fs::read(&z2).unwrap(),
            "fixed level must compress identically every run"
        );

        // The container wraps exactly the plain tar: frame magic at the
        // head, and the decoded payload is byte-identical to the plain
        // export of the same entry.
        let wrapped = fs::read(&z1).unwrap();
        assert!(wrapped.starts_with(&ZSTD_MAGIC), "zstd frame magic");
        assert_ne!(wrapped, fs::read(&plain).unwrap(), "wrapped, not plain");
        let payload = zstd::stream::decode_all(Cursor::new(&wrapped[..])).unwrap();
        assert_eq!(payload, fs::read(&plain).unwrap());
    }

    #[test]
    fn import_bundle_sniffs_zstd_container_and_gates_on_decompressed_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("regA");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&source, "common", "1.0.0", &snap).unwrap();
        let plain = tmp.path().join("plain.tar");
        export_bundle(
            &source,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &plain,
        )
        .unwrap();
        let wrapped = tmp.path().join("wrapped.tar.zst");
        export_bundle(
            &source,
            "common",
            Some("1.0.0"),
            BundleCompression::Zstd,
            &wrapped,
        )
        .unwrap();

        // The compressed bundle imports through the same trust gate: verify
        // clean, resolve, and entry bytes matching the plain form — the
        // ledger hashes uncompressed content, so the container never shows.
        let target = tmp.path().join("regB");
        let report = import_bundle(&target, &wrapped, false).unwrap();
        assert_eq!(report.package, "common");
        assert_eq!(report.version, "1.0.0");
        assert!(!report.already_identical);
        let audit = verify_registry(&target).unwrap();
        assert!(audit.ok(), "{:?}", audit.problems);
        let dir = resolve(&target, "common", Some("1.0.0")).unwrap();
        let source_artifact = source.join("common/1.0.0/data/client/json/Item.json");
        assert_eq!(
            fs::read(dir.join("data/client/json/Item.json")).unwrap(),
            fs::read(source_artifact).unwrap()
        );

        // Dry run of a compressed bundle: full gate, nothing written.
        let dry_target = tmp.path().join("regC");
        let report = import_bundle(&dry_target, &wrapped, true).unwrap();
        assert!(report.dry_run);
        assert!(!dry_target.join("common").exists());

        // Content-based sniff, extension-blind: the compressed form under a
        // plain `.tar` name and the plain form under a `.zst` name both
        // import — the file head, not the suffix, decides.
        let lying_tar = tmp.path().join("actually-zstd.tar");
        fs::copy(&wrapped, &lying_tar).unwrap();
        let lying_zst = tmp.path().join("actually-plain.tar.zst");
        fs::copy(&plain, &lying_zst).unwrap();
        let target_tar = tmp.path().join("regD");
        let report = import_bundle(&target_tar, &lying_tar, false).unwrap();
        assert_eq!(report.version, "1.0.0");
        let target_zst = tmp.path().join("regE");
        let report = import_bundle(&target_zst, &lying_zst, false).unwrap();
        assert_eq!(report.version, "1.0.0");

        // A head claiming the zstd frame but not decoding is a refused
        // bundle (E2103), not a panic and not a silent plain-tar parse.
        let mut corrupt = fs::read(&wrapped).unwrap();
        corrupt.truncate(8);
        let corrupt_path = tmp.path().join("corrupt.tar.zst");
        fs::write(&corrupt_path, &corrupt).unwrap();
        let err = import_bundle(&tmp.path().join("regF"), &corrupt_path, false).unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::distribution::E2103),
            "{err}"
        );
    }

    #[test]
    fn export_bundle_missing_entry_or_bad_name_is_e2101() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&root, "common", "1.0.0", &snap).unwrap();
        let out = tmp.path().join("x.tar");
        let e2101 = crate::error::codes::distribution::E2101;

        // Unknown package / unknown version / path-escape version.
        let err = export_bundle(&root, "ghost", None, BundleCompression::Plain, &out).unwrap_err();
        assert!(err.starts_with(e2101), "{err}");
        let err = export_bundle(
            &root,
            "common",
            Some("9.9.9"),
            BundleCompression::Plain,
            &out,
        )
        .unwrap_err();
        assert!(err.starts_with(e2101), "{err}");
        let err = export_bundle(
            &root,
            "common",
            Some("../evil"),
            BundleCompression::Plain,
            &out,
        )
        .unwrap_err();
        assert!(err.starts_with(e2101), "{err}");
        let err =
            export_bundle(&root, "../evil", None, BundleCompression::Plain, &out).unwrap_err();
        assert!(err.starts_with(e2101), "{err}");

        // An entry stripped of its ledger is not exportable.
        let version_dir = root.join("common/1.0.0");
        fs::remove_file(version_dir.join(LEDGER_FILE)).unwrap();
        let err = export_bundle(
            &root,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &out,
        )
        .unwrap_err();
        assert!(err.starts_with(e2101), "{err}");
    }

    /// Rebuild a bundle with one member's bytes replaced (test tampering:
    /// the content file or the index excerpt) — same deterministic headers.
    fn repack_with_member(src: &Path, dst: &Path, member_suffix: &str, new_bytes: &[u8]) {
        use std::io::Read;
        let data = fs::read(src).unwrap();
        let mut ar = tar::Archive::new(&data[..]);
        let out = fs::File::create(dst).unwrap();
        let mut builder = tar::Builder::new(out);
        for e in ar.entries().unwrap() {
            let mut e = e.unwrap();
            let name = e.path().unwrap().to_string_lossy().into_owned();
            let mut content = Vec::new();
            e.read_to_end(&mut content).unwrap();
            let bytes = if name.ends_with(member_suffix) {
                new_bytes.to_vec()
            } else {
                content
            };
            let mut h = tar::Header::new_ustar();
            h.set_size(bytes.len() as u64);
            h.set_mode(0o644);
            h.set_mtime(0);
            h.set_uid(0);
            h.set_gid(0);
            builder.append_data(&mut h, &name, &bytes[..]).unwrap();
        }
        builder.into_inner().unwrap();
    }

    #[test]
    fn import_bundle_enters_verified_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("regA");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&source, "common", "1.0.0", &snap).unwrap();
        let bundle = tmp.path().join("b.tar");
        export_bundle(
            &source,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &bundle,
        )
        .unwrap();

        let target = tmp.path().join("regB");
        let report = import_bundle(&target, &bundle, false).unwrap();
        assert_eq!(report.package, "common");
        assert_eq!(report.version, "1.0.0");
        assert!(!report.already_identical);
        assert!(!report.dry_run);

        // The imported entry is a first-class citizen: full-registry audit
        // clean, resolve works, bytes match the source entry.
        let audit = verify_registry(&target).unwrap();
        assert!(audit.problems.is_empty(), "{:?}", audit.problems);
        let dir = resolve(&target, "common", Some("1.0.0")).unwrap();
        assert!(dir.ends_with("common/1.0.0"));
        let artifact = fs::read(dir.join("data/client/json/Item.json")).unwrap();
        assert_eq!(artifact, b"one".to_vec());

        // Re-importing the same bundle is an idempotent no-op.
        let again = import_bundle(&target, &bundle, false).unwrap();
        assert!(again.already_identical);
    }

    #[test]
    fn import_bundle_refuses_tamper_conflict_and_respects_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("regA");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&source, "common", "1.0.0", &snap).unwrap();
        let bundle = tmp.path().join("b.tar");
        export_bundle(
            &source,
            "common",
            Some("1.0.0"),
            BundleCompression::Plain,
            &bundle,
        )
        .unwrap();
        let e2103 = crate::error::codes::distribution::E2103;

        // Tampered entry bytes fail the riding ledger at the trust gate.
        let tampered = tmp.path().join("tampered.tar");
        repack_with_member(&bundle, &tampered, "Item.json", b"hacked");
        let target = tmp.path().join("regB");
        let err = import_bundle(&target, &tampered, false).unwrap_err();
        assert!(err.starts_with(e2103), "{err}");
        assert!(
            !target.join("common").exists(),
            "refused bytes must not touch the registry"
        );

        // An index excerpt doctored to claim another content_hash also
        // fails (excerpt ⇔ ledger cross-check) — the JSON itself is valid,
        // so only the cross-check can catch this one.
        let doctored = tmp.path().join("doctored.tar");
        let data = fs::read(&bundle).unwrap();
        let mut ar = tar::Archive::new(&data[..]);
        let mut fake_excerpt_bytes = Vec::new();
        for e in ar.entries().unwrap() {
            let mut e = e.unwrap();
            if e.path().unwrap() == std::path::Path::new(INDEX_FILE) {
                std::io::Read::read_to_end(&mut e, &mut fake_excerpt_bytes).unwrap();
            }
        }
        let mut fake: RegistryIndex = serde_json::from_slice(&fake_excerpt_bytes).unwrap();
        fake.entries[0].content_hash = "0".repeat(24);
        let mut fake_bytes = serde_json::to_vec_pretty(&fake).unwrap();
        fake_bytes.push(b'\n');
        repack_with_member(&bundle, &doctored, INDEX_FILE, &fake_bytes);
        let err = import_bundle(&target, &doctored, false).unwrap_err();
        assert!(err.starts_with(e2103), "{err}");

        // Same version, different bytes already in the target → E1801.
        let other = tmp.path().join("snap2");
        make_snapshot(&other, "two");
        publish(&target, "common", "1.0.0", &other).unwrap();
        let err = import_bundle(&target, &bundle, false).unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::registry::E1801),
            "{err}"
        );

        // Dry run reports without writing: a fresh target stays empty even
        // though the real import would succeed.
        let dry_target = tmp.path().join("regC");
        let report = import_bundle(&dry_target, &bundle, true).unwrap();
        assert!(report.dry_run);
        assert!(!report.already_identical);
        assert!(!dry_target.join("common").exists());
    }

    /// A stand-in remote registry for `push_entry`: stores PUT bodies,
    /// serves GETs from the store, records (method, path, auth header) per
    /// request, and can be forced to answer a fixed status for everything
    /// (401 = auth rejection, 405 = no write channel). `std::net` only.
    struct PushServer {
        port: u16,
        stop: Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl PushServer {
        fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn shutdown(self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            self.handle.map(std::thread::JoinHandle::join);
        }
    }

    type PushServerState = (
        PushServer,
        Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        Arc<Mutex<Vec<String>>>,
    );

    fn start_push_server(force_status: Option<u16>) -> PushServerState {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};

        let store: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let store2 = store.clone();
        let requests2 = requests.clone();
        let handle = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                // Read the head, then exactly Content-Length body bytes.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break buf.len(),
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break pos + 4;
                            }
                            if buf.len() > 1024 * 1024 {
                                break buf.len();
                            }
                        }
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end.min(buf.len())]).into_owned();
                let mut lines = head.split("\r\n");
                let request_line = lines.next().unwrap_or("");
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("/").to_string();
                let mut content_length = 0usize;
                let mut auth = String::new();
                for line in lines {
                    let lower = line.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                    if lower.starts_with("authorization:") {
                        auth = line["authorization:".len()..].trim().to_string();
                    }
                }
                let mut body = buf[head_end.min(buf.len())..].to_vec();
                while body.len() < content_length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&chunk[..n]),
                    }
                }
                requests2
                    .lock()
                    .unwrap()
                    .push(format!("{method} {path} {auth}"));
                let (status, reason, payload): (u16, &str, Vec<u8>) =
                    match (force_status, method.as_str()) {
                        // GETs serve from the store regardless — a static
                        // host reads fine even when it refuses writes.
                        (_, "GET") => match store2.lock().unwrap().get(&path) {
                            Some(bytes) => (200, "OK", bytes.clone()),
                            None => (404, "Not Found", b"not found".to_vec()),
                        },
                        (Some(code), "PUT") => (code, "Refused", b"refused".to_vec()),
                        (None, "PUT") => {
                            store2.lock().unwrap().insert(path.clone(), body);
                            (200, "OK", Vec::new())
                        }
                        _ => (405, "Method Not Allowed", b"method".to_vec()),
                    };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.flush();
            }
        });
        (
            PushServer {
                port,
                stop,
                handle: Some(handle),
            },
            store,
            requests,
        )
    }

    fn put_count(requests: &[String]) -> usize {
        requests.iter().filter(|r| r.starts_with("PUT ")).count()
    }

    #[test]
    fn push_entry_uploads_files_then_merged_index() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("regA");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&source, "common", "1.0.0", &snap).unwrap();
        let snap2 = tmp.path().join("snap2");
        make_snapshot(&snap2, "two");
        publish(&source, "common", "0.2.0", &snap2).unwrap();

        let (server, store, requests) = start_push_server(None);
        std::env::set_var("CAGE_TOK_B", "tok-b");
        let report = push_entry(
            &source,
            &server.url(),
            "common",
            Some("0.2.0"),
            Some("CAGE_TOK_B"),
            false,
        )
        .unwrap();
        assert_eq!(report.package, "common");
        assert_eq!(report.version, "0.2.0");
        assert!(!report.already_identical);
        assert!(!report.dry_run);
        assert_eq!(report.files, 4);

        // Order: index GET first (anonymous — the read protocol is), entry
        // files next, the package index PUT dead last; the token rides only
        // the PUTs.
        let log = requests.lock().unwrap().clone();
        assert_eq!(log[0], "GET /common/index.json ");
        assert_eq!(log.last().unwrap(), "PUT /common/index.json Bearer tok-b");
        assert_eq!(put_count(&log), 5, "4 entry files + merged index");
        assert!(log
            .iter()
            .filter(|r| r.starts_with("PUT "))
            .all(|r| r.ends_with("Bearer tok-b")));

        // The merged index holds exactly the pushed entry (remote was
        // empty), and a second push of another version merges over it.
        let stored = store
            .lock()
            .unwrap()
            .get("/common/index.json")
            .cloned()
            .unwrap();
        let index: RegistryIndex = serde_json::from_slice(&stored).unwrap();
        assert_eq!(
            index
                .entries
                .iter()
                .map(|e| e.version.clone())
                .collect::<Vec<_>>(),
            ["0.2.0"]
        );
        push_entry(
            &source,
            &server.url(),
            "common",
            Some("1.0.0"),
            Some("CAGE_TOK_B"),
            false,
        )
        .unwrap();
        let stored = store
            .lock()
            .unwrap()
            .get("/common/index.json")
            .cloned()
            .unwrap();
        let index: RegistryIndex = serde_json::from_slice(&stored).unwrap();
        // dotted-numeric order: 0.2.0 sorts before 1.0.0
        assert_eq!(
            index
                .entries
                .iter()
                .map(|e| e.version.clone())
                .collect::<Vec<_>>(),
            ["0.2.0", "1.0.0"]
        );

        // Re-pushing identical bytes is a zero-PUT no-op (one GET for the
        // remote state, nothing else).
        let before = requests.lock().unwrap().len();
        let again = push_entry(
            &source,
            &server.url(),
            "common",
            Some("0.2.0"),
            Some("CAGE_TOK_B"),
            false,
        )
        .unwrap();
        assert!(again.already_identical);
        let log = requests.lock().unwrap().clone();
        assert_eq!(log.len(), before + 1);
        assert_eq!(put_count(&log[before..]), 0);
        server.shutdown();
        std::env::remove_var("CAGE_TOK_B");
    }

    #[test]
    fn push_entry_maps_auth_refusals_and_missing_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("regA");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&source, "common", "1.0.0", &snap).unwrap();

        // 401 from the remote → E2102, credentials riding every request.
        let (server, _store, requests) = start_push_server(Some(401));
        std::env::set_var("CAGE_TOK_A", "sekrit-token");
        let err = push_entry(
            &source,
            &server.url(),
            "common",
            Some("1.0.0"),
            Some("CAGE_TOK_A"),
            false,
        )
        .unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::distribution::E2102),
            "{err}"
        );
        assert!(
            !err.contains("sekrit-token"),
            "the token value never leaks into errors: {err}"
        );
        // Anonymous state probe first, then the token rides the refused PUT;
        // the push fails fast — exactly one PUT attempted.
        let log = requests.lock().unwrap().clone();
        assert_eq!(log[0], "GET /common/index.json ");
        assert_eq!(put_count(&log), 1);
        assert!(log[1..]
            .iter()
            .all(|r| r.starts_with("PUT ") && r.ends_with("Bearer sekrit-token")));
        server.shutdown();
        std::env::remove_var("CAGE_TOK_A");

        // 405 → E2104: the server has no write channel.
        let (server, _store, _requests) = start_push_server(Some(405));
        let err =
            push_entry(&source, &server.url(), "common", Some("1.0.0"), None, false).unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::distribution::E2104),
            "{err}"
        );
        server.shutdown();

        // auth_env unset → E2105 before any network contact: the target is
        // a closed port, so any connection attempt would surface as E2101.
        let err = push_entry(
            &source,
            "http://127.0.0.1:1",
            "common",
            Some("1.0.0"),
            Some("CAGE_TOK_MISSING"),
            false,
        )
        .unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::distribution::E2105),
            "{err}"
        );
        // An empty token is as good as absent.
        std::env::set_var("CAGE_TOK_EMPTY", "");
        let err = push_entry(
            &source,
            "http://127.0.0.1:1",
            "common",
            Some("1.0.0"),
            Some("CAGE_TOK_EMPTY"),
            false,
        )
        .unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::distribution::E2105),
            "{err}"
        );
        std::env::remove_var("CAGE_TOK_EMPTY");
    }

    #[test]
    fn push_entry_conflict_refuses_and_dry_run_uploads_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("regA");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        publish(&source, "common", "1.0.0", &snap).unwrap();

        // Dry run: the remote state check happens, nothing is stored.
        let (server, store, requests) = start_push_server(None);
        let report =
            push_entry(&source, &server.url(), "common", Some("1.0.0"), None, true).unwrap();
        assert!(report.dry_run);
        assert!(!report.already_identical);
        assert!(store.lock().unwrap().is_empty(), "dry run stores nothing");
        assert_eq!(put_count(&requests.lock().unwrap()), 0);
        server.shutdown();

        // Same version, different bytes already on the remote → E1801.
        let (server, store, _requests) = start_push_server(None);
        push_entry(&source, &server.url(), "common", Some("1.0.0"), None, false).unwrap();
        let other = tmp.path().join("snap2");
        make_snapshot(&other, "hacked");
        let conflicting = tmp.path().join("regB");
        publish(&conflicting, "common", "1.0.0", &other).unwrap();
        let err = push_entry(
            &conflicting,
            &server.url(),
            "common",
            Some("1.0.0"),
            None,
            false,
        )
        .unwrap_err();
        assert!(
            err.starts_with(crate::error::codes::registry::E1801),
            "{err}"
        );
        // The remote index was not rewritten by the refused push.
        let index: RegistryIndex =
            serde_json::from_slice(store.lock().unwrap().get("/common/index.json").unwrap())
                .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_ne!(index.entries[0].content_hash, "");
        server.shutdown();
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

    fn comparators(spec: &str) -> Vec<(CmpOp, String)> {
        parse_version_req(spec)
            .unwrap()
            .comparators
            .into_iter()
            .map(|c| (c.op, c.version))
            .collect()
    }

    fn req(spec: &str) -> VersionReq {
        parse_version_req(spec).unwrap()
    }

    #[test]
    fn version_req_expands_caret_and_tilde() {
        use CmpOp::{Eq, Ge, Gt, Le, Lt};
        // A bare spec is exact; explicit operators pass through.
        assert_eq!(comparators("1.2.3"), [(Eq, "1.2.3".to_string())]);
        assert_eq!(
            comparators(">=1.0, <2.0"),
            [(Ge, "1.0".to_string()), (Lt, "2.0".to_string())]
        );
        assert_eq!(comparators(">1.2"), [(Gt, "1.2".to_string())]);
        assert_eq!(comparators("<=1.2"), [(Le, "1.2".to_string())]);
        // Caret: bump the left-most non-zero, everything after drops to zero.
        assert_eq!(
            comparators("^1.2"),
            [(Ge, "1.2".to_string()), (Lt, "2".to_string())]
        );
        assert_eq!(
            comparators("^0.2.3"),
            [(Ge, "0.2.3".to_string()), (Lt, "0.3".to_string())]
        );
        assert_eq!(
            comparators("^0.0.3"),
            [(Ge, "0.0.3".to_string()), (Lt, "0.0.4".to_string())]
        );
        // Tilde: lock every component but the second-to-last given.
        assert_eq!(
            comparators("~1.2"),
            [(Ge, "1.2".to_string()), (Lt, "1.3".to_string())]
        );
        assert_eq!(
            comparators("~1"),
            [(Ge, "1.0".to_string()), (Lt, "2".to_string())]
        );
        assert_eq!(
            comparators("~1.2.3"),
            [(Ge, "1.2.3".to_string()), (Lt, "1.3".to_string())]
        );
    }

    #[test]
    fn version_req_satisfies_uses_zero_padded_order() {
        let caret = req("^1.2");
        assert!(satisfies("1.2.0", &caret));
        assert!(satisfies("1.9.0", &caret));
        assert!(satisfies("1.10.0", &caret)); // dotted-numeric: 1.10 > 1.9
        assert!(!satisfies("1.1.0", &caret));
        assert!(!satisfies("2.0.0", &caret));

        let zero_caret = req("^0.2.3");
        assert!(satisfies("0.2.3", &zero_caret));
        assert!(satisfies("0.2.9", &zero_caret));
        assert!(!satisfies("0.3.0", &zero_caret));

        let tilde = req("~1.2");
        assert!(satisfies("1.2.0", &tilde));
        assert!(satisfies("1.2.7", &tilde));
        assert!(!satisfies("1.3.0", &tilde));

        let exact = req("1.2.3");
        assert!(satisfies("1.2.3", &exact));
        assert!(!satisfies("1.2.30", &exact));

        // >=1.2 must not exclude 1.2.0 (comparisons zero-pad).
        assert!(satisfies("1.2.0", &req(">=1.2")));
        assert!(satisfies("1.2.0", &req("<=1.2")));
        assert!(satisfies("2.0.0", &req(">1.9, <=2.0")));
        assert!(!satisfies("2.0.1", &req(">1.9, <=2.0")));
    }

    #[test]
    fn version_req_rejects_malformed_specs() {
        for spec in ["", "   ", "abc", ">= 1.0,", "^x", "~", "1..0"] {
            let err = parse_version_req(spec).unwrap_err();
            assert!(err.contains("E1802"), "{spec}: {err}");
        }
    }

    #[test]
    fn resolve_pinned_picks_max_satisfying_and_gates_explicit_pins() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");
        let snap = tmp.path().join("snap");
        make_snapshot(&snap, "one");
        for v in ["1.0.0", "1.9.0", "2.0.0"] {
            publish(&root, "common", v, &snap).unwrap();
        }

        // Versionless + requirement → highest satisfying version.
        let dir = resolve_pinned(&root, "common", None, Some(&req(">=1.0, <2.0"))).unwrap();
        assert!(dir.ends_with("1.9.0"));
        let dir = resolve_pinned(&root, "common", None, Some(&req("^1.0.0"))).unwrap();
        assert!(dir.ends_with("1.9.0"));
        let dir = resolve_pinned(&root, "common", None, Some(&req("1.0.0"))).unwrap();
        assert!(dir.ends_with("1.0.0"));
        // Explicit version still inside the requirement passes.
        let dir = resolve_pinned(&root, "common", Some("1.0.0"), Some(&req("^1.0"))).unwrap();
        assert!(dir.ends_with("1.0.0"));

        // Explicit version outside the requirement → E1802.
        let err = resolve_pinned(&root, "common", Some("2.0.0"), Some(&req("<2.0"))).unwrap_err();
        assert!(err.contains("E1802"), "{err}");
        assert!(err.contains("does not satisfy"), "{err}");

        // Requirement nothing satisfies → E1802 with the published list.
        let err = resolve_pinned(&root, "common", None, Some(&req(">=3.0"))).unwrap_err();
        assert!(err.contains("E1802"), "{err}");
        assert!(err.contains("satisfies requirement"), "{err}");
        assert!(err.contains("2.0.0"), "{err}");

        // No pin, no requirement → plain latest (unchanged R1 behavior).
        let dir = resolve_pinned(&root, "common", None, None).unwrap();
        assert!(dir.ends_with("2.0.0"));
    }

    #[test]
    fn select_version_is_pure_and_drives_the_remote_fetcher() {
        let index = RegistryIndex {
            package: "common".to_string(),
            entries: vec![
                IndexEntry {
                    version: "1.0.0".to_string(),
                    build_id: "b".to_string(),
                    content_hash: "c".to_string(),
                    files: 3,
                },
                IndexEntry {
                    version: "1.9.0".to_string(),
                    build_id: "b".to_string(),
                    content_hash: "c".to_string(),
                    files: 3,
                },
                IndexEntry {
                    version: "2.0.0".to_string(),
                    build_id: "b".to_string(),
                    content_hash: "c".to_string(),
                    files: 3,
                },
            ],
        };
        // Same decision table as the local resolver — the remote fetcher
        // (R3) runs this exact function on the fetched index.
        assert_eq!(
            select_version(&index, None, Some(&req(">=1.0, <2.0")), "https://r").unwrap(),
            "1.9.0"
        );
        assert_eq!(
            select_version(&index, None, None, "https://r").unwrap(),
            "2.0.0"
        );
        assert_eq!(
            select_version(&index, Some("1.0.0"), None, "https://r").unwrap(),
            "1.0.0"
        );
        let err =
            select_version(&index, Some("2.0.0"), Some(&req("<2.0")), "https://r").unwrap_err();
        assert!(err.contains("E1802"), "{err}");
        assert!(err.contains("does not satisfy"), "{err}");
        let err = select_version(&index, None, Some(&req(">=3.0")), "https://r").unwrap_err();
        assert!(
            err.contains("E1802") && err.contains("satisfies requirement"),
            "{err}"
        );
        let empty = RegistryIndex {
            package: "ghost".to_string(),
            entries: Vec::new(),
        };
        let err = select_version(&empty, None, None, "https://r").unwrap_err();
        assert!(err.contains("not found"), "{err}");

        // Cache keys are deterministic and short enough for a path segment.
        assert_eq!(
            cache_key("https://r/example"),
            cache_key("https://r/example")
        );
        assert_ne!(cache_key("https://r/a"), cache_key("https://r/b"));
        assert_eq!(cache_key("https://r/example").len(), 12);
    }

    #[test]
    fn verify_gc_and_remove_govern_the_registry_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("reg");

        // Three versions of one package.
        for (v, content) in [("1.0.0", "one"), ("1.9.0", "two"), ("2.0.0", "three")] {
            let snap = tmp.path().join("snap");
            if snap.exists() {
                fs::remove_dir_all(&snap).unwrap();
            }
            make_snapshot(&snap, content);
            publish(&root, "common", v, &snap).unwrap();
        }

        // Clean registry verifies clean.
        let report = verify_registry(&root).unwrap();
        assert!(report.ok(), "{:?}", report.problems);
        assert_eq!(report.packages, 1);
        assert_eq!(report.entries_checked, 3);

        // A hand-edit inside an entry breaks its byte hashes; the index
        // record now disagrees with the (self-inconsistent) ledger.
        fs::write(
            root.join("common/1.0.0/data/client/json/Item.json"),
            "forged",
        )
        .unwrap();
        let report = verify_registry(&root).unwrap();
        assert!(!report.ok(), "tampered entry must be reported");
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("E1803") && p.contains("common/1.0.0")),
            "{:?}",
            report.problems
        );

        // gc keeps the newest window (default surfaced as 3; here keep 2)
        // and sweeps an orphan directory with no index record. The dry run
        // computes the identical removal list first and touches nothing.
        fs::create_dir_all(root.join("common/0.5.0")).unwrap();
        let plan = gc_registry(&root, 2, true).unwrap();
        assert_eq!(
            plan.removed,
            vec![
                "common/1.0.0".to_string(),
                "common/0.5.0 (orphan)".to_string()
            ]
        );
        assert_eq!(plan.rewritten, 1);
        assert!(root.join("common/1.0.0").is_dir());
        assert!(root.join("common/0.5.0").is_dir());
        let report = gc_registry(&root, 2, false).unwrap();
        assert_eq!(
            report.removed,
            vec![
                "common/1.0.0".to_string(),
                "common/0.5.0 (orphan)".to_string()
            ]
        );
        assert_eq!(report.rewritten, 1);
        let index = read_index(&root, "common").unwrap();
        let versions: Vec<String> = index.entries.iter().map(|e| e.version.clone()).collect();
        assert_eq!(versions, vec!["1.9.0", "2.0.0"]);
        assert!(root.join("common/1.9.0").is_dir());
        assert!(!root.join("common/1.0.0").exists());

        // keep is floored at one — a registry never loses its whole history.
        gc_registry(&root, 0, false).unwrap();
        assert!(root.join("common/2.0.0").is_dir());
        let index = read_index(&root, "common").unwrap();
        assert_eq!(index.entries.len(), 1);

        // The GC'd window is really gone for consumers.
        let err = resolve(&root, "common", Some("1.0.0")).unwrap_err();
        assert!(err.contains("E1802"), "{err}");

        // Explicit remove takes one version out; the package index stays
        // (empty), and the same snapshot republishes cleanly afterwards —
        // removal is explicit history editing, re-publication is not a
        // conflict.
        remove_entry(&root, "common", "2.0.0", false).unwrap();
        let index = read_index(&root, "common").unwrap();
        assert_eq!(index.entries.len(), 0);
        assert!(!root.join("common/2.0.0").exists());
        let report = verify_registry(&root).unwrap();
        assert!(report.ok(), "{:?}", report.problems);

        let err = remove_entry(&root, "common", "9.9.9", false).unwrap_err();
        assert!(err.contains("E1802"), "{err}");
        let err = remove_entry(&root, "../escape", "1.0.0", false).unwrap_err();
        assert!(err.contains("E1801"), "{err}");

        let snap = tmp.path().join("snap2");
        make_snapshot(&snap, "three");
        publish(&root, "common", "2.0.0", &snap).unwrap();
        let index = read_index(&root, "common").unwrap();
        assert_eq!(index.entries.len(), 1);
    }
}
