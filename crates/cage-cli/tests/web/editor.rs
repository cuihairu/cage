//! Editor document / projection / validate / save coverage (split out of web.rs, LAY-029).

use super::*;

#[test]
fn web_serves_editor_document() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    let (status, body) = request(server.port, "GET", "/api/schema", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true);
    assert_eq!(v["project"], "web-demo");
    assert_eq!(v["schema_path"], "schema.yaml");
    assert_eq!(v["profile_names"], serde_json::json!(["client", "server"]));
    // The editor document: canonical serde shape, Map-adjacent keys intact.
    assert_eq!(
        v["schema"]["tables"]["Item"]["fields"]["id"]["type"]["kind"],
        "Int32"
    );
    assert_eq!(
        v["schema"]["tables"]["Item"]["primary_key"],
        serde_json::json!(["id"])
    );

    // Unknown routes are 404 JSON.
    let (status, body) = request(server.port, "GET", "/api/nope", None);
    assert_eq!(status, 404);
    assert_eq!(json(&body)["ok"], false);
}

#[test]
fn web_schema_projection_gates_by_profile() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    // A server-only field on top of the base fixture — the exact shape
    // `filter_by_profile` gates (examples/game-config carries the same
    // pattern as its `internal_note` field).
    fs::write(
        tmp.path().join("schema.yaml"),
        r#"tables:
  Item:
    name: Item
    description: An inventory item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
      internal_note: { name: internal_note, type: { kind: String }, targets: [server] }
enums: {}
"#,
    )
    .unwrap();
    let server = start_server(tmp.path());

    // No profile → the full document, no projection.
    let (status, body) = request(server.port, "GET", "/api/schema", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert!(v.get("projection").is_none(), "no profile → no projection");

    // client: the projection drops the server-only field, the full
    // document keeps it (hidden stays visibly hidden).
    let (status, body) = request(server.port, "GET", "/api/schema?profile=client", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["projection"]["profile"], "client");
    assert_eq!(
        v["schema"]["tables"]["Item"]["fields"]["internal_note"]["name"], "internal_note",
        "the full document keeps the field"
    );
    assert!(
        v["projection"]["schema"]["tables"]["Item"]["fields"]
            .get("internal_note")
            .is_none(),
        "the client projection drops it"
    );

    // server: the projection keeps the field.
    let (status, body) = request(server.port, "GET", "/api/schema?profile=server", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(
        v["projection"]["schema"]["tables"]["Item"]["fields"]["internal_note"]["targets"],
        serde_json::json!(["server"])
    );

    // Unknown profile → 400 JSON.
    let (status, body) = request(server.port, "GET", "/api/schema?profile=ghost", None);
    assert_eq!(status, 400);
    assert_eq!(json(&body)["ok"], false);
}

#[test]
fn web_serves_editor_page_and_assets() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    // GET / serves the embedded editor page (W3) — the single-page app the
    // browser loads at the server root, same origin as the API (no CORS).
    let (status, body) = request(server.port, "GET", "/", None);
    assert_eq!(status, 200, "editor page must serve at the root");
    assert!(body.contains("Schema 编辑器"), "body: {body}");
    assert_eq!(status, 200);
    assert!(
        body.contains(r#"src="./app.js""#),
        "the page must load its module: {body}"
    );
    assert!(
        body.contains(r#"href="./app.css""#),
        "the page must load its stylesheet: {body}"
    );

    // Assets are embedded and served with the right media types. The entry
    // is an ES module that imports its split parts (LAY-027), so each part
    // must be served at its own path for the browser to load the graph.
    let (status, body) = request(server.port, "GET", "/app.js", None);
    assert_eq!(status, 200, "editor entry module must be served");
    assert!(
        body.contains("/api/schema"),
        "entry must call the API: {body}"
    );
    // Entry wires the split parts together.
    assert!(body.contains("import"), "entry must be a module: {body}");
    assert!(
        body.contains("./forms.js"),
        "entry must import the form renderer: {body}"
    );
    // Planner interface P1: the immediate re-validation toggle lives in the
    // api module (dirty-flag + 700ms debounce).
    let (status, body) = request(server.port, "GET", "/api.js", None);
    assert_eq!(status, 200, "api module must be served");
    assert!(
        body.contains("auto-validate"),
        "the top bar must offer immediate re-validation: {body}"
    );
    assert!(
        body.contains("scheduleAutoValidate"),
        "api module must re-validate on edit: {body}"
    );
    // Planner interface P2: the field template library lives in the forms
    // module (static dictionary, insertion goes through save/validate chain).
    let (status, body) = request(server.port, "GET", "/forms.js", None);
    assert_eq!(status, 200, "forms module must be served");
    assert!(
        body.contains("FIELD_TEMPLATES"),
        "forms module must carry the template library: {body}"
    );
    assert!(
        body.contains("外键引用"),
        "the template library must cover the foreign-key case: {body}"
    );
    // The remaining split parts serve too (constants/state/dom/types).
    for part in ["constants.js", "state.js", "dom.js", "types.js"] {
        let (status, _) = request(server.port, "GET", &format!("/{part}"), None);
        assert_eq!(status, 200, "module {part} must be served");
    }

    let (status, body) = request(server.port, "GET", "/app.css", None);
    assert_eq!(status, 200, "editor stylesheet must be served");
    assert!(body.contains("--accent"), "stylesheet sanity: {body}");

    // Unknown static routes stay 404.
    let (status, _) = request(server.port, "GET", "/nope.js", None);
    assert_eq!(status, 404);
}

#[test]
fn web_validate_reports_e1701_and_e1004() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    let (_, schema_doc) = request(server.port, "GET", "/api/schema", None);
    let doc = json(&schema_doc);
    let clean = serde_json::to_string(&doc["schema"]).expect("schema doc string");

    // Unedited document validates cleanly.
    let (status, body) = request(server.port, "POST", "/api/validate", Some(&clean));
    assert_eq!(status, 200);
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    assert_eq!(v["diagnostics"].as_array().unwrap().len(), 0);

    // Unknown FieldType kind → E1701 with the JSON path as the field.
    let bad = clean.replace("Int32", "Quadruple");
    let (_, body) = request(server.port, "POST", "/api/validate", Some(&bad));
    let v = json(&body);
    assert_eq!(v["ok"], false, "body: {body}");
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "E1701");
    assert_eq!(d["field"], "tables.Item.fields.id.type.kind");

    // Dangling primary key → the L1 E1004 family, not an editor error.
    let broken = clean.replace(r#"["id"]"#, r#"["ghost"]"#);
    let (_, body) = request(server.port, "POST", "/api/validate", Some(&broken));
    let v = json(&body);
    assert_eq!(v["ok"], false, "body: {body}");
    assert!(
        v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "E1004" && d["message"].as_str().unwrap().contains("ghost")),
        "diagnostics: {v}"
    );
}

#[test]
fn web_save_writes_canonical_yaml_that_builds_and_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let before = fs::read_to_string(tmp.path().join("schema.yaml")).unwrap();
    let server = start_server(tmp.path());

    // Edit the document: add an optional rank field, drop a description.
    let (_, schema_doc) = request(server.port, "GET", "/api/schema", None);
    let mut doc = json(&schema_doc);
    doc["schema"]["tables"]["Item"]["fields"]["rank"] =
        serde_json::json!({"name": "rank", "type": {"kind": "UInt8"}});
    let edited = serde_json::to_string(&doc["schema"]).expect("edited doc string");

    // Save → canonical YAML written to the configured single schema file.
    let (status, body) = request(server.port, "POST", "/api/schema", Some(&edited));
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    assert!(v["save_target"].as_str().unwrap().ends_with("schema.yaml"));
    assert!(v["bytes"].as_u64().unwrap() > 0);

    let saved = fs::read_to_string(tmp.path().join("schema.yaml")).unwrap();
    assert_ne!(saved, before, "the file must have been rewritten");

    // The saved file is canonical YAML: parses back and re-renders byte-identically.
    let parsed = cage_core::edit::from_canonical_yaml(&saved).expect("canonical yaml parses");
    assert_eq!(
        cage_core::edit::to_canonical_yaml(&parsed),
        saved,
        "save-back must be a fixed point"
    );
    assert!(parsed
        .get_table("Item")
        .unwrap()
        .fields
        .contains_key("rank"));
    assert!(parsed.get_table("Item").unwrap().fields.contains_key("id"));

    // The saved schema is what the real build path reads: check passes.
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // And GET /api/schema now serves the saved document (reloaded per request).
    let (_, body) = request(server.port, "GET", "/api/schema", None);
    assert_eq!(
        json(&body)["schema"]["tables"]["Item"]["fields"]["rank"]["type"]["kind"],
        "UInt8"
    );
}

#[test]
fn web_save_refuses_directory_schema_path() {
    let tmp = tempfile::tempdir().unwrap();
    write_dir_project(tmp.path());
    let server = start_server(tmp.path());
    let original_fragment = fs::read_to_string(tmp.path().join("schemas/monster.yaml")).unwrap();

    // Minimal valid editor document — but targeted at a multi-file project.
    let doc = serde_json::json!({
        "tables": {"A": {"name": "A", "primary_key": ["id"],
                          "fields": {"id": {"name": "id", "type": {"kind": "Int32"}}}}},
        "enums": {}
    });
    let body = serde_json::to_string(&doc).unwrap();
    let (status, resp) = request(server.port, "POST", "/api/schema", Some(&body));
    assert_eq!(status, 409, "body: {resp}");
    assert!(
        json(&resp)["error"].as_str().unwrap().contains("directory"),
        "response: {resp}"
    );

    // The directory project is untouched.
    assert_eq!(
        fs::read_to_string(tmp.path().join("schemas/monster.yaml")).unwrap(),
        original_fragment
    );
    // ... and so is the served document (still the merged directory schema).
    let (_, resp) = request(server.port, "GET", "/api/schema", None);
    assert!(json(&resp)["schema"]["tables"].get("Monster").is_some());
}

/// Registry-schema project (R2): the schema is owned by a published entry,
/// so the editor must refuse to save over it.
const REG_TOML: &str = r#"schema_path = "registry:common"

[project]
name = "reg-schema"

[source_roots]
main = "registry:common"

[registry]
path = "../reg"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#;

#[test]
fn web_save_refuses_registry_schema_path() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("cage.toml"), REG_TOML).unwrap();
    let server = start_server(tmp.path());

    // GET resolves the schema through the registry — here the registry has
    // no such package, and the E1802 surfaces as an error payload.
    let (status, resp) = request(server.port, "GET", "/api/schema", None);
    assert_eq!(status, 200, "body: {resp}");
    assert_eq!(json(&resp)["ok"], false, "body: {resp}");
    assert!(
        json(&resp)["error"].as_str().unwrap().contains("E1802"),
        "response: {resp}"
    );

    // POST is refused outright: an entry schema is published, not edited.
    let doc = serde_json::json!({
        "tables": {"A": {"name": "A", "primary_key": ["id"],
                          "fields": {"id": {"name": "id", "type": {"kind": "Int32"}}}}},
        "enums": {}
    });
    let body = serde_json::to_string(&doc).unwrap();
    let (status, resp) = request(server.port, "POST", "/api/schema", Some(&body));
    assert_eq!(status, 409, "body: {resp}");
    let v = json(&resp);
    assert_eq!(v["ok"], false, "body: {resp}");
    assert_eq!(v["save_target"], "registry:common", "body: {resp}");
    assert!(
        v["error"].as_str().unwrap().contains("published"),
        "response: {resp}"
    );
    // And the refusal wrote nothing into the project.
    assert!(!tmp.path().join("schema.yaml").exists());
}

#[test]
fn web_serves_empty_schema_when_config_names_none() {
    let tmp = tempfile::tempdir().unwrap();
    // No schema_path at all: the editor starts on an empty schema and the
    // save lands in schema.yaml with a wiring note.
    fs::write(
        tmp.path().join("cage.toml"),
        r#"[project]
name = "blank"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#,
    )
    .unwrap();
    let server = start_server(tmp.path());

    let (_, body) = request(server.port, "GET", "/api/schema", None);
    let v = json(&body);
    assert_eq!(v["ok"], true);
    assert!(v["schema"]
        .get("tables")
        .unwrap()
        .as_object()
        .unwrap()
        .is_empty());

    let doc = serde_json::json!({
        "tables": {"T": {"name": "T", "primary_key": ["id"], "fields": {}}},
        "enums": {}
    });
    let body = serde_json::to_string(&doc).unwrap();
    let (status, resp) = request(server.port, "POST", "/api/schema", Some(&body));
    assert_eq!(status, 200, "body: {resp}");
    let v = json(&resp);
    assert_eq!(v["ok"], true);
    assert!(
        v["note"].as_str().unwrap().contains("schema_path"),
        "resp: {resp}"
    );
    assert!(
        tmp.path().join("schema.yaml").is_file(),
        "default save target must be schema.yaml"
    );
}
