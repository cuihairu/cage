//! Snapshot / diff / registry coverage (split out of web.rs, LAY-029).

use super::*;

#[test]
fn web_snapshots_diff_and_registry_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let server = start_server(tmp.path());

    // Unconfigured registry: ok with an empty list and a note.
    let (status, body) = request(server.port, "GET", "/api/registry", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true);
    assert_eq!(v["packages"], serde_json::json!([]));
    assert!(v["note"].as_str().unwrap().contains("registry"));

    // No snapshots yet.
    let (status, body) = request(server.port, "GET", "/api/snapshots", None);
    assert_eq!(status, 200, "body: {body}");
    assert_eq!(json(&body)["snapshots"], serde_json::json!([]));

    // Pack the first snapshot (the `cage snapshot` pipeline: full build +
    // self-verify — a broken build would come back as diagnostics).
    let (status, body) = request(
        server.port,
        "POST",
        "/api/snapshot",
        Some(r#"{"profile": "client"}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    let snap1 = v["snapshot_dir"].as_str().unwrap().to_string();
    assert!(snap1.starts_with("build/snapshot/client-"), "body: {body}");

    // The listing shows it, freshly verified.
    let (status, body) = request(server.port, "GET", "/api/snapshots", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(v["snapshots"][0]["verified"], true);
    assert_eq!(v["snapshots"][0]["profile"], "client");

    // Unknown profile refuses; broken body refuses.
    let (status, body) = request(
        server.port,
        "POST",
        "/api/snapshot",
        Some(r#"{"profile": "ghost"}"#),
    );
    assert_eq!(status, 400, "body: {body}");
    assert_eq!(json(&body)["ok"], false);
    let (status, body) = request(server.port, "POST", "/api/snapshot", Some("not json"));
    assert_eq!(status, 400, "body: {body}");

    // Identical inputs (determinism) → the layered diff is empty and the
    // roots agree.
    let q = format!("from=build&to={snap1}");
    let (status, body) = request(server.port, "GET", &format!("/api/diff?{q}"), None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true);
    assert_eq!(v["changed_tables"], serde_json::json!([]));
    assert_eq!(v["files"]["changed"], serde_json::json!([]));
    assert_eq!(v["roots"]["build_id"]["from"], v["roots"]["build_id"]["to"]);

    // Edit the source, pack again → the diff names the table, the target
    // and the file.
    fs::write(
        tmp.path().join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
    )
    .unwrap();
    let (status, body) = request(
        server.port,
        "POST",
        "/api/snapshot",
        Some(r#"{"profile": "client"}"#),
    );
    assert_eq!(status, 200, "body: {body}");
    let snap2 = json(&body)["snapshot_dir"].as_str().unwrap().to_string();
    assert_ne!(snap1, snap2);

    let q = format!("from={snap1}&to={snap2}");
    let (status, body) = request(server.port, "GET", &format!("/api/diff?{q}"), None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(
        v["changed_tables"],
        serde_json::json!(["Item"]),
        "body: {body}"
    );
    assert_eq!(
        v["affected_targets"],
        serde_json::json!(["build/json"]),
        "body: {body}"
    );
    let changed = v["files"]["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "body: {body}");
    assert_eq!(changed[0]["path"], "build/json/Item.json");
    assert_eq!(changed[0]["table"], "Item");
    assert_eq!(changed[0]["target"], "build/json");
    assert_ne!(
        v["roots"]["content_hash"]["from"],
        v["roots"]["content_hash"]["to"]
    );

    // Missing / unknown diff inputs refuse with 400 JSON.
    let (status, body) = request(server.port, "GET", "/api/diff?from=build", None);
    assert_eq!(status, 400);
    assert_eq!(json(&body)["ok"], false);
    let (status, body) = request(
        server.port,
        "GET",
        "/api/diff?from=ghost/manifest.json&to=build",
        None,
    );
    assert_eq!(status, 400);
    assert!(
        json(&body)["error"].as_str().unwrap().contains("baseline"),
        "body: {body}"
    );
}
