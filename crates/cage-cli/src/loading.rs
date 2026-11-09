//! Project loading layer: config discovery, schema merging, and the
//! local/remote source dispatch that turns a project directory into the
//! compile inputs (`Project`). Split out of the CLI entry (LAY-001).

use indexmap::IndexMap;
use std::path::{Path, PathBuf};

use cage_core::convention::derive;
use cage_core::convention::{ExcelConvention, TableMeta};
use cage_core::error::codes::registry::E1802;
use cage_core::manifest::{BuildManifest, ProjectConfig};
use cage_core::normalize::{fill_field_defaults, split_cell_arrays};
use cage_core::schema::Schema;
use cage_core::value::Document;
use cage_core::Diagnostics;
use cage_core::DocumentMetadata;

use crate::remote;

/// A loaded Cage project: config + merged schema + merged document.
pub(crate) struct Project {
    pub(crate) config: ProjectConfig,
    pub(crate) schema: Schema,
    pub(crate) document: Document,
}

/// Apply `--env` to a loaded project: the environment name must be
/// declared by some table's `env_overrides`, every override must be
/// structurally sound, then the overrides for this environment are
/// resolved into plain field constraints. Everything downstream —
/// validation, codegen, hashing — sees one coherent schema, so an
/// environment changes exactly the rules those stages apply. `None`
/// keeps the base schema.
pub(crate) fn project_with_env(project: Project, env: Option<&str>) -> Result<Project, String> {
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

/// Load project config (cage.toml / cage.yaml / cage.yml / cage.json), merge
/// all schema files and parse all declared source roots into one Document.
pub(crate) fn load_project(root: &Path, no_cache: bool) -> Result<Project, String> {
    load_project_impl(root, no_cache, true)
}

/// `fill`: whether absent-or-null fields get their schema default
/// (回流-1). Build / check / lsp / gen want the filled document; `migrate`
/// passes `false` — it transforms raw sources, and a from-schema default
/// filling in before the step chain would shadow a to-schema `set_default`.
pub(crate) fn load_project_impl(
    root: &Path,
    no_cache: bool,
    fill: bool,
) -> Result<Project, String> {
    let config = load_project_config(root)?;

    let mut schema = load_schema_for_config(root, &config)?;

    let mut document = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    // `[source.excel]` (design §51): the meta-row convention and the root a
    // declared script path resolves against. `None` is the baseline — every
    // Excel file reads exactly as before.
    let excel = config.source_excel().map(|c| (c, root));
    // Convention metadata each Excel file decoded, per file (X2): the
    // derived/drift chain runs once all sources are in.
    let mut excel_metas: Vec<(PathBuf, IndexMap<String, TableMeta>)> = Vec::new();
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
            let (doc, metas) = load_sources_with_meta(&root.join(rel), excel)?;
            excel_metas.extend(metas);
            doc
        };
        for (name, table) in doc.tables {
            document.tables.insert(name, table);
        }
        document.source_files.extend(doc.source_files);
    }
    document.source_files.sort();
    document.source_files.dedup();

    // X2 (design §51): a convention Excel is either the whole schema when
    // the project adopts none (the derived chain — "no YAML" acceptance),
    // or it is cross-checked against the adopted one (the drift check —
    // E2208, anchored at the declaration cell). Adopting a schema never
    // silences a contradiction; both paths are loud.
    if !excel_metas.is_empty() {
        if config.schema_path.is_some() {
            let mut drift_diags = Diagnostics::new();
            for (file, metas) in &excel_metas {
                drift_diags.extend(derive::drift(metas, &schema, &file.display().to_string()));
            }
            if drift_diags.has_errors() {
                eprintln!("{}", drift_diags.render(false));
                return Err("convention Excel drifts from the adopted schema".to_string());
            }
        } else {
            let mut merged_metas = IndexMap::new();
            for (_, metas) in &excel_metas {
                for (name, meta) in metas {
                    merged_metas.insert(name.clone(), meta.clone());
                }
            }
            derive::fill_derived(&mut schema, &merged_metas);
        }
    }

    // 回流-1: fields with a schema default fill absent-or-null (Excel empty
    // cells load as null) so a typed field always carries its default.
    if fill {
        split_cell_arrays(&schema, &mut document);
        fill_field_defaults(&schema, &mut document);
    }

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
pub(crate) fn registry_entry(
    root: &Path,
    config: &ProjectConfig,
    rel: &str,
) -> Result<PathBuf, String> {
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
pub(crate) fn load_schema_from_entry(entry_dir: &Path) -> Result<Schema, String> {
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

/// One source root's tables plus the convention `TableMeta` each Excel file
/// decoded, kept per file (X2): the derived/drift chain anchors a drift
/// diagnostic at the file that declared the mismatch.
type SourceWithMeta = (Document, Vec<(PathBuf, IndexMap<String, TableMeta>)>);

/// Load one local source root's document, plus the per-file convention
/// metadata the Excel adapter decoded (X2): the `schema-draft`
/// introspection and the derived/drift chain both consume the per-table
/// `TableMeta` the same parser already produced. Non-Excel files contribute
/// no metadata.
pub(crate) fn load_sources_with_meta(
    root: &Path,
    excel: Option<(&ExcelConvention, &Path)>,
) -> Result<SourceWithMeta, String> {
    parse_source_files_with_meta(
        &collect_files(
            root,
            &["json", "yaml", "yml", "csv", "xlsx", "xls", "msgpack"],
        )?,
        excel,
    )
}

/// Parse the given source files into one merged document, in list order.
/// `excel` carries the project's `[source.excel]` convention together with the
/// root a declared script path resolves against (design §51). `None` is the
/// baseline: every Excel file reads with no meta-row convention at all.
pub(crate) fn parse_source_files(
    files: &[PathBuf],
    excel: Option<(&ExcelConvention, &Path)>,
) -> Result<Document, String> {
    parse_source_files_with_meta(files, excel).map(|(doc, _)| doc)
}

/// [`parse_source_files`] plus the convention metadata each Excel file
/// decoded, kept per file (the drift diagnostics anchor at a file's own
/// path). Non-Excel files append nothing.
pub(crate) fn parse_source_files_with_meta(
    files: &[PathBuf],
    excel: Option<(&ExcelConvention, &Path)>,
) -> Result<SourceWithMeta, String> {
    let mut merged = Document {
        tables: IndexMap::new(),
        source_files: Vec::new(),
        metadata: DocumentMetadata::default(),
    };
    let mut metas_by_file = Vec::new();
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
            "xlsx" | "xls" => match excel_adapter(excel).parse_file_with_meta(file) {
                Ok((doc, metas)) => {
                    if !metas.is_empty() {
                        metas_by_file.push((file.clone(), metas));
                    }
                    Ok(doc)
                }
                Err(diags) => Err(diags),
            },
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
    Ok((merged, metas_by_file))
}

/// The Excel adapter for one file: the `[source.excel]` convention (design
/// §51) plus the root a declared script path resolves against, or today's
/// default adapter when no convention is declared.
pub(crate) fn excel_adapter(
    excel: Option<(&ExcelConvention, &Path)>,
) -> cage_source_excel::ExcelSourceAdapter {
    let mut adapter = cage_source_excel::ExcelSourceAdapter::default();
    if let Some((convention, root)) = excel {
        adapter.convention = Some(convention.clone());
        adapter.project_root = Some(root.to_path_buf());
    }
    adapter
}

/// Data-target format fidelity for re-consumption: a snapshot entry packs
/// every data target of one profile view — `msgpack/` / `json/` / `csv/`
/// siblings serializing the SAME canonical tables with different fidelity
/// (csv flattens nested values to strings and absent optionals to empty
/// cells; msgpack keeps floats bit-exact and `bin` payloads as `Bytes`).
/// Resolution therefore loads the single highest-fidelity format present.
pub(crate) fn format_fidelity(ext: &str) -> u8 {
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
pub(crate) fn load_sources_from_entry(entry_dir: &Path) -> Result<Document, String> {
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
        let mut doc = parse_source_files(std::slice::from_ref(&file), None)?;
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
pub(crate) fn collect_files(path: &Path, exts: &[&str]) -> Result<Vec<PathBuf>, String> {
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
pub(crate) fn filter_by_profile<'a>(
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
    // Every doc table key survived the schema retain above, so the schema
    // table always exists for each row loop.
    for (table_name, table) in &mut filtered_doc.tables {
        let schema_table = &filtered_schema.tables[table_name];
        table.rows.iter_mut().for_each(|row| {
            row.fields
                .retain(|name, _| schema_table.fields.contains_key(name));
        });
    }
    (filtered_schema, filtered_doc)
}
