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
        "{method} {path} HTTP/1.1\r\nHost: cage-web-test\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
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
