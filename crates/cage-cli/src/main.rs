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
use std::fs;
use std::path::{Path, PathBuf};

use cage_core::error::codes::distribution::E2101;
use cage_core::error::codes::registry::E1802;
use cage_core::error::codes::remote::E1905;
use cage_core::manifest::{BuildManifest, ManifestGenerator, ProjectConfig, TargetConfig};
use cage_core::normalize::normalize_document;
use cage_core::reference::{DependencyGraph, IncrementalPlanner};
use cage_core::schema::{Schema, ValidatedSchema};
use cage_core::validation::ValidationLevel;
use cage_core::value::Document;
use cage_core::Diagnostics;
use cage_core::DocumentMetadata;

mod remote;
mod web;

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
        /// Environment to validate under (schema `env_overrides` applied
        /// to field constraints before validation)
        #[arg(long)]
        env: Option<String>,
        /// Refuse the remote-source offline fallback: an unreachable
        /// remote source is a hard error even with a cached copy
        #[arg(long)]
        no_cache: bool,
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
        /// Environment to validate under (schema `env_overrides` applied
        /// to field constraints before validation)
        #[arg(long)]
        env: Option<String>,
        /// Skip rebuilding if hashes match last build's manifest
        #[arg(long)]
        incremental: bool,
        /// Refuse the remote-source offline fallback: an unreachable
        /// remote source is a hard error even with a cached copy
        #[arg(long)]
        no_cache: bool,
    },
    /// View Schema and configuration structure
    Inspect {
        /// Configuration project root directory
        path: PathBuf,
        /// Table name (lists all tables when omitted)
        table: Option<String>,
        /// Refuse the remote-source offline fallback: an unreachable
        /// remote source is a hard error even with a cached copy
        #[arg(long)]
        no_cache: bool,
    },
    /// Generate code-target artifacts only (cs/python/lua/ts/js/cpp/go/java), no data validation
    Gen {
        /// Configuration project root directory
        path: PathBuf,
        /// Build profile to generate code for
        #[arg(long, default_value = "client")]
        profile: String,
        /// Environment to apply before generation (schema `env_overrides`)
        #[arg(long)]
        env: Option<String>,
        /// Refuse the remote-source offline fallback: an unreachable
        /// remote source is a hard error even with a cached copy
        #[arg(long)]
        no_cache: bool,
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
        /// Environment to apply before packaging (schema `env_overrides`);
        /// the packed schema.json and artifacts carry the resolved rules
        #[arg(long, conflicts_with = "verify")]
        env: Option<String>,
        /// Verify an existing snapshot directory instead of building
        #[arg(long)]
        verify: bool,
    },
    /// Serve the local Schema editor (third phase W3): editor page + HTTP API
    Web {
        /// Configuration project root directory
        path: PathBuf,
        /// TCP port to listen on (binds 127.0.0.1 only)
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
    /// Publish or list configuration packages in a local Configuration Registry
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
    },
    /// Apply the migration chain in `migrations/` to the project's source
    /// data under the current schema — declarative segment steps, then a
    /// full L0-L6 reverification. Dry-run by default: nothing is written
    /// until --write, and the write only touches local text sources
    /// (JSON/YAML/CSV); Excel and registry/remote sources stay report-only
    Migrate {
        /// Configuration project root directory
        path: PathBuf,
        /// Apply the whole migration chain instead of just the first segment
        #[arg(long, conflicts_with = "to")]
        all: bool,
        /// Apply the chain prefix up to and including the segment whose
        /// target version matches — `latest` means the whole chain
        #[arg(long, value_name = "VERSION")]
        to: Option<String>,
        /// Rewrite the migrated data back into the local text sources
        #[arg(long)]
        write: bool,
    },
    /// Draft a migration rule file from two schema versions (design §46):
    /// mechanically safe transforms (`set_default` / `remove_field` /
    /// `widen_type`) become steps; renames and enum remaps stay "# TODO"
    /// comments for the author — the draft never runs by itself
    MigrateDraft {
        /// The schema version data is coming from (a schema YAML file)
        from_schema: PathBuf,
        /// The schema version data is moving to (a schema YAML file)
        to_schema: PathBuf,
        /// Version label for the rule's `from:` header
        #[arg(long)]
        from: String,
        /// Version label for the rule's `to:` header
        #[arg(long)]
        to: String,
        /// Write the draft to this file instead of stdout
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Draft a schema file from live database tables (design §45): each
    /// `mysql:<table>` / `pg:<table>` spec is introspected read-only
    /// (bound-parameter `information_schema` queries) and rendered as a
    /// reviewable YAML draft — tables, primary keys and NOT NULL →
    /// `required` are mechanical; ranges, patterns, enum domains and
    /// references stay for the author, flagged inline where the server's
    /// type carries a decision
    SchemaDraft {
        /// Configuration project root directory (supplies `[remote.<scheme>]`)
        path: PathBuf,
        /// `mysql:<table>` / `pg:<table>` specs to introspect (a qualified
        /// `schema.table` resolves against that schema)
        #[arg(required = true)]
        specs: Vec<String>,
        /// Write the draft to this file instead of stdout
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum RegistryCmd {
    /// Build a profile and publish its self-verifying snapshot into the
    /// local registry as `<package>/<version>` (re-publishing byte-identical
    /// content is an idempotent no-op; different bytes for the same version
    /// are an E1801 conflict)
    Publish {
        /// Configuration project root directory
        path: PathBuf,
        /// Build profile to publish
        #[arg(long, default_value = "client")]
        profile: String,
        /// Environment to apply before packaging (schema `env_overrides`);
        /// one version stays one pack — a different environment at the
        /// same version is a different pack (E1801 guards the conflict)
        #[arg(long)]
        env: Option<String>,
        /// Package name (defaults to project.name)
        #[arg(long)]
        package: Option<String>,
        /// Version to publish (defaults to project.version)
        #[arg(long)]
        version: Option<String>,
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// List packages and versions recorded in the registry
    List {
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// Verify a full registry: every recorded entry must exist and
    /// self-verify (bytes re-hashed against its ledger, index record
    /// matching the ledger), and no entry directory may sit on disk
    /// without an index record — findings are E1803
    Verify {
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// Garbage-collect a registry: per package keep the newest `keep`
    /// versions (the rollback window consumers pin @versions into, never
    /// fewer than one), delete the older entries and sweep orphaned
    /// directories
    Gc {
        /// Versions to keep per package (minimum 1)
        #[arg(long, default_value_t = 3)]
        keep: usize,
        /// Report what would be removed without touching the registry
        #[arg(long)]
        dry_run: bool,
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// Remove one entry — version directory and index record — explicitly;
    /// the package index stays, so the same snapshot can be re-published
    /// afterwards (removal is explicit history editing)
    Remove {
        /// Package name
        package: String,
        /// Version to remove
        version: String,
        /// Report what would be removed without touching the registry
        #[arg(long)]
        dry_run: bool,
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// Export a published entry as a deterministic tar bundle — every entry
    /// file plus its ledger and the package index excerpt, packed in member
    /// name order with zeroed mtime/uid/gid (same entry, same bytes) — for
    /// offline distribution or manual upload to a static registry host
    Export {
        /// Package name, optionally suffixed `@<version>` (latest when the
        /// version is omitted)
        package: String,
        /// Bundle output file
        #[arg(short, long)]
        output: PathBuf,
        /// Wrap the deterministic tar in a compression container; the only
        /// accepted value is `zstd` (a single frame at a fixed level, so
        /// the wrapped bytes stay reproducible). Import sniffs the frame
        /// magic, so both container forms load identically.
        #[arg(long, value_parser = ["zstd"], value_name = "FORMAT")]
        compress: Option<String>,
        /// Also produce a detached ed25519 signature at `<output>.sig` —
        /// the signature covers the exact bundle file bytes, so a consumer
        /// can prove which key produced it (A6)
        #[arg(long, requires = "key_env")]
        sign: bool,
        /// Environment variable carrying the base64 32-byte Ed25519 seed
        /// (required by --sign; the key never enters cage.toml or logs)
        #[arg(long)]
        key_env: Option<String>,
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// Import a bundle exported by `cage registry export`: the riding ledger
    /// must verify and match the bundle's index excerpt before anything
    /// enters the registry (E2103 otherwise); byte-identical re-imports are
    /// idempotent no-ops, different bytes for an existing version are an
    /// E1801 conflict. The container is sniffed from the file head, not the
    /// name — plain tars and `--compress zstd` frames load identically
    /// under any extension
    Import {
        /// Bundle file produced by `cage registry export`
        file: PathBuf,
        /// Verify the detached ed25519 signature at `<file>.sig` before
        /// the trust gate — the trusted key comes from --key-env (the
        /// riding public key is not a trust anchor) (A6)
        #[arg(long, requires = "key_env")]
        verify_sig: bool,
        /// Environment variable carrying the base64 32-byte Ed25519
        /// public key (required by --verify-sig)
        #[arg(long)]
        key_env: Option<String>,
        /// Run the full trust gate and report what would enter, without
        /// writing anything
        #[arg(long)]
        dry_run: bool,
        /// Registry root directory (overrides cage.toml `[registry].path`)
        #[arg(long)]
        registry: Option<PathBuf>,
    },
    /// Push a published entry from the project's local registry
    /// (`[registry].path`) to a remote http(s) registry root — one PUT per
    /// entry file, the package index written last, merged over the remote's
    /// existing entries (remote history is never rewritten). The bearer
    /// token resolves from the env var named by `--auth-env` or
    /// `[registry].auth_env`; anonymous when neither is set. With
    /// `--presign-map` the entry pushes through per-object presigned URLs
    /// instead — no token is read or sent, the URLs are the grant.
    Push {
        /// Configuration project root directory
        path: PathBuf,
        /// Package name, optionally suffixed `@<version>` (defaults to
        /// project.name at the source registry's latest)
        package: Option<String>,
        /// Remote http(s) registry root to push to (ignored with
        /// --presign-map)
        #[arg(long)]
        registry: Option<String>,
        /// Presign map (JSON): `{"uploads": {"pkg/1.0.0/file": "https://…"},
        /// "index": {"get": "https://…", "put": "https://…"}}` — per-object
        /// presigned URLs issued by whoever owns the object store; the
        /// merged index PUTs last through `index.put`
        #[arg(long, conflicts_with_all = ["registry", "auth_env"])]
        presign_map: Option<PathBuf>,
        /// Environment variable carrying the bearer token (overrides
        /// `[registry].auth_env`)
        #[arg(long)]
        auth_env: Option<String>,
        /// Read the local entry and check the remote state, upload nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate a fresh ed25519 signing keypair: the seed (secret) is
    /// written to the output file with owner-only permissions and is
    /// never printed; only the public key goes to stdout. Feed the seed
    /// to `cage registry export --sign` through an env variable
    /// (`export CAGE_SIGNING_KEY=$(cat <file>)`) (A6)
    Keygen {
        /// File to receive the base64 32-byte seed (the secret; chmod 600)
        #[arg(short, long)]
        output: PathBuf,
    },
}

/// A loaded Cage project: config + merged schema + merged document.
struct Project {
    config: ProjectConfig,
    schema: Schema,
    document: Document,
}

/// Apply `--env` to a loaded project: the environment name must be
/// declared by some table's `env_overrides`, every override must be
/// structurally sound, then the overrides for this environment are
/// resolved into plain field constraints. Everything downstream —
/// validation, codegen, hashing — sees one coherent schema, so an
/// environment changes exactly the rules those stages apply. `None`
/// keeps the base schema.
fn project_with_env(project: Project, env: Option<&str>) -> Result<Project, String> {
    let Some(env) = env else {
        // Base lane: nobody selected an environment, but a broken override
        // in any declared one would surface as a hard error the moment
        // someone runs `--env` on it — lint it now, don't wait.
        for problem in project.schema.lint_env_overrides() {
            eprintln!("warning: {problem}");
        }
        return Ok(project);
    };
    if let Err(e) = project.schema.validate_env_overrides() {
        return Err(format!("invalid env_overrides: {e}"));
    }
    let declared = project.schema.declared_envs();
    if declared.is_empty() {
        return Err(format!(
            "--env '{env}': schema declares no environments (no env_overrides)"
        ));
    }
    if !declared.contains(env) {
        return Err(format!(
            "unknown environment '{env}' (declared: {})",
            declared.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let schema = project.schema.resolve_env(env)?;
    Ok(Project {
        config: project.config,
        schema,
        document: project.document,
    })
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Commands::Check {
            path,
            level,
            profile,
            env,
            no_cache,
        } => run_check(&path, &level, &profile, env.as_deref(), no_cache),
        Commands::Build {
            path,
            level,
            profile,
            env,
            incremental,
            no_cache,
        } => run_build(
            &path,
            &level,
            &profile,
            env.as_deref(),
            incremental,
            no_cache,
        ),
        Commands::Inspect {
            path,
            table,
            no_cache,
        } => run_inspect(&path, table.as_deref(), no_cache),
        Commands::Gen {
            path,
            profile,
            env,
            no_cache,
        } => run_gen(&path, &profile, env.as_deref(), no_cache),
        Commands::Diff { baseline, target } => run_diff(&baseline, &target),
        Commands::Snapshot {
            path,
            profile,
            env,
            verify,
        } => {
            if verify {
                run_snapshot_verify(&path)
            } else {
                run_snapshot(&path, &profile, env.as_deref())
            }
        }
        Commands::Web { path, port } => match web::run_web(&path, port) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("cage web: {e}");
                2
            }
        },
        Commands::Registry { cmd } => match cmd {
            RegistryCmd::Publish {
                path,
                profile,
                env,
                package,
                version,
                registry,
            } => run_registry_publish(
                &path,
                &profile,
                env.as_deref(),
                package.as_deref(),
                version.as_deref(),
                registry.as_deref(),
            ),
            RegistryCmd::List { registry } => run_registry_list(registry.as_deref()),
            RegistryCmd::Verify { registry } => run_registry_verify(registry.as_deref()),
            RegistryCmd::Gc {
                keep,
                dry_run,
                registry,
            } => run_registry_gc(keep, dry_run, registry.as_deref()),
            RegistryCmd::Remove {
                package,
                version,
                dry_run,
                registry,
            } => run_registry_remove(&package, &version, dry_run, registry.as_deref()),
            RegistryCmd::Export {
                package,
                output,
                compress,
                sign,
                key_env,
                registry,
            } => run_registry_export(
                &package,
                &output,
                compress.as_deref(),
                sign,
                key_env.as_deref(),
                registry.as_deref(),
            ),
            RegistryCmd::Import {
                file,
                verify_sig,
                key_env,
                dry_run,
                registry,
            } => run_registry_import(
                &file,
                verify_sig,
                key_env.as_deref(),
                dry_run,
                registry.as_deref(),
            ),
            RegistryCmd::Keygen { output } => run_registry_keygen(&output),
            RegistryCmd::Push {
                path,
                package,
                registry,
                presign_map,
                auth_env,
                dry_run,
            } => run_registry_push(
                &path,
                package.as_deref(),
                registry.as_deref(),
                presign_map.as_deref(),
                auth_env.as_deref(),
                dry_run,
            ),
        },
        Commands::Migrate {
            path,
            all,
            to,
            write,
        } => run_migrate(&path, all, to.as_deref(), write),
        Commands::MigrateDraft {
            from_schema,
            to_schema,
            from,
            to,
            out,
        } => run_migrate_draft(&from_schema, &to_schema, &from, &to, out.as_deref()),
        Commands::SchemaDraft { path, specs, out } => {
            run_schema_draft(&path, &specs, out.as_deref())
        }
    };
    std::process::exit(code);
}

/// Load project config (cage.toml / cage.yaml / cage.yml / cage.json), merge
/// all schema files and parse all declared source roots into one Document.
fn load_project(root: &Path, no_cache: bool) -> Result<Project, String> {
    let config = load_project_config(root)?;

    let schema = load_schema_for_config(root, &config)?;

    let mut document = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    for rel in config.source_roots.values() {
        // `registry:<package>[@<version>]` source roots (R1) resolve to a
        // published entry's data/ directory; the entry's ledger is verified
        // before anything is read, and the `[dependencies]` pin (R2) picks
        // the version when the spec carries none.
        let doc = if rel.starts_with("registry:") {
            let entry = registry_entry(root, &config, rel)?;
            load_sources_from_entry(&entry)?
        } else if rel.starts_with("mysql:") || rel.starts_with("pg:") {
            // Remote Source DB (S2, design §45): `mysql:<表|具名查询>` /
            // `pg:<表|具名查询>` — static read-only whitelist (E1905) +
            // session read-only pin, DSN from env (E1904), row set
            // materialized through the same cache-and-parse path.
            cage_source_db::DbSourceAdapter::load(root, &config, rel, no_cache)?
        } else if rel.starts_with("gsheet:") {
            // Remote Source Sheets (S3, design §45): `gsheet:<id>/<tab>`
            // via Sheets API v4 values (UNFORMATTED_VALUE), first row =
            // header, credential from env (E1904), shape gate (E1903),
            // canonical JSON through the same cache-and-parse path.
            cage_source_sheets::SheetsSourceAdapter::load(root, &config, rel, no_cache)?
        } else if remote::is_remote_root(rel) {
            // Remote Source (S1, design §45): an http(s) URL is fetched,
            // materialized under `.cage-cache/source/`, and parsed by the
            // standard JSON adapter — same shapes, same L0-L7 pipeline,
            // no bypass for remote bytes.
            cage_source_http::HttpSourceAdapter::load(root, rel, no_cache)?
        } else {
            load_sources(&root.join(rel))?
        };
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

pub(crate) fn load_project_config(root: &Path) -> Result<ProjectConfig, String> {
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

pub(crate) fn load_schema(path: &Path) -> Result<Schema, String> {
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

/// Resolve a `registry:<package>[@<version>]` reference (source root or
/// `schema_path`) to a verified entry directory. The version requirement
/// comes from the explicit `@<version>` spec or, when the spec carries
/// none, from the `[dependencies]` pin; an explicit version must still
/// satisfy the pin (E1802 otherwise). A remote root (`http(s)://`, R3)
/// resolves over the network into the project-local cache.
fn registry_entry(root: &Path, config: &ProjectConfig, rel: &str) -> Result<PathBuf, String> {
    let (package, version) = cage_core::registry::parse_spec(rel)?;
    let reg_path = match &config.registry {
        Some(reg) => reg.path.clone(),
        None => {
            return Err(format!(
                "{E1802} registry reference '{rel}' requires '[registry] path' in cage.toml"
            ))
        }
    };
    let requirement = config
        .dependencies
        .get(&package)
        .map(|pin| {
            cage_core::registry::parse_version_req(pin)
                .map_err(|e| format!("{e} (dependency '{package}')"))
        })
        .transpose()?;
    if remote::is_remote_root(&reg_path) {
        return remote::resolve_remote(
            root,
            &reg_path,
            &package,
            version.as_deref(),
            requirement.as_ref(),
        );
    }
    let reg_root = root.join(&reg_path);
    cage_core::registry::resolve_pinned(
        &reg_root,
        &package,
        version.as_deref(),
        requirement.as_ref(),
    )
}

/// Load a resolved registry entry's schema (R2): the entry's `schema.json`
/// — the profile-projected schema the entry was published with, stored in
/// the snapshot packing format (`Schema` serde shape).
fn load_schema_from_entry(entry_dir: &Path) -> Result<Schema, String> {
    let path = entry_dir.join("schema.json");
    let content = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{E1802} registry entry schema missing: {}: {e}",
            path.display()
        )
    })?;
    serde_json::from_str(&content).map_err(|e| {
        format!(
            "{E1802} corrupt registry entry schema {}: {e}",
            path.display()
        )
    })
}

/// Schema load for a project config (shared by `load_project` and the web
/// API): a `registry:` `schema_path` reads the resolved entry's schema.json;
/// otherwise the filesystem path (single file or directory of schema
/// files); none configured → empty schema.
pub(crate) fn load_schema_for_config(
    root: &Path,
    config: &ProjectConfig,
) -> Result<Schema, String> {
    match &config.schema_path {
        Some(rel) if rel.starts_with("registry:") => {
            load_schema_from_entry(&registry_entry(root, config, rel)?)
        }
        Some(rel) => load_schema(&root.join(rel)),
        None => Ok(Schema::new()),
    }
}

fn load_sources(root: &Path) -> Result<Document, String> {
    parse_source_files(&collect_files(
        root,
        &["json", "yaml", "yml", "csv", "xlsx", "xls", "msgpack"],
    )?)
}

/// Parse the given source files into one merged document, in list order.
fn parse_source_files(files: &[PathBuf]) -> Result<Document, String> {
    let mut merged = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    for file in files {
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let result = match ext.as_str() {
            "json" => cage_source_json::JsonSourceAdapter::parse_file(file),
            "yaml" | "yml" => cage_source_yaml::YamlSourceAdapter::parse_file(file),
            "csv" => cage_source_csv::CsvSourceAdapter::default().parse_file(file),
            "xlsx" | "xls" => cage_source_excel::ExcelSourceAdapter::default().parse_file(file),
            "msgpack" => cage_source_msgpack::MsgPackSourceAdapter::parse_file(file),
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

/// Data-target format fidelity for re-consumption: a snapshot entry packs
/// every data target of one profile view — `msgpack/` / `json/` / `csv/`
/// siblings serializing the SAME canonical tables with different fidelity
/// (csv flattens nested values to strings and absent optionals to empty
/// cells; msgpack keeps floats bit-exact and `bin` payloads as `Bytes`).
/// Resolution therefore loads the single highest-fidelity format present.
fn format_fidelity(ext: &str) -> u8 {
    match ext {
        "msgpack" => 5,
        "json" => 4,
        "yaml" | "yml" => 3,
        "csv" => 2,
        _ => 1, // xlsx / xls
    }
}

/// Load a resolved registry entry (R1): the entry's `data/` directory, at
/// the highest-fidelity data-target format present (msgpack > json > yaml >
/// csv > excel). Table identity comes from the entry's manifest.json — the
/// authoritative artifact→table record — because target-format files don't
/// carry it (a msgpack/JSON target file is a bare row array, a CSV only has
/// the file stem). A published profile whose top format is csv re-validates
/// only while its tables stay flat — publish a msgpack/json data target
/// when consumers need nested types.
fn load_sources_from_entry(entry_dir: &Path) -> Result<Document, String> {
    let data_dir = entry_dir.join("data");
    let files = collect_files(
        &data_dir,
        &["json", "yaml", "yml", "csv", "xlsx", "xls", "msgpack"],
    )?;
    if files.is_empty() {
        return Err(format!(
            "{E1802} registry entry has no data artifacts under {}",
            data_dir.display()
        ));
    }
    let ext_of = |f: &Path| {
        f.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
    };
    let top = files
        .iter()
        .map(|f| format_fidelity(&ext_of(f)))
        .max()
        .expect("non-empty file list");
    let picked: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| format_fidelity(&ext_of(f)) == top)
        .collect();

    // The manifest's artifact records are the table-name authority: target
    // files don't carry table identity, and the snapshot packing stripped
    // each artifact's `output_dir` prefix. An entry-relative data path may
    // therefore suffix-match several artifact keys (nested output subtrees);
    // the snapshot carries the LAST one packed, which is what resolution
    // maps back to.
    let manifest_raw = std::fs::read_to_string(entry_dir.join("manifest.json"))
        .map_err(|e| format!("{E1802} cannot read entry manifest: {e}"))?;
    let manifest: BuildManifest = serde_json::from_str(&manifest_raw)
        .map_err(|e| format!("{E1802} corrupt entry manifest.json: {e}"))?;
    let table_for_rel = |rel: &str| -> Option<String> {
        let suffix = format!("/{rel}");
        let mut found: Option<String> = None;
        for (key, art) in &manifest.artifacts {
            if art.table.is_some() && (key.as_str() == rel || key.ends_with(&suffix)) {
                found.clone_from(&art.table);
            }
        }
        found
    };

    let mut merged = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    for file in picked {
        let mut doc = parse_source_files(std::slice::from_ref(&file))?;
        let rel = file
            .strip_prefix(&data_dir)
            .expect("file under data dir")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if doc.tables.len() == 1 {
            if let (Some(expected), Some(actual)) =
                (table_for_rel(&rel), doc.tables.keys().next().cloned())
            {
                if expected != actual {
                    // Target formats parse under a placeholder name ("Data"
                    // for bare arrays) — restore the manifest's table name
                    // on both the map key and the Table itself (the map key
                    // drives schema matching, the struct name drives
                    // artifact file templates).
                    let (_, mut table) = doc.tables.swap_remove_entry(&actual).expect("one table");
                    table.name.clone_from(&expected);
                    doc.tables.insert(expected, table);
                }
            }
        }
        for (name, table) in doc.tables {
            merged.tables.insert(name, table);
        }
        merged.source_files.extend(doc.source_files);
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

fn run_check(path: &Path, level: &str, profile: &str, env: Option<&str>, no_cache: bool) -> i32 {
    let level = match level.parse::<ValidationLevel>() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match load_project(path, no_cache) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_with_env(project, env) {
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

/// How many affected-row location lines one step may render before the
/// rest collapse into a single deterministic summary line (always the
/// first N in source order — rows already are in source order).
const EXCEL_LOCATION_CAP: usize = 8;

/// Render one step's affected-row locations for the migrate report, in
/// source order under the step's row-count line (4-space indent), tail
/// summarized once past [`EXCEL_LOCATION_CAP`]. Core records locations
/// only for Excel-served tables, so an empty list renders nothing.
fn affected_location_lines(step: &cage_core::migrate::StepReport) -> Vec<String> {
    let locations = &step.affected_locations;
    if locations.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = locations
        .iter()
        .take(EXCEL_LOCATION_CAP)
        .map(|loc| format!("    {}", loc.display()))
        .collect();
    if locations.len() > EXCEL_LOCATION_CAP {
        lines.push(format!(
            "    … +{} more row(s)",
            locations.len() - EXCEL_LOCATION_CAP
        ));
    }
    lines
}

/// `cage migrate [--all | --to <ver|latest>] [--write]` — apply the
/// migration chain in `<root>/migrations/` (file-name order = version
/// step chain) to the loaded document, then reverify it under the
/// current schema. `--to latest` resolves to the whole chain, same
/// selection as `--all`.
///
/// The CLI always holds the post-migration schema (rules migrate data;
/// the schema was already edited), so no historical from-schema exists on
/// disk: `validate_spec`'s from-schema reference check stays a library
/// capability, and a bad reference surfaces here as E2003 at apply time
/// (missing table, rename target already taken) or E2004 at reverification.
/// Dry-run by default; `--write` rewrites the local text sources in place,
/// byte-identically skipping unchanged files.
fn run_migrate(path: &Path, all: bool, to: Option<&str>, write: bool) -> i32 {
    let project = match load_project(path, false) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let migrations_dir = path.join("migrations");
    if !migrations_dir.is_dir() {
        println!(
            "cage migrate: nothing to migrate (no {} directory)",
            migrations_dir.display()
        );
        return 0;
    }
    let chain = match cage_core::migrate::parse_migration_dir(&migrations_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if chain.is_empty() {
        println!(
            "cage migrate: nothing to migrate (empty {} directory)",
            migrations_dir.display()
        );
        return 0;
    }
    // `latest` is the symbolic end of the chain — it resolves to the whole
    // chain, exactly like `--all`. A chain that literally contains a
    // segment ending at the version named "latest" resolves as that
    // version first (the concrete target wins over the symbol).
    let latest_symbol = to == Some("latest") && !chain.iter().any(|(_, s)| s.to == "latest");
    let selected: Vec<(String, cage_core::migrate::MigrationSpec)> = if all || latest_symbol {
        chain
    } else if let Some(target) = to {
        let mut picked = Vec::new();
        for (file, spec) in chain {
            let at_target = spec.to == target;
            picked.push((file, spec));
            if at_target {
                break;
            }
        }
        if picked.last().is_none_or(|(_, s)| s.to != target) {
            eprintln!(
                "error: --to {target}: no segment on the migration chain ends at that version"
            );
            return 2;
        }
        picked
    } else {
        // Explicit step-by-step evolution: one segment per run by default.
        vec![chain.into_iter().next().expect("non-empty chain")]
    };

    // Linkage gate (design §46, deferred item): the chain end and the
    // `[dependencies]` pin are two independent declarations of the schema
    // version the data must satisfy — migrating outside the pin leaves
    // the rewritten data no schema to validate under. Gate both dry-run
    // and write.
    if let Some(chain_end) = selected.last().map(|(_, spec)| spec.to.as_str()) {
        if let Err(message) = check_migrate_pin_linkage(&project.config, chain_end) {
            eprintln!("error: {message}");
            return 2;
        }
    }

    let mut doc = project.document;
    let mut migrated_rows = 0usize;
    for (file, spec) in &selected {
        let report = match cage_core::migrate::apply(spec, &mut doc, &project.schema) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        println!("cage migrate: segment {file} ({} → {})", spec.from, spec.to);
        for step in &report.steps {
            println!("  {}: {} row(s)", step.step, step.rows_changed);
            migrated_rows += step.rows_changed;
            // Excel sources are report-only (never written back), so the
            // affected rows' addresses are the one concrete record of what
            // the step moved. Plain-text sources stay file-level — their
            // rewritten file is the record, and per-row lines there would
            // be noise.
            for line in affected_location_lines(step) {
                println!("{line}");
            }
        }
    }

    // Post-migration reverification (L0-L6) under the current schema —
    // a failure means nothing is written.
    if let Err(e) = cage_core::migrate::reverify(&doc, &project.schema) {
        eprintln!("error: {e}");
        eprintln!("cage migrate: verification failed — nothing written");
        return 1;
    }

    let lines = match write_migrated_sources(&doc, path, &project.config, &project.schema, write) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    for line in &lines {
        println!("{line}");
    }
    println!(
        "cage migrate: OK ({} segment(s), {} row(s) migrated, schema {}){}",
        selected.len(),
        migrated_rows,
        project
            .schema
            .metadata
            .as_ref()
            .map_or("(unversioned)", |m| m.version.as_str()),
        if write {
            ""
        } else {
            " — dry run, nothing written"
        }
    );
    0
}

/// Gate the migration chain end against the `[dependencies]` pin of the
/// registry package providing the schema (design §46 linkage): the pin
/// declares which schema versions this project accepts, so a chain end
/// outside it would leave the migrated data no schema to validate under.
/// Local-schema projects have no pin and nothing to link; a registry
/// `schema_path` without a pin is governed by its explicit version alone.
fn check_migrate_pin_linkage(config: &ProjectConfig, chain_end: &str) -> Result<(), String> {
    let Some(rel) = config
        .schema_path
        .as_deref()
        .filter(|r| r.starts_with("registry:"))
    else {
        return Ok(());
    };
    let (package, _) = cage_core::registry::parse_spec(rel)?;
    let Some(pin) = config.dependencies.get(&package) else {
        return Ok(());
    };
    let req = cage_core::registry::parse_version_req(pin)
        .map_err(|e| format!("{e} (dependency '{package}')"))?;
    if cage_core::registry::satisfies(chain_end, &req) {
        Ok(())
    } else {
        Err(format!(
            "{E1802} migrate target version {chain_end} is outside the \
             [dependencies] pin '{pin}' for '{package}' — widen the pin or \
             migrate to a version it admits"
        ))
    }
}

/// `cage migrate-draft <from-schema> <to-schema> --from <ver> --to <ver>
/// [-o <file>]` — draft a migration rule file from two schema versions
/// (design §46 deferred item). Mechanically safe transforms become steps;
/// renames and enum remaps stay "# TODO" comments. The draft is never
/// applied by this command: the author finishes it, then `cage migrate`
/// dry-runs it. The draft skips `validate_spec` on purpose — a
/// `set_default` on a newly-added field references the to-schema, and a
/// `remove_field` references the from-schema, so neither schema alone
/// satisfies the reference check; the real gates are the diff itself and
/// apply's E2003防线.
fn run_migrate_draft(
    from_schema: &Path,
    to_schema: &Path,
    from_version: &str,
    to_version: &str,
    out: Option<&Path>,
) -> i32 {
    let from = match load_schema(from_schema) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let to = match load_schema(to_schema) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let diff = cage_core::migrate::diff::diff_schemas(&from, &to);
    let draft = cage_core::migrate::diff::draft_migration(&diff);
    // Versions are single-quoted so numeric-looking labels (`2.0`) cannot
    // reparse as floats when the rule is read back.
    let mut text = format!(
        "from: '{}'\nto: '{}'\n",
        from_version.replace('\'', "''"),
        to_version.replace('\'', "''"),
    );
    text.push_str(&cage_core::migrate::diff::render_draft(&draft));
    match out {
        Some(path) => {
            // `-o migrations/0001.yaml` with no `migrations/` yet is the
            // documented first-run shape — create the parent, not an error.
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("error: cannot create {}: {e}", parent.display());
                    return 1;
                }
            }
            if let Err(e) = std::fs::write(path, &text) {
                eprintln!("error: cannot write {}: {e}", path.display());
                return 1;
            }
            println!(
                "cage migrate-draft: {} step(s), {} TODO(s) → {}{}",
                draft.steps.len(),
                draft.todos.len(),
                path.display(),
                if draft.steps.is_empty() {
                    " — steps: [] must be filled before cage migrate can parse it"
                } else {
                    ""
                }
            );
        }
        None => {
            print!("{text}");
        }
    }
    0
}

/// `cage schema-draft <project> <spec>... [-o <file>]` — draft a schema
/// file from live database tables (design §45 deferred item). Each
/// `mysql:<table>` / `pg:<table>` spec is introspected read-only over
/// the project's `[remote.<scheme>]` settings and rendered into one
/// deterministic YAML draft; the author reviews the comments and owns
/// the semantic constraints. No project schema or data loads — cage.toml
/// alone settles the DSN discipline.
fn run_schema_draft(path: &Path, specs: &[String], out: Option<&Path>) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let mut tables = Vec::new();
    for spec in specs {
        let Some((scheme, name)) = cage_source_db::parse_spec(spec) else {
            eprintln!("error: {E1905} not a mysql:/pg: source spec: {spec}");
            return 2;
        };
        let dsn = match cage_source_db::resolve_dsn(scheme, config.remote.get(scheme)) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        match cage_source_db::introspect(scheme, &dsn, name) {
            Ok(info) => tables.push(info),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    let text = cage_source_db::draft::render_draft(specs, &tables);
    match out {
        Some(path) => {
            // `-o schemas/draft.yaml` with no `schemas/` yet is the
            // documented first-run shape — create the parent, not an
            // error (same as migrate-draft).
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("error: cannot create {}: {e}", parent.display());
                    return 2;
                }
            }
            if let Err(e) = std::fs::write(path, &text) {
                eprintln!("error: cannot write {}: {e}", path.display());
                return 2;
            }
            println!(
                "cage schema-draft: {} table(s) → {} — review the comments before adopting",
                tables.len(),
                path.display()
            );
        }
        None => {
            print!("{text}");
        }
    }
    0
}

/// Group tables by source file and rewrite each local text source
/// (JSON/YAML/CSV) with its migrated tables. Excel sources and anything
/// outside the project's local source roots (registry entries, remote
/// caches) are never written — they are reported instead. Rows render in
/// schema field order (the stable contract order — the JSON reader loses
/// the file's original key order, so that would not be stable to re-render
/// against), which is what makes a second `--write` run byte-identical.
/// Byte-identical renders are skipped outright.
fn write_migrated_sources(
    doc: &Document,
    root: &Path,
    config: &ProjectConfig,
    schema: &Schema,
    write: bool,
) -> Result<Vec<String>, String> {
    let writable: Vec<PathBuf> = config
        .source_roots
        .values()
        .filter(|rel| {
            !rel.starts_with("registry:")
                && !rel.starts_with("mysql:")
                && !rel.starts_with("pg:")
                && !rel.starts_with("gsheet:")
                && !remote::is_remote_root(rel)
        })
        .map(|rel| root.join(rel))
        .collect();
    let mut groups: std::collections::BTreeMap<String, Vec<&cage_core::value::Table>> =
        std::collections::BTreeMap::new();
    for table in doc.tables.values() {
        if !table.source_file.is_empty() {
            groups
                .entry(table.source_file.clone())
                .or_default()
                .push(table);
        }
    }
    let mut lines = Vec::new();
    for (source, tables) in &groups {
        let file = PathBuf::from(source);
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "xlsx" | "xls" => {
                let rows: usize = tables.iter().map(|t| t.rows.len()).sum();
                lines.push(format!(
                    "  skip {source} (Excel source: report only — {} table(s), {rows} row(s))",
                    tables.len()
                ));
                continue;
            }
            "json" | "yaml" | "yml" | "csv" => {}
            _ => {
                lines.push(format!("  skip {source} (not a text source)"));
                continue;
            }
        }
        if !writable.iter().any(|w| file.starts_with(w)) {
            lines.push(format!(
                "  skip {source} (registry/remote source: read-only, {} table(s))",
                tables.len()
            ));
            continue;
        }
        let new_bytes = render_source_file(&ext, tables, source, schema)?;
        let old_bytes = std::fs::read(&file)
            .map_err(|e| format!("cannot read back source file {source}: {e}"))?;
        if old_bytes == new_bytes {
            lines.push(format!("  unchanged {source}"));
            continue;
        }
        if write {
            std::fs::write(&file, &new_bytes)
                .map_err(|e| format!("cannot write source file {source}: {e}"))?;
            lines.push(format!("  wrote {source} ({} table(s))", tables.len()));
        } else {
            lines.push(format!(
                "  would write {source} ({} table(s))",
                tables.len()
            ));
        }
    }
    Ok(lines)
}

/// Render one source file's tables back into their original format:
/// JSON/YAML as `{ table: [row objects] }` in document order, CSV as a
/// single-table file (the reader's own convention). Row fields render in
/// schema declaration order when the table is defined in the schema —
/// the JSON source reader does not preserve the file's original key
/// order, so the schema order is the only stable one to re-render — with
/// any off-schema fields appended in load order.
fn render_source_file(
    ext: &str,
    tables: &[&cage_core::value::Table],
    source: &str,
    schema: &Schema,
) -> Result<Vec<u8>, String> {
    match ext {
        "json" => {
            let mut outer = IndexMap::new();
            for table in tables {
                outer.insert(table.name.clone(), rows_to_json(table, source, schema)?);
            }
            let mut buf = Vec::new();
            serde_json::to_writer_pretty(&mut buf, &outer)
                .map_err(|e| format!("cannot render {source}: {e}"))?;
            buf.push(b'\n');
            Ok(buf)
        }
        "yaml" | "yml" => {
            let mut outer = IndexMap::new();
            for table in tables {
                outer.insert(table.name.clone(), rows_to_json(table, source, schema)?);
            }
            let text = serde_yaml::to_string(&outer)
                .map_err(|e| format!("cannot render {source}: {e}"))?;
            Ok(text.into_bytes())
        }
        "csv" => render_csv(tables, source, schema),
        _ => Err(format!(
            "cannot render {source}: unsupported text format '{ext}'"
        )),
    }
}

/// Column order for one table: schema-declared fields that actually appear
/// in the data (in schema order), then any off-schema fields in
/// first-appearance order.
fn ordered_columns(table: &cage_core::value::Table, schema: &Schema) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    if let Some(defined) = schema.tables.get(&table.name) {
        for name in defined.fields.keys() {
            if table.rows.iter().any(|r| r.fields.contains_key(name)) {
                columns.push(name.clone());
            }
        }
    }
    let seen: HashSet<String> = columns.iter().cloned().collect();
    let mut extra: Vec<String> = Vec::new();
    for row in &table.rows {
        for name in row.fields.keys() {
            if !seen.contains(name) && !extra.contains(name) {
                extra.push(name.clone());
            }
        }
    }
    columns.extend(extra);
    columns
}

/// Rows of one table as ordered JSON objects (schema field order; the
/// nested payload goes through `serde_json`'s own map, which sorts deep
/// keys — order there carries no meaning, the top level does).
fn rows_to_json(
    table: &cage_core::value::Table,
    source: &str,
    schema: &Schema,
) -> Result<Vec<IndexMap<String, serde_json::Value>>, String> {
    let columns = ordered_columns(table, schema);
    let mut rows = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        let mut obj = IndexMap::with_capacity(row.fields.len());
        for name in &columns {
            if let Some(tv) = row.fields.get(name) {
                obj.insert(
                    name.clone(),
                    value_to_json(&tv.value, source, &table.name, name)?,
                );
            }
        }
        rows.push(obj);
    }
    Ok(rows)
}

fn value_to_json(
    value: &cage_core::value::Value,
    source: &str,
    table: &str,
    field: &str,
) -> Result<serde_json::Value, String> {
    use cage_core::value::Value;
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::UInt(u) => serde_json::Value::from(*u),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::String(s) => serde_json::Value::from(s.clone()),
        // A text source never carries bytes — a base64 write would not
        // read back the same, so refuse instead of mangling the value.
        Value::Bytes(_) => {
            return Err(format!(
                "cannot write {table}.{field}: bytes value has no text-source form ({source})"
            ));
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(value_to_json(item, source, table, field)?);
            }
            serde_json::Value::from(out)
        }
        Value::Object(entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                map.insert(k.clone(), value_to_json(v, source, table, field)?);
            }
            serde_json::Value::Object(map)
        }
    })
}

/// One CSV file = one table, headers in schema declaration order (see
/// [`ordered_columns`]), cells formatted so each value reads back as
/// itself.
fn render_csv(
    tables: &[&cage_core::value::Table],
    source: &str,
    schema: &Schema,
) -> Result<Vec<u8>, String> {
    if tables.len() != 1 {
        return Err(format!(
            "cannot render {source}: a CSV file carries exactly one table, found {}",
            tables.len()
        ));
    }
    let table = tables[0];
    let headers = ordered_columns(table, schema);
    let headers = if headers.is_empty() && !table.primary_key_fields.is_empty() {
        table.primary_key_fields.clone()
    } else {
        headers
    };
    let mut writer = csv::Writer::from_writer(Vec::new());
    if !headers.is_empty() {
        writer
            .write_record(&headers)
            .map_err(|e| format!("cannot render {source}: {e}"))?;
    }
    for row in &table.rows {
        let mut record = Vec::with_capacity(headers.len());
        for header in &headers {
            match row.fields.get(header) {
                Some(tv) => record.push(csv_cell(&tv.value, source, &table.name, header)?),
                None => record.push(String::new()),
            }
        }
        writer
            .write_record(&record)
            .map_err(|e| format!("cannot render {source}: {e}"))?;
    }
    writer
        .flush()
        .map_err(|e| format!("cannot render {source}: {e}"))?;
    writer
        .into_inner()
        .map_err(|e| format!("cannot render {source}: {e}"))
}

/// One CSV cell: the source reader's inference rules inverted, so a
/// written cell parses back as the same value — bool before number,
/// integral floats carry a `.0` to stay clear of the integer branches,
/// an empty cell is null.
fn csv_cell(
    value: &cage_core::value::Value,
    source: &str,
    table: &str,
    field: &str,
) -> Result<String, String> {
    use cage_core::value::Value;
    Ok(match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) if f.fract() == 0.0 => format!("{f}.0"),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        // No round-trippable cell form: refusing beats writing something
        // that silently reads back as a string.
        Value::Bytes(_) | Value::Array(_) | Value::Object(_) => {
            return Err(format!(
                "cannot write {table}.{field}: {} value has no CSV cell form ({source})",
                value.type_name()
            ));
        }
    })
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
    env: Option<&str>,
    incremental: bool,
    no_cache: bool,
) -> Result<BuildOutput, BuildFailure> {
    let level = match level.parse::<ValidationLevel>() {
        Ok(l) => l,
        Err(e) => return Err(BuildFailure::Io(e)),
    };
    let project = match load_project(path, no_cache) {
        Ok(p) => p,
        Err(e) => return Err(BuildFailure::Io(e)),
    };
    let project = match project_with_env(project, env) {
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

    // Incremental build, three layers (docs/build.md 增量构建):
    // Layer 1: skip regeneration entirely when the previous manifest recorded
    // the same schema/source/target fingerprints (same profile) and every
    // artifact it lists is still on disk.
    // Layer 2 (dependency-graph propagation): when only data changed (schema
    // and target fingerprints identical — the schema drives code-target
    // shapes, so any schema change falls back to a full build), per-table
    // hashes detect which tables changed, the dependency graph propagates
    // the change set to its transitive dependents, and only affected tables
    // are regenerated; untouched artifacts are carried over from disk.
    // Layer 3 (target propagation): when a target's config moved (or a
    // target was added/removed) while the schema stayed put, only the
    // changed targets' artifacts regenerate — each from the full document,
    // since a target's bytes depend on every table it serializes — while
    // unchanged targets keep the layer-2 table propagation. Removed or
    // re-configured targets' stale artifacts are deleted from disk.
    // Generators are deterministic, so the merged manifest matches a full
    // build byte-for-byte. Layers 2/3 also fall back to a full build when
    // the previous manifest predates per-table hashes or target records, a
    // table was deleted, or an untouched artifact is missing/unreadable.
    let target_records = ManifestGenerator::target_records(&build_profile.targets);
    let prev_manifest = if incremental {
        load_manifest(&manifest_dir).ok()
    } else {
        None
    };

    let mut affected: Option<HashSet<String>> = None;
    let mut carried: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    // Output dirs of targets whose config moved (or that were removed):
    // their previous artifacts are neither carried nor trusted — the
    // targets regenerate from the full document, and stale paths that no
    // longer reappear are deleted after generation succeeds.
    let mut regenerate_prefixes: Vec<String> = Vec::new();
    if let Some(prev) = &prev_manifest {
        let targets_match = prev.targets == target_records;
        let unchanged = prev.profile == profile
            && prev.environment.as_deref() == env
            && targets_match
            && prev.schema_hash == schema_hash
            && prev.source_hash == source_hash
            && prev.artifacts.keys().all(|rel| path.join(rel).is_file());
        if unchanged {
            return Err(BuildFailure::UpToDate {
                artifacts: prev.artifacts.len(),
                manifest: manifest_dir.join("manifest.json"),
            });
        }
        // Legacy manifests carry no target records: one full build
        // migrates them, then target propagation participates.
        let layer2_ok = prev.profile == profile
            && prev.environment.as_deref() == env
            && prev.schema_hash == schema_hash
            && !prev.table_hashes.is_empty()
            && !prev.targets.is_empty()
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

            // Target-level diff, identity (format, output_dir): a hash
            // move regenerates that target, a removal marks its old
            // output dir stale.
            let cur_keys: HashSet<(&str, &str)> = target_records
                .iter()
                .map(|r| (r.format.as_str(), r.output_dir.as_str()))
                .collect();
            let changed_targets: Vec<String> = target_records
                .iter()
                .filter(|r| {
                    prev.targets
                        .iter()
                        .find(|p| p.format == r.format && p.output_dir == r.output_dir)
                        .is_none_or(|p| p.hash != r.hash)
                })
                .map(|r| r.output_dir.clone())
                .collect();
            let removed_dirs: Vec<String> = prev
                .targets
                .iter()
                .filter(|p| !cur_keys.contains(&(p.format.as_str(), p.output_dir.as_str())))
                .map(|p| p.output_dir.clone())
                .collect();
            regenerate_prefixes = changed_targets;
            regenerate_prefixes.extend(removed_dirs);
            regenerate_prefixes.sort();
            regenerate_prefixes.dedup();

            let mut carry_ok = true;
            for (rel, info) in &prev.artifacts {
                // Shared units (enum files) are schema-wide and are
                // always regenerated — never carried.
                let Some(table) = &info.table else { continue };
                if aff.contains(table) {
                    continue;
                }
                if regenerate_prefixes
                    .iter()
                    .any(|dir| rel == dir || rel.starts_with(&format!("{dir}/")))
                {
                    continue;
                }
                match std::fs::read(path.join(rel)) {
                    Ok(bytes) => {
                        carried.push((rel.clone(), bytes, info.format.clone(), info.table.clone()));
                    }
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

    // Generation inputs per target: full when layer 2 is off or the
    // target's config moved (a target's bytes depend on every table it
    // serializes); otherwise filtered to the affected tables. Code targets
    // keep the full enum set (the shared enums unit renders from
    // schema-wide enums even when only some tables rebuild).
    let filtered_inputs = |keep: &HashSet<String>| {
        let mut fs = schema.clone();
        fs.tables.retain(|name, _| keep.contains(name));
        let mut fd = normalized.clone();
        fd.tables.retain(|name, _| keep.contains(name));
        (fs, fd)
    };
    let target_inputs = |target: &TargetConfig| -> (Schema, Document) {
        let target_changed = regenerate_prefixes.contains(&target.output_dir);
        if target_changed {
            return (schema.clone(), normalized.clone());
        }
        match &affected {
            Some(keep) => filtered_inputs(keep),
            None => (schema.clone(), normalized.clone()),
        }
    };

    let carried_len = carried.len();
    let mut artifacts = carried;
    for target in &build_profile.targets {
        let (gen_schema, gen_doc) = target_inputs(target);
        let generated = match code_target_items(target, &gen_schema, &schema_hash, path) {
            // Bundled code targets (cs/python/lua/ts/…) are schema-driven
            // and infallible; the user-template target can fail (template
            // fault or missing directory) and surfaces through Io.
            Some(Ok(items)) => Ok(items),
            Some(Err(e)) => Err(BuildFailure::Io(e)),
            None => match target.format.as_str() {
                "json" => cage_target_json::JsonTargetGenerator::from_config(target)
                    .generate(&gen_doc, &[])
                    .map_err(BuildFailure::Validation),
                "csv" => cage_target_csv::CsvTargetGenerator::from_config(target)
                    .generate(&gen_doc, &[])
                    .map_err(BuildFailure::Validation),
                "msgpack" | "messagepack" => {
                    cage_target_msgpack::MsgPackTargetGenerator::from_config(target)
                        .generate(&gen_doc, &[])
                        .map_err(BuildFailure::Validation)
                }
                other => {
                    return Err(BuildFailure::Io(format!(
                        "unsupported target format '{other}'"
                    )));
                }
            },
        };
        let items = generated?;
        write_artifact_files(path, items, &target.format, &mut artifacts)
            .map_err(BuildFailure::Io)?;
    }
    // Canonical artifact order (path-sorted) so a layer-2 incremental run and
    // a full build produce byte-identical manifests (same inputs → same
    // manifest bytes, the determinism contract).
    artifacts.sort_by(|a, b| a.0.cmp(&b.0));

    // Stale artifacts of removed or re-configured targets: only paths the
    // previous manifest recorded, only after generation succeeded, and
    // only when they did not reappear. Full builds leave such files on
    // disk; the incremental path does better.
    if !regenerate_prefixes.is_empty() {
        if let Some(prev) = &prev_manifest {
            let fresh: HashSet<&str> = artifacts.iter().map(|(p, ..)| p.as_str()).collect();
            for rel in prev.artifacts.keys() {
                let stale = regenerate_prefixes
                    .iter()
                    .any(|dir| rel == dir || rel.starts_with(&format!("{dir}/")))
                    && !fresh.contains(rel.as_str());
                if !stale {
                    continue;
                }
                match std::fs::remove_file(path.join(rel)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => eprintln!("warning: could not remove stale artifact '{rel}' ({e})"),
                }
            }
        }
    }

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .with_environment(env)
    .with_targets(&build_profile.targets)
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

fn run_build(
    path: &Path,
    level: &str,
    profile: &str,
    env: Option<&str>,
    incremental: bool,
    no_cache: bool,
) -> i32 {
    match build_project(path, level, profile, env, incremental, no_cache) {
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

/// Pack a finished build into its self-verifying snapshot directory
/// (`<output_dir>/snapshot/<profile>[-<env>]-<build_id[..12]>`) and
/// self-verify it clean before handing it out. Shared by `cage snapshot`
/// and `cage registry publish` — the packing format is core's
/// (`cage_core::snapshot`), the deterministic directory name is the
/// fingerprint, not a date. Environment-ized builds tag the name so packs
/// are distinguishable at a glance; the `build_id` segment already rotates
/// with the environment (the resolved schema feeds the `schema_hash`).
fn pack_snapshot(path: &Path, out: &BuildOutput) -> Result<(PathBuf, usize), String> {
    let schema_json =
        serde_json::to_vec_pretty(&out.schema).map_err(|e| format!("schema serialization: {e}"))?;
    let manifest_json = std::fs::read(&out.manifest_path)
        .map_err(|e| format!("{}: {e}", out.manifest_path.display()))?;
    let files = cage_core::snapshot::snapshot_files(
        &manifest_json,
        &schema_json,
        &out.artifacts,
        &out.output_dir,
        &out.manifest.build_id,
        &out.manifest.content_hash,
    );
    let dir_name = match &out.manifest.environment {
        Some(env) => format!("{}-{env}-{}", out.profile, &out.manifest.build_id[..12]),
        None => format!("{}-{}", out.profile, &out.manifest.build_id[..12]),
    };
    let snap_dir = path.join(&out.output_dir).join("snapshot").join(dir_name);
    write_snapshot_files(&snap_dir, &files)?;
    // Self-check: the just-written snapshot must verify clean.
    let report = cage_core::snapshot::verify_snapshot(&snap_dir)?;
    debug_assert!(
        report.ok,
        "self-verification mismatch: {:?}",
        report.mismatches
    );
    Ok((snap_dir, files.len()))
}

/// `cage snapshot` — build the profile, then package the build into a
/// self-verifying Configuration Snapshot under
/// `<output_dir>/snapshot/<profile>[-<env>]-<build_id[..12]>`. Identical
/// inputs → identical snapshot bytes (the determinism contract). With
/// `--env` the schema's `env_overrides` are resolved before validation,
/// and the packed schema.json/artifacts carry the environment's rules.
fn run_snapshot(path: &Path, profile: &str, env: Option<&str>) -> i32 {
    // A snapshot packages a fresh full build — no incremental carry-over.
    match build_project(path, "gamerule", profile, env, false, false) {
        Ok(out) => match pack_snapshot(path, &out) {
            Ok((snap_dir, files)) => {
                let env_note = env.map_or_else(String::new, |e| format!(", env '{e}'"));
                println!(
                    "cage snapshot: OK (profile '{profile}'{env_note}, {files} files, {} artifacts, verified, {})",
                    out.artifacts.len(),
                    snap_dir.display()
                );
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        },
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

/// Resolve the registry root for publish/list: the `--registry` flag wins,
/// else the project's `[registry].path` (relative to the project root).
fn registry_root(
    path: &Path,
    config: &ProjectConfig,
    flag: Option<&Path>,
) -> Result<PathBuf, String> {
    match flag {
        Some(r) => Ok(r.to_path_buf()),
        None => match &config.registry {
            Some(reg) => Ok(path.join(&reg.path)),
            None => Err(format!(
                "{E1802} no registry root (pass --registry or set '[registry] path' in cage.toml)"
            )),
        },
    }
}

/// `cage registry publish` — build the profile, pack its self-verifying
/// snapshot and enter it into the local registry as
/// `<package>/<version>`. Package defaults to `project.name`, version to
/// `project.version`. Only a ledger-verified snapshot is written; a
/// byte-identical re-publish is a no-op, different bytes for the same
/// version are an E1801 conflict (the registry never rewrites history).
/// With `--env` the entry's packed manifest names the environment; one
/// version stays one pack — a different environment packs differently and
/// the same E1801 conflict guards it.
fn run_registry_publish(
    path: &Path,
    profile: &str,
    env: Option<&str>,
    package: Option<&str>,
    version: Option<&str>,
    registry_flag: Option<&Path>,
) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let package = package.map_or_else(|| config.project.name.clone(), str::to_string);
    let version = if let Some(v) = version {
        v.to_string()
    } else if let Some(v) = &config.project.version {
        v.clone()
    } else {
        eprintln!(
            "error: no version to publish (pass --version or set project.version in cage.toml)"
        );
        return 2;
    };
    let reg_root = match registry_root(path, &config, registry_flag) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // R3: remote registry roots are read-only — publish stays local.
    let root_spec = registry_flag.map_or_else(
        || config.registry.as_ref().map(|r| r.path.clone()),
        |p| Some(p.to_string_lossy().into_owned()),
    );
    if let Some(spec) = &root_spec {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; publish requires a local registry path"
            );
            return 2;
        }
    }

    // A publish is a fresh full build — no incremental carry-over.
    match build_project(path, "gamerule", profile, env, false, false) {
        Ok(out) => match pack_snapshot(path, &out) {
            Ok((snap_dir, files)) => {
                match cage_core::registry::publish(&reg_root, &package, &version, &snap_dir) {
                    Ok(report) => {
                        let env_note = env.map_or_else(String::new, |e| format!(", env '{e}'"));
                        println!(
                            "cage registry: published {package}/{version} (profile '{profile}'{env_note}, {files} files, build_id {}, content_hash {}){}",
                            &out.manifest.build_id[..12],
                            &out.manifest.content_hash[..12],
                            if report.already_identical {
                                " — identical, no-op"
                            } else {
                                ""
                            }
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        1
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        },
        Err(BuildFailure::Validation(diags)) => {
            println!("{}", diags.render(false));
            println!(
                "cage registry: FAILED validation ({} errors)",
                diags.errors().len()
            );
            1
        }
        Err(BuildFailure::Io(msg)) => {
            eprintln!("error: {msg}");
            2
        }
        Err(BuildFailure::UpToDate { .. }) => {
            // incremental is false for registry publishes — unreachable.
            eprintln!("error: internal: registry publish build did not rebuild");
            2
        }
    }
}

/// `cage registry list` — every package with its recorded versions, in
/// deterministic (name, dotted-numeric version) order.
fn run_registry_list(registry_flag: Option<&Path>) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    // R3: remote roots cannot be listed — the read-only protocol has no
    // package enumeration.
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — the read-only protocol has no \
                 package enumeration; reference packages directly via registry:<package>"
            );
            return 2;
        }
    }
    match cage_core::registry::packages(reg_root) {
        Ok(indexes) => {
            if indexes.is_empty() {
                println!("cage registry: empty ({})", reg_root.display());
                return 0;
            }
            println!(
                "cage registry: {} package(s) in {}",
                indexes.len(),
                reg_root.display()
            );
            for index in indexes {
                println!("{}", index.package);
                for entry in &index.entries {
                    println!(
                        "  {:<12} build {}  content {}  {} files",
                        entry.version,
                        &entry.build_id[..entry.build_id.len().min(12)],
                        &entry.content_hash[..entry.content_hash.len().min(12)],
                        entry.files
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// `cage registry export` — pack a published entry into a deterministic tar
/// bundle (A1). The bundle is byte-reproducible from the registry alone
/// (member-name order, zeroed mtime/uid/gid) and self-verifying — the entry
/// ledger rides along, and A2's import re-checks it before anything enters
/// a registry. `--compress zstd` wraps that same tar in a single zstd frame
/// at a fixed level, so the wrapped bytes stay reproducible; import sniffs
/// the frame magic, so both container forms load identically. Local
/// registry roots only: the remote read protocol has no file enumeration,
/// so remote consumers resolve `registry:` sources instead.
fn run_registry_export(
    package_spec: &str,
    output: &Path,
    compress: Option<&str>,
    sign: bool,
    key_env: Option<&str>,
    registry_flag: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — export packs a local entry; \
                 fetch remote packages via registry: source roots instead"
            );
            return 2;
        }
    }
    // clap's value_parser already restricts --compress to the literal
    // "zstd"; the guard keeps a non-CLI caller honest anyway.
    let compression = match compress {
        None => cage_core::registry::BundleCompression::Plain,
        Some("zstd") => cage_core::registry::BundleCompression::Zstd,
        Some(other) => {
            eprintln!("error: --compress only accepts 'zstd', got '{other}'");
            return 2;
        }
    };
    let (package, version) = match package_spec.split_once('@') {
        Some((p, v)) => (p, Some(v)),
        None => (package_spec, None),
    };
    match cage_core::registry::export_bundle(reg_root, package, version, compression, output) {
        Ok(report) => {
            println!(
                "cage registry: exported {}/{} → {} ({} entry files + index excerpt{})",
                report.package,
                report.version,
                output.display(),
                report.files,
                if matches!(compression, cage_core::registry::BundleCompression::Zstd) {
                    ", zstd container"
                } else {
                    ""
                }
            );
            // A6: detached ed25519 signature over the exact bundle bytes,
            // riding beside it as `<output>.sig`. The signing key resolves
            // from the env var named by --key-env only — never from
            // config, never echoed.
            if sign {
                let key_env = key_env.unwrap_or_else(|| {
                    eprintln!("error: --sign requires --key-env");
                    std::process::exit(2);
                });
                let signed = || -> Result<(), String> {
                    let key = cage_core::registry::signing_key_from_env(key_env)?;
                    let bytes = std::fs::read(output)
                        .map_err(|e| format!("cannot read back {}: {e}", output.display()))?;
                    let sig = cage_core::registry::sign_bundle_bytes(&bytes, &key);
                    cage_core::registry::write_bundle_signature(output, &sig)?;
                    Ok(())
                };
                match signed() {
                    Ok(()) => {
                        println!(
                            "cage registry: signed bundle → {}",
                            cage_core::registry::sidecar_path(output).display()
                        );
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 1;
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry import` — enter a bundle into the local registry (A2).
/// The bundle's riding ledger gates the entry exactly like publish does;
/// refused bytes never touch the target. The container form is sniffed from
/// the file head, not the name: plain tars load unchanged and a
/// `--compress zstd` frame decompresses first, so both import identically
/// under any extension. Local registry roots only — import writes, and
/// writes over the network stay out of scope.
fn run_registry_import(
    file: &Path,
    verify_sig: bool,
    key_env: Option<&str>,
    dry_run: bool,
    registry_flag: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — import writes a local registry; \
                 publish to a local root and host it statically instead"
            );
            return 2;
        }
    }
    // A6: the signature gate runs before the ledger trust gate — a
    // bundle that cannot be attributed to the trusted key is refused
    // before anything is staged, even though the ledger would also
    // catch byte-level tampering. The trusted key comes from --key-env;
    // the riding public key is never the anchor.
    if verify_sig {
        let key_env = key_env.unwrap_or_else(|| {
            eprintln!("error: --verify-sig requires --key-env");
            std::process::exit(2);
        });
        let gate = || -> Result<(), String> {
            let trusted = cage_core::registry::verifying_key_from_env(key_env)?;
            let bytes =
                std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let sidecar = cage_core::registry::read_bundle_signature(file)?;
            cage_core::registry::verify_bundle_bytes(&bytes, &sidecar, &trusted)
        };
        if let Err(e) = gate() {
            eprintln!("error: {e}");
            return 1;
        }
        println!("cage registry: signature verified");
    }
    match cage_core::registry::import_bundle(reg_root, file, dry_run) {
        Ok(report) => {
            println!(
                "cage registry: {}{}/{} ({} files){}",
                if report.dry_run {
                    "would import "
                } else {
                    "imported "
                },
                report.package,
                report.version,
                report.files,
                if report.already_identical {
                    " — identical, no-op"
                } else {
                    ""
                }
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry keygen` (A6): generate an ed25519 signing keypair. The
/// seed goes only to the user-named file (owner-only permissions); stdout
/// carries the public key and the usage lines — the secret never appears
/// in any output.
fn run_registry_keygen(output: &Path) -> i32 {
    match cage_core::registry::generate_signing_key() {
        Ok(key) => {
            let (seed, public) = cage_core::registry::key_material(&key);
            if let Err(e) = std::fs::write(output, format!("{seed}\n")) {
                eprintln!("error: cannot write {}: {e}", output.display());
                return 1;
            }
            // Best-effort owner-only permissions (unix); failure is not
            // fatal — the file sits in the user's own workspace either way.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o600));
            }
            println!("cage registry keygen: seed (secret) → {}", output.display());
            println!("cage registry keygen: public key: {public}");
            println!(
                "export CAGE_SIGNING_KEY=$(cat {}) # then: cage registry export --sign --key-env CAGE_SIGNING_KEY",
                output.display()
            );
            println!(
                "export CAGE_VERIFYING_KEY={public} # consumers: cage registry import --verify-sig --key-env CAGE_VERIFYING_KEY"
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry push` — upload a published entry from the project's local
/// registry (`[registry].path`) to a remote http(s) registry root (A3): one
/// PUT per entry file, the package index written last and merged over the
/// remote's existing entries. The bearer token resolves from the
/// environment via `--auth-env` or `[registry].auth_env` (anonymous when
/// neither is set); the value never enters logs or the config.
fn run_registry_push(
    path: &Path,
    package_spec: Option<&str>,
    remote_flag: Option<&str>,
    presign_map_path: Option<&Path>,
    auth_env_flag: Option<&str>,
    dry_run: bool,
) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let Some(registry_cfg) = &config.registry else {
        eprintln!(
            "error: {E1802} no local registry to push from (set '[registry] path' in cage.toml — \
             the local registry is the push source)"
        );
        return 2;
    };
    let source_root = path.join(&registry_cfg.path);
    // Package/version defaults mirror publish: the project's own name, at
    // the source registry's latest when no @version is given.
    let (package, version) = match package_spec {
        Some(spec) => match spec.split_once('@') {
            Some((p, v)) => (p.to_string(), Some(v.to_string())),
            None => (spec.to_string(), None),
        },
        None => (config.project.name.clone(), None),
    };

    // Presigned route (design §47 A3): the map is the only credential —
    // no bearer token is read or sent, and --registry is meaningless.
    if let Some(map_path) = presign_map_path {
        let bytes = match fs::read(map_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "error: {E2101} cannot read presign map {}: {e}",
                    map_path.display()
                );
                return 2;
            }
        };
        let map = match cage_core::registry::parse_presign_map(&bytes) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        return match cage_core::registry::push_entry_presigned(
            &source_root,
            &map,
            &package,
            version.as_deref(),
            dry_run,
        ) {
            Ok(report) => {
                println!(
                    "cage registry: {}{}/{} → presigned targets ({}) ({} file(s){}){}",
                    if report.dry_run {
                        "would push "
                    } else {
                        "pushed "
                    },
                    report.package,
                    report.version,
                    map_path.display(),
                    report.files,
                    if report.dry_run { ", zero PUT" } else { "" },
                    if report.already_identical {
                        " — identical, no-op"
                    } else {
                        ""
                    }
                );
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }

    // Direct route.
    let Some(remote_flag) = remote_flag else {
        eprintln!(
            "error: push needs a destination: --registry <http(s) root> or --presign-map <file>"
        );
        return 2;
    };
    // Flag overrides the config declaration; neither set pushes anonymously.
    let auth_env = auth_env_flag.or(registry_cfg.auth_env.as_deref());
    if !remote::is_remote_root(remote_flag) {
        eprintln!(
            "error: push targets a remote http(s) registry root, got '{remote_flag}' — local \
             destinations belong to 'cage registry publish'"
        );
        return 2;
    }
    match cage_core::registry::push_entry(
        &source_root,
        remote_flag,
        &package,
        version.as_deref(),
        auth_env,
        dry_run,
    ) {
        Ok(report) => {
            println!(
                "cage registry: {}{}/{} → {} ({} file(s){}){}",
                if report.dry_run {
                    "would push "
                } else {
                    "pushed "
                },
                report.package,
                report.version,
                remote_flag,
                report.files,
                if report.dry_run { ", zero PUT" } else { "" },
                if report.already_identical {
                    " — identical, no-op"
                } else {
                    ""
                }
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry verify` — the full-registry audit (R4): every recorded
/// entry exists and self-verifies (its bytes re-hashed against its ledger,
/// the index record matching the ledger's `build_id`/`content_hash`), and no
/// entry directory sits on disk without an index record. Read-only; a
/// finding is an E1803 and fails the command.
fn run_registry_verify(registry_flag: Option<&Path>) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; verify requires a local registry path"
            );
            return 2;
        }
    }
    match cage_core::registry::verify_registry(reg_root) {
        Ok(report) => {
            if report.ok() {
                println!(
                    "cage registry: OK — {} package(s), {} entry/ies verified in {}",
                    report.packages,
                    report.entries_checked,
                    reg_root.display()
                );
                0
            } else {
                println!(
                    "cage registry: {} problem(s) across {} package(s) in {}",
                    report.problems.len(),
                    report.packages,
                    reg_root.display()
                );
                for problem in &report.problems {
                    println!("  {problem}");
                }
                2
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// `cage registry gc [--keep N] [--dry-run]` — per package keep the newest
/// `keep` versions (the rollback window; consumers pin old @versions and
/// keep resolving), delete the older entries plus orphaned directories,
/// rewrite the touched indexes. --dry-run reports the identical removal
/// list without touching anything.
fn run_registry_gc(keep: usize, dry_run: bool, registry_flag: Option<&Path>) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; gc requires a local registry path"
            );
            return 2;
        }
    }
    match cage_core::registry::gc_registry(reg_root, keep, dry_run) {
        Ok(report) => {
            if report.removed.is_empty() {
                println!("cage registry: nothing to collect (keep {keep})");
            } else {
                println!(
                    "cage registry: {} to remove, {} index(es) rewritten{}",
                    report.removed.len(),
                    report.rewritten,
                    if dry_run {
                        " (dry run — nothing touched)"
                    } else {
                        ""
                    }
                );
                for removed in &report.removed {
                    println!("  {removed}");
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// `cage registry remove <package> <version> [--dry-run]` — explicitly
/// remove one entry (version directory + index record). The package index
/// stays even when empty, so the same snapshot can be re-published
/// afterwards; removing an unrecorded version is E1802.
fn run_registry_remove(
    package: &str,
    version: &str,
    dry_run: bool,
    registry_flag: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; remove requires a local registry path"
            );
            return 2;
        }
    }
    match cage_core::registry::remove_entry(reg_root, package, version, dry_run) {
        Ok(()) => {
            if dry_run {
                println!(
                    "cage registry: would remove {package}/{version} from {}",
                    reg_root.display()
                );
            } else {
                println!(
                    "cage registry: removed {package}/{version} (registry {})",
                    reg_root.display()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// `cage gen` — schema-driven code generation only. Writes the code-target
/// artifacts (cs/python/lua/ts/js/cpp/go/java) of a profile without running
/// data validation:
/// types and metadata all come from the Schema, so source rows are not
/// needed. Data targets (json/csv) in the profile are skipped — run
/// `cage build` for those. The manifest is written like a build's, so gen
/// and build manifests share the same 口径 (schema/source/content hashes).
fn run_gen(path: &Path, profile: &str, env: Option<&str>, no_cache: bool) -> i32 {
    let project = match load_project(path, no_cache) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_with_env(project, env) {
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
        match code_target_items(target, &schema, &schema_hash, path) {
            Some(Ok(items)) => {
                if let Err(e) = write_artifact_files(path, items, &target.format, &mut artifacts) {
                    eprintln!("error: {e}");
                    return 2;
                }
            }
            Some(Err(e)) => {
                eprintln!("error: {e}");
                return 2;
            }
            None => {}
        }
    }
    if artifacts.is_empty() {
        eprintln!(
            "error: profile '{profile}' has no code targets (cs/python/lua/ts/js/cpp/go/java/proto/jsonschema/template)"
        );
        return 2;
    }

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .with_environment(env)
    .with_targets(&build_profile.targets)
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
/// format is a data target (json/csv) rather than a code target. The
/// bundled language generators are schema-driven, deterministic, and
/// infallible; the user-template target (`format = "template"`) renders
/// `.tera` files from disk and can fail — its `Err` names the offending
/// template file or the missing template directory. Each file header is
/// stamped with the manifest's schema hash.
type CodeTargetItems = Result<Vec<(String, Vec<u8>)>, String>;

fn code_target_items(
    target: &TargetConfig,
    schema: &Schema,
    schema_hash: &str,
    root: &Path,
) -> Option<CodeTargetItems> {
    // "csharp"/"python"/"typescript" per docs; short aliases ("cs"/"py"/
    // "ts"/"js") and the common alternates ("golang", "c++"/"cxx") accepted.
    match target.format.as_str() {
        "cs" | "csharp" => Some(Ok(cage_target_cs::CsTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "python" | "py" => Some(Ok(cage_target_py::PyTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "lua" => Some(Ok(cage_target_lua::LuaTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "typescript" | "ts" | "javascript" | "js" => {
            Some(Ok(cage_target_ts::TsTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash))))
        }
        "cpp" | "c++" | "cxx" => Some(Ok(cage_target_cpp::CppTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "go" | "golang" => Some(Ok(cage_target_go::GoTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "java" => Some(Ok(cage_target_java::JavaTargetGenerator::from_config(
            target,
        )
        .generate(schema, Some(schema_hash)))),
        "proto" | "protobuf" => Some(Ok(cage_target_proto::ProtoTargetGenerator::from_config(
            target,
        )
        .generate(schema, Some(schema_hash)))),
        // Standard JSON Schema documents (draft-07 default, 2020-12 via
        // options.draft) — the one bundled target that can fail on config
        // alone (an unknown draft value), like the template target.
        "jsonschema" | "json-schema" | "json_schema" => {
            let gen = cage_target_jsonschema::JsonSchemaTargetGenerator::from_config(target);
            Some(gen.generate(schema, Some(schema_hash)))
        }
        "template" => {
            let mut gen = cage_target_template::TemplateTargetGenerator::from_config(target);
            // `options.template_dir` (default `.cage/templates`) is relative
            // to the project root, like output_dir — not the process CWD.
            if gen.template_dir.is_relative() {
                gen.template_dir = root.join(&gen.template_dir);
            }
            // G4 filter libraries: `options.lang_filters = "py,go"` mounts
            // the named languages' type/default filters so user templates
            // can reuse the exact decisions the official generators make.
            let langs = match lang_filters(target) {
                Ok(langs) => langs,
                Err(e) => return Some(Err(e)),
            };
            let holder = java_holder(target);
            Some(
                gen.generate_with_setup(schema, Some(schema_hash), |tera, schema| {
                    for lang in &langs {
                        match lang.as_str() {
                            "py" | "python" => cage_target_py::register_filters(tera, schema),
                            "cs" | "csharp" => cage_target_cs::register_filters(tera, schema),
                            "ts" | "typescript" => cage_target_ts::register_filters(tera, schema),
                            "go" | "golang" => cage_target_go::register_filters(tera, schema),
                            "java" => cage_target_java::register_filters(
                                tera,
                                schema,
                                Some(holder.as_str()),
                            ),
                            "cpp" | "c++" | "cxx" => {
                                cage_target_cpp::register_filters(tera, schema);
                            }
                            "lua" => cage_target_lua::register_filters(tera, schema),
                            _ => {}
                        }
                    }
                }),
            )
        }
        _ => None,
    }
}

/// `options.lang_filters` (design §22 G4): comma-separated language keys
/// whose filter libraries mount into the user-template engine. Unknown
/// keys fail up front, before any rendering.
fn lang_filters(target: &TargetConfig) -> Result<Vec<String>, String> {
    let Some(v) = target.options.as_ref().and_then(|o| o.get("lang_filters")) else {
        return Ok(Vec::new());
    };
    let s = v
        .as_str()
        .ok_or_else(|| "template target: options.lang_filters must be a string".to_string())?;
    let mut langs = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "py" | "python" | "cs" | "csharp" | "ts" | "typescript" | "go" | "golang" | "java"
            | "cpp" | "c++" | "cxx" | "lua" => langs.push(part.to_string()),
            other => {
                return Err(format!(
                    "template target: unknown lang_filters entry '{other}' \
                     (supported: py/cs/ts/go/java/cpp/lua)"
                ));
            }
        }
    }
    Ok(langs)
}

/// The shared-enums holder class stem for the Java filter library — the
/// same derivation `JavaTargetGenerator::from_config` applies
/// (`options.enums_file`'s file stem, default `CageEnums`).
fn java_holder(target: &TargetConfig) -> String {
    let f = target
        .options
        .as_ref()
        .and_then(|o| o.get("enums_file"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("CageEnums.java");
    Path::new(f)
        .file_stem()
        .map_or_else(|| f.to_string(), |s| s.to_string_lossy().into_owned())
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

fn run_inspect(path: &Path, table: Option<&str>, no_cache: bool) -> i32 {
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
                env,
                no_cache,
            } => {
                let env = env.as_deref().unwrap_or("<base>");
                format!(
                    "check {} {level} {profile} {env} {no_cache}",
                    path.display()
                )
            }
            Commands::Build {
                path,
                level,
                profile,
                env,
                incremental,
                no_cache,
            } => format!(
                "build {} {level} {profile} {} {incremental} {no_cache}",
                path.display(),
                env.as_deref().unwrap_or("<base>")
            ),
            Commands::Inspect {
                path,
                table,
                no_cache,
            } => format!(
                "inspect {} {} {no_cache}",
                path.display(),
                table.as_deref().unwrap_or("<all>")
            ),
            Commands::Gen {
                path,
                profile,
                env,
                no_cache,
            } => format!(
                "gen {} {profile} {} {no_cache}",
                path.display(),
                env.as_deref().unwrap_or("<base>")
            ),
            Commands::Diff { baseline, target } => {
                format!("diff {} {}", baseline.display(), target.display())
            }
            Commands::Snapshot {
                path,
                profile,
                env,
                verify,
            } => format!(
                "snapshot {} {profile} {} {verify}",
                path.display(),
                env.as_deref().unwrap_or("<base>")
            ),
            Commands::Web { path, port } => format!("web {} {port}", path.display()),
            Commands::Registry { cmd } => match cmd {
                RegistryCmd::Publish {
                    path,
                    profile,
                    env,
                    package,
                    version,
                    registry,
                } => format!(
                    "registry publish {} {profile} {} {} {} {}",
                    path.display(),
                    env.as_deref().unwrap_or("<base>"),
                    package.as_deref().unwrap_or("<project.name>"),
                    version.as_deref().unwrap_or("<project.version>"),
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<cage.toml>").display(), Path::display)
                ),
                RegistryCmd::List { registry } => format!(
                    "registry list {}",
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<flag required>").display(), Path::display)
                ),
                RegistryCmd::Verify { registry } => format!(
                    "registry verify {}",
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<flag required>").display(), Path::display)
                ),
                RegistryCmd::Gc {
                    keep,
                    dry_run,
                    registry,
                } => format!(
                    "registry gc {keep} {dry_run} {}",
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<flag required>").display(), Path::display)
                ),
                RegistryCmd::Remove {
                    package,
                    version,
                    dry_run,
                    registry,
                } => format!(
                    "registry remove {package} {version} {dry_run} {}",
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<flag required>").display(), Path::display)
                ),
                RegistryCmd::Export {
                    package,
                    output,
                    compress,
                    sign,
                    key_env,
                    registry,
                } => format!(
                    "registry export {package} {} {} {sign} {} {}",
                    output.display(),
                    compress.as_deref().unwrap_or("-"),
                    key_env.as_deref().unwrap_or("<no key-env>"),
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<flag required>").display(), Path::display)
                ),
                RegistryCmd::Import {
                    file,
                    verify_sig,
                    key_env,
                    dry_run,
                    registry,
                } => format!(
                    "registry import {} {verify_sig} {} {dry_run} {}",
                    file.display(),
                    key_env.as_deref().unwrap_or("<no key-env>"),
                    registry
                        .as_deref()
                        .map_or_else(|| Path::new("<flag required>").display(), Path::display)
                ),
                RegistryCmd::Keygen { output } => {
                    format!("registry keygen {}", output.display())
                }
                RegistryCmd::Push {
                    path,
                    package,
                    registry,
                    presign_map,
                    auth_env,
                    dry_run,
                } => format!(
                    "registry push {} {} {} {dry_run} {} {}",
                    path.display(),
                    package.as_deref().unwrap_or("<project.name>"),
                    registry.as_deref().unwrap_or("<no registry>"),
                    presign_map
                        .as_deref()
                        .map_or_else(|| Path::new("<no presign-map>").display(), Path::display),
                    auth_env.as_deref().unwrap_or("<no auth_env>")
                ),
            },
            Commands::Migrate {
                path,
                all,
                to,
                write,
            } => format!(
                "migrate {} {all} {} {write}",
                path.display(),
                to.as_deref().unwrap_or("<chain end>")
            ),
            Commands::MigrateDraft {
                from_schema,
                to_schema,
                from,
                to,
                out,
            } => format!(
                "migrate-draft {} {} {from} {to} {}",
                from_schema.display(),
                to_schema.display(),
                out.as_ref()
                    .map_or_else(|| "<stdout>".to_string(), |p| p.display().to_string(),)
            ),
            Commands::SchemaDraft { path, specs, out } => format!(
                "schema-draft {} {} {}",
                path.display(),
                specs.join(","),
                out.as_ref()
                    .map_or_else(|| "<stdout>".to_string(), |p| p.display().to_string(),)
            ),
        }
    }

    #[test]
    fn parse_check_subcommand() {
        let cli = Cli::try_parse_from(["cage", "check", "proj"]).expect("parse check");
        assert_eq!(
            describe(&cli.command),
            "check proj semantic client <base> false"
        );
        // The offline-strictness flag round-trips (S4).
        let cli = Cli::try_parse_from(["cage", "check", "proj", "--no-cache"]).expect("parse flag");
        assert_eq!(
            describe(&cli.command),
            "check proj semantic client <base> true"
        );
    }

    #[test]
    fn parse_build_subcommand() {
        let cli = Cli::try_parse_from(["cage", "build", "proj", "--level", "table"])
            .expect("parse build");
        assert_eq!(
            describe(&cli.command),
            "build proj table client <base> false false"
        );
    }

    #[test]
    fn parse_diff_subcommand() {
        let cli = Cli::try_parse_from(["cage", "diff", "a", "b"]).expect("parse diff");
        assert_eq!(describe(&cli.command), "diff a b");
    }

    #[test]
    fn parse_inspect_subcommand() {
        let cli = Cli::try_parse_from(["cage", "inspect", "proj"]).expect("parse inspect");
        assert_eq!(describe(&cli.command), "inspect proj <all> false");
        // The optional table argument round-trips too.
        let cli = Cli::try_parse_from(["cage", "inspect", "proj", "Item"]).expect("parse table");
        assert_eq!(describe(&cli.command), "inspect proj Item false");
    }

    #[test]
    fn parse_gen_subcommand() {
        let cli =
            Cli::try_parse_from(["cage", "gen", "proj", "--profile", "server"]).expect("parse gen");
        assert_eq!(describe(&cli.command), "gen proj server <base> false");
    }

    #[test]
    fn parse_migrate_subcommand() {
        // Defaults: single segment, dry-run.
        let cli = Cli::try_parse_from(["cage", "migrate", "proj"]).expect("parse migrate");
        assert_eq!(
            describe(&cli.command),
            "migrate proj false <chain end> false"
        );
        let cli = Cli::try_parse_from(["cage", "migrate", "proj", "--all", "--write"])
            .expect("parse migrate all");
        assert_eq!(describe(&cli.command), "migrate proj true <chain end> true");
        let cli = Cli::try_parse_from(["cage", "migrate", "proj", "--to", "1.2.0"])
            .expect("parse migrate to");
        assert_eq!(describe(&cli.command), "migrate proj false 1.2.0 false");
        // `latest` is accepted as a literal target value (resolution to
        // the whole chain happens at run time, chain-dependent).
        let cli = Cli::try_parse_from(["cage", "migrate", "proj", "--to", "latest"])
            .expect("parse migrate to latest");
        assert_eq!(describe(&cli.command), "migrate proj false latest false");
        // --all and --to are mutually exclusive segment selectors.
        let cli = Cli::try_parse_from(["cage", "migrate", "proj", "--all", "--to", "1.2.0"]);
        assert!(cli.is_err(), "--all and --to must conflict");
    }

    #[test]
    fn parse_migrate_draft_subcommand() {
        let cli = Cli::try_parse_from([
            "cage",
            "migrate-draft",
            "old.yaml",
            "new.yaml",
            "--from",
            "1.0.0",
            "--to",
            "2.0.0",
        ])
        .expect("parse migrate-draft");
        assert_eq!(
            describe(&cli.command),
            "migrate-draft old.yaml new.yaml 1.0.0 2.0.0 <stdout>"
        );
        let cli = Cli::try_parse_from([
            "cage",
            "migrate-draft",
            "old.yaml",
            "new.yaml",
            "--from",
            "1.0.0",
            "--to",
            "2.0.0",
            "-o",
            "migrations/0001-draft.yaml",
        ])
        .expect("parse migrate-draft out");
        assert_eq!(
            describe(&cli.command),
            "migrate-draft old.yaml new.yaml 1.0.0 2.0.0 migrations/0001-draft.yaml"
        );
    }

    #[test]
    fn parse_registry_signing_flags() {
        // export --sign requires --key-env (clap `requires`), and the
        // flag pair round-trips through describe.
        let cli = Cli::try_parse_from([
            "cage",
            "registry",
            "export",
            "game",
            "-o",
            "b.tar",
            "--sign",
            "--key-env",
            "K",
            "--registry",
            "reg",
        ])
        .expect("parse export sign");
        assert_eq!(
            describe(&cli.command),
            "registry export game b.tar - true K reg"
        );
        let cli = Cli::try_parse_from([
            "cage",
            "registry",
            "import",
            "b.tar",
            "--verify-sig",
            "--key-env",
            "K",
            "--registry",
            "reg",
        ])
        .expect("parse import verify");
        assert_eq!(
            describe(&cli.command),
            "registry import b.tar true K false reg"
        );
        let cli = Cli::try_parse_from(["cage", "registry", "keygen", "-o", "key.txt"])
            .expect("parse keygen");
        assert_eq!(describe(&cli.command), "registry keygen key.txt");
        assert!(
            Cli::try_parse_from(["cage", "registry", "export", "game", "-o", "b.tar", "--sign"])
                .is_err(),
            "--sign without --key-env must be a usage error"
        );
        assert!(
            Cli::try_parse_from(["cage", "registry", "import", "b.tar", "--verify-sig"]).is_err(),
            "--verify-sig without --key-env must be a usage error"
        );
    }

    #[test]
    fn affected_location_lines_cap_deterministically() {
        // No locations (text-sourced table): no lines at all.
        let step = cage_core::migrate::StepReport {
            step: "set_default(Item.rarity)".into(),
            rows_changed: 0,
            affected_locations: Vec::new(),
        };
        assert_eq!(affected_location_lines(&step), [] as [&str; 0]);

        // Within the cap: one line per row, source order.
        let step = cage_core::migrate::StepReport {
            step: "set_default(Items.rarity)".into(),
            rows_changed: 2,
            affected_locations: vec![
                cage_core::value::SourceLocation::new("config/Items.xlsx")
                    .with_sheet("Items")
                    .with_row(3),
                cage_core::value::SourceLocation::new("config/Items.xlsx")
                    .with_sheet("Items")
                    .with_row(4),
            ],
        };
        assert_eq!(
            affected_location_lines(&step),
            vec![
                "    config/Items.xlsx | Sheet: Items | Row: 3".to_string(),
                "    config/Items.xlsx | Sheet: Items | Row: 4".to_string(),
            ]
        );

        // Past the cap: the first EXCEL_LOCATION_CAP rows, then exactly
        // one summary line for the rest.
        let step = cage_core::migrate::StepReport {
            step: "widen_type(Items.level → Int64)".into(),
            rows_changed: EXCEL_LOCATION_CAP + 3,
            affected_locations: (1..=EXCEL_LOCATION_CAP + 3)
                .map(|row| {
                    cage_core::value::SourceLocation::new("config/Items.xlsx")
                        .with_sheet("Items")
                        .with_row(row)
                })
                .collect(),
        };
        let lines = affected_location_lines(&step);
        assert_eq!(lines.len(), EXCEL_LOCATION_CAP + 1);
        assert!(lines[0].contains("Row: 1"), "{lines:?}");
        let cap = EXCEL_LOCATION_CAP;
        assert!(lines[cap - 1].contains(&format!("Row: {cap}")), "{lines:?}");
        assert_eq!(lines[cap], "    … +3 more row(s)", "{lines:?}");
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
