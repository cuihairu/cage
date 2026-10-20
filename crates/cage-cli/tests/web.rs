//! `cage web` process-level coverage (W2): the local HTTP API serves the
//! merged schema as the editor document, validates edited documents
//! (E1701 / E1004), and writes canonical YAML back — multi-file schemas
//! are refused with a 409 instead of being rewritten.

use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

/// A live `cage web` instance for one test (killed on drop).
struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Bind an ephemeral port, release it and hand the number to the server —
/// good enough for a fresh loopback listener.
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    l.local_addr().expect("local addr").port()
}

fn start_server(root: &Path) -> Server {
    let port = free_port();
    let mut server = Server {
        port,
        child: Command::new(env!("CARGO_BIN_EXE_cage"))
            .args([
                "web",
                root.to_str().expect("utf8 path"),
                "--port",
                &port.to_string(),
            ])
            .spawn()
            .expect("spawn cage web"),
    };
    for _ in 0..100 {
        if let Some(st) = server.child.try_wait().expect("try_wait") {
            panic!("cage web exited early with {st} (its stderr went to this test's stderr)");
        }
        // Probe for readiness without panicking: the first attempts race the
        // child's bind, so a refused connection is expected, not a failure.
        if TcpStream::connect(("127.0.0.1", server.port)).is_ok() {
            return server;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = server.child.kill();
    let _ = server.child.wait();
    panic!("cage web did not come up on port {port}");
}

/// One raw HTTP exchange against the loopback server. Reads the response as
/// bytes and decodes chunked transfer encoding — tiny_http switches to
/// chunked for larger bodies (like the embedded editor bundle), and a
/// chunk boundary can land inside a multi-byte UTF-8 sequence.
///
/// Retries on connection-level failures (refused / reset, bounded): under
/// parallel test load many `cage web` instances share the accept backlog and
/// the listener occasionally resets a just-accepted connection. Every request
/// here is idempotent (GET, validate, or canonical single-file save), so a
/// bounded retry turns that flake into a pass without masking a dead server —
/// the retry cap is what still fails if the server is really gone.
fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..4 {
        match request_once(port, method, path, body) {
            Ok(res) => return res,
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                last_err = Some(err);
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => panic!("request failed: {err}"),
        }
    }
    panic!("request kept failing: {}", last_err.unwrap());
}

fn request_once(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    let len = body.map(str::len).unwrap_or(0);
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
    )?;
    if let Some(b) = body {
        stream.write_all(b.as_bytes())?;
    }
    stream.shutdown(Shutdown::Write)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;

    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .expect("status line")
        .parse()
        .expect("numeric status");
    let mut payload = &raw[head_end + 4..];
    let chunked;
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        chunked = decode_chunked(payload);
        payload = &chunked;
    }
    let body = String::from_utf8(payload.to_vec()).expect("utf-8 body");
    Ok((status, body))
}

/// Strip one chunked-encoding framing block (`<hex-size>\r\n<data>\r\n...`,
/// terminated by a zero-size chunk), returning the payload bytes.
fn decode_chunked(raw: &[u8]) -> Vec<u8> {
    let mut rest = raw;
    let mut out = Vec::new();
    loop {
        let nl = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("chunk size line");
        let size = usize::from_str_radix(
            std::str::from_utf8(&rest[..nl]).expect("chunk size is ascii"),
            16,
        )
        .expect("chunk size is hex");
        rest = &rest[nl + 2..];
        if size == 0 {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..]; // per-chunk trailing CRLF
    }
    out
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("valid json response")
}

const WEB_TOML: &str = r#"output_dir = "build"
schema_path = "schema.yaml"

[source_roots]
main = "config"

[project]
name = "web-demo"
version = "0.1.0"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"

[profiles.server]
name = "server"

[[profiles.server.targets]]
format = "csv"
output_dir = "build/csv"
file_template = "{table}.csv"
"#;

const WEB_SCHEMA: &str = r#"tables:
  Item:
    name: Item
    description: An inventory item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
enums: {}
"#;

const WEB_DATA: &str = r#"{"Item": [{"id": 1, "name": "Sword"}]}"#;

/// Single-file schema project: schema.yaml + one data source.
fn write_project(root: &Path) {
    fs::write(root.join("cage.toml"), WEB_TOML).unwrap();
    fs::write(root.join("schema.yaml"), WEB_SCHEMA).unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/item.json"), WEB_DATA).unwrap();
}

/// Directory-schema project: the editor must refuse to rewrite it.
const DIR_TOML: &str = r#"schema_path = "schemas"

[project]
name = "dir-schema"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#;

const DIR_SCHEMA_FRAGMENT: &str = r#"tables:
  Monster:
    name: Monster
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
"#;

fn write_dir_project(root: &Path) {
    fs::write(root.join("cage.toml"), DIR_TOML).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(root.join("schemas/monster.yaml"), DIR_SCHEMA_FRAGMENT).unwrap();
}

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

    // Assets are embedded and served with the right media types.
    let (status, body) = request(server.port, "GET", "/app.js", None);
    assert_eq!(status, 200, "editor module must be served");
    assert!(
        body.contains("/api/schema"),
        "module must call the API: {body}"
    );
    assert!(body.contains("cage"), "module content sanity: {body}");
    // Planner interface P1: rule form editing rides this editor, and the
    // immediate re-validation toggle lives in the top bar.
    assert!(
        body.contains("auto-validate"),
        "the top bar must offer immediate re-validation: {body}"
    );
    assert!(
        body.contains("scheduleAutoValidate"),
        "module must re-validate on edit: {body}"
    );
    // Planner interface P2: the field template library lives in the module
    // (static dictionary, insertion goes through the normal save/validate chain).
    assert!(
        body.contains("FIELD_TEMPLATES"),
        "module must carry the template library: {body}"
    );
    assert!(
        body.contains("外键引用"),
        "the template library must cover the foreign-key case: {body}"
    );

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
