//! Web API handlers: schema/validate/save, status, tables, upload,
//! build/artifacts/snapshots, diff, and registry faces. Split out of the
//! web module (LAY-004).

use super::{query_params, rel_of_root, Ctx};
use crate::load_schema_for_config;
use crate::schema_save::{SaveError, SchemaSourceIndex};
use cage_core::diagnostics::Diagnostics;
use cage_core::edit::{from_editor_json, to_canonical_yaml, to_editor_json};
use cage_core::error::codes::Severity;
use cage_core::value::{Document, Value};
use std::path::{Path, PathBuf};

/// target). With a `profile` the payload also carries the projected
/// schema: `filter_by_profile` — the exact gating the build pipeline
/// applies — run against the same merged schema. The frontend diffs
/// full-minus-projected to grey out what the profile would drop (hidden
/// things stay visibly hidden), so no visibility semantics live in JS.
pub(crate) fn api_schema(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let schema = match load_schema_for_config(&ctx.root, &ctx.config) {
        Ok(s) => s,
        Err(e) => {
            return (200, serde_json::json!({"ok": false, "error": e}));
        }
    };
    let profile = query_params(query)
        .into_iter()
        .find(|(k, _)| k == "profile")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty());
    let projection = match profile.as_deref() {
        None => None,
        Some(name) => {
            if !ctx.config.profiles.contains_key(name) {
                return (
                    400,
                    serde_json::json!({"ok": false, "error": format!("unknown profile '{name}'")}),
                );
            }
            // The projection only gates schema tables/fields; the empty
            // document just satisfies filter_by_profile's signature (its
            // row filtering is a no-op on no rows).
            let empty = Document {
                tables: indexmap::IndexMap::new(),
                source_files: Vec::new(),
                metadata: cage_core::DocumentMetadata::default(),
            };
            let (projected, _) = crate::filter_by_profile(&schema, &empty, name);
            Some(serde_json::json!({
                "profile": name,
                "schema": to_editor_json(&projected),
                "visible_tables": projected.tables.len(),
            }))
        }
    };
    let mut payload = serde_json::json!({
        "ok": true,
        "project": ctx.config.project.name,
        "schema_path": ctx.config.schema_path,
        "profile_names": ctx.config.profiles.keys().collect::<Vec<_>>(),
        "warnings_as_errors": ctx.config.warnings_as_errors,
        "schema": to_editor_json(&schema),
    });
    // The per-source-file index (file + sha256 revision, table/enum →
    // owner): the save envelope echoes it back so `POST /api/schema` can
    // conflict-check and rewrite only the involved source files.
    if let Some(rel) = ctx.config.schema_path.as_deref() {
        if !rel.starts_with("registry:") {
            match crate::schema_save::load_schema_indexed(&ctx.root, &ctx.root.join(rel)) {
                Ok((_, index)) => {
                    payload["sources"] = index.to_json();
                }
                Err(e) => {
                    return (200, serde_json::json!({"ok": false, "error": e}));
                }
            }
        }
    }
    if let Some(p) = projection {
        payload["projection"] = p;
    }
    (200, payload)
}

/// POST /api/validate — run the editor-side checks on an edited document:
/// E1701 (does not parse into a Schema) then L1 schema consistency
/// checks (raises the existing `E1004` codes). Full L2-L7 validation needs
/// the sources and stays with `cage check` — the editor is an authoring
/// surface, not a replacement for the validation pipeline.
pub(crate) fn api_validate(body: &str) -> serde_json::Value {
    let (ok, diagnostics) = match from_editor_json(body) {
        Err(diags) => (false, diags),
        Ok(schema) => {
            let diags = schema.validate();
            (!diags.has_errors(), diags)
        }
    };
    serde_json::json!({"ok": ok, "diagnostics": diags_json(&diagnostics)})
}

/// The editor contract delivers diagnostics as a bare JSON list.
/// `Diagnostics` derives Serialize over its named `items` field and would
/// render as `{"items": [...]}` — the API flattens that wrapper.
pub(crate) fn diags_json(diags: &Diagnostics) -> serde_json::Value {
    serde_json::to_value(diags.iter().collect::<Vec<_>>()).expect("diagnostics serialize")
}

/// POST /api/schema — save the given editor document as canonical YAML.
///
/// Two body shapes are accepted:
///
/// - the raw editor document (single-file compatibility, unchanged), and
/// - the save envelope `{"schema": <editor doc>, "sources": <index>}` —
///   the index `GET /api/schema` delivered with the document.
///
/// The write-back target is the single schema file the project config
/// names, or — with the envelope's `sources` index — the multi-file
/// `schema_path` directory: the index is conflict-checked (every file's
/// sha256 must still match its load-time revision, 409 otherwise), only
/// the files owning a changed table/enum/metadata are rewritten, and each
/// rewrite is an atomic temp-file+rename. A registry `schema_path` (R2)
/// is refused with 409: the published entry is owned by the publisher —
/// edits belong in the publisher project.
/// With no `schema_path` configured the document is written to
/// `schema.yaml` and the response reminds the user to wire it into the
/// config; the server never rewrites `cage.toml`.
pub(crate) fn api_save(ctx: &Ctx, body: &str) -> (u16, serde_json::Value) {
    // The envelope is detected by its `schema` member — a raw editor
    // document can never carry one (the Schema shape has no such key).
    let (document, client_index) = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v.get("schema").is_some() => {
            let index = v.get("sources").and_then(SchemaSourceIndex::from_json);
            match serde_json::to_string(&v["schema"]) {
                Ok(doc) => (doc, index),
                Err(e) => {
                    return (
                        400,
                        serde_json::json!({"ok": false, "error": format!("bad save envelope: {e}")}),
                    );
                }
            }
        }
        _ => (body.to_string(), None),
    };

    let schema = match from_editor_json(&document) {
        Ok(schema) => schema,
        Err(diags) => {
            return (
                200,
                serde_json::json!({"ok": false, "diagnostics": diags_json(&diags)}),
            );
        }
    };

    if let Some(rel) = ctx.config.schema_path.as_deref() {
        if rel.starts_with("registry:") {
            return (
                409,
                serde_json::json!({
                    "ok": false,
                    "save_target": rel,
                    "error": format!(
                        "schema_path '{rel}' resolves into the Configuration Registry; the \
                         entry schema is published, not edited — change the schema in the \
                         publisher project and re-publish"
                    ),
                }),
            );
        }
    }

    let target: PathBuf = match &ctx.config.schema_path {
        Some(rel) => ctx.root.join(rel),
        None => ctx.root.join("schema.yaml"),
    };

    // ── multi-file schema_path: incremental save over the sources index ──
    if target.is_dir() {
        let Some(index) = client_index else {
            return (
                409,
                serde_json::json!({
                    "ok": false,
                    "save_target": target.display().to_string(),
                    "error": format!(
                        "schema_path '{}' is a directory; saving it needs the sources index \
                         from GET /api/schema (reload the schema and save through the editor, \
                         which echoes the index back)",
                        ctx.config.schema_path.as_deref().unwrap_or("schemas")
                    ),
                }),
            );
        };
        return match crate::schema_save::save_schema_incremental(
            &ctx.root, &target, &schema, &index,
        ) {
            Ok(out) => (
                200,
                serde_json::json!({
                    "ok": true,
                    "save_target": target.display().to_string(),
                    "written": out.written,
                    "unchanged": out.untouched,
                    "bytes": out.bytes,
                    "sources": out.sources.to_json(),
                }),
            ),
            Err(SaveError::Conflict(error)) => (
                409,
                serde_json::json!({
                    "ok": false,
                    "save_target": target.display().to_string(),
                    "error": error,
                }),
            ),
            Err(SaveError::Io(error)) => (500, serde_json::json!({"ok": false, "error": error})),
        };
    }

    // ── single-file schema_path: whole-document save-back ──
    // An echoed one-file index enables the same conflict check.
    if let Some(index) = &client_index {
        let target_rel = rel_of_root(&ctx.root, &target);
        if let Some(f) = index.files.iter().find(|f| f.file == target_rel) {
            let current = std::fs::read(&target)
                .ok()
                .map(|b| crate::schema_save::sha256_hex(&b));
            if current.as_deref() != Some(f.sha256.as_str()) {
                return (
                    409,
                    serde_json::json!({
                        "ok": false,
                        "save_target": target.display().to_string(),
                        "error": format!(
                            "conflict: schema source '{}' changed on disk since it was \
                             loaded — reload the schema and re-apply the edit",
                            target_rel
                        ),
                    }),
                );
            }
        }
    }

    let yaml = to_canonical_yaml(&schema);
    let bytes = yaml.len();
    if let Err(e) = crate::schema_save::atomic_write(&target, yaml.as_bytes()) {
        return (500, serde_json::json!({"ok": false, "error": e}));
    }
    let mut payload = serde_json::json!({
        "ok": true,
        "save_target": target.display().to_string(),
        "bytes": bytes,
    });
    if ctx.config.schema_path.is_none() {
        payload["note"] = serde_json::json!(
            "schema_path is not set in the project config; add \
             'schema_path = \"schema.yaml\"' for builds to read this schema"
        );
    }
    (200, payload)
}

pub(crate) fn api_status(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let profile = match get("profile") {
        Some(p) => p,
        // No query → the first configured profile (config order, the same
        // order `cage build` lists when an unknown one is rejected).
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    if !ctx.config.profiles.contains_key(&profile) {
        return (
            400,
            serde_json::json!({"ok": false, "error": format!(
                "unknown profile '{profile}' (available: {})",
                ctx.config.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )}),
        );
    }
    let env = get("env");
    // Default matches `cage check --level` (semantic).
    let level = get("level").unwrap_or_else(|| "semantic".to_string());

    let validation =
        match crate::validate_project(&ctx.root, &level, env.as_deref(), &profile, false) {
            Ok(out) => {
                let (mut errors, mut warnings, mut infos) = (0usize, 0usize, 0usize);
                for d in &out.diagnostics {
                    match d.severity {
                        Severity::Error => errors += 1,
                        Severity::Warning => warnings += 1,
                        Severity::Info => infos += 1,
                    }
                }
                serde_json::json!({
                    "ok": !out.diagnostics.has_errors(),
                    "profile": profile,
                    "env": env,
                    "level": level,
                    "errors": errors,
                    "warnings": warnings,
                    "info": infos,
                    "visible_tables": out.visible_tables,
                    "diagnostics": diags_json(&out.diagnostics),
                })
            }
            // Project load failure (bad cage.toml, unresolvable schema or
            // registry reference): the page stays up and names the problem —
            // this is exactly the state a status face exists for.
            Err(e) => serde_json::json!({"ok": false, "error": e, "diagnostics": []}),
        };

    (
        200,
        serde_json::json!({
            "ok": true,
            "project": ctx.config.project.name,
            "schema_path": ctx.config.schema_path,
            "warnings_as_errors": ctx.config.warnings_as_errors,
            "output_dir": ctx.config.output_dir.as_deref().unwrap_or("build"),
            "profile_names": ctx.config.profiles.keys().collect::<Vec<_>>(),
            "profile": profile,
            "validation": validation,
            "builds": build_facts(ctx, env.as_deref()),
        }),
    )
}

/// Shared profile/env resolution for the data-panel endpoints: the same
/// contract as `/api/status` (query wins, else first configured profile;
/// unknown profile 400; no profile 500; bad env 400 through the same
/// load chain).
pub(crate) fn data_panel_project(
    ctx: &Ctx,
    profile: &str,
    env: Option<&str>,
) -> Result<(cage_core::schema::Schema, cage_core::value::Document), (u16, serde_json::Value)> {
    if !ctx.config.profiles.contains_key(profile) {
        return Err((
            400,
            serde_json::json!({"ok": false, "error": format!(
                "unknown profile '{profile}' (available: {})",
                ctx.config.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )}),
        ));
    }
    let project = crate::load_project(&ctx.root, false)
        .and_then(|p| crate::project_with_env(p, env))
        .map_err(|e| (400u16, serde_json::json!({"ok": false, "error": e})))?;
    let (schema, document) = crate::filter_by_profile(&project.schema, &project.document, profile);
    Ok((schema, document))
}

/// GET /api/tables[?profile=NAME&env=ENV] — the data panel's table
/// inventory (planner interface P0). One entry per profile-visible table:
/// name, row count, source facts, primary key — straight from the same
/// load chain `check` consumes, so the list can never show a table the
/// CLI does not see. No validation here: the panel pairs this list with
/// the `/api/status` diagnostics, and the anchoring data (table / row /
/// column) already rides each diagnostic.
pub(crate) fn api_tables(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    let env = get("env");
    let (schema, document) = match data_panel_project(ctx, &profile, env.as_deref()) {
        Ok(pair) => pair,
        Err((status, value)) => return (status, value),
    };

    // Primary key comes from the schema (authoritative and always
    // present), not the loaded table — some source readers (JSON) leave
    // the value-table's own field empty.
    let mut tables: Vec<serde_json::Value> = document
        .tables
        .values()
        .map(|t| {
            let pk = schema
                .tables
                .get(&t.name)
                .map(|def| def.primary_key.clone())
                .unwrap_or_default();
            serde_json::json!({
                "name": t.name,
                "rows": t.rows.len(),
                "source_file": t.source_file,
                "sheet": t.sheet,
                "primary_key": pk,
            })
        })
        .collect();
    // Name order, not load order: the panel is an inventory, and a
    // stable list beats "the order the source files happened to mention
    // tables in" (which changes with every source reorganization).
    tables.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    (
        200,
        serde_json::json!({"ok": true, "profile": profile, "tables": tables}),
    )
}

/// GET /api/table?name=NAME[&profile=&env=&offset=&limit=] — one table's
/// rows for the data panel (planner interface P0). Rows render with the
/// schema-declared column order (the same `ordered_columns` the migrate
/// write-back uses), paginated and truncated: default 200 rows, hard cap
/// 1000, so a huge table cannot blow up either the response or the
/// browser. Cell values go through a total JSON mapping (bytes, which
/// have no display form, become a marker object). 404 for a table the
/// profile projection hid — same visibility rule as the CLI.
// Total JSON mapping: bytes have no display form, so they become a
// marker object instead of failing the whole request.
pub(crate) fn data_cell(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::UInt(u) => serde_json::Value::from(*u),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::String(s) => serde_json::Value::from(s.clone()),
        Value::Bytes(b) => serde_json::json!({"bytes": b.len()}),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(data_cell).collect()),
        Value::Object(map) => {
            let mut m = serde_json::Map::new();
            for (k, v) in map {
                m.insert(k.clone(), data_cell(v));
            }
            serde_json::Value::Object(m)
        }
    }
}

pub(crate) fn api_table(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let Some(name) = get("name") else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "missing required parameter 'name'"}),
        );
    };
    // Parse failures fall back to the defaults (the panel always sends
    // well-formed numbers; hand-typed URLs should not error out).
    let offset = get("offset")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = match get("limit").and_then(|v| v.parse::<usize>().ok()) {
        Some(n) if n > 0 => n.min(1000),
        _ => 200,
    };

    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    let env = get("env");
    let (schema, document) = match data_panel_project(ctx, &profile, env.as_deref()) {
        Ok(pair) => pair,
        Err((status, value)) => return (status, value),
    };
    let Some(table) = document.tables.values().find(|t| t.name == name) else {
        return (
            404,
            serde_json::json!({
                "ok": false,
                "error": format!("no table '{name}' in this profile's view")
            }),
        );
    };

    let columns = crate::ordered_columns(table, &schema);
    let total = table.rows.len();
    let start = offset.min(total);
    let end = (start + limit).min(total);
    let rows: Vec<serde_json::Value> = table.rows[start..end]
        .iter()
        .map(|row| {
            let mut cells = serde_json::Map::new();
            for col in &columns {
                match row.fields.get(col) {
                    Some(tv) => cells.insert(col.clone(), data_cell(&tv.value)),
                    None => cells.insert(col.clone(), serde_json::Value::Null),
                };
            }
            serde_json::json!({
                "index": row.index,
                "pk": row.primary_key.iter().map(data_cell).collect::<Vec<_>>(),
                "cells": cells,
            })
        })
        .collect();

    let pk = schema.tables.get(&name).map_or_else(
        || table.primary_key_fields.clone(),
        |def| def.primary_key.clone(),
    );
    (
        200,
        serde_json::json!({
            "ok": true,
            "name": name,
            "profile": profile,
            "primary_key": pk,
            "source_file": table.source_file,
            "sheet": table.sheet,
            "columns": columns,
            "total": total,
            "offset": start,
            "limit": limit,
            "rows": rows,
        }),
    )
}

/// Per-profile build facts for `/api/status`. The manifest is single-slot
/// (`<output_dir>/manifest.json`, one profile at a time on disk), so
/// "fresh" additionally implies the last build on disk was for this
/// profile: same profile, same environment, same schema/source/target
/// fingerprints, and every listed artifact still intact — the exact
/// comparison `build_project` layer 1 makes (kept in step by pointing at
/// the same manifest fields; drift here is a status-page bug, not a build
/// bug). `changed_tables` is the data-staleness detail: the tables whose
/// rows fingerprint moved since that build (the layer-2 detector).
pub(crate) fn build_facts(ctx: &Ctx, env: Option<&str>) -> serde_json::Value {
    use cage_core::manifest::ManifestGenerator;

    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let prev = crate::load_manifest(&ctx.root.join(output_dir)).ok();
    // Current inputs: one project load (one pass over the sources), then
    // each profile projects its own view and hashes.
    let project =
        match crate::load_project(&ctx.root, false).and_then(|p| crate::project_with_env(p, env)) {
            Ok(p) => p,
            Err(e) => return serde_json::json!({"ok": false, "error": e}),
        };
    // The script-adapter fingerprint (design §51 Q2): `build_project` mixes
    // this into `source_hash`, so the status page must too — otherwise a
    // script edit (byte-identical parsed model) is misreported as fresh.
    let source_extra = match crate::source_script_fingerprint(&ctx.root, &project.config) {
        Ok(extra) => extra,
        Err(e) => return serde_json::json!({"ok": false, "error": e}),
    };

    let mut builds = serde_json::Map::new();
    for (name, build_profile) in &ctx.config.profiles {
        let target_records = ManifestGenerator::target_records(&build_profile.targets);
        let (schema, document) = crate::filter_by_profile(&project.schema, &project.document, name);
        let normalized = cage_core::normalize::normalize_document(&document);
        let (schema_hash, source_hash) =
            ManifestGenerator::input_hashes(&schema, &normalized, source_extra.as_deref());
        let cur_table_hashes = ManifestGenerator::table_hashes(&normalized);

        let mut entry = serde_json::json!({
            "target_count": build_profile.targets.len(),
            "targets": build_profile.targets.iter().map(|t| serde_json::json!(
                {"format": t.format, "output_dir": t.output_dir}
            )).collect::<Vec<_>>(),
            "manifest_present": prev.is_some(),
            "fresh": false,
            "changed_tables": [],
        });
        if let Some(m) = &prev {
            entry["last_build_profile"] = serde_json::json!(m.profile);
            if m.profile == *name {
                entry["build_id"] = serde_json::json!(m.build_id);
                entry["content_hash"] = serde_json::json!(m.content_hash);
                entry["artifact_count"] = serde_json::json!(m.artifacts.len());
                let same_inputs = m.environment.as_deref() == env
                    && m.schema_hash == schema_hash
                    && m.source_hash == source_hash
                    && m.targets == target_records
                    && m.artifacts.keys().all(|rel| ctx.root.join(rel).is_file());
                entry["fresh"] = serde_json::json!(same_inputs);
                if !same_inputs && !m.table_hashes.is_empty() {
                    let changed: Vec<&String> = cur_table_hashes
                        .keys()
                        .filter(|t| m.table_hashes.get(*t) != cur_table_hashes.get(*t))
                        .collect();
                    entry["changed_tables"] = serde_json::json!(changed);
                }
            }
        } else {
            entry["last_build_profile"] = serde_json::Value::Null;
        }
        builds.insert(name.clone(), entry);
    }
    serde_json::Value::Object(builds)
}

/// Resolve a table's registered source file to the one writable upload
/// target (planner interface P0 upload; decision record in docs/ui.md).
/// Guards run on the canonicalized path: the file must sit inside the
/// project root AND inside one of the configured source roots — the same
/// verify-first posture as the artifact endpoint, so an upload can only
/// ever touch the files the loader itself reads.
pub(crate) fn resolve_upload_target(
    root: &Path,
    source_file: &str,
    source_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let root_canon =
        std::fs::canonicalize(root).map_err(|e| format!("project root unreadable: {e}"))?;
    let raw = Path::new(source_file);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root_canon.join(raw)
    };
    let target = std::fs::canonicalize(&joined)
        .map_err(|e| format!("source file '{source_file}' cannot be resolved: {e}"))?;
    if !target.starts_with(&root_canon) {
        return Err(format!(
            "source file '{source_file}' resolves outside the project"
        ));
    }
    if !target.is_file() {
        return Err(format!("source file '{source_file}' is not a file"));
    }
    let inside_a_root = source_roots.iter().any(|sr| {
        let sr_canon = if sr.is_absolute() {
            sr.clone()
        } else {
            root_canon.join(sr)
        };
        target.starts_with(sr_canon)
    });
    if !inside_a_root {
        return Err(format!(
            "source file '{source_file}' sits outside every configured source root"
        ));
    }
    Ok(target)
}

/// Best-effort removal of the swap files. `disarm()` is called when the
/// backup holds the only surviving copy of the original (a failed
/// restore) — deleting it then would lose the user's data, so it stays
/// on disk for manual recovery and the error names its path.
pub(crate) struct UploadSwap {
    tmp: PathBuf,
    backup: PathBuf,
    armed: bool,
}

impl UploadSwap {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UploadSwap {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp);
        if self.armed {
            let _ = std::fs::remove_file(&self.backup);
        }
    }
}

/// POST /api/upload?name=NAME[&profile=P&env=E] — replace one table's
/// registered source file with the raw request body (planner interface
/// P0 upload; policy decision record in docs/ui.md「策划规则界面」).
/// Contract: `name` must be a profile-visible single-file table; its
/// registered source file (guarded by `resolve_upload_target`) is the
/// only write target. Bytes land via tmp + rename (atomic), the original
/// moves aside, and the project is re-validated through the same
/// `validate_project` entry the CLI uses — errors or a load failure
/// restore the original before responding. Size rides the global body
/// cap; the format is not guessed, the bytes must parse under the
/// loader chain for that file.
pub(crate) fn api_upload(ctx: &Ctx, query: &str, body: &[u8]) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let Some(name) = get("name") else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "missing required parameter 'name'"}),
        );
    };
    if body.is_empty() {
        return (
            400,
            serde_json::json!({"ok": false, "error": "empty upload body"}),
        );
    }
    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    let env = get("env");
    let (_, document) = match data_panel_project(ctx, &profile, env.as_deref()) {
        Ok(pair) => pair,
        Err((status, value)) => return (status, value),
    };
    let Some(table) = document.tables.values().find(|t| t.name == name) else {
        return (
            404,
            serde_json::json!({
                "ok": false,
                "error": format!("no table '{name}' in this profile's view")
            }),
        );
    };
    // A table fed by several files cannot be replaced by one upload
    // without silently orphaning rows in the other files.
    let mut files = std::collections::BTreeSet::new();
    for row in &table.rows {
        files.insert(row.location.file.clone());
    }
    if files.len() > 1 {
        return (
            400,
            serde_json::json!({
                "ok": false,
                "error": format!(
                    "table '{name}' spans {} source files ({}); upload replaces single-file tables only",
                    files.len(),
                    files.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            }),
        );
    }
    let source_roots: Vec<PathBuf> = ctx
        .config
        .source_roots
        .values()
        .map(PathBuf::from)
        .collect();
    let target = match resolve_upload_target(&ctx.root, &table.source_file, &source_roots) {
        Ok(p) => p,
        Err(e) => return (400, serde_json::json!({"ok": false, "error": e})),
    };

    let Some(dir) = target.parent().map(Path::to_path_buf) else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "source file has no parent directory"}),
        );
    };
    let file_name = target
        .file_name()
        .map_or_else(|| "table".to_string(), |n| n.to_string_lossy().into_owned());
    let pid = std::process::id();
    let mut swap = UploadSwap {
        tmp: dir.join(format!(".{file_name}.upload-tmp-{pid}")),
        backup: dir.join(format!(".{file_name}.upload-bak-{pid}")),
        armed: true,
    };
    if let Err(e) = std::fs::write(&swap.tmp, body) {
        return (
            500,
            serde_json::json!({"ok": false, "error": format!("cannot write upload temp file: {e}")}),
        );
    }
    if let Err(e) = std::fs::rename(&target, &swap.backup) {
        return (
            500,
            serde_json::json!({"ok": false, "error": format!("cannot move original aside: {e}")}),
        );
    }
    if let Err(e) = std::fs::rename(&swap.tmp, &target) {
        // The original is still intact in the backup; put it back.
        let _ = std::fs::rename(&swap.backup, &target);
        return (
            500,
            serde_json::json!({"ok": false, "error": format!("cannot move upload into place: {e}")}),
        );
    }

    // Post-swap validation through the single CLI entry: whichever way it
    // goes, the response reflects a repo state the loader accepts.
    match crate::validate_project(&ctx.root, "semantic", env.as_deref(), &profile, false) {
        Ok(outcome) if !outcome.diagnostics.has_errors() => {
            let _ = std::fs::remove_file(&swap.backup);
            let (errors, warnings) = (
                outcome.diagnostics.errors().len(),
                outcome.diagnostics.warnings().len(),
            );
            (
                200,
                serde_json::json!({
                    "ok": true,
                    "profile": profile,
                    "name": name,
                    "source_file": table.source_file,
                    "bytes": body.len(),
                    "warnings": warnings,
                    "info": errors,
                    "diagnostics": diags_json(&outcome.diagnostics),
                }),
            )
        }
        Ok(outcome) => match restore_original(&target, &swap.backup) {
            Ok(()) => (
                200,
                serde_json::json!({
                    "ok": false,
                    "refused": "validation",
                    "error": "uploaded data failed validation; the original source file was restored",
                    "diagnostics": diags_json(&outcome.diagnostics),
                }),
            ),
            Err(msg) => {
                swap.disarm();
                (500, serde_json::json!({"ok": false, "error": msg}))
            }
        },
        Err(e) => match restore_original(&target, &swap.backup) {
            Ok(()) => (
                200,
                serde_json::json!({
                    "ok": false,
                    "refused": "load",
                    "error": format!("uploaded data cannot be loaded: {e}; the original source file was restored"),
                }),
            ),
            Err(msg) => {
                swap.disarm();
                (500, serde_json::json!({"ok": false, "error": msg}))
            }
        },
    }
}

/// Move the aside original back over the swapped-in upload. `Err` carries
/// a message that names the surviving backup path so nothing is lost
/// silently.
pub(crate) fn restore_original(target: &Path, backup: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::remove_file(target) {
        if target.exists() {
            return Err(format!(
                "cannot restore {}: removing the uploaded file failed ({e}); the original is kept at {}",
                target.display(),
                backup.display()
            ));
        }
    }
    match std::fs::rename(backup, target) {
        Ok(()) => Ok(()),
        Err(e) => Err(format!(
            "cannot restore {}: rename failed ({e}); the original is kept at {}",
            target.display(),
            backup.display()
        )),
    }
}

/// POST /api/build — run the build pipeline for one profile: the same
/// `build_project` entry `cage build` uses (full validation, three-layer
/// incremental, deterministic manifest). Body is `{"profile": "..."}` with
/// optional `"env"`, `"level"` and `"incremental": false`. Validation
/// failures answer `{"ok": false, "diagnostics": [...]}` with 200 (the
/// /api/validate shape — the page renders them in the panel), usage / I/O
/// errors answer 400 with `error`, and an up-to-date build answers
/// `up_to_date: true` without touching anything.
pub(crate) fn api_build(ctx: &Ctx, body: &str) -> (u16, serde_json::Value) {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("invalid JSON body: {e}")}),
            )
        }
    };
    let Some(profile) = parsed.get("profile").and_then(|p| p.as_str()) else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "body must carry {\"profile\": \"...\"}"}),
        );
    };
    let env = parsed.get("env").and_then(|e| e.as_str());
    let level = parsed
        .get("level")
        .and_then(|l| l.as_str())
        .unwrap_or("semantic");
    let incremental = parsed
        .get("incremental")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    match crate::build_project(&ctx.root, level, profile, env, incremental, false) {
        Ok(out) => (
            200,
            serde_json::json!({
                "ok": true,
                "profile": out.profile,
                "up_to_date": false,
                "layer2": out.layer2,
                "regenerated": out.artifacts.len() - out.carried_count,
                "carried": out.carried_count,
                "artifact_count": out.artifacts.len(),
                "manifest_path": out.manifest_path.display().to_string(),
                "build_id": out.manifest.build_id,
                "content_hash": out.manifest.content_hash,
            }),
        ),
        Err(crate::BuildFailure::UpToDate {
            artifacts,
            manifest,
        }) => (
            200,
            serde_json::json!({
                "ok": true,
                "profile": profile,
                "up_to_date": true,
                "artifact_count": artifacts,
                "manifest_path": manifest.display().to_string(),
            }),
        ),
        Err(crate::BuildFailure::Validation(diags)) => (
            200,
            serde_json::json!({"ok": false, "diagnostics": diags_json(&diags)}),
        ),
        Err(crate::BuildFailure::Io(msg)) => (400, serde_json::json!({"ok": false, "error": msg})),
    }
}

/// GET /api/artifacts[?profile=NAME&env=ENV] — the artifact tree of one
/// profile's build, straight from the build manifest (the ledger the build
/// wrote). The manifest is single-slot: when another profile owns
/// `<output_dir>/manifest.json`, this profile reports `manifest_profile`
/// and an empty list instead of reading unverifiable leftovers — build it
/// to browse it. `stale` repeats the status face's layer-1 verdict: a
/// manifest whose inputs moved describes a build that no longer matches
/// the sources.
pub(crate) fn api_artifacts(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    if !ctx.config.profiles.contains_key(&profile) {
        return (
            400,
            serde_json::json!({"ok": false, "error": format!(
                "unknown profile '{profile}' (available: {})",
                ctx.config.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )}),
        );
    }
    let env = get("env");
    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let Ok(manifest) = crate::load_manifest(&ctx.root.join(output_dir)) else {
        return (
            200,
            serde_json::json!({
                "ok": true, "profile": profile, "manifest_profile": null,
                "fresh": false, "stale": false, "targets": [], "artifacts": [],
            }),
        );
    };
    if manifest.profile != profile {
        return (
            200,
            serde_json::json!({
                "ok": true, "profile": profile, "manifest_profile": manifest.profile,
                "fresh": false, "stale": false, "targets": [], "artifacts": [],
            }),
        );
    }

    let builds = build_facts(ctx, env.as_deref());
    let fresh = builds
        .get(&profile)
        .and_then(|b| b.get("fresh"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    (
        200,
        serde_json::json!({
            "ok": true,
            "profile": profile,
            "manifest_profile": manifest.profile,
            "fresh": fresh,
            "stale": !fresh,
            "build_id": manifest.build_id,
            "content_hash": manifest.content_hash,
            "targets": manifest.targets,
            "artifacts": manifest.artifacts.values().collect::<Vec<_>>(),
        }),
    )
}

/// GET /api/artifact?path=REL — one artifact's bytes for preview /
/// download. The path must be a key of the build manifest (a lookup,
/// never join-and-pray: nothing outside the ledger is servable), and the
/// bytes are re-hashed against the recorded blake3 before serving — a
/// file that no longer matches answers 409 instead of handing out
/// tampered content, the same verify-first stance the snapshot loader
/// takes.
pub(crate) fn api_artifact(ctx: &Ctx, query: &str) -> (u16, &'static str, Vec<u8>) {
    const JSON: &str = "application/json; charset=utf-8";
    let Some(rel) = query_params(query)
        .iter()
        .find(|(k, _)| k == "path")
        .map(|(_, v)| v.clone())
    else {
        return (
            400,
            JSON,
            br#"{"ok": false, "error": "missing path parameter"}"#.to_vec(),
        );
    };
    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let manifest = match crate::load_manifest(&ctx.root.join(output_dir)) {
        Ok(m) => m,
        Err(e) => {
            let mut err = serde_json::json!({"ok": false, "error": e});
            err["error"] = serde_json::json!(format!(
                "no build manifest under {output_dir} — build first",
            ));
            return (404, JSON, json(&err).into_bytes());
        }
    };
    let Some(info) = manifest.artifacts.get(&rel) else {
        let err = serde_json::json!({"ok": false, "error": format!(
            "'{rel}' is not in the build manifest — only ledgered artifacts are served"
        )});
        return (404, JSON, json(&err).into_bytes());
    };
    let Ok(bytes) = std::fs::read(ctx.root.join(&rel)) else {
        let err = serde_json::json!({"ok": false, "error": format!(
            "cannot read artifact '{rel}'"
        )});
        return (500, JSON, json(&err).into_bytes());
    };
    if blake3::hash(&bytes).to_hex().to_string() != info.hash {
        let err = serde_json::json!({"ok": false, "error": format!(
            "artifact '{rel}' no longer matches its manifest hash — rebuild (the file \
             changed or was tampered with after the build)"
        )});
        return (409, JSON, json(&err).into_bytes());
    }
    (200, artifact_mime(&rel), bytes)
}

/// Content type for an artifact path — preview-rendering hint for the
/// status page; anything unknown serves as plain text.
pub(crate) fn artifact_mime(rel: &str) -> &'static str {
    let ext = Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "json" => "application/json; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "yaml" | "yml" => "text/yaml; charset=utf-8",
        "msgpack" => "application/octet-stream",
        _ => "text/plain; charset=utf-8",
    }
}

/// GET /api/snapshots — the packed snapshots under
/// `<output_dir>/snapshot/` (U3 列/验): each pack with its ledger facts
/// (profile / environment / `build_id` / `content_hash` / artifact count)
/// and a fresh verification pass — the same `verify_snapshot` entry
/// `cage snapshot --verify` runs, so the page never claims a clean pack it
/// did not just check.
pub(crate) fn api_snapshots(ctx: &Ctx) -> (u16, serde_json::Value) {
    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let snap_root = ctx.root.join(output_dir).join("snapshot");
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(&snap_root) {
        Ok(rd) => rd
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    let snapshots: Vec<serde_json::Value> = dirs
        .iter()
        .map(|dir| {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            let (verified, files_checked, problems) =
                match cage_core::snapshot::verify_snapshot(dir) {
                    Ok(report) if report.ok => (true, report.files_checked, Vec::new()),
                    Ok(report) => (false, report.files_checked, report.mismatches),
                    Err(e) => (false, 0, vec![e]),
                };
            let manifest = crate::load_manifest(dir).ok();
            let name_ref = &name;
            serde_json::json!({
                "name": name_ref,
                "profile": manifest.as_ref().map(|m| m.profile.clone()),
                "environment": manifest.as_ref().and_then(|m| m.environment.clone()),
                "build_id": manifest.as_ref().map(|m| m.build_id.clone()),
                "content_hash": manifest.as_ref().map(|m| m.content_hash.clone()),
                "artifact_count": manifest.as_ref().map(|m| m.artifacts.len()),
                "verified": verified,
                "files_checked": files_checked,
                "problems": problems,
            })
        })
        .collect();
    (
        200,
        serde_json::json!({
            "ok": true,
            "output_dir": output_dir,
            "snapshots": snapshots,
        }),
    )
}

/// POST /api/snapshot — pack one profile into a self-verifying snapshot
/// (U3 打包): the same pipeline as `cage snapshot` — a fresh full build
/// (no incremental carry-over) through `build_project`, then
/// `pack_snapshot`, which refuses to hand out a pack that does not
/// self-verify. `{profile, env}`; env optional.
pub(crate) fn api_snapshot(ctx: &Ctx, body: &str) -> (u16, serde_json::Value) {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("invalid JSON body: {e}")}),
            );
        }
    };
    let profile = parsed
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| ctx.config.profiles.keys().next().cloned());
    let Some(profile) = profile else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "no profile in body and no profiles configured"}),
        );
    };
    if !ctx.config.profiles.contains_key(&profile) {
        return (
            400,
            serde_json::json!({"ok": false, "error": format!("unknown profile '{profile}'")}),
        );
    }
    let env = parsed
        .get("env")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty());
    match crate::build_project(&ctx.root, "gamerule", &profile, env, false, false) {
        Ok(out) => match crate::pack_snapshot(&ctx.root, &out) {
            Ok((snap_dir, files)) => {
                // Portable `/` separators for the JSON-facing rel (the page
                // splits the name off with `/`); native separators would
                // leak `\` on Windows.
                let rel = snap_dir
                    .strip_prefix(&ctx.root)
                    .unwrap_or(&snap_dir)
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                (
                    200,
                    serde_json::json!({
                        "ok": true,
                        "snapshot_dir": rel,
                        "files": files,
                        "artifacts": out.artifacts.len(),
                        "build_id": out.manifest.build_id,
                    }),
                )
            }
            Err(e) => (500, serde_json::json!({"ok": false, "error": e})),
        },
        Err(crate::BuildFailure::Validation(diags)) => {
            (200, serde_json::json!({"ok": false, "diagnostics": diags}))
        }
        Err(crate::BuildFailure::Io(msg)) => (400, serde_json::json!({"ok": false, "error": msg})),
        Err(crate::BuildFailure::UpToDate { .. }) => (
            500,
            serde_json::json!({"ok": false, "error": "internal: snapshot build did not rebuild"}),
        ),
    }
}

/// GET /api/diff?from=REL&to=REL — the layered diff between two build
/// manifests (U3): paths are project-root-relative and name a directory
/// holding `manifest.json` (the build output dir or a snapshot dir) or a
/// manifest file. Shape per the UI ruling: `{changed_tables,
/// affected_targets, files:{added,removed,changed},
/// roots:{content_hash, build_id}}` — the page folds 表 → target → 文件
/// from it. `table`/`target` ride on each file entry so the fold is pure
/// grouping.
pub(crate) fn api_diff(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let (Some(from), Some(to)) = (get("from"), get("to")) else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "missing from/to parameters (?from=REL&to=REL)"}),
        );
    };
    let safe_path = |rel: &str| -> Result<std::path::PathBuf, String> {
        let relative = std::path::Path::new(rel);
        if relative.is_absolute()
            || relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("path must be a normalized project-relative path".to_string());
        }
        let root = std::fs::canonicalize(&ctx.root).map_err(|e| format!("project root: {e}"))?;
        let path = std::fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
        if !path.starts_with(&root) {
            return Err("path resolves outside project root".to_string());
        }
        Ok(path)
    };
    let from_path = match safe_path(&from) {
        Ok(path) => path,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("baseline '{from}': {e}")}),
            )
        }
    };
    let to_path = match safe_path(&to) {
        Ok(path) => path,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("target '{to}': {e}")}),
            )
        }
    };
    let base = match crate::load_manifest(&from_path) {
        Ok(m) => m,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("baseline '{from}': {e}")}),
            );
        }
    };
    let new = match crate::load_manifest(&to_path) {
        Ok(m) => m,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("target '{to}': {e}")}),
            );
        }
    };

    // Layer 1: tables whose per-table fingerprint moved (either side).
    let mut changed_tables: Vec<String> = base
        .table_hashes
        .keys()
        .chain(new.table_hashes.keys())
        .filter(|t| base.table_hashes.get(*t) != new.table_hashes.get(*t))
        .cloned()
        .collect();
    changed_tables.sort();
    changed_tables.dedup();

    // Layer 3: the files, each tagged with its table (the manifests'
    // ledger) and target (the output_dir the path lives under) so the UI
    // folds without re-deriving either.
    let mut target_dirs: Vec<&str> = base
        .targets
        .iter()
        .chain(new.targets.iter())
        .map(|t| t.output_dir.as_str())
        .collect();
    target_dirs.sort_unstable();
    target_dirs.dedup();
    let target_of = |file: &str| -> Option<String> {
        target_dirs
            .iter()
            .rfind(|dir| {
                let dir = dir.trim_end_matches('/');
                *file == *dir || file.starts_with(&format!("{dir}/"))
            })
            .map(std::string::ToString::to_string)
    };
    let entry = |path: &str,
                 from: Option<&cage_core::manifest::ArtifactInfo>,
                 to: Option<&cage_core::manifest::ArtifactInfo>| {
        let table = to
            .and_then(|i| i.table.clone())
            .or_else(|| from.and_then(|i| i.table.clone()));
        serde_json::json!({
            "path": path,
            "table": table,
            "target": target_of(path),
            "from_hash": from.map(|i| i.hash.clone()),
            "to_hash": to.map(|i| i.hash.clone()),
        })
    };
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (path, info) in &new.artifacts {
        match base.artifacts.get(path) {
            None => added.push(entry(path, None, Some(info))),
            Some(old) if old.hash != info.hash => changed.push(entry(path, Some(old), Some(info))),
            Some(_) => {}
        }
    }
    for (path, info) in &base.artifacts {
        if !new.artifacts.contains_key(path) {
            removed.push(entry(path, Some(info), None));
        }
    }

    // Layer 2: the targets whose outputs sit under any moved file.
    let touches = |file: &str, dir: &str| {
        let dir = dir.trim_end_matches('/');
        file == dir || file.starts_with(&format!("{dir}/"))
    };
    let moved: Vec<&str> = added
        .iter()
        .chain(removed.iter())
        .chain(changed.iter())
        .filter_map(|e| e["path"].as_str())
        .collect();
    let affected_targets: Vec<String> = target_dirs
        .iter()
        .filter(|dir| moved.iter().any(|f| touches(f, dir)))
        .map(std::string::ToString::to_string)
        .collect();

    (
        200,
        serde_json::json!({
            "ok": true,
            "from": from,
            "to": to,
            "changed_tables": changed_tables,
            "affected_targets": affected_targets,
            "files": {"added": added, "removed": removed, "changed": changed},
            "roots": {
                "content_hash": {"from": base.content_hash, "to": new.content_hash},
                "build_id": {"from": base.build_id, "to": new.build_id},
            },
        }),
    )
}

/// GET /api/registry — the local registry this project would publish into
/// (`[registry] path` in cage.toml), browsed (U3): packages with their
/// published versions. Unconfigured → ok with an empty list and a note;
/// a corrupt index is the registry's own error string.
pub(crate) fn api_registry(ctx: &Ctx) -> (u16, serde_json::Value) {
    let Some(reg) = &ctx.config.registry else {
        return (
            200,
            serde_json::json!({
                "ok": true,
                "packages": [],
                "note": "no [registry] configured in cage.toml",
            }),
        );
    };
    let reg_root = ctx.root.join(&reg.path);
    match cage_core::registry::packages(&reg_root) {
        Ok(indexes) => (
            200,
            serde_json::json!({
                "ok": true,
                "registry": reg.path,
                "packages": indexes,
            }),
        ),
        Err(e) => (
            200,
            serde_json::json!({"ok": false, "error": e, "registry": reg.path}),
        ),
    }
}

pub(crate) fn json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).expect("api response serializes")
}
