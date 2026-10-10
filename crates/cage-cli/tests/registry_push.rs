//! Process-level coverage of the registry push path (A3, design §47):
//! `cage registry push` uploads a published entry from the project's local
//! registry to a remote http(s) root — one PUT per entry file, the package
//! index last — and the pushed entry then resolves for a consumer exactly
//! like a hosted one (R3 read path over the pushed bytes). 401 → E2102,
//! 405 → E2104, a missing credential fails as E2105 before any network
//! contact, and `--dry-run` uploads nothing.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

fn run_cage_env(args: &[&str], var: &str, value: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .env(var, value)
        .output()
        .unwrap()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_code(out: &std::process::Output, want: i32, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(want),
        "{what}: expected exit {want}\nstdout:\n{}\nstderr:\n{}",
        stdout(out),
        stderr(out)
    );
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn item_rows(marker: &str) -> String {
    format!(
        r#"{{
  "Item": [
    {{ "id": 1, "name": "{marker}" }}
  ]
}}
"#
    )
}

const SCHEMA: &str = r#"tables:
  Item:
    name: Item
    description: An inventory item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
        required: true
      name:
        name: name
        type: { kind: String }
        required: true
enums: {}
"#;

/// Publisher whose cage.toml declares `[registry] path = "reg"` — the local
/// registry is both the publish target and the push source.
fn write_push_publisher(root: &Path) {
    write(
        &root.join("pub/cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "common"
version = "0.1.0"

[registry]
path = "reg"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#,
    );
    write(&root.join("pub/schema.yaml"), SCHEMA);
    write(&root.join("pub/config/item.json"), &item_rows("Sword"));
}

/// Consumer resolving straight off the remote root (R3 read path).
fn write_remote_consumer(root: &Path, name: &str, url: &str) {
    write(
        &root.join(format!("{name}/cage.toml")),
        &r#"output_dir = "build"
schema_path = "registry:common"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "registry:common@0.1.0"

[registry]
path = "URL_PLACEHOLDER"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
        .replace("URL_PLACEHOLDER", url),
    );
}

/// Served bytes keyed by request path.
type Store = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;
/// Every request, logged as "METHOD path auth".
type RequestLog = Arc<Mutex<Vec<String>>>;

/// Remote registry stand-in: GETs serve from the store, PUTs store (or are
/// forced to a fixed refusal status), every request is logged as
/// "METHOD path auth".
struct PushServer {
    url: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

fn start_push_server(force_put_status: Option<u16>) -> (PushServer, Store, RequestLog) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let store: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let store2 = store.clone();
    let requests2 = requests.clone();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop2.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break buf.len(),
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break pos + 4;
                        }
                        if buf.len() > 1024 * 1024 {
                            break buf.len();
                        }
                    }
                    Err(_) => break buf.len(),
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end.min(buf.len())]).into_owned();
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap_or("");
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("/").to_string();
            let mut content_length = 0usize;
            let mut auth = String::new();
            for line in lines {
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    auth = line["authorization:".len()..].trim().to_string();
                }
            }
            let mut body = buf[head_end.min(buf.len())..].to_vec();
            while body.len() < content_length {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                    Err(_) => break,
                }
            }
            requests2
                .lock()
                .unwrap()
                .push(format!("{method} {path} {auth}"));
            let (status, reason, payload): (u16, &str, Vec<u8>) =
                match (force_put_status, method.as_str()) {
                    (_, "GET") => match store2.lock().unwrap().get(&path) {
                        Some(bytes) => (200, "OK", bytes.clone()),
                        None => (404, "Not Found", b"not found".to_vec()),
                    },
                    (Some(code), "PUT") => (code, "Refused", b"refused".to_vec()),
                    (None, "PUT") => {
                        store2.lock().unwrap().insert(path.clone(), body);
                        (200, "OK", Vec::new())
                    }
                    _ => (405, "Method Not Allowed", b"method".to_vec()),
                };
            let head = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&payload);
            let _ = stream.flush();
        }
    });
    (
        PushServer {
            url,
            stop,
            handle: Some(handle),
        },
        store,
        requests,
    )
}

fn shutdown(mut server: PushServer) {
    server.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    server.handle.take().map(thread::JoinHandle::join);
}

fn put_count(requests: &[String]) -> usize {
    requests.iter().filter(|r| r.starts_with("PUT ")).count()
}

#[test]
fn registry_push_roundtrips_to_a_resolving_remote_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_push_publisher(root);
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    // Publish through the project's own [registry].path — the push source.
    let out = run_cage(&["registry", "publish", &pub_root]);
    assert_code(&out, 0, "publish into local reg");

    let (server, store, requests) = start_push_server(None);

    // Push with a bearer token; the anonymous state probe precedes it.
    let out = run_cage_env(
        &[
            "registry",
            "push",
            &pub_root,
            "common@0.1.0",
            "--registry",
            &server.url,
            "--auth-env",
            "CAGE_TOK_CLI",
        ],
        "CAGE_TOK_CLI",
        "tok-cli",
    );
    assert_code(&out, 0, "push");
    assert!(
        stdout(&out).contains("pushed common/0.1.0"),
        "{}",
        stdout(&out)
    );
    let log = requests.lock().unwrap().clone();
    assert_eq!(log[0], "GET /common/index.json ");
    assert!(
        log.iter()
            .filter(|r| r.starts_with("PUT "))
            .all(|r| r.ends_with("Bearer tok-cli")),
        "{log:?}"
    );
    assert!(put_count(&log) >= 4, "entry files + index: {log:?}");
    assert!(store.lock().unwrap().contains_key("/common/index.json"));

    // 远端根 resolve 复现: a consumer resolves and builds straight off the
    // pushed bytes — entry data and entry schema both.
    write_remote_consumer(root, "consumer", &server.url);
    let consumer = root.join("consumer").to_str().unwrap().to_string();
    let out = run_cage(&["build", &consumer, "--profile", "client"]);
    assert_code(&out, 0, "consumer builds off the pushed remote root");

    // Re-pushing identical bytes is a zero-PUT no-op.
    let before = requests.lock().unwrap().len();
    let out = run_cage_env(
        &[
            "registry",
            "push",
            &pub_root,
            "common@0.1.0",
            "--registry",
            &server.url,
            "--auth-env",
            "CAGE_TOK_CLI",
        ],
        "CAGE_TOK_CLI",
        "tok-cli",
    );
    assert_code(&out, 0, "re-push identical");
    assert!(
        stdout(&out).contains("identical, no-op"),
        "{}",
        stdout(&out)
    );
    assert_eq!(put_count(&requests.lock().unwrap()[before..]), 0);

    // --dry-run: full local read + remote state check, zero PUTs.
    let before = requests.lock().unwrap().len();
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "--registry",
        &server.url,
        "--dry-run",
    ]);
    assert_code(&out, 0, "dry-run push");
    assert!(stdout(&out).contains("would push"), "{}", stdout(&out));
    assert_eq!(put_count(&requests.lock().unwrap()[before..]), 0);

    // auth_env unset → E2105 before any network contact (request count
    // unchanged).
    let before = requests.lock().unwrap().len();
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "--registry",
        &server.url,
        "--auth-env",
        "CAGE_TOK_CLI_MISSING",
    ]);
    assert_code(&out, 1, "missing credential");
    assert!(stderr(&out).contains("E2105"), "{}", stderr(&out));
    assert_eq!(requests.lock().unwrap().len(), before);

    shutdown(server);
}

#[test]
fn registry_push_maps_refusals_to_e2102_e2104() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_push_publisher(root);
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    let out = run_cage(&["registry", "publish", &pub_root]);
    assert_code(&out, 0, "publish into local reg");

    // 401 on the writes → E2102.
    let (server, _store, requests) = start_push_server(Some(401));
    let out = run_cage_env(
        &[
            "registry",
            "push",
            &pub_root,
            "--registry",
            &server.url,
            "--auth-env",
            "CAGE_TOK_CLI2",
        ],
        "CAGE_TOK_CLI2",
        "tok-2",
    );
    assert_code(&out, 1, "401 push");
    assert!(stderr(&out).contains("E2102"), "{}", stderr(&out));
    assert!(!stderr(&out).contains("tok-2"), "{}", stderr(&out));
    assert_eq!(put_count(&requests.lock().unwrap()), 1, "fail fast");
    shutdown(server);

    // 405 on the writes → E2104 (no write channel).
    let (server, _store, _requests) = start_push_server(Some(405));
    let out = run_cage(&["registry", "push", &pub_root, "--registry", &server.url]);
    assert_code(&out, 1, "405 push");
    assert!(stderr(&out).contains("E2104"), "{}", stderr(&out));
    shutdown(server);

    // A local path is not a push target — that's publish's job.
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "--registry",
        "./somewhere-local",
    ]);
    assert_code(&out, 2, "local push target refused");
}

/// Every file under `dir`, as `/`-separated paths relative to it, sorted —
/// the shape of a published entry's file list.
fn entry_rels(entry_dir: &Path) -> Vec<String> {
    let mut rels = Vec::new();
    let mut stack = vec![entry_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                rels.push(
                    p.strip_prefix(entry_dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    rels.sort();
    rels
}

#[test]
fn registry_push_presign_map_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_push_publisher(root);
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    let out = run_cage(&["registry", "publish", &pub_root]);
    assert_code(&out, 0, "publish into local reg");

    let (server, store, requests) = start_push_server(None);

    // The presign map: one PUT URL per entry file plus the index pair —
    // here the URLs just point at the stand-in server's own layout.
    let entry_dir = root.join("pub/reg/common/0.1.0");
    let rels = entry_rels(&entry_dir);
    assert!(rels.len() >= 4, "{rels:?}");
    let mut uploads = serde_json::Map::new();
    for rel in &rels {
        uploads.insert(
            format!("common/0.1.0/{rel}"),
            serde_json::Value::String(format!("{}/common/0.1.0/{rel}", server.url)),
        );
    }
    let map = serde_json::json!({
        "uploads": uploads,
        "index": {
            "get": format!("{}/common/index.json", server.url),
            "put": format!("{}/common/index.json", server.url),
        }
    });
    let map_path = root.join("presign.json");
    write(&map_path, &serde_json::to_string(&map).unwrap());
    let map_arg = map_path.to_str().unwrap();

    // A map that misses one entry file fails closed before any PUT —
    // tried first, against the fresh remote, while the push would still
    // have real work to do (identical no-op returns before coverage).
    let mut short = map.clone();
    let dropped = format!("common/0.1.0/{}", rels[0]);
    short["uploads"].as_object_mut().unwrap().remove(&dropped);
    let short_path = root.join("presign-short.json");
    write(&short_path, &serde_json::to_string(&short).unwrap());
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "common@0.1.0",
        "--presign-map",
        short_path.to_str().unwrap(),
    ]);
    assert_code(&out, 1, "coverage gap");
    assert!(
        stderr(&out).contains("E2101") && stderr(&out).contains("nothing uploaded"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        put_count(&requests.lock().unwrap()),
        0,
        "zero PUTs on the gap"
    );

    // Push through the map — no --registry, no --auth-env anywhere.
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "common@0.1.0",
        "--presign-map",
        map_arg,
    ]);
    assert_code(&out, 0, "presigned push");
    assert!(
        stdout(&out).contains("pushed common/0.1.0 → presigned targets")
            && stdout(&out).contains("presign.json"),
        "{}",
        stdout(&out)
    );
    // The URLs are the grant: no request carries an Authorization header.
    let log = requests.lock().unwrap().clone();
    assert_eq!(log[0], "GET /common/index.json ");
    assert!(
        log.iter().all(|r| !r.contains("Bearer")),
        "presigned requests stay anonymous: {log:?}"
    );
    assert_eq!(put_count(&log), rels.len() + 1, "files + merged index");
    let index: serde_json::Value = serde_json::from_slice(
        &store
            .lock()
            .unwrap()
            .get("/common/index.json")
            .cloned()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(index["entries"][0]["version"], "0.1.0");

    // Re-pushing identical bytes: the same zero-PUT no-op as direct.
    let before = requests.lock().unwrap().len();
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "common@0.1.0",
        "--presign-map",
        map_arg,
    ]);
    assert_code(&out, 0, "presigned re-push");
    assert!(
        stdout(&out).contains("identical, no-op"),
        "{}",
        stdout(&out)
    );
    assert_eq!(put_count(&requests.lock().unwrap()[before..]), 0);

    // Dry run through the map: zero PUTs, "would push".
    let before = requests.lock().unwrap().len();
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "--presign-map",
        map_arg,
        "--dry-run",
    ]);
    assert_code(&out, 0, "presigned dry-run");
    assert!(stdout(&out).contains("would push"), "{}", stdout(&out));
    assert_eq!(put_count(&requests.lock().unwrap()[before..]), 0);

    // --presign-map and --auth-env are mutually exclusive (clap, exit 2):
    // the URLs are the only credential on this route.
    let out = run_cage_env(
        &[
            "registry",
            "push",
            &pub_root,
            "--presign-map",
            map_arg,
            "--auth-env",
            "CAGE_TOK_CLI",
        ],
        "CAGE_TOK_CLI",
        "tok-cli",
    );
    assert_code(&out, 2, "presign-map + auth-env refused");
    // And likewise with --registry.
    let out = run_cage(&[
        "registry",
        "push",
        &pub_root,
        "--presign-map",
        map_arg,
        "--registry",
        &server.url,
    ]);
    assert_code(&out, 2, "presign-map + registry refused");

    // Neither destination given → usage error.
    let out = run_cage(&["registry", "push", &pub_root]);
    assert_code(&out, 2, "no destination");
    assert!(
        stderr(&out).contains("needs a destination"),
        "{}",
        stderr(&out)
    );

    shutdown(server);
}
