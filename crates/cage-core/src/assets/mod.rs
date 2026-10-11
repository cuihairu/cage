//! Asset inventory (design §50, B1): the source-side asset ledger.
//!
//! `cage assets` walks the three asset classes a project is made of —
//! rule assets (schema files + the migration chain), source assets (the
//! files under `[source_roots]`), and the build-rule asset (`cage.toml`)
//! — and records one blake3 per file, sorted by path so the report and
//! the committed baseline are byte-stable.
//!
//! Remote specs (`registry:…` / `http(s)://` / `gsheet:` / `mysql:` /
//! `pg:`) are deliberately **not** walked: they are version-locked pins
//! (registry) or remote sources with their own contract, not files in
//! the workspace — the manifest's `dependencies` + `source_hash` cover
//! them. `.cage-cache/` and `build/` are rebuildable, never inventoried.

use crate::manifest::ProjectConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;

/// Asset classes of the taxonomy (design §50): what the data should look
/// like (rule), the data itself (source), how to build (build).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetClass {
    /// Schema files + migration chain — the rule fact source.
    Rule,
    /// Authoring data under `[source_roots]`.
    Source,
    /// `cage.toml` — profiles / targets / registry / remote config.
    Build,
}

impl fmt::Display for AssetClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Rule => "rule",
            Self::Source => "source",
            Self::Build => "build",
        };
        f.write_str(text)
    }
}

/// One inventoried asset: project-relative path, class, blake3, size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRecord {
    /// Project-relative path, forward slashes, no leading `./`.
    pub path: String,
    /// Asset class (rule / source / build).
    pub class: AssetClass,
    /// blake3 of the file bytes (house hash, same as HASHES.json).
    pub hash: String,
    /// File size in bytes.
    pub size: u64,
}

/// The full inventory of one project at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetInventory {
    /// Project name (`[project].name`).
    pub project: String,
    /// cage version that produced the baseline (informational).
    pub cage_version: String,
    /// Assets sorted by path.
    pub assets: Vec<AssetRecord>,
}

/// Result of verifying a working tree against a saved baseline.
#[derive(Debug, Default)]
pub struct VerificationReport {
    /// `true` when the tree matches the baseline exactly.
    pub ok: bool,
    /// Number of assets re-hashed and found identical.
    pub matched: usize,
    /// One line per problem, in path order.
    pub problems: Vec<String>,
}

const CONFIG_NAMES: [&str; 4] = ["cage.toml", "cage.yaml", "cage.yml", "cage.json"];

/// Inventory the project's assets (fail closed: an unreadable file is an
/// error, never a silent skip — an inventory that hides files defeats
/// its purpose).
pub fn inventory(root: &Path, config: &ProjectConfig) -> Result<AssetInventory, String> {
    let mut assets: Vec<AssetRecord> = Vec::new();

    let config_name = CONFIG_NAMES
        .iter()
        .find(|name| root.join(name).is_file())
        .ok_or_else(|| {
            format!(
                "no project config found in {} (expected cage.toml/cage.yaml/cage.json)",
                root.display()
            )
        })?;
    let record = record(root, config_name, AssetClass::Build)?;
    assets.push(record);

    // Rule assets: the schema file set (design §50: rules live inside the
    // schema — no separate rules/ directory), then the migration chain.
    if let Some(schema_path) = &config.schema_path {
        if !is_remote_spec(schema_path) {
            collect_files(&root.join(schema_path), &mut assets, AssetClass::Rule, root)?;
        }
    }
    let migrations = root.join("migrations");
    if migrations.is_dir() {
        collect_files(&migrations, &mut assets, AssetClass::Rule, root)?;
    }

    // Source assets: local source roots only (remote specs are pins).
    let roots: BTreeSet<&String> = config.source_roots.values().collect();
    for spec in roots {
        if is_remote_spec(spec) {
            continue;
        }
        collect_files(&root.join(spec), &mut assets, AssetClass::Source, root)?;
    }

    assets.sort_by(|a, b| a.path.cmp(&b.path).then(a.class.cmp(&b.class)));
    Ok(AssetInventory {
        project: config.project.name.clone(),
        cage_version: env!("CARGO_PKG_VERSION").to_string(),
        assets,
    })
}

/// Verify the working tree against a saved baseline: missing, changed,
/// and unlisted (never recorded) assets are all reported; a clean match
/// reports `ok = true`.
pub fn verify(
    root: &Path,
    config: &ProjectConfig,
    baseline: &AssetInventory,
) -> Result<VerificationReport, String> {
    if baseline.project.is_empty() {
        return Err("baseline has no project name — is it an asset inventory file?".to_string());
    }
    if baseline.project != config.project.name {
        return Err(format!(
            "baseline is for project '{}' but this project is '{}'",
            baseline.project, config.project.name
        ));
    }
    let current = inventory(root, config)?;
    let current: BTreeSet<&str> = current.assets.iter().map(|a| a.path.as_str()).collect();
    let saved: BTreeSet<&str> = baseline.assets.iter().map(|a| a.path.as_str()).collect();

    let mut report = VerificationReport::default();
    for asset in &baseline.assets {
        match current.iter().find(|p| **p == asset.path.as_str()) {
            None => report.problems.push(format!("missing   {}", asset.path)),
            Some(_) => report.matched += 1,
        }
    }
    for path in current.difference(&saved) {
        report.problems.push(format!("unlisted  {path}"));
    }
    // Re-hash matched paths: a same-path file with different bytes is the
    // integrity case the baseline exists to catch.
    for asset in &baseline.assets {
        if !current.contains(asset.path.as_str()) {
            continue;
        }
        let actual = record(root, &asset.path, asset.class)?;
        if actual.hash != asset.hash || actual.size != asset.size {
            report.problems.push(format!(
                "changed   {} ({} {} B → {} {} B)",
                asset.path, asset.hash, asset.size, actual.hash, actual.size
            ));
        }
    }
    report.problems.sort();
    report.ok = report.problems.is_empty();
    Ok(report)
}

/// `true` for source roots / schema paths that are not files in the
/// workspace (versioned pins and remote sources — design §50 taxonomy).
fn is_remote_spec(spec: &str) -> bool {
    spec.starts_with("registry:")
        || spec.starts_with("gsheet:")
        || spec.starts_with("mysql:")
        || spec.starts_with("pg:")
        || spec.contains("://")
}

/// One record for one path (the caller guarantees the file exists).
fn record(root: &Path, rel: &str, class: AssetClass) -> Result<AssetRecord, String> {
    let abs = root.join(rel);
    let bytes = fs::read(&abs).map_err(|e| format!("failed to read {}: {e}", abs.display()))?;
    Ok(AssetRecord {
        path: to_rel(rel),
        class,
        hash: blake3::hash(&bytes).to_hex().to_string(),
        size: bytes.len() as u64,
    })
}

/// Recursively inventory every regular file under `dir` as one class.
fn collect_files(
    dir: &Path,
    out: &mut Vec<AssetRecord>,
    class: AssetClass,
    root: &Path,
) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("failed to read {}: {e}", dir.display()))?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read {}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to inspect {}: {e}", dir.join(&name).display()))?;
        if file_type.is_dir() {
            collect_files(&dir.join(&name), out, class, root)?;
        } else if file_type.is_file() {
            names.push(name);
        } else {
            return Err(format!(
                "unsupported file type at {} (expected a regular file or directory)",
                dir.join(&name).display()
            ));
        }
    }
    // Path-order traversal keeps the pre-sort stable; the sort in
    // `inventory` is the real determinism anchor.
    names.sort();
    for name in names {
        let rel = rel_of(&dir.join(&name), root);
        out.push(record(root, &rel, class)?);
    }
    Ok(())
}

/// Project-relative, forward-slashed path (always present — every path
/// passed in is already project-relative by construction).
fn rel_of(abs: &Path, root: &Path) -> String {
    abs.strip_prefix(root).map_or_else(
        |_| abs.to_string_lossy().replace('\\', "/"),
        |p| p.to_string_lossy().replace('\\', "/"),
    )
}

/// Same normalization for the caller-supplied relative path.
fn to_rel(rel: &str) -> String {
    rel.replace('\\', "/")
}

#[cfg(test)]
mod tests;
