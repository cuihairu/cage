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
use std::path::{Path, PathBuf};

use cage_core::manifest::{BuildManifest, ManifestGenerator, ProjectConfig};
use cage_core::normalize::normalize_document;
use cage_core::schema::{DependencyGraph, Schema, ValidatedSchema};
use cage_core::validation::ValidationLevel;
use cage_core::value::Document;
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
    },
    /// View Schema and configuration structure
    Inspect {
        /// Configuration project root directory
        path: PathBuf,
        /// Table name (lists all tables when omitted)
        table: Option<String>,
    },
    /// Compare artifacts of two configuration builds
    Diff {
        /// Baseline build directory (or manifest.json)
        baseline: PathBuf,
        /// Target build directory (or manifest.json)
        target: PathBuf,
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
        } => run_build(&path, &level, &profile),
        Commands::Inspect { path, table } => run_inspect(&path, table.as_deref()),
        Commands::Diff { baseline, target } => run_diff(&baseline, &target),
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
    let (schema, document) = filter_by_profile(&project.schema, &project.document, profile);
    let validated = ValidatedSchema {
        schema,
        dependency_graph: DependencyGraph::default(),
    };
    let diagnostics = cage_core::validation::validate(
        &validated,
        &document,
        level,
        project.config.warnings_as_errors,
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

fn run_build(path: &Path, level: &str, profile: &str) -> i32 {
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

    let (schema, document) = filter_by_profile(&project.schema, &project.document, profile);
    let validated = ValidatedSchema {
        schema: schema.clone(),
        dependency_graph: DependencyGraph::default(),
    };
    let diagnostics = cage_core::validation::validate(
        &validated,
        &document,
        level,
        project.config.warnings_as_errors,
    );
    if !diagnostics.is_empty() {
        println!("{}", diagnostics.render(false));
    }
    if diagnostics.has_errors() {
        println!(
            "cage build: FAILED validation ({} errors)",
            diagnostics.errors().len()
        );
        return 1;
    }

    let normalized = normalize_document(&document);

    let mut artifacts: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    for target in &build_profile.targets {
        let generated = match target.format.as_str() {
            "json" => cage_target_json::JsonTargetGenerator::from_config(target)
                .generate(&normalized, &[]),
            "csv" => {
                cage_target_csv::CsvTargetGenerator::from_config(target).generate(&normalized, &[])
            }
            other => {
                eprintln!("error: unsupported target format '{other}'");
                return 2;
            }
        };
        match generated {
            Ok(items) => {
                for (rel_path, content) in items {
                    let abs = path.join(&rel_path);
                    if let Some(parent) = abs.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            eprintln!("error: cannot create {}: {e}", parent.display());
                            return 2;
                        }
                    }
                    if let Err(e) = std::fs::write(&abs, &content) {
                        eprintln!("error: cannot write {}: {e}", abs.display());
                        return 2;
                    }
                    let table = rel_path
                        .rsplit('/')
                        .next()
                        .and_then(|f| f.rsplit_once('.'))
                        .map(|(stem, _)| stem.to_string());
                    artifacts.push((rel_path, content, target.format.clone(), table));
                }
            }
            Err(diags) => {
                println!("{}", diags.render(false));
                return 1;
            }
        }
    }

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .generate(&schema, &normalized, &artifacts);
    let output_dir = project.config.output_dir.as_deref().unwrap_or("build");
    let manifest_dir = path.join(output_dir);
    if let Err(e) = std::fs::create_dir_all(&manifest_dir) {
        eprintln!("error: cannot create {}: {e}", manifest_dir.display());
        return 2;
    }
    let manifest_path = manifest_dir.join("manifest.json");
    match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&manifest_path, bytes) {
                eprintln!("error: cannot write {}: {e}", manifest_path.display());
                return 2;
            }
        }
        Err(e) => {
            eprintln!("error: manifest serialization failed: {e}");
            return 2;
        }
    }

    println!(
        "cage build: OK (profile '{profile}', {} artifacts, manifest {})",
        artifacts.len(),
        manifest_path.display()
    );
    0
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

    #[test]
    fn parse_check_subcommand() {
        let cli = Cli::try_parse_from(["cage", "check", "proj"]).expect("parse check");
        assert!(matches!(cli.command, Commands::Check { .. }));
    }

    #[test]
    fn parse_build_subcommand() {
        let cli = Cli::try_parse_from(["cage", "build", "proj", "--level", "table"])
            .expect("parse build");
        assert!(matches!(cli.command, Commands::Build { .. }));
    }

    #[test]
    fn parse_diff_subcommand() {
        let cli = Cli::try_parse_from(["cage", "diff", "a", "b"]).expect("parse diff");
        assert!(matches!(cli.command, Commands::Diff { .. }));
    }

    #[test]
    fn parse_inspect_subcommand() {
        let cli = Cli::try_parse_from(["cage", "inspect", "proj"]).expect("parse inspect");
        assert!(matches!(cli.command, Commands::Inspect { .. }));
    }
}
