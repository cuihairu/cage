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
/// 12 hex chars of the URL's blake3. Pure — the network layer lives in the
/// CLI; core only fixes the key so every consumer derives the same cache.
pub fn cache_key(root_url: &str) -> String {
    blake3::hash(root_url.as_bytes()).to_hex()[..12].to_string()
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
