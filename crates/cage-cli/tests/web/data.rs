//! Data panel / upload / build / artifact coverage (split out of web.rs, LAY-029).

use super::*;

/// Planner data panel (P0): the table inventory and the paginated row
/// view run against the same load chain the CLI uses — the panel cannot
/// show a table `cage check` does not see, and a hidden/misspelled table
/// is a 404, not an empty page.
#[test]
fn web_tables_and_table_rows_serve_the_data_panel() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    // Five rows so a pagination window has something to window.
    let rows: Vec<String> = (0..5)
        .map(|i| format!(r#"{{"id": {i}, "name": "row{i}"}}"#))
        .collect();
    fs::write(
        tmp.path().join("config/item.json"),
        format!("{{\"Item\": [{}]}}", rows.join(", ")),
    )
    .unwrap();
    let server = start_server(tmp.path());

    let (status, body) = request(server.port, "GET", "/api/tables", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true);
    assert_eq!(v["profile"], "client");
    let tables = v["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0]["name"], "Item");
    assert_eq!(tables[0]["rows"], 5);
    assert_eq!(tables[0]["primary_key"], serde_json::json!(["id"]));

    // Unknown profile: same 400 contract as /api/status.
    let (status, _) = request(server.port, "GET", "/api/tables?profile=ghost", None);
    assert_eq!(status, 400);

    let (status, body) = request(server.port, "GET", "/api/table?name=Item", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["total"], 5);
    assert_eq!(v["columns"], serde_json::json!(["id", "name"]));
    assert_eq!(v["rows"].as_array().unwrap().len(), 5);

    // A window keeps absolute row indexes; a ghost table is a 404 and a
    // missing name a 400.
    let (status, body) = request(
        server.port,
        "GET",
        "/api/table?name=Item&offset=2&limit=2",
        None,
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["index"], 2);
    assert_eq!(rows[0]["cells"]["name"], "row2");

    let (status, body) = request(server.port, "GET", "/api/table?name=Ghost", None);
    assert_eq!(status, 404, "body: {body}");
    assert!(json(&body)["error"].as_str().unwrap().contains("Ghost"));

    let (status, _) = request(server.port, "GET", "/api/table", None);
    assert_eq!(status, 400);

    // The status page module knows about the panel (the anchor link is
    // the planner path from diagnostic to data).
    let (status, body) = request(server.port, "GET", "/status/app.js", None);
    assert_eq!(status, 200);
    assert!(body.contains("/api/table"), "status page must call the API");
}

#[test]
fn web_upload_replaces_table_source_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::write(
        tmp.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
    )
    .unwrap();
    let server = start_server(tmp.path());
    let source = tmp.path().join("config/item.json");
    let original = fs::read(&source).unwrap();

    // Missing name / ghost table: the same 400/404 contract as the read
    // side, and nothing on disk moves.
    let (status, body) = request(server.port, "POST", "/api/upload", Some("{}"));
    assert_eq!(status, 400, "body: {body}");
    let (status, body) = request(server.port, "POST", "/api/upload?name=Ghost", Some("{}"));
    assert_eq!(status, 404, "body: {body}");
    assert_eq!(original, fs::read(&source).unwrap());

    // A valid replacement lands atomically and the panel sees it through
    // the same load chain.
    let good = r#"{"Item": [{"id": 2, "name": "Shield"}, {"id": 3, "name": "Bow"}]}"#;
    let (status, body) = request(server.port, "POST", "/api/upload?name=Item", Some(good));
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    let (_, body) = request(server.port, "GET", "/api/table?name=Item", None);
    assert_eq!(json(&body)["total"], 2);
    assert_eq!(json(&body)["rows"][1]["cells"]["name"], "Bow");

    // Invalid data (id is Int32): refused with diagnostics, the source
    // is byte-identical to what the accepted upload left, and no
    // temp/backup litter stays behind.
    let accepted = fs::read(&source).unwrap();
    let bad = r#"{"Item": [{"id": "not-an-int", "name": "x"}]}"#;
    let (status, body) = request(server.port, "POST", "/api/upload?name=Item", Some(bad));
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], false, "body: {body}");
    assert!(!v["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(accepted, fs::read(&source).unwrap());
    let litter: Vec<String> = fs::read_dir(tmp.path().join("config"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("upload"))
        .collect();
    assert!(litter.is_empty(), "leftover swap files: {litter:?}");

    // Unparseable bytes: refused on the post-swap load, original restored.
    let (status, body) = request(
        server.port,
        "POST",
        "/api/upload?name=Item",
        Some("<binary>"),
    );
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(json(&body)["ok"], false);
    assert_eq!(accepted, fs::read(&source).unwrap());
}

#[test]
fn web_build_refuses_broken_data_with_diagnostics() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    // Row without the required id: the build must refuse with diagnostics,
    // same shape /api/validate speaks, and nothing lands in the build dir.
    fs::write(
        tmp.path().join("config/item.json"),
        r#"{"Item": [{"name": "Sword"}]}"#,
    )
    .unwrap();
    let server = start_server(tmp.path());

    let (status, body) = request(
        server.port,
        "POST",
        "/api/build",
        Some(r#"{"profile": "client"}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], false, "body: {body}");
    assert!(
        v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["severity"] == "error"),
        "body: {body}"
    );
    assert!(!tmp.path().join("build/client/json/Item.json").exists());
}

#[test]
fn web_artifacts_serve_ledgered_files_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    // Build, then list artifacts for the built profile.
    let (status, body) = request(
        server.port,
        "POST",
        "/api/build",
        Some(r#"{"profile": "client"}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(json(&body)["ok"], true);

    let (status, body) = request(server.port, "GET", "/api/artifacts?profile=client", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["manifest_profile"], "client");
    assert_eq!(v["fresh"], true);
    assert_eq!(v["stale"], false);
    let arts = v["artifacts"].as_array().expect("artifact list");
    assert!(!arts.is_empty(), "body: {body}");
    let item = arts
        .iter()
        .find(|a| a["path"] == "build/json/Item.json")
        .expect("Item.json must be ledgered");
    assert!(item["hash"].as_str().unwrap().len() >= 12, "body: {body}");

    // A ledgered file serves byte-identical to disk.
    let (status, body) = request(
        server.port,
        "GET",
        "/api/artifact?path=build/json/Item.json",
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(
        body,
        fs::read_to_string(tmp.path().join("build/json/Item.json")).unwrap()
    );

    // Outside the ledger: 404, never join-and-pray. Edited after the
    // build: 409, never tampered content.
    let (status, body) = request(server.port, "GET", "/api/artifact?path=schema.yaml", None);
    assert_eq!(status, 404);
    assert!(
        json(&body)["error"]
            .as_str()
            .unwrap()
            .contains("not in the build manifest"),
        "body: {body}"
    );
    fs::write(tmp.path().join("build/json/Item.json"), r#"{"hacked": 1}"#).unwrap();
    let (status, body) = request(
        server.port,
        "GET",
        "/api/artifact?path=build/json/Item.json",
        None,
    );
    assert_eq!(status, 409, "body: {body}");
    assert!(
        json(&body)["error"]
            .as_str()
            .unwrap()
            .contains("no longer matches"),
        "body: {body}"
    );
}
