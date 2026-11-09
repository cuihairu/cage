//! Read-only inspection commands: table inspect, schema diff, dependency
//! graph (text/dot/json), asset inventory, and build verification. Split
//! out of the CLI entry.

use indexmap::IndexMap;
use std::fs;
use std::path::{Path, PathBuf};

use cage_core::assets::AssetInventory;
use cage_core::manifest::{BuildManifest, ProjectConfig};
use cage_core::reference::DependencyGraph;
use cage_core::schema::Schema;

use crate::{load_project, load_project_config, load_schema_for_config};
pub(crate) fn run_inspect(path: &Path, table: Option<&str>, no_cache: bool) -> i32 {
    let project = match load_project(path, no_cache) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match table {
        None => {
            println!("project: {}", project.config.project.name);
            if let Some(v) = &project.config.project.version {
                println!("version: {v}");
            }
            println!("tables:");
            for (name, table) in &project.document.tables {
                println!("  {} ({} rows)", name, table.rows.len());
            }
            println!("schemas:");
            for (name, schema_table) in &project.schema.tables {
                println!(
                    "  {} ({} fields, primary key: {})",
                    name,
                    schema_table.fields.len(),
                    schema_table.primary_key.join(", ")
                );
            }
            0
        }
        Some(name) => match project.schema.tables.get(name) {
            None => {
                eprintln!("error: unknown table '{name}'");
                2
            }
            Some(schema_table) => {
                println!("table: {}", schema_table.name);
                if let Some(d) = &schema_table.description {
                    println!("description: {d}");
                }
                println!("primary key: {}", schema_table.primary_key.join(", "));
                let rows = project
                    .document
                    .tables
                    .get(name)
                    .map_or(0, |t| t.rows.len());
                println!("rows: {rows}");
                println!("fields:");
                for (fname, field) in &schema_table.fields {
                    println!(
                        "  {}: {:?}{}",
                        fname,
                        field.field_type,
                        if field.required { " (required)" } else { "" }
                    );
                }
                0
            }
        },
    }
}

pub(crate) fn run_diff(baseline: &Path, target: &Path) -> i32 {
    let base = match load_manifest(baseline) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let new = match load_manifest(target) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut same = 0usize;

    for (path, info) in &new.artifacts {
        match base.artifacts.get(path) {
            None => added.push(path.clone()),
            Some(old) if old.hash != info.hash => changed.push(path.clone()),
            Some(_) => same += 1,
        }
    }
    for path in base.artifacts.keys() {
        if !new.artifacts.contains_key(path) {
            removed.push(path.clone());
        }
    }

    // Layered summary (U3): what moved semantically, above the per-file
    // listing — tables (the manifest's per-table fingerprints) and the
    // targets whose outputs sit under any moved file.
    let mut changed_tables: Vec<&String> = base
        .table_hashes
        .keys()
        .chain(new.table_hashes.keys())
        .filter(|t| base.table_hashes.get(*t) != new.table_hashes.get(*t))
        .collect();
    changed_tables.sort();
    changed_tables.dedup();
    let mut targets: Vec<&str> = base
        .targets
        .iter()
        .chain(new.targets.iter())
        .map(|t| t.output_dir.as_str())
        .collect();
    targets.sort_unstable();
    targets.dedup();
    let touches = |file: &str, dir: &str| {
        let dir = dir.trim_end_matches('/');
        file == dir || file.starts_with(&format!("{dir}/"))
    };
    let affected: Vec<&str> = targets
        .iter()
        .filter(|dir| {
            added
                .iter()
                .chain(removed.iter())
                .chain(changed.iter())
                .any(|f| touches(f, dir))
        })
        .copied()
        .collect();

    for p in &added {
        println!("+ {p}");
    }
    for p in &removed {
        println!("- {p}");
    }
    for p in &changed {
        println!("~ {p}");
    }
    println!(
        "cage diff: {} added, {} removed, {} changed, {} unchanged",
        added.len(),
        removed.len(),
        changed.len(),
        same
    );
    if !changed_tables.is_empty() {
        println!(
            "tables changed ({}): {}",
            changed_tables.len(),
            changed_tables
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !affected.is_empty() {
        println!("targets affected: {}", affected.join(", "));
    }
    if base.content_hash != new.content_hash {
        println!(
            "content_hash: {} -> {}",
            base.content_hash, new.content_hash
        );
    }
    0
}

/// The `cage graph` view: reference edges (with the referencing fields as
/// labels, in schema order), the topological build order, and any cycles.
pub(crate) struct GraphView {
    pub(crate) project: String,
    pub(crate) tables: usize,
    pub(crate) edges: Vec<(String, String, Vec<String>)>,
    pub(crate) order: Vec<String>,
    pub(crate) cycles: Vec<Vec<String>>,
}

/// Build the graph view from the merged schema: the same `DependencyGraph`
/// the reference check and the incremental planner consume.
pub(crate) fn build_graph_view(config: &ProjectConfig, schema: &Schema) -> GraphView {
    let graph = DependencyGraph::from_schema(schema);
    let mut edges: IndexMap<(&str, &str), Vec<&str>> = IndexMap::new();
    for (table, table_schema) in &schema.tables {
        for (field_name, field_schema) in &table_schema.fields {
            if let Some(reference) = &field_schema.reference {
                edges
                    .entry((table.as_str(), reference.table.as_str()))
                    .or_default()
                    .push(field_name.as_str());
            }
        }
    }
    GraphView {
        project: config.project.name.clone(),
        tables: schema.tables.len(),
        edges: edges
            .into_iter()
            .map(|((from, to), fields)| {
                (
                    from.to_string(),
                    to.to_string(),
                    fields.into_iter().map(String::from).collect(),
                )
            })
            .collect(),
        order: graph.topological_sort().unwrap_or_default(),
        cycles: graph.find_cycles(),
    }
}

pub(crate) fn render_graph_text(view: &GraphView) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "project: {}", view.project);
    let _ = writeln!(out, "tables: {}", view.tables);
    let _ = writeln!(out, "edges: {}", view.edges.len());
    for (from, to, fields) in &view.edges {
        let _ = writeln!(out, "  {from} -> {to} ({})", fields.join(", "));
    }
    if view.order.is_empty() && !view.cycles.is_empty() {
        out.push_str("build order: (cycles present)\n");
    } else {
        let _ = writeln!(out, "build order: {}", view.order.join(", "));
    }
    if view.cycles.is_empty() {
        out.push_str("cycles: 0\n");
    } else {
        let _ = writeln!(out, "cycles: {}", view.cycles.len());
        for cycle in &view.cycles {
            let _ = writeln!(out, "  {} -> {}", cycle.join(" -> "), cycle[0]);
        }
    }
    out
}

pub(crate) fn render_graph_dot(view: &GraphView) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("digraph config {\n  rankdir=LR;\n");
    for (from, to, fields) in &view.edges {
        let _ = writeln!(
            out,
            "  \"{from}\" -> \"{to}\" [label=\"{}\"];",
            fields.join(", ")
        );
    }
    out.push_str("}\n");
    out
}

pub(crate) fn render_graph_json(view: &GraphView) -> String {
    let edges: Vec<serde_json::Value> = view
        .edges
        .iter()
        .map(|(from, to, fields)| serde_json::json!({"from": from, "to": to, "fields": fields}))
        .collect();
    let payload = serde_json::json!({
        "project": view.project,
        "tables": view.tables,
        "edges": edges,
        "build_order": view.order,
        "cycles": view.cycles,
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
}

/// `cage graph` — the table reference dependency graph (design §30). Schema
/// reference declarations feed the same `DependencyGraph` the reference
/// check and the incremental planner consume; output is text (default),
/// Graphviz dot, or json. Cycles are reported in the rendering, never fatal
/// — the reference check owns the error, this command is the view.
pub(crate) fn run_graph(path: &Path, format: &str) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let schema = match load_schema_for_config(path, &config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let view = build_graph_view(&config, &schema);
    let body = match format {
        "text" => render_graph_text(&view),
        "dot" => render_graph_dot(&view),
        "json" => render_graph_json(&view),
        other => {
            eprintln!("error: unknown format '{other}' (text | dot | json)");
            return 2;
        }
    };
    print!("{body}");
    0
}

/// `cage assets` — inventory (default), `--json`, `--write <baseline>`,
/// or `--check <baseline>` (design §50 B1: the source-side asset ledger).
pub(crate) fn run_assets(
    path: &Path,
    json: bool,
    write: Option<&Path>,
    check: Option<&Path>,
) -> i32 {
    if let Some(baseline) = check {
        return run_assets_check(path, baseline);
    }
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let inventory = match cage_core::assets::inventory(path, &config) {
        Ok(inv) => inv,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if let Some(file) = write {
        let bytes = assets_json(&inventory);
        return match fs::write(file, bytes) {
            Ok(()) => {
                println!(
                    "assets: wrote {} asset(s) to {}",
                    inventory.assets.len(),
                    file.display()
                );
                0
            }
            Err(e) => {
                eprintln!("error: cannot write {}: {e}", file.display());
                2
            }
        };
    }
    if json {
        println!("{}", assets_json(&inventory));
    } else {
        print_assets(&inventory);
    }
    0
}

/// `cage assets --check <baseline>`: verify the working tree against the
/// saved ledger — clean match exits 0, any missing / changed / unlisted
/// asset is named and exits 1.
pub(crate) fn run_assets_check(path: &Path, baseline_path: &Path) -> i32 {
    let baseline = match fs::read_to_string(baseline_path) {
        Ok(text) => match serde_json::from_str::<AssetInventory>(&text) {
            Ok(inv) => inv,
            Err(e) => {
                eprintln!(
                    "error: invalid asset inventory {}: {e}",
                    baseline_path.display()
                );
                return 2;
            }
        },
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", baseline_path.display());
            return 2;
        }
    };
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match cage_core::assets::verify(path, &config, &baseline) {
        Ok(report) => {
            if report.ok {
                println!("assets: {} asset(s) verified", report.matched);
                0
            } else {
                println!(
                    "assets: {} asset(s) verified, {} problem(s):",
                    report.matched,
                    report.problems.len()
                );
                for problem in &report.problems {
                    println!("  {problem}");
                }
                1
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Pretty JSON form of the inventory (the baseline file content and the
/// `--json` report — same bytes, same determinism contract).
pub(crate) fn assets_json(inventory: &AssetInventory) -> String {
    serde_json::to_string_pretty(inventory).expect("inventory serializes")
}

/// Human-readable inventory report: path-aligned, class-tagged, full hash.
pub(crate) fn print_assets(inventory: &AssetInventory) {
    println!("project: {}", inventory.project);
    println!("cage: {}", inventory.cage_version);
    println!("assets: {}", inventory.assets.len());
    println!();
    let width = inventory
        .assets
        .iter()
        .map(|a| a.path.len())
        .max()
        .unwrap_or(0);
    for asset in &inventory.assets {
        println!(
            "{:<6}  {:<width$}  {:>7} B  blake3:{}",
            asset.class, asset.path, asset.size, asset.hash
        );
    }
}

/// One artifact's on-disk verdict against its manifest ledger entry.
enum ArtifactVerdict {
    Ok,
    Missing,
    SizeMismatch { recorded: usize, actual: usize },
    HashMismatch { recorded: String, actual: String },
}

/// `cage verify` — audit generated artifacts on disk against the build
/// manifest ledger (design §30). Project-root input anchors at the root
/// (manifest keys are project-relative); a bare output dir / manifest path
/// also resolves keys with the output-dir prefix stripped, the layout a
/// deployed build tree usually has. Snapshot packs keep their own
/// loader-side check (`cage snapshot --verify`); this audits raw build
/// trees. Exit 0 clean, 1 findings, 2 usage.
pub(crate) fn run_verify(path: &Path) -> i32 {
    // Project root (cage.toml parses) → manifest from the configured
    // output_dir; otherwise the path is the manifest's own dir/file.
    let (manifest, manifest_path, base) = match load_project_config(path) {
        Ok(config) => {
            let output_dir = config.output_dir.as_deref().unwrap_or("build");
            let dir = path.join(output_dir);
            match load_manifest(&dir) {
                Ok(m) => {
                    let manifest_path = dir.join("manifest.json");
                    (m, manifest_path, path.to_path_buf())
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    eprintln!(
                        "hint: build first (cage build {path})",
                        path = path.display()
                    );
                    return 2;
                }
            }
        }
        Err(_) => match load_manifest(path) {
            Ok(m) => {
                let manifest_path = if path.is_dir() {
                    path.join("manifest.json")
                } else {
                    path.to_path_buf()
                };
                let base = manifest_path
                    .parent()
                    .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
                (m, manifest_path, base)
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
    };

    // Keys are project-relative ("build/client/json/Item.json"); a bare
    // output dir carries the files one level down, so also try the key with
    // the leading component stripped. Both candidates are ledgered keys —
    // nothing outside the manifest is ever read.
    let resolve = |key: &str| -> Option<PathBuf> {
        let plain = base.join(key);
        if plain.exists() {
            return Some(plain);
        }
        let stripped = key.split_once('/')?.1;
        let stripped_path = base.join(stripped);
        stripped_path.exists().then_some(stripped_path)
    };

    println!("manifest: {}", manifest_path.display());
    println!("profile: {}", manifest.profile);
    if let Some(env) = &manifest.environment {
        println!("environment: {env}");
    }
    println!("build_id: {}", manifest.build_id);
    println!("content_hash: {}", manifest.content_hash);
    println!("artifacts: {}", manifest.artifacts.len());

    let mut problems = 0usize;
    for (key, info) in &manifest.artifacts {
        let verdict = match resolve(key) {
            None => ArtifactVerdict::Missing,
            Some(file) => match std::fs::read(&file) {
                Err(_) => ArtifactVerdict::Missing,
                Ok(bytes) => {
                    if bytes.len() == info.size {
                        let actual = blake3::hash(&bytes).to_hex().to_string();
                        if actual == info.hash {
                            ArtifactVerdict::Ok
                        } else {
                            ArtifactVerdict::HashMismatch {
                                recorded: info.hash.clone(),
                                actual,
                            }
                        }
                    } else {
                        ArtifactVerdict::SizeMismatch {
                            recorded: info.size,
                            actual: bytes.len(),
                        }
                    }
                }
            },
        };
        match verdict {
            ArtifactVerdict::Ok => println!("ok      {key}"),
            ArtifactVerdict::Missing => {
                problems += 1;
                println!("FAILED  {key} (missing)");
            }
            ArtifactVerdict::SizeMismatch { recorded, actual } => {
                problems += 1;
                println!("FAILED  {key} (size {actual} != recorded {recorded})");
            }
            ArtifactVerdict::HashMismatch { recorded, actual } => {
                problems += 1;
                println!(
                    "FAILED  {key} (hash {actual} != recorded {recorded}) — rebuild \
                     (the file changed or was tampered with after the build)"
                );
            }
        }
    }
    if problems == 0 {
        println!(
            "cage verify: {} artifact(s) verified, 0 problems",
            manifest.artifacts.len()
        );
        0
    } else {
        println!(
            "cage verify: {} artifact(s), {} problem(s)",
            manifest.artifacts.len(),
            problems
        );
        1
    }
}

pub(crate) fn load_manifest(path: &Path) -> Result<BuildManifest, String> {
    let manifest_path = if path.is_dir() {
        path.join("manifest.json")
    } else {
        path.to_path_buf()
    };
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("invalid manifest {}: {e}", manifest_path.display()))
}

/// A manifest-sourced path may only steer file operations when it is a
/// normalized project-relative path: non-empty, relative, and every
/// component normal (no root, no prefix, no `..`). `.` components are
/// skipped by `Path::components` and join to the same file, so they are
/// harmless. Guards the incremental carry/delete paths, which join these
/// strings onto the project root (review 2026-10-18 表一#1).
pub(crate) fn manifest_rel_path_is_safe(rel: &str) -> bool {
    use std::path::Component;
    let p = Path::new(rel);
    !p.as_os_str().is_empty()
        && p.is_relative()
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Second line of defense: the canonical path must stay inside the
/// canonical root (blocks symlink escapes). Unresolvable paths count as
/// outside — there is nothing to read or delete at them anyway.
pub(crate) fn resolved_within_root(full: &Path, root: &Path) -> bool {
    match (full.canonicalize(), root.canonicalize()) {
        (Ok(p), Ok(r)) => p.starts_with(&r),
        _ => false,
    }
}
