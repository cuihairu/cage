//! Cage CLI - command-line interface for the Cage configuration compiler.
//!
//! Wires the full pipeline (T5.2): project loading, source adapters,
//! schema loading, validation level selection, normalization, target
//! generators and the deterministic build manifest.

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: the Value/normalize layer is a numeric coercion engine —
// float<->int casts are its job, not a defect.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::approx_constant,
    clippy::checked_conversions
)]
// Stage: crate-prefixed type names (JsonTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: Diagnostics is the deliberate first-class error type,
// returned by value from every pipeline stage.
#![allow(clippy::result_large_err)]
// Restriction lints kept off for a 0.1.0 codebase.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names
)]
use clap::{Parser, Subcommand};
use indexmap::IndexMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cage_core::manifest::{BuildManifest, ManifestGenerator, ProjectConfig, TargetConfig};
use cage_core::normalize::normalize_document;
use cage_core::reference::{DependencyGraph, IncrementalPlanner};
use cage_core::schema::{Schema, ValidatedSchema};
use cage_core::validation::ValidationLevel;
use cage_core::value::Document;
use cage_core::Diagnostics;
use cage_core::DocumentMetadata;

/// Game configuration compilation and validation framework
#[derive(Parser)]
#[command(
    name = "cage",
    version,
    about = "Game configuration compilation and validation framework",
    long_about = "Cage compiles heterogeneous configuration sources (Excel/CSV/JSON/YAML) into validated, deterministically built runtime configuration assets."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate only, do not generate runtime artifacts
    Check {
        /// Configuration project root directory
        path: PathBuf,
        /// Highest validation level to run (parse/schema/type/value/table/reference/semantic/gamerule)
        #[arg(long, default_value = "semantic")]
        level: String,
        /// Build profile to check against
        #[arg(long, default_value = "client")]
        profile: String,
    },
    /// Validate and generate target artifacts
    Build {
        /// Configuration project root directory
        path: PathBuf,
        /// Highest validation level to run before building
        #[arg(long, default_value = "semantic")]
        level: String,
        /// Build profile to generate artifacts for
        #[arg(long, default_value = "client")]
        profile: String,
        /// Skip rebuilding if hashes match last build's manifest
        #[arg(long)]
        incremental: bool,
    },
    /// View Schema and configuration structure
    Inspect {
        /// Configuration project root directory
        path: PathBuf,
        /// Table name (lists all tables when omitted)
        table: Option<String>,
    },
    /// Generate code-target artifacts only (cs/python/lua/ts/js/cpp/go/java), no data validation
    Gen {
        /// Configuration project root directory
        path: PathBuf,
        /// Build profile to generate code for
        #[arg(long, default_value = "client")]
        profile: String,
    },
    /// Compare artifacts of two configuration builds
    Diff {
        /// Baseline build directory (or manifest.json)
        baseline: PathBuf,
        /// Target build directory (or manifest.json)
        target: PathBuf,
    },
    /// Build and package a self-verifying Configuration Snapshot, or verify
    /// an existing snapshot directory without rebuilding
    Snapshot {
        /// Project root directory (or snapshot directory with --verify)
        path: PathBuf,
        /// Build profile to snapshot
        #[arg(long, default_value = "client")]
        profile: String,
        /// Verify an existing snapshot directory instead of building
        #[arg(long)]
        verify: bool,
    },
}

/// A loaded Cage project: config + merged schema + merged document.
struct Project {
    config: ProjectConfig,
    schema: Schema,
    document: Document,
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Commands::Check {
            path,
            level,
            profile,
        } => run_check(&path, &level, &profile),
        Commands::Build {
            path,
            level,
            profile,
            incremental,
        } => run_build(&path, &level, &profile, incremental),
        Commands::Inspect { path, table } => run_inspect(&path, table.as_deref()),
        Commands::Gen { path, profile } => run_gen(&path, &profile),
        Commands::Diff { baseline, target } => run_diff(&baseline, &target),
        Commands::Snapshot {
            path,
            profile,
            verify,
        } => {
            if verify {
                run_snapshot_verify(&path)
            } else {
                run_snapshot(&path, &profile)
            }
        }
    };
    std::process::exit(code);
}

/// Load project config (cage.toml / cage.yaml / cage.yml / cage.json), merge
/// all schema files and parse all declared source roots into one Document.
fn load_project(root: &Path) -> Result<Project, String> {
    let config = load_project_config(root)?;

    let schema = match &config.schema_path {
        Some(rel) => load_schema(&root.join(rel))?,
        None => Schema::new(),
    };

    let mut document = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    for rel in config.source_roots.values() {
        let doc = load_sources(&root.join(rel))?;
        for (name, table) in doc.tables {
            document.tables.insert(name, table);
        }
        document.source_files.extend(doc.source_files);
    }
    document.source_files.sort();
    document.source_files.dedup();

    Ok(Project {
        config,
        schema,
        document,
    })
}

fn load_project_config(root: &Path) -> Result<ProjectConfig, String> {
    for name in ["cage.toml", "cage.yaml", "cage.yml", "cage.json"] {
        let path = root.join(name);
        if path.is_file() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let config =
                match path.extension().and_then(|e| e.to_str()) {
                    Some("toml") => {
                        toml::from_str(&content).map_err(|e| format!("invalid cage.toml: {e}"))?
                    }
                    Some("json") => serde_json::from_str(&content)
                        .map_err(|e| format!("invalid cage.json: {e}"))?,
                    _ => serde_yaml::from_str(&content)
                        .map_err(|e| format!("invalid cage.yaml: {e}"))?,
                };
            return Ok(config);
        }
    }
    Err(format!(
        "no project config found in {} (expected cage.toml/cage.yaml/cage.json)",
        root.display()
    ))
}

fn load_schema(path: &Path) -> Result<Schema, String> {
    let mut merged = Schema::new();
    for file in collect_files(path, &["yaml", "yml", "json"])? {
        let content = std::fs::read_to_string(&file)
            .map_err(|e| format!("failed to read {}: {e}", file.display()))?;
        let schema: Schema = match file.extension().and_then(|e| e.to_str()) {
            Some("json") => serde_json::from_str(&content)
                .map_err(|e| format!("invalid schema {}: {e}", file.display()))?,
            _ => serde_yaml::from_str(&content)
                .map_err(|e| format!("invalid schema {}: {e}", file.display()))?,
        };
        for (name, table) in schema.tables {
            merged.tables.insert(name, table);
        }
        for (name, enum_schema) in schema.enums {
            merged.enums.insert(name, enum_schema);
        }
        if merged.metadata.is_none() {
            merged.metadata = schema.metadata;
        }
    }
    Ok(merged)
}

fn load_sources(root: &Path) -> Result<Document, String> {
    let mut merged = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    for file in collect_files(root, &["json", "yaml", "yml", "csv", "xlsx", "xls"])? {
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let result = match ext.as_str() {
            "json" => cage_source_json::JsonSourceAdapter::parse_file(&file),
            "yaml" | "yml" => cage_source_yaml::YamlSourceAdapter::parse_file(&file),
            "csv" => cage_source_csv::CsvSourceAdapter::default().parse_file(&file),
            "xlsx" | "xls" => cage_source_excel::ExcelSourceAdapter::default().parse_file(&file),
            _ => continue,
        };
        match result {
            Ok(doc) => {
                for (name, table) in doc.tables {
                    merged.tables.insert(name, table);
                }
                merged.source_files.extend(doc.source_files);
            }
            Err(diags) => {
                eprintln!("{}", diags.render(false));
                return Err(format!("failed to parse {}", file.display()));
            }
        }
    }
    Ok(merged)
}

/// Recursively collect files under `path` (a file or directory) with the
/// given lowercase extensions, sorted for deterministic order.
fn collect_files(path: &Path, exts: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if path.is_file() {
        files.push(path.to_path_buf());
    } else if path.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| format!("failed to read dir {}: {e}", dir.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| exts.contains(&e.to_lowercase().as_str()))
                {
                    files.push(p);
                }
            }
        }
    } else {
        return Err(format!("path not found: {}", path.display()));
    }
    files.sort();
    Ok(files)
}

/// Keep only tables/fields visible to `profile`: an empty `targets` list
/// means "all targets"; a non-empty list restricts visibility.
fn filter_by_profile<'a>(
    schema: &'a Schema,
    document: &'a Document,
    profile: &str,
) -> (Schema, Document) {
    let mut filtered_schema = schema.clone();
    filtered_schema
        .tables
        .retain(|_, t| t.targets.is_empty() || t.targets.iter().any(|t| t == profile || t == "*"));
    for table in filtered_schema.tables.values_mut() {
        table.fields.retain(|_, f| {
            f.targets.is_empty() || f.targets.iter().any(|t| t == profile || t == "*")
        });
    }

    let mut filtered_doc = document.clone();
    filtered_doc
        .tables
        .retain(|name, _| filtered_schema.tables.contains_key(name));
    for (table_name, table) in &mut filtered_doc.tables {
        if let Some(schema_table) = filtered_schema.tables.get(table_name) {
            table.rows.iter_mut().for_each(|row| {
                row.fields
                    .retain(|name, _| schema_table.fields.contains_key(name));
            });
        }
    }
    (filtered_schema, filtered_doc)
}

fn run_check(path: &Path, level: &str, profile: &str) -> i32 {
    let level = match level.parse::<ValidationLevel>() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match load_project(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let (_, document) = filter_by_profile(&project.schema, &project.document, profile);
    // Profile-aware validation runs on the FULL schema/document with the
    // active profile in the validation context: E9006 fires when the
    // projection would silently drop a structurally required field (Profile
    // 语义化——报冲突码而非静默过滤), and every table's data stays checked —
    // profiles gate runtime views, not data quality. `document` (filtered)
    // below only feeds the summary count.
    let validated = ValidatedSchema {
        schema: project.schema.clone(),
        dependency_graph: DependencyGraph::from_schema(&project.schema),
    };
    let diagnostics = cage_core::validation::validate_with_profile(
        &validated,
        &project.document,
        level,
        project.config.warnings_as_errors,
        Some(profile),
    );
    if !diagnostics.is_empty() {
        println!("{}", diagnostics.render(false));
    }
    if diagnostics.has_errors() {
        println!(
            "cage check: FAILED ({} errors, {} warnings)",
            diagnostics.errors().len(),
            diagnostics.warnings().len()
        );
        1
    } else {
        println!(
            "cage check: OK ({} tables, {} warnings, level <= {})",
            document.tables.len(),
            diagnostics.warnings().len(),
            level.as_str()
        );
        0
    }
}

/// Everything `build_project` produced — shared by `cage build` and
/// `cage snapshot`.
struct BuildOutput {
    /// Active profile
    profile: String,
    /// Profile (filtered) schema — the view the artifacts were built from
    schema: Schema,
    /// Artifact records: (project-relative path, bytes, format, owning table)
    artifacts: Vec<(String, Vec<u8>, String, Option<String>)>,
    /// Project-relative build root (config `output_dir`)
    output_dir: String,
    /// Whether layer-2 incremental engaged (report says regenerated/carried)
    layer2: bool,
    /// Number of artifacts carried over from disk when layer 2 engaged
    carried_count: usize,
    /// Generated build manifest
    manifest: BuildManifest,
    /// Written manifest location
    manifest_path: PathBuf,
}

/// Build failures mapped to exit codes by the callers (1 = validation,
/// 2 = usage/I/O).
enum BuildFailure {
    /// Validation produced errors (rendered before reporting)
    Validation(Diagnostics),
    /// Usage or I/O errors (printed as `error: {msg}`)
    Io(String),
    /// Layer-1 incremental skip: the last build's inputs are unchanged and
    /// every artifact is intact
    UpToDate { artifacts: usize, manifest: PathBuf },
}

fn build_project(
    path: &Path,
    level: &str,
    profile: &str,
    incremental: bool,
) -> Result<BuildOutput, BuildFailure> {
    let level = match level.parse::<ValidationLevel>() {
        Ok(l) => l,
        Err(e) => return Err(BuildFailure::Io(e)),
    };
    let project = match load_project(path) {
        Ok(p) => p,
        Err(e) => return Err(BuildFailure::Io(e)),
    };
    let Some(build_profile) = project.config.profiles.get(profile) else {
        let msg = format!(
            "unknown profile '{profile}' (available: {})",
            project
                .config
                .profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Err(BuildFailure::Io(msg));
    };

    let (schema, document) = filter_by_profile(&project.schema, &project.document, profile);
    // Profile-aware validation on the FULL pair (see run_check): E9006 for
    // structurally broken projections; all data stays checked regardless of
    // profile. The filtered pair below drives hashing and generation only.
    let validated = ValidatedSchema {
        schema: project.schema.clone(),
        dependency_graph: DependencyGraph::from_schema(&project.schema),
    };
    let diagnostics = cage_core::validation::validate_with_profile(
        &validated,
        &project.document,
        level,
        project.config.warnings_as_errors,
        Some(profile),
    );
    if !diagnostics.is_empty() {
        println!("{}", diagnostics.render(false));
    }
    if diagnostics.has_errors() {
        return Err(BuildFailure::Validation(diagnostics));
    }

    let normalized = normalize_document(&document);

    let (schema_hash, source_hash) = ManifestGenerator::input_hashes(&schema, &normalized);
    let output_dir = project.config.output_dir.as_deref().unwrap_or("build");
    let manifest_dir = path.join(output_dir);

    // Incremental build, two layers (docs/build.md 增量构建):
    // Layer 1: skip regeneration entirely when the previous manifest recorded
    // the same schema/source hashes (same profile) and every artifact it
    // lists is still on disk. Target config changes are NOT hashed — re-run
    // a full build after editing cage.toml targets.
    // Layer 2 (dependency-graph propagation): when only data changed (schema
    // hash identical — the schema drives code-target shapes, so any schema
    // change falls back to a full build), per-table hashes detect which
    // tables changed, the dependency graph propagates the change set to its
    // transitive dependents, and only affected tables are regenerated;
    // untouched artifacts are carried over from disk. Generators are
    // deterministic, so the carried bytes equal a full build's — the merged
    // manifest matches a full build byte-for-byte. Layer 2 also falls back to
    // a full build when the previous manifest predates per-table hashes, a
    // table was deleted, or an untouched artifact is missing/unreadable.
    let mut affected: Option<HashSet<String>> = None;
    let mut carried: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    if incremental {
        if let Ok(prev) = load_manifest(&manifest_dir) {
            let unchanged = prev.profile == profile
                && prev.schema_hash == schema_hash
                && prev.source_hash == source_hash
                && prev.artifacts.keys().all(|rel| path.join(rel).is_file());
            if unchanged {
                return Err(BuildFailure::UpToDate {
                    artifacts: prev.artifacts.len(),
                    manifest: manifest_dir.join("manifest.json"),
                });
            }
            let layer2_ok = prev.profile == profile
                && prev.schema_hash == schema_hash
                && !prev.table_hashes.is_empty()
                && prev
                    .table_hashes
                    .keys()
                    .all(|t| normalized.tables.contains_key(t));
            if layer2_ok {
                let cur = ManifestGenerator::table_hashes(&normalized);
                let mut changed: Vec<String> = cur
                    .keys()
                    .filter(|name| prev.table_hashes.get(*name) != cur.get(*name))
                    .cloned()
                    .collect();
                for (rel, info) in &prev.artifacts {
                    if let Some(table) = &info.table {
                        if !changed.contains(table) && !path.join(rel).is_file() {
                            changed.push(table.clone());
                        }
                    }
                }
                changed.sort();
                let graph = DependencyGraph::from_schema(&schema);
                let planner = IncrementalPlanner::new(&graph);
                let aff = planner.compute_affected(&changed);
                let mut carry_ok = true;
                for (rel, info) in &prev.artifacts {
                    // Shared units (enum files) are schema-wide and are
                    // always regenerated — never carried.
                    let Some(table) = &info.table else { continue };
                    if aff.contains(table) {
                        continue;
                    }
                    match std::fs::read(path.join(rel)) {
                        Ok(bytes) => carried.push((
                            rel.clone(),
                            bytes,
                            info.format.clone(),
                            info.table.clone(),
                        )),
                        Err(e) => {
                            eprintln!(
                                "warning: carried artifact '{rel}' unreadable ({e}); \
                                 falling back to a full build"
                            );
                            carried.clear();
                            carry_ok = false;
                            break;
                        }
                    }
                }
                if carry_ok {
                    affected = Some(aff);
                }
            }
        }
    }

    // Generation inputs: full when layer 2 is off, otherwise filtered to the
    // affected tables. Code targets keep the full enum set (the shared enums
    // unit renders from schema-wide enums even when only some tables rebuild).
    let (gen_schema, gen_doc) = match &affected {
        Some(keep) => {
            let mut fs = schema.clone();
            fs.tables.retain(|name, _| keep.contains(name));
            let mut fd = normalized.clone();
            fd.tables.retain(|name, _| keep.contains(name));
            (fs, fd)
        }
        None => (schema.clone(), normalized.clone()),
    };

    let carried_len = carried.len();
    let mut artifacts = carried;
    for target in &build_profile.targets {
        let generated = match code_target_items(target, &gen_schema, &schema_hash) {
            // Code targets (cs/python/lua/ts/…) are schema-driven and infallible.
            Some(items) => Ok(items),
            None => match target.format.as_str() {
                "json" => cage_target_json::JsonTargetGenerator::from_config(target)
                    .generate(&gen_doc, &[]),
                "csv" => {
                    cage_target_csv::CsvTargetGenerator::from_config(target).generate(&gen_doc, &[])
                }
                other => {
                    return Err(BuildFailure::Io(format!(
                        "unsupported target format '{other}'"
                    )));
                }
            },
        };
        match generated {
            Ok(items) => {
                write_artifact_files(path, items, &target.format, &mut artifacts)
                    .map_err(BuildFailure::Io)?;
            }
            Err(diags) => return Err(BuildFailure::Validation(diags)),
        }
    }
    // Canonical artifact order (path-sorted) so a layer-2 incremental run and
    // a full build produce byte-identical manifests (same inputs → same
    // manifest bytes, the determinism contract).
    artifacts.sort_by(|a, b| a.0.cmp(&b.0));

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .generate(&schema, &normalized, &artifacts);
    let manifest_path = match write_manifest(&manifest_dir, &manifest) {
        Ok(p) => p,
        Err(e) => return Err(BuildFailure::Io(e)),
    };

    Ok(BuildOutput {
        profile: profile.to_string(),
        schema,
        artifacts,
        output_dir: output_dir.to_string(),
        layer2: affected.is_some(),
        carried_count: carried_len,
        manifest,
        manifest_path,
    })
}

fn run_build(path: &Path, level: &str, profile: &str, incremental: bool) -> i32 {
    match build_project(path, level, profile, incremental) {
        Ok(out) => {
            if out.layer2 {
                println!(
                    "cage build: OK (profile '{}', {} artifacts — {} regenerated, \
                     {} unchanged via dependency graph, manifest {})",
                    out.profile,
                    out.artifacts.len(),
                    out.artifacts.len() - out.carried_count,
                    out.carried_count,
                    out.manifest_path.display()
                );
            } else {
                println!(
                    "cage build: OK (profile '{}', {} artifacts, manifest {})",
                    out.profile,
                    out.artifacts.len(),
                    out.manifest_path.display()
                );
            }
            0
        }
        Err(BuildFailure::Validation(diags)) => {
            println!("{}", diags.render(false));
            println!(
                "cage build: FAILED validation ({} errors)",
                diags.errors().len()
            );
            1
        }
        Err(BuildFailure::Io(msg)) => {
            eprintln!("error: {msg}");
            2
        }
        Err(BuildFailure::UpToDate {
            artifacts,
            manifest,
        }) => {
            println!(
                "cage build: up to date (profile '{profile}', {artifacts} artifacts, manifest {})",
                manifest.display()
            );
            0
        }
    }
}

/// `cage snapshot` — build the profile, then package the build into a
/// self-verifying Configuration Snapshot under
/// `<output_dir>/snapshot/<profile>-<build_id[..12]>`. The directory name is
/// a deterministic fingerprint, not a date: identical inputs → identical
/// snapshot bytes (the determinism contract). Packing lives in
/// `cage_core::snapshot` (the format is core's, not the cli's).
fn run_snapshot(path: &Path, profile: &str) -> i32 {
    // A snapshot packages a fresh full build — no incremental carry-over.
    match build_project(path, "gamerule", profile, false) {
        Ok(out) => {
            let schema_json = match serde_json::to_vec_pretty(&out.schema) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: schema serialization: {e}");
                    return 2;
                }
            };
            let manifest_json = match std::fs::read(&out.manifest_path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("error: {}: {e}", out.manifest_path.display());
                    return 2;
                }
            };
            let files = cage_core::snapshot::snapshot_files(
                &manifest_json,
                &schema_json,
                &out.artifacts,
                &out.output_dir,
                &out.manifest.build_id,
                &out.manifest.content_hash,
            );
            let snap_dir = path.join(&out.output_dir).join("snapshot").join(format!(
                "{}-{}",
                profile,
                &out.manifest.build_id[..12]
            ));
            if let Err(e) = write_snapshot_files(&snap_dir, &files) {
                eprintln!("error: {e}");
                return 2;
            }
            // Self-check: the just-written snapshot must verify clean.
            let report = match cage_core::snapshot::verify_snapshot(&snap_dir) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: snapshot self-verification: {e}");
                    return 2;
                }
            };
            println!(
                "cage snapshot: OK (profile '{profile}', {} files, {} artifacts, verified, {})",
                files.len(),
                out.artifacts.len(),
                snap_dir.display()
            );
            debug_assert!(
                report.ok,
                "self-verification mismatch: {:?}",
                report.mismatches
            );
            0
        }
        Err(BuildFailure::Validation(diags)) => {
            println!("{}", diags.render(false));
            println!(
                "cage snapshot: FAILED validation ({} errors)",
                diags.errors().len()
            );
            1
        }
        Err(BuildFailure::Io(msg)) => {
            eprintln!("error: {msg}");
            2
        }
        Err(BuildFailure::UpToDate { .. }) => {
            // incremental is false for snapshots — unreachable.
            eprintln!("error: internal: snapshot build did not rebuild");
            2
        }
    }
}

/// `cage snapshot --verify <dir>` — the load-time check the server would run
/// (same core entry: `cage_core::snapshot::verify_snapshot` / `load`).
fn run_snapshot_verify(dir: &Path) -> i32 {
    match cage_core::snapshot::verify_snapshot(dir) {
        Ok(report) => {
            if report.ok {
                println!(
                    "cage snapshot: verified ({} — {} files checked)",
                    dir.display(),
                    report.files_checked
                );
                0
            } else {
                for m in &report.mismatches {
                    println!("mismatch: {m}");
                }
                println!(
                    "cage snapshot: VERIFICATION FAILED ({} problem(s))",
                    report.mismatches.len()
                );
                1
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Write a snapshot file map under `dir`, creating parents as needed.
fn write_snapshot_files(dir: &Path, files: &IndexMap<String, Vec<u8>>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (rel, bytes) in files {
        let abs = dir.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&abs, bytes).map_err(|e| format!("cannot write {}: {e}", abs.display()))?;
    }
    Ok(())
}

/// `cage gen` — schema-driven code generation only. Writes the code-target
/// artifacts (cs/python/lua/ts/js/cpp/go/java) of a profile without running
/// data validation:
/// types and metadata all come from the Schema, so source rows are not
/// needed. Data targets (json/csv) in the profile are skipped — run
/// `cage build` for those. The manifest is written like a build's, so gen
/// and build manifests share the same 口径 (schema/source/content hashes).
fn run_gen(path: &Path, profile: &str) -> i32 {
    let project = match load_project(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let Some(build_profile) = project.config.profiles.get(profile) else {
        eprintln!(
            "error: unknown profile '{profile}' (available: {})",
            project
                .config
                .profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return 2;
    };

    // Schema-only pass with profile semantics: a projection that would strip
    // required/key/unique/reference-critical fields is E9006, not a silent
    // generation (code targets render field metadata from the view).
    let validated = ValidatedSchema {
        schema: project.schema.clone(),
        dependency_graph: DependencyGraph::from_schema(&project.schema),
    };
    let diagnostics = cage_core::validation::validate_with_profile(
        &validated,
        &project.document,
        ValidationLevel::Schema,
        false,
        Some(profile),
    );
    if !diagnostics.is_empty() {
        println!("{}", diagnostics.render(false));
    }
    if diagnostics.has_errors() {
        println!(
            "cage gen: FAILED schema validation ({} errors)",
            diagnostics.errors().len()
        );
        return 1;
    }

    let (schema, document) = filter_by_profile(&project.schema, &project.document, profile);
    let normalized = normalize_document(&document);
    let (schema_hash, _) = ManifestGenerator::input_hashes(&schema, &normalized);

    let mut artifacts: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    for target in &build_profile.targets {
        if let Some(items) = code_target_items(target, &schema, &schema_hash) {
            if let Err(e) = write_artifact_files(path, items, &target.format, &mut artifacts) {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    if artifacts.is_empty() {
        eprintln!(
            "error: profile '{profile}' has no code targets (cs/python/lua/ts/js/cpp/go/java)"
        );
        return 2;
    }

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .generate(&schema, &normalized, &artifacts);
    let output_dir = project.config.output_dir.as_deref().unwrap_or("build");
    let manifest_path = match write_manifest(&path.join(output_dir), &manifest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    println!(
        "cage gen: OK (profile '{profile}', {} artifacts, manifest {})",
        artifacts.len(),
        manifest_path.display()
    );
    0
}

/// Generate code-target artifacts for one target config; `None` when the
/// format is a data target (json/csv) rather than a code target. All
/// generators are schema-driven, deterministic, and infallible; each file
/// header is stamped with the manifest's schema hash.
fn code_target_items(
    target: &TargetConfig,
    schema: &Schema,
    schema_hash: &str,
) -> Option<Vec<(String, Vec<u8>)>> {
    // "csharp"/"python"/"typescript" per docs; short aliases ("cs"/"py"/
    // "ts"/"js") and the common alternates ("golang", "c++"/"cxx") accepted.
    match target.format.as_str() {
        "cs" | "csharp" => Some(
            cage_target_cs::CsTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        "python" | "py" => Some(
            cage_target_py::PyTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        "lua" => Some(
            cage_target_lua::LuaTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        "typescript" | "ts" | "javascript" | "js" => Some(
            cage_target_ts::TsTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        "cpp" | "c++" | "cxx" => Some(
            cage_target_cpp::CppTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        "go" | "golang" => Some(
            cage_target_go::GoTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        "java" => Some(
            cage_target_java::JavaTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        ),
        _ => None,
    }
}

/// Write generated artifacts under `root`, recording manifest entries
/// (relative path, content, raw format string, table stem when derivable).
fn write_artifact_files(
    root: &Path,
    items: Vec<(String, Vec<u8>)>,
    format: &str,
    artifacts: &mut Vec<(String, Vec<u8>, String, Option<String>)>,
) -> Result<(), String> {
    for (rel_path, content) in items {
        let abs = root.join(&rel_path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&abs, &content)
            .map_err(|e| format!("cannot write {}: {e}", abs.display()))?;
        let table = rel_path
            .rsplit('/')
            .next()
            .and_then(|f| f.rsplit_once('.'))
            .map(|(stem, _)| stem.to_string());
        artifacts.push((rel_path, content, format.to_string(), table));
    }
    Ok(())
}

/// Persist the build manifest as `manifest.json` under `manifest_dir`,
/// returning its full path.
fn write_manifest(manifest_dir: &Path, manifest: &BuildManifest) -> Result<PathBuf, String> {
    std::fs::create_dir_all(manifest_dir)
        .map_err(|e| format!("cannot create {}: {e}", manifest_dir.display()))?;
    let manifest_path = manifest_dir.join("manifest.json");
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| format!("manifest serialization failed: {e}"))?;
    std::fs::write(&manifest_path, bytes)
        .map_err(|e| format!("cannot write {}: {e}", manifest_path.display()))?;
    Ok(manifest_path)
}

fn run_inspect(path: &Path, table: Option<&str>) -> i32 {
    let project = match load_project(path) {
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

fn run_diff(baseline: &Path, target: &Path) -> i32 {
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
    if base.content_hash != new.content_hash {
        println!(
            "content_hash: {} -> {}",
            base.content_hash, new.content_hash
        );
    }
    0
}

fn load_manifest(path: &Path) -> Result<BuildManifest, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Render a parsed subcommand as a canonical string. Every `Commands`
    /// arm executes when the five `parse_*` tests call it, so the dispatch
    /// match has no dead arm — unlike `assert!(matches!(...))`, whose
    /// never-taken false arm leaves an uncovered region on the assert line.
    fn describe(cmd: &Commands) -> String {
        match cmd {
            Commands::Check {
                path,
                level,
                profile,
            } => {
                format!("check {} {level} {profile}", path.display())
            }
            Commands::Build {
                path,
                level,
                profile,
                incremental,
            } => format!("build {} {level} {profile} {incremental}", path.display()),
            Commands::Inspect { path, table } => format!(
                "inspect {} {}",
                path.display(),
                table.as_deref().unwrap_or("<all>")
            ),
            Commands::Gen { path, profile } => format!("gen {} {profile}", path.display()),
            Commands::Diff { baseline, target } => {
                format!("diff {} {}", baseline.display(), target.display())
            }
            Commands::Snapshot {
                path,
                profile,
                verify,
            } => format!("snapshot {} {profile} {verify}", path.display()),
        }
    }

    #[test]
    fn parse_check_subcommand() {
        let cli = Cli::try_parse_from(["cage", "check", "proj"]).expect("parse check");
        assert_eq!(describe(&cli.command), "check proj semantic client");
    }

    #[test]
    fn parse_build_subcommand() {
        let cli = Cli::try_parse_from(["cage", "build", "proj", "--level", "table"])
            .expect("parse build");
        assert_eq!(describe(&cli.command), "build proj table client false");
    }

    #[test]
    fn parse_diff_subcommand() {
        let cli = Cli::try_parse_from(["cage", "diff", "a", "b"]).expect("parse diff");
        assert_eq!(describe(&cli.command), "diff a b");
    }

    #[test]
    fn parse_inspect_subcommand() {
        let cli = Cli::try_parse_from(["cage", "inspect", "proj"]).expect("parse inspect");
        assert_eq!(describe(&cli.command), "inspect proj <all>");
        // The optional table argument round-trips too.
        let cli = Cli::try_parse_from(["cage", "inspect", "proj", "Item"]).expect("parse table");
        assert_eq!(describe(&cli.command), "inspect proj Item");
    }

    #[test]
    fn parse_gen_subcommand() {
        let cli =
            Cli::try_parse_from(["cage", "gen", "proj", "--profile", "server"]).expect("parse gen");
        assert_eq!(describe(&cli.command), "gen proj server");
    }

    #[test]
    fn collect_files_single_file_nested_dirs_and_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // A path that is itself a file is returned as-is.
        let single = root.join("single.yaml");
        std::fs::write(&single, "tables: {}\n").expect("write file");
        let files = collect_files(&single, &["yaml"]).expect("file path");
        assert_eq!(files, vec![single.clone()]);

        // Directories are walked recursively in deterministic (sorted) order,
        // keeping only the requested extensions.
        std::fs::create_dir_all(root.join("nested/deeper")).expect("dirs");
        std::fs::write(root.join("a.yaml"), "a").expect("a");
        std::fs::write(root.join("nested/b.yaml"), "b").expect("b");
        std::fs::write(root.join("nested/deeper/c.yaml"), "c").expect("c");
        std::fs::write(root.join("nested/skip.json"), "{}").expect("skip");
        let files = collect_files(root, &["yaml"]).expect("dir walk");
        let names: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "a.yaml",
                "nested/b.yaml",
                "nested/deeper/c.yaml",
                "single.yaml"
            ]
        );

        // A path that exists neither as file nor directory is an error.
        let err = collect_files(&root.join("ghost"), &["yaml"]).expect_err("missing");
        assert!(err.contains("path not found"), "{err}");
    }

    #[test]
    fn filter_by_profile_gates_tables_and_fields() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("schemas")).expect("schemas");
        std::fs::create_dir_all(root.join("config")).expect("config");
        std::fs::write(
            root.join("schemas/profile.yaml"),
            r"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      secret: { name: secret, type: { kind: String }, targets: [server] }
  ServerOnly:
    name: ServerOnly
    primary_key: [id]
    targets: [server]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
        )
        .expect("schema");
        std::fs::write(
            root.join("config/data.json"),
            r#"{"Item": [{"id": 1, "secret": "s"}], "ServerOnly": [{"id": 2}], "Orphan": [{"id": 3}]}"#,
        )
        .expect("data");

        let schema = load_schema(&root.join("schemas")).expect("load schema");
        let document = load_sources(&root.join("config")).expect("load sources");

        // client: the server-only table, the server-gated field and the
        // schema-less doc table all disappear.
        let (schema_c, doc_c) = filter_by_profile(&schema, &document, "client");
        assert!(schema_c.tables.contains_key("Item"));
        assert!(!schema_c.tables.contains_key("ServerOnly"));
        assert!(schema_c.tables["Item"].fields.contains_key("id"));
        assert!(!schema_c.tables["Item"].fields.contains_key("secret"));
        assert!(doc_c.tables.contains_key("Item"));
        assert!(!doc_c.tables.contains_key("ServerOnly"));
        assert!(!doc_c.tables.contains_key("Orphan"));
        let item = &doc_c.tables["Item"];
        assert!(item.rows[0].fields.contains_key("id"));
        assert!(!item.rows[0].fields.contains_key("secret"));

        // server: both tables survive and the server-gated field stays.
        let (schema_s, doc_s) = filter_by_profile(&schema, &document, "server");
        assert!(schema_s.tables.contains_key("Item"));
        assert!(schema_s.tables.contains_key("ServerOnly"));
        assert!(schema_s.tables["Item"].fields.contains_key("secret"));
        assert!(doc_s.tables.contains_key("Item"));
        assert!(doc_s.tables.contains_key("ServerOnly"));
        assert!(doc_s.tables["Item"].rows[0].fields.contains_key("secret"));
        assert!(!doc_s.tables.contains_key("Orphan"));
    }

    #[test]
    fn write_artifact_files_records_entries_and_absolute_path_parent_edge() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut artifacts = Vec::new();
        let items = vec![("out/Item.json".to_string(), b"{\"id\":1}".to_vec())];
        write_artifact_files(tmp.path(), items, "json", &mut artifacts).expect("write");
        assert_eq!(artifacts.len(), 1);
        let (rel, content, format, table) = &artifacts[0];
        assert_eq!(rel, "out/Item.json");
        assert_eq!(content, b"{\"id\":1}");
        assert_eq!(format, "json");
        // Manifest entries record the table stem of the artifact path.
        assert_eq!(table.as_deref(), Some("Item"));
        assert!(tmp.path().join("out/Item.json").is_file());

        // An absolute artifact path replaces the root on join, so `parent()`
        // is None: the create_dir_all branch is skipped and the write fails.
        let err = write_artifact_files(
            tmp.path(),
            vec![("/".to_string(), b"x".to_vec())],
            "json",
            &mut artifacts,
        )
        .expect_err("cannot write /");
        assert!(err.starts_with("cannot write"), "{err}");
    }

    #[test]
    fn manifest_roundtrip_and_error_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest = ManifestGenerator::new(
            "proj".to_string(),
            "client".to_string(),
            "0.1.0".to_string(),
        )
        .generate(&Schema::new(), &Document::new(), &[]);

        // Save, then load from the directory form and the file form.
        let out_dir = tmp.path().join("out");
        let path = write_manifest(&out_dir, &manifest).expect("write manifest");
        assert!(path.ends_with("manifest.json"));
        let from_dir = load_manifest(&out_dir).expect("load dir");
        assert_eq!(from_dir.content_hash, manifest.content_hash);
        let from_file = load_manifest(&path).expect("load file");
        assert_eq!(from_file.project, "proj");
        assert_eq!(from_file.profile, "client");

        // Missing manifest → cannot read.
        let err = load_manifest(&tmp.path().join("ghost")).expect_err("missing");
        assert!(err.contains("cannot read"), "{err}");
        // Malformed manifest → invalid manifest.
        let bad = tmp.path().join("bad.json");
        std::fs::write(&bad, "{not json").expect("bad json");
        let err = load_manifest(&bad).expect_err("invalid");
        assert!(err.contains("invalid manifest"), "{err}");
        // Manifest directory blocked by a regular file → cannot create.
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, "x").expect("blocker");
        let err = write_manifest(&blocker, &manifest).expect_err("blocked");
        assert!(err.contains("cannot create"), "{err}");
    }

    /// The manifest directory exists (so `create_dir_all` succeeds) but
    /// `manifest.json` inside it is a directory: `fs::write` fails and the
    /// `cannot write` error path surfaces.
    #[test]
    fn write_manifest_fails_when_manifest_json_is_a_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest = ManifestGenerator::new(
            "proj".to_string(),
            "client".to_string(),
            "0.1.0".to_string(),
        )
        .generate(&Schema::new(), &Document::new(), &[]);
        let out = tmp.path().join("out");
        std::fs::create_dir_all(out.join("manifest.json")).expect("manifest.json dir");
        let err = write_manifest(&out, &manifest).expect_err("manifest.json is a directory");
        assert!(err.contains("cannot write"), "{err}");
    }

    /// Schema files merge into one document: tables and enums from every
    /// file, metadata from the first file that carries it — later files take
    /// the `is_none()` guard's false branch instead of overwriting it.
    #[test]
    fn load_schema_merges_tables_enums_and_first_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let schemas = tmp.path().join("schemas");
        std::fs::create_dir_all(&schemas).expect("schemas");
        // collect_files sorts, so a_meta.yaml is read first and its metadata
        // is the one kept; b_plain.yaml hits the guard's false edge.
        std::fs::write(
            schemas.join("a_meta.yaml"),
            r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums:
  Rarity:
    name: Rarity
    description: drop rates
    values:
      - { name: Common, value: 0 }
      - { name: Rare, value: 1 }
metadata:
  version: "1.2"
  description: merged schema metadata
"#,
        )
        .expect("a_meta.yaml");
        std::fs::write(
            schemas.join("b_plain.yaml"),
            r"tables:
  Extra:
    name: Extra
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
        )
        .expect("b_plain.yaml");

        let merged = load_schema(&schemas).expect("load schema");
        assert_eq!(merged.tables.len(), 2, "both files contribute tables");
        assert!(merged.enums.contains_key("Rarity"), "enum merged");
        let meta = merged.metadata.as_ref().expect("first metadata kept");
        assert_eq!(meta.version, "1.2");
        assert_eq!(meta.description.as_deref(), Some("merged schema metadata"));
    }

    /// I/O failures on unreadable paths surface as `failed to read` (config
    /// and schema files) and `failed to read dir` (source walk). Mode-000
    /// only denies reads to non-root; the assertions are written so a root
    /// run short-circuits (`!denied || …`) instead of branching away.
    #[cfg(unix)]
    #[test]
    fn unreadable_paths_surface_io_errors() {
        use std::os::unix::fs::PermissionsExt;

        fn deny_read(path: &Path) {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))
                .expect("chmod 000");
        }
        fn allow_read(path: &Path, mode: u32) {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .expect("chmod restore");
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // 1. load_project_config: read_to_string on cage.toml fails.
        let config = root.join("cage.toml");
        std::fs::write(&config, "output_dir = \"build\"\n").expect("write config");
        deny_read(&config);
        let denied = std::fs::read_to_string(&config).is_err();
        assert!(
            !denied || load_project_config(root).is_err_and(|e| e.contains("failed to read")),
            "unreadable cage.toml must surface 'failed to read'"
        );
        allow_read(&config, 0o644);

        // 2. load_schema: read_to_string on a schema file fails.
        let schemas = root.join("schemas");
        std::fs::create_dir_all(&schemas).expect("schemas");
        let schema_file = schemas.join("a.yaml");
        std::fs::write(&schema_file, "tables: {}\nenums: {}\n").expect("write schema");
        deny_read(&schema_file);
        let denied = std::fs::read_to_string(&schema_file).is_err();
        assert!(
            !denied || load_schema(&schemas).is_err_and(|e| e.contains("failed to read")),
            "unreadable schema file must surface 'failed to read'"
        );
        allow_read(&schema_file, 0o644);

        // 3. load_sources → collect_files: read_dir on the source root fails,
        //    and the error propagates through the `?` in the file loop.
        let sources = root.join("config");
        std::fs::create_dir_all(&sources).expect("config");
        std::fs::write(sources.join("data.json"), "{}").expect("write source");
        deny_read(&sources);
        let denied = std::fs::read_dir(&sources).is_err();
        assert!(
            !denied || load_sources(&sources).is_err_and(|e| e.contains("failed to read dir")),
            "unreadable source dir must surface 'failed to read dir'"
        );
        allow_read(&sources, 0o755);
    }
}
