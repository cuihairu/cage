//! Web integration behavior tests.

use super::handlers::{artifact_mime, data_cell, resolve_upload_target, restore_original};
use super::*;
use cage_core::edit::{from_editor_json, to_canonical_yaml, to_editor_json};
use cage_core::schema::Schema;
use std::fs;
use std::io::{Read as _, Write as _};
use tempfile::TempDir;

/// Editor document for a schema with one table whose primary key points
/// at a declared field — the smallest document the API accepts.
pub(crate) fn good_doc() -> String {
    let schema = Schema::new();
    to_editor_json(&schema).to_string()
}

/// Editor document whose table primary key references an undeclared
/// field — parses as a Schema but fails the L1 consistency checks
/// (E1004) that `POST /api/validate` runs.
fn dangling_pk_doc() -> String {
    use cage_core::schema::TableSchema;
    let mut schema = Schema::new();
    schema.add_table(TableSchema {
        name: "Item".to_string(),
        description: None,
        primary_key: vec!["ghost".to_string()],
        fields: indexmap::IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: indexmap::IndexMap::new(),
    });
    to_editor_json(&schema).to_string()
}

pub(crate) fn ctx_in(dir: &TempDir, schema_path: Option<&str>) -> Ctx {
    let mut config = ProjectConfig::default();
    config.project.name = "demo".to_string();
    config.schema_path = schema_path.map(str::to_string);
    Ctx {
        root: dir.path().to_path_buf(),
        config,
    }
}

/// A full project for the status/build endpoints (U1): two profiles
/// (client → json, server → csv), one table, one JSON source. The
/// status/build handlers re-read the project from disk (they reuse the
/// `check` / `build` entries), so the on-disk cage.toml matters, not
/// just the Ctx config object.
pub(crate) fn demo_ctx(dir: &TempDir) -> Ctx {
    use cage_core::manifest::{BuildProfile, TargetConfig};

    fs::write(
        dir.path().join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[source_roots]
main = "config"

[project]
name = "demo"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"

[profiles.server]
name = "server"

[[profiles.server.targets]]
format = "csv"
output_dir = "build/server/csv"
"#,
    )
    .expect("write config");
    fs::write(
        dir.path().join("schema.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\n      \
             name: { name: name, type: { kind: String } }\nenums: {}\n",
    )
    .expect("write schema");
    fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
    )
    .expect("write source");

    let target = |format: &str, output_dir: &str| TargetConfig {
        format: format.to_string(),
        output_dir: output_dir.to_string(),
        file_template: None,
        options: None,
    };
    let profile = |name: &str, targets: Vec<TargetConfig>| BuildProfile {
        name: name.to_string(),
        targets,
        description: None,
        variables: indexmap::IndexMap::new(),
    };
    let mut config = ProjectConfig::default();
    config.project.name = "demo".to_string();
    config.schema_path = Some("schema.yaml".to_string());
    config
        .source_roots
        .insert("main".to_string(), "config".to_string());
    config.profiles.insert(
        "client".to_string(),
        profile("client", vec![target("json", "build/client/json")]),
    );
    config.profiles.insert(
        "server".to_string(),
        profile("server", vec![target("csv", "build/server/csv")]),
    );
    Ctx {
        root: dir.path().to_path_buf(),
        config,
    }
}

#[test]
fn api_validate_accepts_clean_and_rejects_broken_documents() {
    let ok = api_validate(&good_doc());
    assert_eq!(ok["ok"], serde_json::json!(true));
    assert_eq!(ok["diagnostics"], serde_json::json!([]));

    // Not JSON at all → E1701 from the editor-door parse.
    let bad = api_validate("{not json");
    assert_eq!(bad["ok"], serde_json::json!(false));
    assert_eq!(bad["diagnostics"].as_array().map(Vec::len), Some(1));
    assert_eq!(bad["diagnostics"][0]["code"], serde_json::json!("E1701"));

    // Parses as a Schema but the primary key dangles → E1004.
    let dangling = api_validate(&dangling_pk_doc());
    assert_eq!(dangling["ok"], serde_json::json!(false));
    let codes: Vec<&str> = dangling["diagnostics"]
        .as_array()
        .expect("diagnostics list")
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(codes.contains(&"E1004"), "expected E1004, got {codes:?}");
}

#[test]
fn api_schema_serves_the_editor_document_and_project_facts() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("write schema");
    let ctx = ctx_in(&dir, Some("schema.yaml"));

    let (status, doc) = api_schema(&ctx, "");
    assert_eq!(status, 200);
    assert_eq!(doc["ok"], serde_json::json!(true));
    assert_eq!(doc["project"], serde_json::json!("demo"));
    assert_eq!(doc["schema_path"], serde_json::json!("schema.yaml"));
    assert!(doc["schema"].is_object(), "editor schema present");
    let profiles = doc["profile_names"].as_array().expect("profiles");
    assert!(profiles.iter().any(|p| p == "client"));
    assert!(
        doc.get("projection").is_none(),
        "no profile → no projection"
    );

    // No schema_path configured → an empty editor document, still ok.
    let empty = ctx_in(&dir, None);
    let (status, doc) = api_schema(&empty, "");
    assert_eq!(status, 200);
    assert_eq!(doc["ok"], serde_json::json!(true));
    assert_eq!(doc["schema_path"], serde_json::json!(null));
}

#[test]
fn api_schema_projection_gates_tables_and_fields() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    // Rewrite the demo schema with a server-only field and a
    // server-only table — filter_by_profile's exact input shape.
    fs::write(
            dir.path().join("schema.yaml"),
            "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\n      \
             name: { name: name, type: { kind: String } }\n      \
             internal_note: { name: internal_note, type: { kind: String }, targets: [server] }\n  \
             ServerOnly:\n    name: ServerOnly\n    primary_key: [id]\n    targets: [server]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
        )
        .expect("rewrite schema");

    // client: both the server-only field and table drop out of the
    // projection while the full document keeps them.
    let (status, doc) = api_schema(&ctx, "profile=client");
    assert_eq!(status, 200);
    assert_eq!(doc["ok"], serde_json::json!(true));
    assert_eq!(doc["projection"]["profile"], serde_json::json!("client"));
    let full = &doc["schema"]["tables"];
    let proj = &doc["projection"]["schema"]["tables"];
    assert!(full.get("ServerOnly").is_some(), "full keeps the table");
    assert!(
        proj.get("ServerOnly").is_none(),
        "client projection drops it"
    );
    assert!(
        full["Item"]["fields"].get("internal_note").is_some(),
        "full keeps the field"
    );
    assert!(
        proj["Item"]["fields"].get("internal_note").is_none(),
        "client projection drops it"
    );
    assert_eq!(doc["projection"]["visible_tables"], serde_json::json!(1));

    // server: the projected view matches the full document.
    let (_, doc) = api_schema(&ctx, "profile=server");
    let proj = &doc["projection"]["schema"]["tables"];
    assert!(proj.get("ServerOnly").is_some(), "server sees the table");
    assert!(
        proj["Item"]["fields"].get("internal_note").is_some(),
        "server sees the field"
    );
}

#[test]
fn api_schema_rejects_an_unknown_projection_profile() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    let (status, doc) = api_schema(&ctx, "profile=ghost");
    assert_eq!(status, 400);
    assert_eq!(doc["ok"], serde_json::json!(false));
    let err = doc["error"].as_str().expect("error message");
    assert!(err.contains("ghost"), "error names the profile: {err}");
}

#[test]
fn api_save_writes_canonical_yaml_and_reports_the_target() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, Some("schema.yaml"));
    let doc = good_doc();

    let (status, payload) = api_save(&ctx, &doc);
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));
    // bytes counts the canonical YAML payload, not the request
    let schema = from_editor_json(&doc).expect("doc parses");
    let yaml = to_canonical_yaml(&schema);
    assert_eq!(payload["bytes"], serde_json::json!(yaml.len()));
    let written = fs::read_to_string(dir.path().join("schema.yaml")).expect("saved file");
    assert_eq!(written, yaml);
    assert!(
        payload.get("note").is_none(),
        "no note when schema_path is set"
    );
}

#[test]
fn api_save_without_schema_path_defaults_and_notes_the_gap() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, None);

    let (status, payload) = api_save(&ctx, &good_doc());
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert!(
        fs::exists(dir.path().join("schema.yaml")).expect("stat"),
        "default target written"
    );
    assert!(payload["note"]
        .as_str()
        .is_some_and(|n| n.contains("schema_path = \"schema.yaml\"")));
}

#[test]
fn api_save_refuses_directory_and_registry_targets_without_writing() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir(dir.path().join("schemas")).expect("mkdir");

    let (status, payload) = api_save(&ctx_in(&dir, Some("schemas")), &good_doc());
    assert_eq!(status, 409);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("is a directory")));
    assert!(!dir.path().join("schemas").is_file(), "nothing written");

    let (status, payload) = api_save(&ctx_in(&dir, Some("registry:pkg")), &good_doc());
    assert_eq!(status, 409);
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("Configuration Registry")));
}

#[test]
fn api_save_reports_broken_documents_as_diagnostics_without_writing() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, Some("schema.yaml"));

    let (status, payload) = api_save(&ctx, "{not json");
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert_eq!(
        payload["diagnostics"][0]["code"],
        serde_json::json!("E1701")
    );
    assert!(
        !fs::exists(dir.path().join("schema.yaml")).expect("stat"),
        "nothing written for a broken document"
    );
}

// ── multi-file (directory) schema_path: incremental save-back ──

/// A three-fragment schema directory: `a.yaml` owns the metadata plus
/// the Item/Weapon tables, `b.yaml` Quest, `c.yaml` Shop (the file the
/// tests never touch). Returns the file bytes at load time.
fn multi_file_project(dir: &TempDir) -> std::collections::BTreeMap<String, Vec<u8>> {
    let schemas = dir.path().join("schemas");
    fs::create_dir(&schemas).expect("mkdir");
    fs::write(
            schemas.join("a.yaml"),
            "metadata:\n  version: \"1.0\"\nenums: {}\ntables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id:\n        name: id\n        type: { kind: UInt32 }\n        required: true\n  Weapon:\n    name: Weapon\n    primary_key: [id]\n    fields:\n      id:\n        name: id\n        type: { kind: UInt32 }\n        required: true\n",
        )
        .expect("write a.yaml");
    fs::write(
            schemas.join("b.yaml"),
            "enums: {}\ntables:\n  Quest:\n    name: Quest\n    primary_key: [id]\n    fields:\n      id:\n        name: id\n        type: { kind: UInt32 }\n        required: true\n",
        )
        .expect("write b.yaml");
    fs::write(
            schemas.join("c.yaml"),
            "enums: {}\ntables:\n  Shop:\n    name: Shop\n    primary_key: [id]\n    fields:\n      id:\n        name: id\n        type: { kind: UInt32 }\n        required: true\n",
        )
        .expect("write c.yaml");
    ["schemas/a.yaml", "schemas/b.yaml", "schemas/c.yaml"]
        .into_iter()
        .map(|f| {
            (
                f.to_string(),
                fs::read(dir.path().join(f)).expect("read fragment"),
            )
        })
        .collect()
}

/// Load the merged schema + index the way GET /api/schema does, edit a
/// member through the editor JSON (so equality goes through the wire
/// shape), and serialize the save envelope.
fn edited_envelope(
    dir: &TempDir,
    edit: impl FnOnce(&mut serde_json::Value),
) -> (String, Schema, crate::schema_save::SchemaSourceIndex) {
    let (merged, index) =
        crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("schemas"))
            .expect("schema loads");
    let mut doc = to_editor_json(&merged);
    edit(&mut doc);
    let schema = from_editor_json(&doc.to_string()).expect("edited doc parses");
    let envelope = serde_json::json!({
        "schema": doc,
        "sources": index.to_json(),
    });
    (
        serde_json::to_string(&envelope).expect("envelope serializes"),
        schema,
        index,
    )
}

#[test]
fn api_save_multi_file_rewrites_only_the_changed_source_file() {
    let dir = TempDir::new().expect("tempdir");
    let before = multi_file_project(&dir);
    let ctx = ctx_in(&dir, Some("schemas"));

    // Touch exactly one member of a.yaml: Item.id gains a `min` bound.
    let (envelope, edited, index) = edited_envelope(&dir, |doc| {
        doc["tables"]["Item"]["fields"]["id"]["min"] = serde_json::json!(1);
    });

    let (status, payload) = api_save(&ctx, &envelope);
    assert_eq!(status, 200, "{payload}");
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert_eq!(
        payload["written"],
        serde_json::json!(["schemas/a.yaml"]),
        "only the owner of the changed table is rewritten"
    );
    assert_eq!(payload["unchanged"], serde_json::json!(2));

    // Untouched files are byte-identical — never even rewritten.
    for f in ["schemas/b.yaml", "schemas/c.yaml"] {
        let after = fs::read(dir.path().join(f)).expect("file");
        assert_eq!(after, before[f], "{f} must stay byte-identical");
    }
    assert_eq!(
        payload["bytes"],
        serde_json::json!(fs::read(dir.path().join("schemas/a.yaml"))
            .expect("a")
            .len())
    );

    // The saved-back merged schema is exactly the edited one, and the
    // response's index carries the new revisions (next save conflicts
    // with itself no more).
    let (reloaded, reloaded_index) =
        crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("schemas"))
            .expect("reload");
    assert_eq!(to_editor_json(&reloaded), to_editor_json(&edited));
    assert_eq!(reloaded_index.to_json(), payload["sources"]);
    assert_ne!(reloaded_index.to_json(), index.to_json());

    // No temp siblings left behind by the atomic writes.
    let leftovers: Vec<_> = fs::read_dir(dir.path().join("schemas"))
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "no temp files: {leftovers:?}");
}

#[test]
fn api_save_multi_file_routes_added_and_removed_tables_to_their_owners() {
    let dir = TempDir::new().expect("tempdir");
    let before = multi_file_project(&dir);
    let ctx = ctx_in(&dir, Some("schemas"));

    // Add a Buff table (lands in the metadata owner, a.yaml) and drop
    // Quest (owned by b.yaml). c.yaml keeps its Shop.
    let (envelope, edited, _) = edited_envelope(&dir, |doc| {
        doc["tables"]["Buff"] = serde_json::json!({
            "name": "Buff",
            "primary_key": ["id"],
            "fields": {
                "id": { "name": "id", "type": { "kind": "UInt32" }, "required": true }
            }
        });
        doc["tables"]["Quest"] = serde_json::Value::Null;
        let obj = doc["tables"].as_object_mut().expect("tables object");
        obj.remove("Quest");
    });

    let (status, payload) = api_save(&ctx, &envelope);
    assert_eq!(status, 200, "{payload}");
    assert_eq!(
        payload["written"],
        serde_json::json!(["schemas/a.yaml", "schemas/b.yaml"])
    );
    let c = fs::read(dir.path().join("schemas/c.yaml")).expect("c");
    assert_eq!(c, before["schemas/c.yaml"], "c.yaml byte-identical");

    let b = fs::read_to_string(dir.path().join("schemas/b.yaml")).expect("b");
    assert!(!b.contains("Quest"), "Quest removed: {b}");
    let a = fs::read_to_string(dir.path().join("schemas/a.yaml")).expect("a");
    assert!(a.contains("Buff"), "Buff added to the metadata owner: {a}");
    assert!(a.contains("version:"), "metadata stays with its owner");

    let (reloaded, _) =
        crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("schemas"))
            .expect("reload");
    assert_eq!(to_editor_json(&reloaded), to_editor_json(&edited));
}

#[test]
fn api_save_multi_file_rejects_a_stale_sources_index_without_writing() {
    let dir = TempDir::new().expect("tempdir");
    let before = multi_file_project(&dir);
    let ctx = ctx_in(&dir, Some("schemas"));

    // The editor loads and edits (index captured here) …
    let (envelope, _, _) = edited_envelope(&dir, |doc| {
        doc["tables"]["Item"]["fields"]["id"]["min"] = serde_json::json!(1);
    });
    // … and only afterwards does a concurrent edit land on b.yaml.
    fs::write(
            dir.path().join("schemas/b.yaml"),
            "enums: {}\ntables:\n  Quest:\n    name: Quest\n    primary_key: [id]\n    fields:\n      id:\n        name: id\n        type: { kind: UInt32 }\n        required: true\n      reward:\n        name: reward\n        type: { kind: String }\n",
        )
        .expect("concurrent edit");
    let (status, payload) = api_save(&ctx, &envelope);
    assert_eq!(status, 409);
    assert_eq!(payload["ok"], serde_json::json!(false));
    let err = payload["error"].as_str().expect("error");
    assert!(err.contains("schemas/b.yaml"), "names the file: {err}");
    assert!(err.contains("conflict"), "{err}");

    // Nothing written: a.yaml/c.yaml are byte-identical to load time
    // (a.yaml does NOT receive the edited Item) and b.yaml still holds
    // exactly the concurrent edit.
    for f in ["schemas/a.yaml", "schemas/c.yaml"] {
        let now = fs::read(dir.path().join(f)).expect("file");
        assert_eq!(now, before[f], "{f} untouched by a refused save");
    }
}

#[test]
fn api_save_multi_file_still_needs_the_sources_index_envelope() {
    let dir = TempDir::new().expect("tempdir");
    multi_file_project(&dir);
    let ctx = ctx_in(&dir, Some("schemas"));

    // Raw editor document (no envelope): refused, nothing written.
    let (status, payload) = api_save(&ctx, &good_doc());
    assert_eq!(status, 409);
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("is a directory")));
}

#[test]
fn api_save_single_file_keeps_the_raw_contract_and_honors_an_echoed_index() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, Some("schema.yaml"));
    fs::write(
            dir.path().join("schema.yaml"),
            "enums: {}\ntables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id:\n        name: id\n        type: { kind: UInt32 }\n        required: true\n",
        )
        .expect("write schema");

    // Raw document, no envelope: the pre-existing contract (200).
    let (status, payload) = api_save(&ctx, &good_doc());
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));

    // Envelope with the current revision: accepted.
    let (envelope, _) = {
        let (_, index) =
            crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("schema.yaml"))
                .expect("indexed load");
        let mut doc = to_editor_json(&from_editor_json(&good_doc()).expect("parses"));
        doc["tables"]["Item"] = serde_json::json!({
            "name": "Item",
            "primary_key": ["id"],
            "fields": {
                "id": { "name": "id", "type": { "kind": "UInt32" }, "required": true }
            }
        });
        let envelope = serde_json::json!({ "schema": doc, "sources": index.to_json() });
        (serde_json::to_string(&envelope).expect("json"), index)
    };
    let (status, payload) = api_save(&ctx, &envelope);
    assert_eq!(status, 200, "{payload}");

    // A stale revision (file changed on disk after load): 409.
    fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("concurrent edit");
    let (status, payload) = api_save(&ctx, &envelope);
    assert_eq!(status, 409);
    let err = payload["error"].as_str().expect("error");
    assert!(
        err.contains("schema.yaml") && err.contains("conflict"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("schema.yaml")).expect("file"),
        "tables: {}\nenums: {}\n",
        "a refused save never writes"
    );
}

#[test]
fn sources_index_round_trips_through_json_and_hashes_file_bytes() {
    let dir = TempDir::new().expect("tempdir");
    multi_file_project(&dir);
    let (_, index) =
        crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("schemas"))
            .expect("indexed load");

    // Wire shape → parse back → identical index.
    let json = index.to_json();
    assert_eq!(
        crate::schema_save::SchemaSourceIndex::from_json(&json).expect("sources parse"),
        index
    );

    // Revisions are the real sha256 of the file bytes; ownership and
    // the metadata owner follow the sorted merge order.
    let a_bytes = fs::read(dir.path().join("schemas/a.yaml")).expect("a");
    assert_eq!(
        index
            .files
            .iter()
            .find(|f| f.file == "schemas/a.yaml")
            .expect("a listed")
            .sha256,
        crate::schema_save::sha256_hex(&a_bytes)
    );
    assert_eq!(
        index.tables.get("Item").map(String::as_str),
        Some("schemas/a.yaml")
    );
    assert_eq!(
        index.tables.get("Quest").map(String::as_str),
        Some("schemas/b.yaml")
    );
    assert_eq!(index.metadata.as_deref(), Some("schemas/a.yaml"));
    assert!(
        index
            .files
            .windows(2)
            .all(|w| w[0].file.as_str() < w[1].file.as_str()),
        "files sorted: {:?}",
        index.files
    );
}

#[test]
fn api_status_reports_validation_and_untouched_build_facts() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_status(&ctx, "");
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert_eq!(payload["project"], serde_json::json!("demo"));
    assert_eq!(payload["profile"], serde_json::json!("client"));
    assert_eq!(
        payload["profile_names"],
        serde_json::json!(["client", "server"])
    );
    // Validation panel: the on-disk project validates clean.
    assert_eq!(payload["validation"]["ok"], serde_json::json!(true));
    assert_eq!(payload["validation"]["errors"], serde_json::json!(0));
    assert_eq!(
        payload["validation"]["visible_tables"],
        serde_json::json!(1)
    );
    assert_eq!(payload["validation"]["diagnostics"], serde_json::json!([]));
    // No manifest yet: nothing built, nothing fresh.
    let client = &payload["builds"]["client"];
    assert_eq!(client["fresh"], serde_json::json!(false));
    assert_eq!(client["last_build_profile"], serde_json::json!(null));
    assert_eq!(client["target_count"], serde_json::json!(1));
    assert_eq!(client["changed_tables"], serde_json::json!([]));
    assert!(client.get("build_id").is_none(), "no build to report");
    assert_eq!(
        payload["builds"]["server"]["target_count"],
        serde_json::json!(1)
    );
}

#[test]
fn api_status_rejects_an_unknown_profile() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_status(&ctx, "profile=ghost");
    assert_eq!(status, 400);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("unknown profile 'ghost'")));
}

#[test]
fn api_tables_lists_visible_tables_with_facts() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_tables(&ctx, "");
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert_eq!(payload["profile"], serde_json::json!("client"));
    let tables = payload["tables"].as_array().expect("tables array");
    assert_eq!(tables.len(), 1, "the demo project has one table");
    let item = &tables[0];
    assert_eq!(item["name"], serde_json::json!("Item"));
    assert_eq!(item["rows"], serde_json::json!(1));
    assert_eq!(item["primary_key"], serde_json::json!(["id"]));
    assert!(item["source_file"]
        .as_str()
        .is_some_and(|s| s.ends_with("item.json")));

    // Same visibility contract as the CLI: an unknown profile is
    // rejected, not silently defaulted.
    let (status, payload) = api_tables(&ctx, "profile=ghost");
    assert_eq!(status, 400);
    assert_eq!(payload["ok"], serde_json::json!(false));
}

#[test]
fn api_table_paginates_rows_and_hides_hidden_tables() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    // Five rows so pagination has something to paginate — the panel
    // handlers re-read the project from disk, so the source file is
    // what matters.
    let data: Vec<String> = (0..5)
        .map(|i| format!(r#"{{ "id": {i}, "name": "row{i}" }}"#))
        .collect();
    fs::write(
        dir.path().join("config/item.json"),
        format!("{{\"Item\": [{}]}}", data.join(", ")),
    )
    .expect("write source");

    // Missing name is a 400, not a silent empty list.
    let (status, payload) = api_table(&ctx, "");
    assert_eq!(status, 400);
    assert_eq!(payload["ok"], serde_json::json!(false));

    // Ghost table is a 404: hidden by the profile projection or
    // misspelled — either way the panel must not pretend it exists.
    let (status, payload) = api_table(&ctx, "name=Ghost");
    assert_eq!(status, 404);
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("Ghost")));

    // Default page: all five rows, schema column order.
    let (status, payload) = api_table(&ctx, "name=Item");
    assert_eq!(status, 200);
    assert_eq!(payload["total"], serde_json::json!(5));
    assert_eq!(payload["offset"], serde_json::json!(0));
    assert_eq!(payload["columns"], serde_json::json!(["id", "name"]));
    assert_eq!(payload["rows"].as_array().expect("rows").len(), 5);

    // A window in the middle keeps its absolute indexes.
    let (status, payload) = api_table(&ctx, "name=Item&offset=2&limit=2");
    assert_eq!(status, 200);
    assert_eq!(payload["offset"], serde_json::json!(2));
    let rows = payload["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["index"], serde_json::json!(2));
    assert_eq!(rows[0]["cells"]["name"], serde_json::json!("row2"));

    // Offset past the end stays 200 with zero rows (the panel just
    // shows an empty page; it does not error the user out).
    let (status, payload) = api_table(&ctx, "name=Item&offset=50");
    assert_eq!(status, 200);
    assert_eq!(payload["rows"].as_array().expect("rows").len(), 0);
}

#[test]
fn api_upload_replaces_table_source_after_validation() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_upload(
        &ctx,
        "name=Item",
        br#"{"Item": [{"id": 2, "name": "Shield"}]}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));

    // The swap really happened on disk, and the panel sees it through
    // the same load chain the CLI uses.
    let on_disk = fs::read_to_string(dir.path().join("config/item.json")).expect("read");
    assert!(
        on_disk.contains("Shield"),
        "source must hold new bytes: {on_disk}"
    );
    let (_, page) = api_table(&ctx, "name=Item");
    assert_eq!(page["total"], serde_json::json!(1));
    assert_eq!(
        page["rows"][0]["cells"]["name"],
        serde_json::json!("Shield")
    );

    // Atomic swap means no temp/backup litter beside the source.
    let litter: Vec<String> = fs::read_dir(dir.path().join("config"))
        .expect("dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("upload"))
        .collect();
    assert!(litter.is_empty(), "leftover temp files: {litter:?}");
}

#[test]
fn api_upload_refuses_invalid_data_and_restores_original() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    let before = fs::read(dir.path().join("config/item.json")).expect("read");

    // `id` is Int32 and required: string ids must not enter the repo.
    let (status, payload) = api_upload(
        &ctx,
        "name=Item",
        br#"{"Item": [{"id": "no", "name": "x"}]}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(false));
    let diags = payload["diagnostics"].as_array().expect("diagnostics");
    assert!(!diags.is_empty(), "refusal must carry the diagnostics");

    let after = fs::read(dir.path().join("config/item.json")).expect("read");
    assert_eq!(
        before, after,
        "a refused upload must leave the source byte-identical"
    );
    // No half-swapped litter either way.
    assert!(!dir.path().join("config/item.json.upload-bak").exists());
}

#[test]
fn api_upload_refuses_unparseable_body_and_restores_original() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    let before = fs::read(dir.path().join("config/item.json")).expect("read");

    // The post-swap load fails (this is a .json source), the original
    // is restored, and the error names what was wrong.
    let (status, payload) = api_upload(&ctx, "name=Item", b"not json at all");
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert!(payload["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert_eq!(
        before,
        fs::read(dir.path().join("config/item.json")).expect("read")
    );
}

#[test]
fn api_upload_rejects_missing_name_and_ghost_table() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_upload(&ctx, "", b"{}");
    assert_eq!(status, 400);
    assert_eq!(payload["ok"], serde_json::json!(false));

    let (status, payload) = api_upload(&ctx, "name=Ghost", b"{}");
    assert_eq!(status, 404);
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("Ghost")));
}

#[test]
fn resolve_upload_target_keeps_paths_inside_source_roots() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("config")).expect("mkdir");
    fs::create_dir_all(root.join("config/ro")).expect("mkdir");
    fs::write(root.join("config/item.json"), "{}").expect("write");
    fs::write(root.join("outside.json"), "{}").expect("write");
    fs::write(root.join("config/ro/item.json"), "{}").expect("write");

    let roots = vec![PathBuf::from("config")];
    // Registered relative path inside a root: fine.
    assert!(resolve_upload_target(root, "config/item.json", &roots).is_ok());
    // Outside every configured root is refused even inside the project.
    assert!(resolve_upload_target(root, "outside.json", &roots)
        .err()
        .is_some_and(|e| e.contains("source root")));
    // A parent escape that does not exist on disk fails at
    // resolution; one that exists is caught by the root guard (the
    // symlink case below proves that branch). Either way: refused.
    assert!(resolve_upload_target(root, "../escape.json", &roots).is_err());
    // A directory as the target is refused.
    assert!(resolve_upload_target(root, "config", &roots)
        .err()
        .is_some_and(|e| e.contains("not a file")));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("outside.json"), root.join("config/link.json"))
            .expect("symlink");
        assert!(
            resolve_upload_target(root, "config/link.json", &roots)
                .err()
                .is_some_and(|e| e.contains("outside")),
            "symlink escape must be caught by canonicalization"
        );
    }
}

#[test]
fn api_build_then_status_reports_fresh_and_up_to_date() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert_eq!(payload["up_to_date"], serde_json::json!(false));
    assert!(payload["artifact_count"].as_u64().unwrap() >= 1);
    assert!(payload["build_id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(
        fs::exists(dir.path().join("build/client/json/Item.json")).expect("stat"),
        "the json artifact landed"
    );

    // Status now: client fresh with the manifest facts, server stale
    // (the single-slot manifest belongs to client).
    let (_, payload) = api_status(&ctx, "profile=client");
    let client = &payload["builds"]["client"];
    assert_eq!(client["fresh"], serde_json::json!(true));
    assert_eq!(client["last_build_profile"], serde_json::json!("client"));
    assert_eq!(client["changed_tables"], serde_json::json!([]));
    assert_eq!(client["build_id"], payload["builds"]["client"]["build_id"]);
    assert_eq!(
        payload["builds"]["server"]["fresh"],
        serde_json::json!(false)
    );
    assert_eq!(
        payload["builds"]["server"]["last_build_profile"],
        serde_json::json!("client")
    );

    // A second build is a no-op, not a rebuild.
    let (_, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(payload["up_to_date"], serde_json::json!(true));
}

#[test]
fn api_status_names_stale_tables_after_a_source_edit() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile": "client"}"#);

    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
    )
    .expect("edit source");

    let (_, payload) = api_status(&ctx, "profile=client");
    let client = &payload["builds"]["client"];
    assert_eq!(client["fresh"], serde_json::json!(false));
    assert_eq!(client["changed_tables"], serde_json::json!(["Item"]));
}

/// A script-adapter project (design §51 Q2): `build_project` mixes the
/// script fingerprint into `source_hash`, and the status page must too —
/// otherwise editing the script (byte-identical parsed model) is
/// misreported as fresh (review finding).
#[test]
fn api_status_marks_script_edit_stale() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
    fs::create_dir_all(dir.path().join("tools")).expect("mkdir tools");
    fs::write(
        dir.path().join("tools/meta.py"),
        "#!/usr/bin/env python3\nprint('v1')\n",
    )
    .expect("write script");
    fs::write(
        dir.path().join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[source_roots]
main = "config"

[project]
name = "demo"

# A declared script adapter — the script bytes feed `source_hash` even
# though this demo's source is JSON (the fingerprint is config-driven).
[source.excel]
script = "tools/meta.py"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
"#,
    )
    .expect("write config");
    fs::write(
        dir.path().join("schema.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\n      \
             name: { name: name, type: { kind: String } }\nenums: {}\n",
    )
    .expect("write schema");
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
    )
    .expect("write source");

    // Build a Ctx carrying the on-disk config (with the script wiring),
    // the way `run_web` does — `ctx_in` only carries the default config.
    let config = crate::load_project_config(dir.path()).expect("config");
    let ctx = Ctx {
        root: dir.path().to_path_buf(),
        config,
    };

    // First build mixes the v1 script bytes into source_hash.
    let (status, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(status, 200, "{payload}");
    assert_eq!(payload["ok"], serde_json::json!(true));
    let (_, payload) = api_status(&ctx, "profile=client");
    assert_eq!(
        payload["builds"]["client"]["fresh"],
        serde_json::json!(true),
        "unchanged script → still fresh"
    );

    // Edit the script: the parsed JSON model is byte-identical, but the
    // script fingerprint rotates — status must not report fresh.
    fs::write(
        dir.path().join("tools/meta.py"),
        "#!/usr/bin/env python3\nprint('v2')\n",
    )
    .expect("edit script");
    let (_, payload) = api_status(&ctx, "profile=client");
    assert_eq!(
        payload["builds"]["client"]["fresh"],
        serde_json::json!(false),
        "a script edit must mark the build stale"
    );
}

#[test]
fn api_build_reports_broken_data_as_diagnostics() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    // Row without the required id — L1 validation must refuse the build.
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"name": "Sword"}]}"#,
    )
    .expect("write broken source");

    let (status, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert!(payload["diagnostics"]
        .as_array()
        .is_some_and(|d| !d.is_empty()));
    assert!(
        !fs::exists(dir.path().join("build/client/json/Item.json")).expect("stat"),
        "nothing built on validation failure"
    );
}

#[test]
fn api_build_rejects_an_unknown_profile() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (status, payload) = api_build(&ctx, r#"{"profile": "ghost"}"#);
    assert_eq!(status, 400);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert!(payload["error"]
        .as_str()
        .is_some_and(|e| e.contains("unknown profile 'ghost'")));
}

#[test]
fn api_artifacts_lists_manifest_files_for_the_built_profile() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    // Nothing built: an empty list, no manifest_profile.
    let (status, payload) = api_artifacts(&ctx, "profile=client");
    assert_eq!(status, 200);
    assert_eq!(payload["manifest_profile"], serde_json::json!(null));
    assert_eq!(payload["artifacts"], serde_json::json!([]));

    api_build(&ctx, r#"{"profile": "client"}"#);
    let (_, payload) = api_artifacts(&ctx, "profile=client");
    assert_eq!(payload["manifest_profile"], serde_json::json!("client"));
    assert_eq!(payload["fresh"], serde_json::json!(true));
    assert_eq!(payload["stale"], serde_json::json!(false));
    let artifacts = payload["artifacts"].as_array().expect("artifact list");
    assert_ne!(artifacts.len(), 0);
    assert!(artifacts[0]["path"].as_str().is_some());
    assert!(artifacts[0]["hash"].as_str().is_some());
    assert_ne!(artifacts.len(), 0);
    let targets = payload["targets"].as_array().expect("target records");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["format"], serde_json::json!("json"));

    // The single-slot manifest belongs to client: server reports the
    // owner and an empty list instead of unverifiable leftovers.
    let (_, payload) = api_artifacts(&ctx, "profile=server");
    assert_eq!(payload["manifest_profile"], serde_json::json!("client"));
    assert_eq!(payload["artifacts"], serde_json::json!([]));
}

#[test]
fn api_artifact_serves_ledgered_bytes_and_refuses_the_rest() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    let (_, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
    drop(payload);

    // A ledgered file serves byte-identical content with a JSON type.
    let (status, mime, bytes) = api_artifact(&ctx, "path=build/client/json/Item.json");
    assert_eq!(status, 200);
    assert_eq!(mime, "application/json; charset=utf-8");
    assert_eq!(
        bytes,
        fs::read(dir.path().join("build/client/json/Item.json")).expect("read artifact")
    );

    // A path outside the manifest is refused outright — lookup, never
    // join-and-pray (path traversal included).
    let (status, _, bytes) = api_artifact(&ctx, "path=schema.yaml");
    assert_eq!(status, 404);
    assert!(String::from_utf8_lossy(&bytes).contains("not in the build manifest"));
    let (status, _, bytes) = api_artifact(&ctx, "path=../cage.toml");
    assert_eq!(status, 404);
    assert!(String::from_utf8_lossy(&bytes).contains("not in the build manifest"));
    let (status, _, _bytes) = api_artifact(&ctx, "");
    assert_eq!(status, 400);

    // A file edited after the build no longer matches its hash: 409,
    // not tampered content.
    fs::write(
        dir.path().join("build/client/json/Item.json"),
        r#"{"hacked":1}"#,
    )
    .expect("tamper");
    let (status, _, bytes) = api_artifact(&ctx, "path=build/client/json/Item.json");
    assert_eq!(status, 409);
    assert!(
        String::from_utf8_lossy(&bytes).contains("no longer matches"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
}

#[test]
fn api_artifacts_names_stale_manifests() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile": "client"}"#);

    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
    )
    .expect("edit source");

    let (_, payload) = api_artifacts(&ctx, "profile=client");
    assert_eq!(payload["stale"], serde_json::json!(true));
    assert_eq!(payload["fresh"], serde_json::json!(false));
    // The manifest still lists what the build produced — with the
    // stale badge the page can say "看可以，别当新鲜".
    assert_ne!(payload["artifacts"].as_array().expect("list").len(), 0);
}

#[test]
fn api_artifacts_reports_unknown_profiles_and_uses_the_first_profile() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile": "client"}"#);

    let (status, payload) = api_artifacts(&ctx, "profile=ghost");
    assert_eq!(status, 400);
    assert!(payload["error"].as_str().unwrap().contains("available"));

    let (_, payload) = api_artifacts(&ctx, "");
    assert_eq!(payload["profile"], serde_json::json!("client"));
}

#[test]
fn api_artifact_reports_missing_manifest_and_unreadable_file() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    let (status, _, bytes) = api_artifact(&ctx, "path=build/client/json/Item.json");
    assert_eq!(status, 404);
    let message = String::from_utf8_lossy(&bytes);
    assert!(message.contains("no build manifest"), "{message}");

    api_build(&ctx, r#"{"profile": "client"}"#);
    let artifact = dir.path().join("build/client/json/Item.json");
    fs::remove_file(artifact).expect("remove artifact");
    let (status, _, bytes) = api_artifact(&ctx, "path=build/client/json/Item.json");
    assert_eq!(status, 500);
    assert!(
        String::from_utf8_lossy(&bytes).contains("cannot read artifact"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
}

#[test]
fn api_tables_reject_unknown_profiles() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    let (status, payload) = api_tables(&ctx, "profile=ghost");
    assert_eq!(status, 400);
    assert!(payload["error"].as_str().unwrap().contains("ghost"));
}

#[test]
fn api_table_maps_bytes_and_nested_values() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
    )
    .expect("write source");

    let (_, page) = api_table(&ctx, "name=Item&limit=1");
    let cells = &page["rows"][0]["cells"];
    assert_eq!(cells["name"], serde_json::json!("Sword"));
    assert_eq!(cells["meta"], serde_json::json!(null));
}

#[test]
fn api_upload_hides_tables_outside_the_profile_view() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    fs::write(
        dir.path().join("config/item-more.json"),
        r#"{"Ghost": [{"id": 2, "name": "Bow"}]}"#,
    )
    .expect("write second source");

    let (status, payload) = api_upload(&ctx, "name=Ghost", br#"{"Ghost": []}"#);
    assert_eq!(status, 404);
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert!(payload["error"].as_str().unwrap().contains("Ghost"));
}

#[test]
fn api_upload_writes_the_first_registered_root_even_when_later_roots_have_data() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = demo_ctx(&dir);
    ctx.config
        .source_roots
        .insert("extra".into(), "more".into());
    fs::create_dir_all(dir.path().join("more")).unwrap();
    fs::write(
        dir.path().join("more/items.json"),
        r#"{"Item": [{"id": 2, "name": "Bow"}]}"#,
    )
    .unwrap();

    let (status, payload) = api_upload(&ctx, "name=Item", br#"{"Item": [{"id": 1}]}"#);
    assert_eq!(status, 200, "{payload}");
    let expected = ctx.root.join("config/item.json").display().to_string();
    assert_eq!(payload["source_file"], serde_json::json!(expected));
}

#[test]
fn api_upload_reports_500_when_the_source_parent_is_unwritable() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    let cfg_path = dir.path().join("cage.toml");
    let cfg = fs::read_to_string(&cfg_path)
        .unwrap()
        .replace("main = \"config\"", "main = \"config/ro\"");
    fs::write(&cfg_path, cfg).unwrap();
    fs::create_dir_all(dir.path().join("config/ro")).unwrap();
    fs::rename(
        dir.path().join("config/item.json"),
        dir.path().join("config/ro/item.json"),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(
        dir.path().join("config/ro"),
        fs::Permissions::from_mode(0o555),
    )
    .unwrap();

    let (status, payload) = api_upload(&ctx, "name=Item", br#"{"Item": []}"#);
    assert_eq!(status, 500, "{payload}");
    assert!(payload["error"].as_str().unwrap().contains("cannot"));
    #[cfg(unix)]
    fs::set_permissions(
        dir.path().join("config/ro"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}

#[test]
fn api_snapshot_packs_and_api_snapshots_verifies() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);

    // No snapshots yet — ok with an empty list.
    let (status, payload) = api_snapshots(&ctx);
    assert_eq!(status, 200);
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert_eq!(payload["snapshots"], serde_json::json!([]));

    // Pack one profile: the `cage snapshot` pipeline, self-verified.
    let (status, pack) = api_snapshot(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(status, 200, "{pack}");
    assert_eq!(pack["ok"], serde_json::json!(true));
    let rel = pack["snapshot_dir"].as_str().expect("snapshot dir");
    assert!(
        rel.starts_with("build/snapshot/client-"),
        "pack lands under output_dir/snapshot: {rel}"
    );
    assert!(pack["files"].as_u64().expect("files") >= 3, "{pack}");

    // The listing now shows it, freshly verified.
    let (_, list) = api_snapshots(&ctx);
    let snaps = list["snapshots"].as_array().expect("list");
    assert_eq!(snaps.len(), 1, "{list}");
    assert_eq!(
        snaps[0]["name"],
        serde_json::json!(rel.rsplit('/').next().unwrap())
    );
    assert_eq!(snaps[0]["verified"], serde_json::json!(true));
    assert_eq!(snaps[0]["profile"], serde_json::json!("client"));
    assert_eq!(snaps[0]["build_id"], pack["build_id"]);

    // Unknown profile and broken body refuse.
    let (status, err) = api_snapshot(&ctx, r#"{"profile": "ghost"}"#);
    assert_eq!(status, 400);
    assert_eq!(err["ok"], serde_json::json!(false));
    let (status, err) = api_snapshot(&ctx, "not json");
    assert_eq!(status, 400);
    assert_eq!(err["ok"], serde_json::json!(false));
}

#[test]
fn api_diff_layers_tables_targets_and_files() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile": "client"}"#);

    // Identical manifests: nothing moved, roots agree.
    let (status, d) = api_diff(&ctx, "from=build&to=build");
    assert_eq!(status, 200);
    assert_eq!(d["ok"], serde_json::json!(true));
    assert_eq!(d["changed_tables"], serde_json::json!([]));
    assert_eq!(d["affected_targets"], serde_json::json!([]));
    assert_eq!(d["files"]["changed"], serde_json::json!([]));
    assert_eq!(
        d["roots"]["content_hash"]["from"],
        d["roots"]["content_hash"]["to"]
    );

    // Baseline manifest aside, then edit the source and rebuild.
    fs::copy(
        dir.path().join("build/manifest.json"),
        dir.path().join("baseline.json"),
    )
    .expect("keep baseline");
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
    )
    .expect("edit source");
    api_build(&ctx, r#"{"profile": "client"}"#);

    let (status, d) = api_diff(&ctx, "from=baseline.json&to=build");
    assert_eq!(status, 200, "{d}");
    assert_eq!(
        d["changed_tables"],
        serde_json::json!(["Item"]),
        "the table fingerprint moved"
    );
    assert_eq!(
        d["affected_targets"],
        serde_json::json!(["build/client/json"]),
        "the target under the changed file"
    );
    let changed = d["files"]["changed"].as_array().expect("changed");
    assert_eq!(changed.len(), 1, "{d}");
    assert_eq!(
        changed[0]["path"],
        serde_json::json!("build/client/json/Item.json")
    );
    assert_eq!(changed[0]["table"], serde_json::json!("Item"));
    assert_eq!(changed[0]["target"], serde_json::json!("build/client/json"));
    assert_ne!(
        d["roots"]["content_hash"]["from"],
        d["roots"]["content_hash"]["to"]
    );
    assert_ne!(d["roots"]["build_id"]["from"], d["roots"]["build_id"]["to"]);

    // Missing parameters and missing manifests refuse with 400.
    let (status, err) = api_diff(&ctx, "from=build");
    assert_eq!(status, 400);
    assert_eq!(err["ok"], serde_json::json!(false));
    let (status, err) = api_diff(&ctx, "from=ghost/manifest.json&to=build");
    assert_eq!(status, 400);
    assert!(err["error"].as_str().expect("msg").contains("baseline"));
}

#[test]
fn api_diff_rejects_absolute_paths_and_parent_traversal() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile": "client"}"#);

    let absolute = format!("from={}&to=build", dir.path().join("build").display());
    let (status, err) = api_diff(&ctx, &absolute);
    assert_eq!(status, 400);
    assert!(err["error"]
        .as_str()
        .unwrap()
        .contains("normalized project-relative path"));

    let (status, err) = api_diff(&ctx, "from=../build&to=build");
    assert_eq!(status, 400);
    assert!(err["error"]
        .as_str()
        .unwrap()
        .contains("normalized project-relative path"));

    let (status, err) = api_diff(&ctx, "from=build&to=absent");
    assert_eq!(status, 400);
    assert!(err["error"].as_str().unwrap().contains("No such file"));
}

#[test]
fn api_registry_browses_the_configured_registry() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = demo_ctx(&dir);

    // Unconfigured: ok with an empty list and a note.
    let (status, reg) = api_registry(&ctx);
    assert_eq!(status, 200);
    assert_eq!(reg["ok"], serde_json::json!(true));
    assert_eq!(reg["packages"], serde_json::json!([]));
    assert!(reg["note"].as_str().expect("note").contains("registry"));

    // Configure [registry], pack a snapshot, publish it for real, and
    // the browse endpoint lists the package.
    ctx.config.registry = Some(cage_core::manifest::RegistryConfig {
        path: "reg".to_string(),
        auth_env: None,
    });
    let (_, pack) = api_snapshot(&ctx, r#"{"profile": "client"}"#);
    let snap_rel = pack["snapshot_dir"].as_str().expect("snapshot dir");
    cage_core::registry::publish(
        &dir.path().join("reg"),
        "demo",
        "0.1.0",
        &dir.path().join(snap_rel),
    )
    .expect("publish");

    let (status, reg) = api_registry(&ctx);
    assert_eq!(status, 200);
    assert_eq!(reg["ok"], serde_json::json!(true));
    assert_eq!(reg["registry"], serde_json::json!("reg"));
    let packages = reg["packages"].as_array().expect("packages");
    assert_eq!(packages.len(), 1, "{reg}");
    assert_eq!(packages[0]["package"], serde_json::json!("demo"));
    assert_eq!(
        packages[0]["entries"][0]["version"],
        serde_json::json!("0.1.0")
    );
}

#[test]
fn handle_routes_every_static_and_api_route() {
    /// Push one raw HTTP request through the bound server, let `handle`
    /// serve it, and return the raw reply (head + body).
    fn round_trip(
        server: &tiny_http::Server,
        ctx: &Ctx,
        port: u16,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> String {
        use std::fmt::Write as _;
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("read timeout");
        let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
        let _ = write!(head, "Content-Length: {}\r\n", body.len());
        head.push_str("Connection: close\r\n\r\n");
        client.write_all(head.as_bytes()).expect("send head");
        client.write_all(body).expect("send body");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");
        let request = server.incoming_requests().next().expect("request");
        handle(request, ctx);
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).expect("read reply");
        String::from_utf8_lossy(&reply).into_owned()
    }

    let dir = TempDir::new().expect("tempdir");
    let mut ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile": "client"}"#);
    ctx.config.schema_path = Some("absent.yaml".to_string());

    let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
    let port = server.server_addr().to_ip().expect("addr").port();

    for path in [
        "/?x=1",
        "/index.html",
        "/app.js",
        "/app.css",
        "/status/app.js",
        "/status/app.css",
        "/favicon.ico",
        "/ghost",
        "/api/snapshots",
        "/api/registry",
        "/api/artifacts?profile=client",
        "/api/artifact?path=build/client/json/Item.json",
        "/api/schema?profile=client",
        "/api/status?profile=server",
        "/api/tables?profile=server",
        "/api/table?name=Item",
        "/api/diff?from=build&to=build",
    ] {
        let reply = round_trip(&server, &ctx, port, "GET", path, b"");
        let expected = if path == "/favicon.ico" || path == "/ghost" {
            "HTTP/1.1 404"
        } else {
            "HTTP/1.1 2"
        };
        assert!(reply.starts_with(expected), "{path}: {reply}");
    }

    for (method, path, body) in [
        (
            "POST",
            "/api/validate",
            br#"{"tables":{},"enums":{}}"#.as_slice(),
        ),
        ("POST", "/api/build", br#"{"profile":"client"}"#),
        ("POST", "/api/snapshot", br#"{"profile":"client"}"#),
        (
            "POST",
            "/api/upload?name=Item",
            br#"{"Item":[{"id":1,"name":"Shield"}]}"#,
        ),
    ] {
        let reply = round_trip(&server, &ctx, port, method, path, body);
        assert!(reply.contains("HTTP/1.1 200"), "{path}: {reply}");
    }
}

#[test]
fn api_endpoints_cover_profile_defaults_and_contract_errors() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = ctx_in(&dir, Some("schema.yaml"));
    ctx.config.profiles.clear();
    fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").unwrap();
    let (status, value) = api_status(&ctx, "");
    assert_eq!(status, 500);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("no build profile"));

    let ctx = demo_ctx(&dir);
    fs::write(dir.path().join("schema.yaml"), "not [ a schema").unwrap();
    let (status, value) = api_status(&ctx, "profile=client");
    assert_eq!(status, 200);
    assert_eq!(value["validation"]["ok"], serde_json::json!(false));
    assert_eq!(value["validation"]["ok"], serde_json::json!(false));

    for (status, value) in [
        api_tables(&ctx, "profile=client"),
        api_table(&ctx, "name=Item&profile=client"),
        api_upload(&ctx, "name=Item&profile=client", br#"{"Item":[]}"#),
    ] {
        assert!((200..500).contains(&status), "{status}: {value}");
        assert_eq!(value["ok"], serde_json::json!(false));
    }
    let (status, value) = api_artifacts(&ctx, "profile=client");
    assert!((200..500).contains(&status), "{status}: {value}");

    let (status, value) = api_schema(&ctx, "");
    assert_eq!(status, 200);
    assert_eq!(value["ok"], serde_json::json!(false));
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("expected struct Schema"));

    let (status, value) = api_save(&ctx, "{\"schema\":12");
    assert_eq!(status, 200);
    assert_eq!(value["ok"], serde_json::json!(false));

    let valid = serde_json::json!({"schema": {"tables": {}, "enums": {}}});
    let (status, value) = api_save(&ctx, &serde_json::to_string(&valid).unwrap());
    assert_eq!(status, 200);
    assert_eq!(value["ok"], serde_json::json!(true));

    // Directory schema_path with no sources index → 409, and registry
    // schema_path → 409 (both covered by the existing targeted tests,
    // re-exercised here to make the contract explicit).
    let dir2 = TempDir::new().expect("tempdir");
    fs::create_dir(dir2.path().join("schemas")).unwrap();
    let (status, _value) = api_save(&ctx_in(&dir2, Some("schemas")), &good_doc());
    assert_eq!(status, 409);
    let (status, value) = api_save(&ctx_in(&dir2, Some("registry:pkg")), &good_doc());
    assert_eq!(status, 409);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Configuration Registry"));

    assert_eq!(api_build(&ctx, "not json").0, 400);
    assert_eq!(api_build(&ctx, "{}").0, 400);
    assert_eq!(api_build(&ctx, "not json").0, 400);
    assert_eq!(api_build(&ctx, "{}").0, 400);

    // The up-to-date and forced-rebuild branches, on a clean demo root.
    let dir3 = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir3);
    let (_, value) = api_build(&ctx, r#"{"profile":"client"}"#);
    assert_eq!(value["up_to_date"], serde_json::json!(false));
    let (_, value) = api_build(&ctx, r#"{"profile":"client"}"#);
    assert_eq!(value["up_to_date"], serde_json::json!(true));
}

#[test]
fn api_snapshot_reports_no_profile_and_covers_snapshot_errors() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = ctx_in(&dir, None);
    ctx.config.profiles.clear();
    let (status, value) = api_snapshot(&ctx, "{}");
    assert_eq!(status, 400);
    assert!(value["error"].as_str().unwrap().contains("no profile"));

    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile":"client"}"#);
    fs::create_dir_all(dir.path().join("build/snapshot/broken")).unwrap();
    fs::write(
        dir.path().join("build/snapshot/broken/manifest.json"),
        "{broken",
    )
    .unwrap();
    let (_, list) = api_snapshots(&ctx);
    let snaps = list["snapshots"].as_array().unwrap();
    let broken = snaps.iter().find(|s| s["name"] == "broken").unwrap();
    assert_eq!(broken["verified"], serde_json::json!(false));
    assert_eq!(broken["problems"].as_array().unwrap().len(), 1);
}
/// Reader that returns one byte per call, sleeping in between — the
/// drip-feed a cross-origin page would use to pin the serve loop.
struct DripReader {
    left: u32,
    delay: std::time::Duration,
}
impl std::io::Read for DripReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        std::thread::sleep(self.delay);
        self.left -= 1;
        buf[0] = b'x';
        Ok(1)
    }
}
struct TimedOutReader;
impl std::io::Read for TimedOutReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "read timed out",
        ))
    }
}
/// Reader that fails with a plain I/O error — a connection dying
/// mid-body is not a timeout, it is a malformed request.
struct BrokenReader;
impl std::io::Read for BrokenReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"))
    }
}

/// The body cap and the read budget are both enforced
/// (review 2026-10-18 表一#2).
#[test]
fn body_reader_limits_size_and_read_budget() {
    // Under both limits → fine
    let mut r = std::io::Cursor::new(b"{}".to_vec());
    let got = read_body_with(
        &mut r,
        16,
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    );
    assert_eq!(got, Ok(b"{}".to_vec()));

    // Over the byte cap → TooLarge (checked as soon as it overflows)
    let mut r = std::io::Cursor::new(vec![b'x'; 33]);
    assert_eq!(
        read_body_with(
            &mut r,
            16,
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ),
        Err(BodyRead::TooLarge)
    );

    // Drip-feed past the wall-clock budget → TooSlow even under the cap
    let mut r = DripReader {
        left: 64,
        delay: std::time::Duration::from_millis(10),
    };
    assert_eq!(
        read_body_with(
            &mut r,
            1024,
            std::time::Instant::now() + std::time::Duration::from_millis(50)
        ),
        Err(BodyRead::TooSlow)
    );

    // The socket-level tiny_http read timeout also surfaces as the
    // bounded-time response rather than a generic malformed-body error.
    assert_eq!(
        read_body_with(
            &mut TimedOutReader,
            1024,
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ),
        Err(BodyRead::TooSlow)
    );

    // A non-timeout I/O failure is not a timeout: the body is invalid.
    assert_eq!(
        read_body_with(
            &mut BrokenReader,
            1024,
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ),
        Err(BodyRead::Invalid)
    );
}

/// The connection-level timeout reaches the real request body reader:
/// an idle client receives 408 instead of pinning the web handler.
#[test]
fn stalled_body_socket_timeout_returns_408() {
    let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
    let port = server.server_addr().to_ip().expect("addr").port();
    let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("client read timeout");
    write!(
            client,
            "POST /api/validate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1\r\nConnection: close\r\n\r\n"
        )
        .expect("send headers without body");

    // tiny_http reads small request bodies while parsing headers, so its
    // parser itself returns the timeout response before a Request reaches
    // the application handler.
    let mut status = [0u8; 12];
    client
        .read_exact(&mut status)
        .expect("read timeout response");
    assert_eq!(&status, b"HTTP/1.1 408");
}

/// The loopback boundary: foreign Host, missing Host, and cross-origin
/// writes are refused; local Host with or without Origin passes
/// (review 2026-10-18 表一#2). Drives real requests through `handle`
/// over a socket, the same way the unreadable-body test does.
#[test]
fn local_boundary_checks_host_and_origin() {
    fn round_trip(
        server: &tiny_http::Server,
        ctx: &Ctx,
        port: u16,
        method: &str,
        path: &str,
        host: Option<&str>,
        origin: Option<&str>,
    ) -> String {
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("read timeout");
        let mut head = format!("{method} {path} HTTP/1.1\r\n");
        if let Some(h) = host {
            head.push_str("Host: ");
            head.push_str(h);
            head.push_str("\r\n");
        }
        if let Some(o) = origin {
            head.push_str("Origin: ");
            head.push_str(o);
            head.push_str("\r\n");
        }
        head.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
        client.write_all(head.as_bytes()).expect("send head");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");

        let request = server.incoming_requests().next().expect("request");
        handle(request, ctx);

        let mut reply = Vec::new();
        client.read_to_end(&mut reply).expect("read reply");
        String::from_utf8(reply).expect("utf8")
    }

    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, None);
    let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
    let port = server.server_addr().to_ip().expect("addr").port();

    // Local host, any loopback spelling, GET → past the boundary
    // (the schema endpoint answers 500 with no schema configured —
    // anything but 403 means the boundary let it through)
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let out = round_trip(
            &server,
            &ctx,
            port,
            "GET",
            "/api/schema",
            Some(&format!("{host}:{port}")),
            None,
        );
        assert!(!out.contains("403"), "host {host} refused: {out}");
    }

    // Foreign host (DNS rebinding) and missing host (HTTP/1.0) → 403
    assert!(round_trip(
        &server,
        &ctx,
        port,
        "GET",
        "/api/schema",
        Some("evil.example"),
        None
    )
    .contains("403"));
    assert!(round_trip(
        &server,
        &ctx,
        port,
        "GET",
        "/api/schema",
        Some(&format!("evil.example:{port}")),
        None
    )
    .contains("403"));
    assert!(round_trip(&server, &ctx, port, "GET", "/api/schema", None, None).contains("403"));

    // Writes: no Origin (curl) or loopback Origin → past the boundary
    // (empty body fails validation with ok:false, status 200)
    let out = round_trip(
        &server,
        &ctx,
        port,
        "POST",
        "/api/validate",
        Some(&format!("127.0.0.1:{port}")),
        None,
    );
    assert!(!out.contains("403"), "originless POST refused: {out}");
    for host in ["127.0.0.1", "localhost"] {
        let out = round_trip(
            &server,
            &ctx,
            port,
            "POST",
            "/api/validate",
            Some(&format!("{host}:{port}")),
            Some(&format!("http://{host}:{port}")),
        );
        assert!(!out.contains("403"), "local origin {host} refused: {out}");
    }

    // Cross-origin write → 403 (this is the CSRF surface); the
    // sandboxed/null origin is not a loopback origin either
    assert!(round_trip(
        &server,
        &ctx,
        port,
        "POST",
        "/api/validate",
        Some(&format!("127.0.0.1:{port}")),
        Some("https://evil.example")
    )
    .contains("403"));
    assert!(round_trip(
        &server,
        &ctx,
        port,
        "POST",
        "/api/validate",
        Some(&format!("127.0.0.1:{port}")),
        Some("null")
    )
    .contains("403"));
}

#[test]
fn data_cell_covers_every_value_variant() {
    use cage_core::value::{Row, Table, TypedValue, Value};
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        "null".to_string(),
        TypedValue::new(Value::Null, cage_core::value::SourceLocation::default()),
    );
    fields.insert(
        "bool".to_string(),
        TypedValue::new(
            Value::Bool(true),
            cage_core::value::SourceLocation::default(),
        ),
    );
    fields.insert(
        "int".to_string(),
        TypedValue::new(Value::Int(-3), cage_core::value::SourceLocation::default()),
    );
    fields.insert(
        "uint".to_string(),
        TypedValue::new(Value::UInt(7), cage_core::value::SourceLocation::default()),
    );
    fields.insert(
        "float".to_string(),
        TypedValue::new(
            Value::Float(2.5),
            cage_core::value::SourceLocation::default(),
        ),
    );
    fields.insert(
        "str".to_string(),
        TypedValue::new(
            Value::String("hi".to_string()),
            cage_core::value::SourceLocation::default(),
        ),
    );
    fields.insert(
        "bytes".to_string(),
        TypedValue::new(
            Value::Bytes(vec![1, 2, 3]),
            cage_core::value::SourceLocation::default(),
        ),
    );
    fields.insert(
        "arr".to_string(),
        TypedValue::new(
            Value::Array(vec![Value::Int(1)]),
            cage_core::value::SourceLocation::default(),
        ),
    );
    let mut obj = indexmap::IndexMap::new();
    obj.insert("k".to_string(), Value::Bool(false));
    fields.insert(
        "obj".to_string(),
        TypedValue::new(
            Value::Object(obj.clone()),
            cage_core::value::SourceLocation::default(),
        ),
    );
    let row = Row {
        primary_key: vec![Value::UInt(1)],
        fields,
        location: cage_core::value::SourceLocation::default(),
        index: 0,
    };
    let table = Table {
        name: "Item".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: vec![row],
        source_file: "config/item.json".to_string(),
        sheet: None,
    };
    let cells = data_cell(&Value::Array(vec![
        Value::Int(1),
        Value::String("x".to_string()),
    ]));
    assert!(cells.is_array());
    let obj_cell = data_cell(&Value::Object(obj.clone()));
    assert!(obj_cell.is_object());
    assert_eq!(obj_cell["k"], serde_json::json!(false));
    let _ = table; // the row builds the value families
}

#[test]
fn artifact_mime_classifies_extensions() {
    assert!(artifact_mime("a.json").contains("json"));
    assert!(artifact_mime("a.csv").contains("csv"));
    assert!(artifact_mime("a.yaml").contains("yaml"));
    assert!(artifact_mime("a.yml").contains("yaml"));
    assert!(artifact_mime("a.msgpack").contains("octet-stream"));
    assert_eq!(artifact_mime("a.unknownext"), "text/plain; charset=utf-8");
    assert_eq!(artifact_mime("noext"), "text/plain; charset=utf-8");
}

#[test]
fn redirect_response_is_a_301_to_the_location() {
    let resp = redirect_response("/status/");
    let status = resp.status_code();
    assert_eq!(status, 301);
    let headers: Vec<String> = resp
        .headers()
        .iter()
        .filter(|h| h.field.equiv("Location"))
        .map(|h| h.value.as_str().to_string())
        .collect();
    assert!(headers.iter().any(|h| h == "/status/"), "{headers:?}");
}

#[test]
fn api_diff_reports_missing_and_invalid_paths() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    // Missing from/to.
    let (status, value) = api_diff(&ctx, "from=build");
    assert_eq!(status, 400);
    assert!(value["error"]
        .as_str()
        .is_some_and(|e| e.contains("missing from/to")));
    // Absolute path refused.
    let (status, _) = api_diff(&ctx, "from=/etc&to=build");
    assert_eq!(status, 400);
    // Relative path that does not exist.
    let (status, _) = api_diff(&ctx, "from=nope&to=build");
    assert_eq!(status, 400);
    // Exists but is not a manifest dir.
    let (status, _) = api_diff(&ctx, "from=config&to=config");
    assert_eq!(status, 400);
}

#[test]
fn api_upload_rejects_empty_body_and_multi_file_sources() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    // Empty body → 400.
    let (status, value) = api_upload(&ctx, "name=Item", b"");
    assert_eq!(status, 400);
    assert!(value["error"]
        .as_str()
        .is_some_and(|e| e.contains("empty upload body")));
}

#[test]
fn data_cell_hits_every_scalar_arm_directly() {
    use cage_core::value::Value;
    assert_eq!(data_cell(&Value::Null), serde_json::Value::Null);
    assert_eq!(data_cell(&Value::Bool(true)), serde_json::json!(true));
    assert_eq!(data_cell(&Value::Int(-5)), serde_json::json!(-5));
    assert_eq!(data_cell(&Value::UInt(9)), serde_json::json!(9));
    assert_eq!(data_cell(&Value::Float(1.5)), serde_json::json!(1.5));
    assert_eq!(
        data_cell(&Value::String("s".into())),
        serde_json::json!("s")
    );
    assert_eq!(
        data_cell(&Value::Bytes(vec![1, 2])),
        serde_json::json!({"bytes": 2})
    );
}

#[test]
fn panel_endpoints_report_when_no_profile_is_configured() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = ctx_in(&dir, Some("schema.yaml"));
    ctx.config.profiles.clear();

    let (status, v) = api_tables(&ctx, "");
    assert_eq!(status, 500);
    assert!(v["error"].as_str().unwrap().contains("no build profile"));
    let (status, v) = api_table(&ctx, "name=Item");
    assert_eq!(status, 500);
    assert!(v["error"].as_str().unwrap().contains("no build profile"));
    let (status, v) = api_upload(&ctx, "name=Item", b"{}");
    assert_eq!(status, 500);
    assert!(v["error"].as_str().unwrap().contains("no build profile"));
    let (status, v) = api_artifacts(&ctx, "");
    assert_eq!(status, 500);
    assert!(v["error"].as_str().unwrap().contains("no build profile"));
}

#[test]
fn api_upload_reports_rename_and_restore_failures() {
    let dir = TempDir::new().expect("tempdir");
    let _ctx = demo_ctx(&dir); // creates config/item.json; the API isn't exercised here
    let target = dir.path().join("config/item.json");
    fs::read(&target).expect("read original");

    // restore_original with a missing backup → rename fails.
    let err = restore_original(&target, &dir.path().join("nope.bak"));
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("rename failed"));
}

#[test]
fn api_save_covers_incremental_io_and_bad_envelope() {
    let dir = TempDir::new().expect("tempdir");
    // schema_path is a directory (multi-file) with a corrupt index → the
    // save refuses before writing. Force the Io error branch by pointing
    // the multi-file save at a path whose directory cannot be read.
    let ctx = ctx_in(&dir, Some("schemas"));
    fs::create_dir_all(dir.path().join("schemas")).expect("mkdir schemas");
    fs::write(dir.path().join("schemas/a.yaml"), "tables: {}\nenums: {}\n").expect("write");
    let body = serde_json::json!({
        "schema": serde_json::from_str::<serde_json::Value>(&good_doc()).unwrap(),
        "sources": {"files": [], "tables": {}, "enums": {}, "metadata": null},
    });
    // A directory schema_path needs the index; the save refuses with 409
    // because the on-disk file is not in the echoed index (conflict).
    let (status, v) = api_save(&ctx, &body.to_string());
    assert_eq!(status, 409, "save: {v}");
    assert_eq!(v["ok"], serde_json::json!(false));
}

#[test]
fn api_diff_reports_target_manifest_error_and_added_removed_changed() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    api_build(&ctx, r#"{"profile":"client"}"#);
    let build = dir.path().join("build");
    // Build twice so a second manifest exists; then corrupt the target
    // so load_manifest(target) fails.
    let from = build.join("client/csv");
    fs::create_dir_all(&from).expect("mkdir from");
    // from is not a manifest dir → baseline error
    let (status, _) = api_diff(
        &ctx,
        &format!("from={}&to=build/client/json", "build/client/csv"),
    );
    assert_eq!(status, 400);

    // A real baseline but a missing target → target error
    let (status, v) = api_diff(&ctx, "from=build/client/json&to=build/ghost");
    assert_eq!(status, 400);
    assert!(v["error"].as_str().unwrap().contains("target"));
}

#[test]
fn api_registry_reports_a_corrupt_index() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = demo_ctx(&dir);
    fs::create_dir_all(dir.path().join("reg")).expect("mkdir reg");
    fs::create_dir_all(dir.path().join("reg/demo")).expect("mkdir pkg");
    fs::write(dir.path().join("reg/demo/index.json"), "{corrupt").expect("write index");
    ctx.config.registry = Some(cage_core::manifest::RegistryConfig {
        path: "reg".to_string(),
        auth_env: None,
    });
    let (status, v) = api_registry(&ctx);
    assert_eq!(status, 200);
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(v["error"].as_str().unwrap().contains("index"));
}

#[test]
fn api_schema_reports_an_unreadable_index() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, Some("schemas"));
    // schema_path does not exist → load_schema_indexed errors.
    let (status, v) = api_schema(&ctx, "");
    assert_eq!(status, 200);
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()));
}
/// A project whose `Ctx` carries no build profiles: the status and
/// table panels answer 500 naming the gap instead of guessing a
/// profile that does not exist.
#[test]
fn api_status_and_table_refuse_a_profile_free_project() {
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = ctx_in(&dir, None);
    ctx.config.profiles.clear();

    let (status, doc) = api_status(&ctx, "");
    assert_eq!(status, 500);
    assert_eq!(doc["ok"], serde_json::json!(false));
    assert_eq!(
        doc["error"],
        serde_json::json!("no build profile is configured")
    );

    let (status, doc) = api_table(&ctx, "name=Item");
    assert_eq!(status, 500);
    assert_eq!(
        doc["error"],
        serde_json::json!("no build profile is configured")
    );
}

/// A `Ctx` that names a profile the disk cannot back: the panel load
/// fails and the error surfaces as a 400. On a healthy project, a row
/// missing an optional column renders that cell as null instead of
/// dropping the column.
#[test]
fn api_table_reports_a_failed_panel_load_and_null_cells() {
    // Panel load failure: the profile exists in the Ctx only.
    let dir = TempDir::new().expect("tempdir");
    let mut ctx = ctx_in(&dir, Some("schema.yaml"));
    ctx.config.profiles.insert(
        "client".to_string(),
        cage_core::manifest::BuildProfile {
            name: "client".to_string(),
            targets: Vec::new(),
            description: None,
            variables: indexmap::IndexMap::new(),
        },
    );
    let (status, doc) = api_table(&ctx, "name=Item");
    assert_eq!(status, 400, "{doc}");
    assert_eq!(doc["ok"], serde_json::json!(false));
    let err = doc["error"].as_str().expect("load error");
    assert!(err.contains("cage.toml") || !err.is_empty(), "{err}");

    // Null cells: a row without the optional `name` field.
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}, {"id": 2}]}"#,
    )
    .expect("rewrite source");
    let (status, doc) = api_table(&ctx, "name=Item&profile=client");
    assert_eq!(status, 200, "{doc}");
    let rows = doc["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["cells"]["name"], serde_json::json!("Sword"));
    assert!(
        rows[1]["cells"]["name"].is_null(),
        "absent optional renders null: {}",
        rows[1]
    );
}

/// Multi-file save with an unwritable source directory reports Io as
/// a 500; a single-file save with an echoed index that does not list
/// the target skips the conflict check and still saves.
#[test]
fn api_save_reports_io_failures_and_tolerates_unrelated_indexes() {
    // Io: the source directory denies writes (and re-admits them on
    // the way out so the tempdir can be removed).
    let dir = TempDir::new().expect("tempdir");
    multi_file_project(&dir);
    let ctx = ctx_in(&dir, Some("schemas"));
    let schemas = dir.path().join("schemas");
    let writable = fs::metadata(&schemas).expect("stat").permissions();
    let mut perms = writable.clone();
    perms.set_readonly(true);
    fs::set_permissions(&schemas, perms).expect("chmod");

    let (envelope, _, _) = edited_envelope(&dir, |doc| {
        doc["tables"]["Item"]["fields"]["id"]["min"] = serde_json::json!(1);
    });
    let (status, payload) = api_save(&ctx, &envelope);
    fs::set_permissions(&schemas, writable).expect("chmod back");
    assert_eq!(status, 500, "{payload}");
    assert_eq!(payload["ok"], serde_json::json!(false));
    let err = payload["error"].as_str().expect("io error");
    assert!(err.contains("schemas"), "{err}");

    // Unrelated index: the echoed sources list another file, so no
    // conflict applies and the plain save proceeds.
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("schema.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
    )
    .expect("write schema");
    fs::write(
        dir.path().join("other.yaml"),
        "enums: {}\ntables:\n  Shop:\n    name: Shop\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\n",
    )
    .expect("write other");
    let (_, other_index) =
        crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("other.yaml"))
            .expect("other loads");
    let (merged, _) =
        crate::schema_save::load_schema_indexed(dir.path(), &dir.path().join("schema.yaml"))
            .expect("schema loads");
    let mut doc = to_editor_json(&merged);
    doc["tables"]["Item"]["fields"]["id"]["max"] = serde_json::json!(9);
    let envelope = serde_json::json!({
        "schema": doc,
        "sources": other_index.to_json(),
    });
    let ctx = ctx_in(&dir, Some("schema.yaml"));
    let (status, payload) = api_save(
        &ctx,
        &serde_json::to_string(&envelope).expect("envelope serializes"),
    );
    assert_eq!(status, 200, "{payload}");
    assert_eq!(payload["ok"], serde_json::json!(true));
    let saved = fs::read_to_string(dir.path().join("schema.yaml")).expect("saved");
    assert!(saved.contains("max: 9"), "{saved}");
}

/// Uploads whose table's source file cannot be resolved inside the
/// configured roots are refused before anything is touched: a symlink
/// escaping the project, and a file loaded from a root the Ctx does
/// not list.
#[test]
fn api_upload_refuses_targets_that_escape_the_configured_roots() {
    // Symlink escape: the source resolves to a path outside the root.
    let dir = TempDir::new().expect("tempdir");
    let outside = TempDir::new().expect("outside tempdir");
    let outside_file = outside.path().join("elsewhere.json");
    fs::write(&outside_file, r#"{"Item": [{"id": 1}]}"#).expect("write outside");
    let ctx = demo_ctx(&dir);
    fs::remove_file(dir.path().join("config/item.json")).expect("drop original");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_file, dir.path().join("config/item.json"))
        .expect("symlink");
    let (status, doc) = api_upload(&ctx, "name=Item&profile=client", b"[1]");
    assert_eq!(status, 400, "{doc}");
    let err = doc["error"].as_str().expect("escape error");
    assert!(err.contains("outside the project"), "{err}");

    // Root mismatch: the disk config feeds the table from `data/`,
    // but the Ctx only knows `config/` — outside every root it lists.
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("cage.toml"),
        "output_dir = \"build\"\nschema_path = \"schema.yaml\"\n\n[source_roots]\n\
             main = \"config\"\nextra = \"data\"\n\n[project]\nname = \"demo\"\n\n\
             [profiles.client]\nname = \"client\"\n\n[[profiles.client.targets]]\n\
             format = \"json\"\noutput_dir = \"build/client/json\"\n",
    )
    .expect("write config");
    fs::write(
        dir.path().join("schema.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
    )
    .expect("write schema");
    fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
    fs::write(dir.path().join("config/keep.json"), r#"{"Note": []}"#).expect("write keep");
    fs::create_dir_all(dir.path().join("data")).expect("mkdir data");
    fs::write(
        dir.path().join("data/item.json"),
        r#"{"Item": [{"id": 1}]}"#,
    )
    .expect("write data");

    let mut config = ProjectConfig::default();
    config.project.name = "demo".to_string();
    config.schema_path = Some("schema.yaml".to_string());
    config
        .source_roots
        .insert("main".to_string(), "config".to_string());
    let mut profiles = indexmap::IndexMap::new();
    profiles.insert(
        "client".to_string(),
        cage_core::manifest::BuildProfile {
            name: "client".to_string(),
            targets: Vec::new(),
            description: None,
            variables: indexmap::IndexMap::new(),
        },
    );
    config.profiles = profiles;
    let ctx = Ctx {
        root: dir.path().to_path_buf(),
        config,
    };
    let (status, doc) = api_upload(&ctx, "name=Item&profile=client", b"[1]");
    assert_eq!(status, 400, "{doc}");
    let err = doc["error"].as_str().expect("root error");
    assert!(
        err.contains("outside every configured source root"),
        "{err}"
    );
}

/// A snapshot pack whose ledger disagrees with its bytes is listed
/// with `verified: false` and the concrete problems — the page never
/// claims a clean pack it did not just check.
#[test]
fn api_snapshots_reports_a_corrupt_pack() {
    let dir = TempDir::new().expect("tempdir");
    let pack = dir.path().join("build/snapshot/junk");
    fs::create_dir_all(&pack).expect("mkdir pack");
    fs::write(
        pack.join("HASHES.json"),
        r#"{"files": {"extra.txt": "deadbeef"}}"#,
    )
    .expect("write ledger");
    fs::write(pack.join("extra.txt"), b"payload").expect("write payload");

    let ctx = ctx_in(&dir, None);
    let (status, doc) = api_snapshots(&ctx);
    assert_eq!(status, 200, "{doc}");
    assert_eq!(doc["ok"], serde_json::json!(true));
    let packs = doc["snapshots"].as_array().expect("snapshots");
    assert_eq!(packs.len(), 1, "{doc}");
    assert_eq!(packs[0]["name"], serde_json::json!("junk"));
    assert_eq!(packs[0]["verified"], serde_json::json!(false));
    let problems = packs[0]["problems"].as_array().expect("problems");
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().unwrap_or("").contains("hash mismatch")),
        "{problems:?}"
    );
}

/// POST /api/snapshot surfaces build failures by their kind: a broken
/// schema is a validation report (200), an unwritable output
/// directory is an I/O failure (400).
#[test]
fn api_snapshot_reports_validation_and_io_failures() {
    // Validation: a row drops the required primary key field.
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    fs::write(
        dir.path().join("config/item.json"),
        r#"{"Item": [{"name": "x"}]}"#,
    )
    .expect("break source");
    let (status, doc) = api_snapshot(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(status, 200, "{doc}");
    assert_eq!(doc["ok"], serde_json::json!(false));
    assert!(
        doc["diagnostics"]["items"]
            .as_array()
            .expect("diagnostics items")
            .iter()
            .any(|d| d["code"] == serde_json::json!("E1001")),
        "{doc}"
    );

    // Io: the output directory exists as a file.
    let dir = TempDir::new().expect("tempdir");
    let ctx = demo_ctx(&dir);
    fs::write(dir.path().join("build"), b"not a dir").expect("block output");
    let (status, doc) = api_snapshot(&ctx, r#"{"profile": "client"}"#);
    assert_eq!(status, 400, "{doc}");
    assert_eq!(doc["ok"], serde_json::json!(false));
    let err = doc["error"].as_str().expect("io error");
    assert!(!err.is_empty(), "{err}");
}

/// GET /api/diff refuses paths that escape the root, targets without
/// a readable manifest, and folds added/removed artifacts into the
/// affected targets.
#[test]
fn api_diff_refuses_escapes_and_missing_manifests_and_reports_moves() {
    let dir = TempDir::new().expect("tempdir");
    let ctx = ctx_in(&dir, None);

    // Baseline path resolving outside the root.
    let outside = TempDir::new().expect("outside tempdir");
    let outside_file = outside.path().join("elsewhere.json");
    fs::write(&outside_file, b"{}").expect("write outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_file, dir.path().join("bad")).expect("symlink");
    let (status, doc) = api_diff(&ctx, "from=bad&to=build");
    assert_eq!(status, 400, "{doc}");
    let err = doc["error"].as_str().expect("escape error");
    assert!(err.contains("outside project root"), "{err}");

    let manifest_json = |artifacts: serde_json::Value| {
        serde_json::json!({
            "project": "demo",
            "profile": "client",
            "cage_version": "0",
            "schema_hash": "a",
            "source_hash": "b",
            "content_hash": "c",
            "targets": [{"format": "csv", "output_dir": "out", "hash": "h"}],
            "artifacts": artifacts,
        })
    };

    // Target without a manifest: canonicalized, then refused.
    fs::create_dir_all(dir.path().join("from")).expect("mkdir from");
    fs::write(
        dir.path().join("from/manifest.json"),
        manifest_json(serde_json::json!({})).to_string(),
    )
    .expect("write base manifest");
    fs::create_dir_all(dir.path().join("to")).expect("mkdir to");
    let (status, doc) = api_diff(&ctx, "from=from&to=to");
    assert_eq!(status, 400, "{doc}");
    let err = doc["error"].as_str().expect("missing manifest error");
    assert!(err.contains("target 'to'"), "{err}");

    // Added + removed artifacts fold into the touched target.
    let artifact_json = |path: &str| {
        serde_json::json!({
            "path": path,
            "hash": "deadbeef",
            "size": 3,
            "format": "csv",
            "table": "Item",
            "encoding": "utf-8",
        })
    };
    fs::write(
        dir.path().join("from/manifest.json"),
        manifest_json(serde_json::json!({"out/old.csv": artifact_json("out/old.csv")})).to_string(),
    )
    .expect("rewrite base manifest");
    fs::write(
        dir.path().join("to/manifest.json"),
        manifest_json(serde_json::json!({"out/new.json": artifact_json("out/new.json")}))
            .to_string(),
    )
    .expect("write new manifest");
    let (status, doc) = api_diff(&ctx, "from=from&to=to");
    assert_eq!(status, 200, "{doc}");
    assert_eq!(doc["ok"], serde_json::json!(true));
    assert!(
        doc["changed_tables"].as_array().expect("tables").is_empty(),
        "table hashes equal: {doc}"
    );
    let removed = doc["files"]["removed"].as_array().expect("removed");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["path"], serde_json::json!("out/old.csv"));
    let added = doc["files"]["added"].as_array().expect("added");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["path"], serde_json::json!("out/new.json"));
    let affected = doc["affected_targets"].as_array().expect("targets");
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0], serde_json::json!("out"));
}
#[cfg(test)]
impl Ctx {
    /// Drive [`handle`] over a real bound socket: the test acts as the HTTP
    /// client, so the routing, response helpers, and asset endpoints get the
    /// same coverage the smoke script gives `run_web` — inside `cargo test`.
    fn served(self, count: usize) -> (std::thread::JoinHandle<()>, u16) {
        let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("bind ephemeral port");
        let port = server.server_addr().to_ip().expect("bound").port();
        let driver = std::thread::spawn(move || {
            for request in server.incoming_requests().take(count) {
                handle(request, &self);
            }
        });
        (driver, port)
    }
}

#[cfg(test)]
mod handle_tests {
    use super::super::{api_save, api_schema, handle, run_web};
    use super::{ctx_in, good_doc};
    use std::fs;
    use std::io::{Read, Write};
    use tempfile::TempDir;

    fn send(port: u16, request: &str) -> String {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.write_all(request.as_bytes()).expect("write request");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read response");
        String::from_utf8_lossy(&raw).into_owned()
    }

    fn get(port: u16, path: &str) -> String {
        send(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
        )
    }

    fn post(port: u16, path: &str, body: &str) -> String {
        send(
            port,
            &format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    #[test]
    fn handle_routes_the_api_pages_and_assets() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("write schema");
        let (driver, port) = ctx_in(&dir, Some("schema.yaml")).served(8);

        // API: schema document, validate, and save over the wire.
        let raw = get(port, "/api/schema");
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("\"project\":\"demo\""), "{raw}");
        assert!(raw.contains("application/json"), "{raw}");

        let raw = post(port, "/api/validate", &good_doc());
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("\"ok\":true"), "{raw}");
        assert!(raw.contains("\"diagnostics\":[]"), "{raw}");

        let raw = post(port, "/api/schema", &good_doc());
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("\"ok\":true"), "{raw}");
        assert!(
            fs::read_to_string(dir.path().join("schema.yaml"))
                .expect("saved file")
                .contains("tables:"),
            "save endpoint wrote the canonical YAML"
        );

        // Editor page and assets, then the two refusal routes.
        assert!(get(port, "/").contains("text/html"), "index page");
        assert!(
            get(port, "/app.js").contains("text/javascript"),
            "app.js asset"
        );
        assert!(get(port, "/app.css").contains("text/css"), "app.css asset");
        assert!(get(port, "/favicon.ico").starts_with("HTTP/1.1 404"));
        let raw = get(port, "/nope");
        assert!(raw.starts_with("HTTP/1.1 404"), "{raw}");
        assert!(raw.contains("not found"), "{raw}");

        driver.join().expect("driver thread");
    }

    #[test]
    fn run_web_rejects_without_a_project_config() {
        let dir = TempDir::new().expect("tempdir");
        let err = run_web(dir.path(), 1).unwrap_err();
        assert!(!err.contains("cannot bind"), "{err}");
    }

    #[test]
    fn run_web_rejects_when_the_port_is_taken() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("cage.toml"),
            r#"[project]
name = "demo"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#,
        )
        .expect("write config");
        // Hold the port: the bind must fail before the serve loop starts.
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = held.local_addr().expect("addr").port();
        let err = run_web(dir.path(), port).unwrap_err();
        assert!(
            err.contains(&format!("cannot bind 127.0.0.1:{port}:")),
            "{err}"
        );
    }

    #[test]
    fn api_schema_reports_a_missing_schema_file() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, Some("absent.yaml"));
        let (status, doc) = api_schema(&ctx, "");
        assert_eq!(status, 200);
        assert_eq!(doc["ok"], serde_json::json!(false));
        assert!(doc["schema"].is_null(), "{doc}");
    }

    #[test]
    #[cfg(unix)]
    fn api_save_reports_an_unwritable_target() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("write schema");
        // Atomic save-back replaces the file via rename, which needs write
        // permission on the *directory*, not the file — so an unwritable
        // target is modeled at the directory level (the temp file the
        // atomic write needs cannot be created there).
        let file = dir.path().join("schema.yaml");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).expect("chmod");
        let ctx = ctx_in(&dir, Some("schema.yaml"));

        let (status, payload) = api_save(&ctx, &good_doc());
        assert_eq!(status, 500, "{payload}");
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["error"].as_str().unwrap().contains("cannot write"));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmod back");
        drop(file);
    }

    /// POST body that is not valid UTF-8: the body read fails and the
    /// handler must answer 400 instead of wedging or panicking.
    #[test]
    fn handle_reports_an_unreadable_post_body() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, None);
        let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
        let port = server.server_addr().to_ip().expect("addr").port();

        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("read timeout");
        // Invalid UTF-8 bytes: the body read rejects them mid-body.
        let body = [0xFFu8, 0xFE, 0x80, 0x81, 0x82];
        let head = format!(
            "POST /api/validate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        client.write_all(head.as_bytes()).expect("send head");
        client.write_all(&body).expect("send body");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");

        let request = server.incoming_requests().next().expect("request");
        handle(request, &ctx);

        let mut reply = Vec::new();
        client.read_to_end(&mut reply).expect("read reply");
        let out = String::from_utf8(reply).expect("utf8");
        assert!(out.contains("400"), "{out}");
        assert!(out.contains("cannot read request body"), "{out}");
    }
}
