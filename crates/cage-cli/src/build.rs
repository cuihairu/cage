//! Build and snapshot pipeline: validate, normalize, filter, render
//! artifacts, fingerprint inputs, and pack/verify rolling snapshots.
//! Split out of the CLI entry.

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

use crate::{
    filter_by_profile, generate_target_items, load_manifest, load_project, load_project_config,
    manifest_rel_path_is_safe, project_with_env, resolved_within_root, write_artifact_files,
    write_manifest,
};

/// Everything `build_project` produced — shared by `cage build` and
/// `cage snapshot`.
pub(crate) struct BuildOutput {
    /// Active profile
    pub(crate) profile: String,
    /// Profile (filtered) schema — the view the artifacts were built from
    pub(crate) schema: Schema,
    /// Artifact records: (project-relative path, bytes, format, owning table)
    pub(crate) artifacts: Vec<(String, Vec<u8>, String, Option<String>)>,
    /// Project-relative build root (config `output_dir`)
    pub(crate) output_dir: String,
    /// Whether layer-2 incremental engaged (report says regenerated/carried)
    pub(crate) layer2: bool,
    /// Number of artifacts carried over from disk when layer 2 engaged
    pub(crate) carried_count: usize,
    /// Generated build manifest
    pub(crate) manifest: BuildManifest,
    /// Written manifest location
    pub(crate) manifest_path: PathBuf,
}

/// Build failures mapped to exit codes by the callers (1 = validation,
/// 2 = usage/I/O).
pub(crate) enum BuildFailure {
    /// Validation produced errors (rendered before reporting)
    Validation(Diagnostics),
    /// Usage or I/O errors (printed as `error: {msg}`)
    Io(String),
    /// Layer-1 incremental skip: the last build's inputs are unchanged and
    /// every artifact is intact
    UpToDate { artifacts: usize, manifest: PathBuf },
}

/// The combined script-adapter fingerprint for the project's declared
/// `[source.excel]` / `[source.sheets]` conventions (design §51 Q2). The
/// four inputs — script bytes, resolved path, canonical `script_config`,
/// contract version — are mixed into `source_hash` / `build_id` so editing a
/// script or its config rotates the id even when the parsed model is
/// unchanged. `None` when no script adapter is declared; the extra must be
/// read before the incremental skip compares the previous manifest.
pub(crate) fn source_script_fingerprint(
    root: &Path,
    config: &ProjectConfig,
) -> Result<Option<String>, String> {
    use cage_core::convention::script_fingerprint;
    let mut parts: Vec<(String, String)> = Vec::new();
    if let Some(excel) = config.source_excel() {
        if let Some(fp) = script_fingerprint(root, excel).map_err(|diags| diags.render(false))? {
            parts.push(("excel".to_string(), fp));
        }
    }
    if let Some(sheets) = config.source_sheets() {
        if let Some(fp) = script_fingerprint(root, &sheets.as_excel_convention())
            .map_err(|diags| diags.render(false))?
        {
            parts.push(("sheets".to_string(), fp));
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    parts.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (kind, fp) in parts {
        if !out.is_empty() {
            out.push('|');
        }
        out.push_str(&kind);
        out.push('|');
        out.push_str(&fp);
    }
    Ok(Some(out))
}

pub(crate) fn build_project(
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

    let source_extra =
        source_script_fingerprint(path, &project.config).map_err(BuildFailure::Io)?;
    let (schema_hash, source_hash) =
        ManifestGenerator::input_hashes(&schema, &normalized, source_extra.as_deref());
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
        match load_manifest(&manifest_dir) {
            Ok(prev) => {
                // A tampered or corrupted manifest must not steer the
                // carry/delete paths outside the project root (review
                // 2026-10-18 表一#1): reject any non-normalized relative
                // path and fall back to a full build instead.
                let safe = prev
                    .artifacts
                    .keys()
                    .all(|rel| manifest_rel_path_is_safe(rel))
                    && prev
                        .targets
                        .iter()
                        .all(|t| manifest_rel_path_is_safe(&t.output_dir));
                if safe {
                    Some(prev)
                } else {
                    eprintln!(
                        "warning: previous manifest contains unsafe paths; \
                         ignoring it and running a full build"
                    );
                    None
                }
            }
            Err(_) => None,
        }
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
                let full = path.join(rel);
                if !resolved_within_root(&full, path) {
                    eprintln!(
                        "warning: carried artifact '{rel}' resolves outside the project root; \
                         falling back to a full build"
                    );
                    carried.clear();
                    carry_ok = false;
                    break;
                }
                match std::fs::read(&full) {
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
        let items = generate_target_items(target, &gen_schema, &gen_doc, &schema_hash, path)?;
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
                let full = path.join(rel);
                if !resolved_within_root(&full, path) {
                    eprintln!(
                        "warning: refusing to remove stale artifact '{rel}' \
                         outside the project root"
                    );
                    continue;
                }
                match std::fs::remove_file(&full) {
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
    .with_source_extra(source_extra)
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

pub(crate) fn run_build(
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
pub(crate) fn pack_snapshot(path: &Path, out: &BuildOutput) -> Result<(PathBuf, usize), String> {
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
    // Self-check: the just-written snapshot must verify clean — a stray
    // file in the pack directory (a leftover pack, a mid-pack write) must
    // refuse the pack, not hand out unverified bytes.
    let report = cage_core::snapshot::verify_snapshot(&snap_dir)?;
    if !report.ok {
        return Err(format!(
            "snapshot self-verification failed: {}",
            report.mismatches.join("; ")
        ));
    }
    Ok((snap_dir, files.len()))
}

/// `cage snapshot` — build the profile, then package the build into a
/// self-verifying Configuration Snapshot under
/// `<output_dir>/snapshot/<profile>[-<env>]-<build_id[..12]>`. Identical
/// inputs → identical snapshot bytes (the determinism contract). With
/// `--env` the schema's `env_overrides` are resolved before validation,
/// and the packed schema.json/artifacts carry the environment's rules.
pub(crate) fn run_snapshot(path: &Path, profile: &str, env: Option<&str>) -> i32 {
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
pub(crate) fn run_snapshot_verify(dir: &Path) -> i32 {
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

/// `cage snapshot --list <root>` — the packed snapshots under
/// `<output_dir>/snapshot/`, each named, ledgered (manifest facts) and
/// freshly verified (the same `verify_snapshot` entry `--verify` uses):
/// listing never claims a clean pack it did not just check.
pub(crate) fn run_snapshot_list(path: &Path) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let output_dir = config.output_dir.as_deref().unwrap_or("build");
    let snap_root = path.join(output_dir).join("snapshot");
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(&snap_root) {
        Ok(rd) => rd
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) => {
            println!("cage snapshot: no snapshots ({})", snap_root.display());
            let _ = e;
            return 0;
        }
    };
    dirs.sort();
    if dirs.is_empty() {
        println!("cage snapshot: no snapshots ({})", snap_root.display());
        return 0;
    }
    println!(
        "cage snapshot: {} pack(s) in {}",
        dirs.len(),
        snap_root.display()
    );
    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let facts = match load_manifest(&dir) {
            Ok(m) => format!(
                "profile {}{}  build {}  content {}  {} artifacts",
                m.profile,
                m.environment
                    .as_deref()
                    .map_or_else(String::new, |e| format!("/env {e}")),
                &m.build_id[..m.build_id.len().min(12)],
                &m.content_hash[..m.content_hash.len().min(12)],
                m.artifacts.len(),
            ),
            Err(e) => format!("unreadable manifest ({e})"),
        };
        let verdict = match cage_core::snapshot::verify_snapshot(&dir) {
            Ok(report) if report.ok => format!("verified ({} files)", report.files_checked),
            Ok(report) => format!("FAILED ({} problem(s))", report.mismatches.len()),
            Err(e) => format!("FAILED ({e})"),
        };
        println!("  {name}  {facts}  {verdict}");
    }
    0
}

/// Write a snapshot file map under `dir`, creating parents as needed.
pub(crate) fn write_snapshot_files(
    dir: &Path,
    files: &IndexMap<String, Vec<u8>>,
) -> Result<(), String> {
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
