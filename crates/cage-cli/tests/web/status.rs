//! Status page and freshness coverage (split out of web.rs, LAY-029).

use super::*;

// ── U1：项目状态页与状态/构建 API ────────────────────────────────────

#[test]
fn web_serves_status_page_with_redirect_and_assets() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    // Bare /status redirects so ./app.js resolves under /status/ (not into
    // the editor's /app.js).
    let (status, _) = request(server.port, "GET", "/status", None);
    assert_eq!(status, 301, "bare /status must redirect to /status/");

    // The page and its assets are embedded and served with the right types.
    let (status, body) = request(server.port, "GET", "/status/", None);
    assert_eq!(status, 200, "status page must serve at /status/");
    assert!(body.contains("项目状态"), "body: {body}");
    assert!(
        body.contains(r#"src="./app.js""#),
        "the page must load its module: {body}"
    );

    let (status, body) = request(server.port, "GET", "/status/app.js", None);
    assert_eq!(status, 200, "status module must be served");
    assert!(body.contains("/api/status"), "module must call the API");
    assert!(body.contains("/api/build"), "module must trigger builds");
    // Planner interface P1 export: one click fetches every artifact of a
    // target through the hash-verified artifact endpoint (no new endpoint).
    assert!(
        body.contains("/api/artifact"),
        "the export button must ride the verified artifact endpoint: {body}"
    );
    // Planner interface P2: the diagnostics panel marks what a second
    // validation run added or removed (product diff stays with /api/diff).
    assert!(
        body.contains("markDiagDiff"),
        "module must diff two validation runs: {body}"
    );
    assert!(
        body.contains("diag-new"),
        "module must mark new diagnostics: {body}"
    );

    let (status, body) = request(server.port, "GET", "/status/app.css", None);
    assert_eq!(status, 200, "status stylesheet must be served");
    assert!(body.contains("--accent"), "stylesheet sanity: {body}");
}

#[test]
fn web_status_and_build_report_freshness_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    // Initial status: validation runs on the real project (the fixture data
    // matches the fixture schema), nothing has been built.
    let (status, body) = request(server.port, "GET", "/api/status", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true);
    assert_eq!(v["project"], "web-demo");
    assert_eq!(
        v["profile"], "client",
        "first configured profile is default"
    );
    assert_eq!(v["profile_names"], serde_json::json!(["client", "server"]));
    assert_eq!(v["validation"]["ok"], true, "body: {body}");
    assert_eq!(v["validation"]["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(v["builds"]["client"]["fresh"], false);
    assert_eq!(
        v["builds"]["client"]["last_build_profile"],
        serde_json::Value::Null
    );

    // Build the client profile through the same pipeline `cage build` runs.
    let (status, body) = request(
        server.port,
        "POST",
        "/api/build",
        Some(r#"{"profile": "client"}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    assert_eq!(v["up_to_date"], false);
    assert!(v["artifact_count"].as_u64().unwrap() >= 1, "body: {body}");
    assert!(tmp.path().join("build/json/Item.json").is_file());

    // The build rotated freshness: client fresh, server still not built,
    // and an unknown profile is a 400 with the available list.
    let (_, body) = request(server.port, "GET", "/api/status?profile=client", None);
    let v = json(&body);
    assert_eq!(v["builds"]["client"]["fresh"], true, "body: {body}");
    assert_eq!(
        v["builds"]["client"]["changed_tables"],
        serde_json::json!([])
    );
    assert!(v["builds"]["client"]["build_id"].as_str().unwrap().len() >= 12);
    assert_eq!(v["builds"]["server"]["fresh"], false);
    assert_eq!(v["builds"]["server"]["last_build_profile"], "client");

    let (status, body) = request(server.port, "GET", "/api/status?profile=ghost", None);
    assert_eq!(status, 400);
    assert!(json(&body)["error"].as_str().unwrap().contains("available"));

    // A second build is a no-op, not a rebuild.
    let (_, body) = request(
        server.port,
        "POST",
        "/api/build",
        Some(r#"{"profile": "client"}"#),
    );
    assert_eq!(json(&body)["up_to_date"], true, "body: {body}");
}
