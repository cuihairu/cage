//! Process-level coverage of the HTTP API source (S1, design §45):
//! an http(s) URL in `[source_roots]` is fetched, materialized under
//! `.cage-cache/source/<cache_key>/`, and built through the standard
//! pipeline — the body follows the same shape rules as a local JSON
//! file. Remote data changes rotate source_hash/build_id (the change is
//! visible in the manifest), identical bytes rebuild to identical
//! manifest bytes, and failures surface with their codes: unreachable /
//! 404 → E1901, 401 → E1902, malformed body → the same E0001 a local
//! file would report.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
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

const DATA_V1: &str = r#"{ "Item": [ { "id": 1, "name": "Sword" } ] }"#;
const DATA_V2: &str = r#"{ "Item": [ { "id": 1, "name": "Shield" } ] }"#;

/// Static-file HTTP server over std::net — the remote API stand-in.
/// Serves GET /<path> from `dir` (404 for anything missing); the special
/// `/secret.json` answers 401 so the auth-rejection path is testable
/// without a real identity provider.
struct TestServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

fn start_server(dir: &Path) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let dir = dir.to_path_buf();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop2.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    loop {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                head.extend_from_slice(&buf[..n]);
                                if head.windows(4).any(|w| w == b"\r\n\r\n")
                                    || head.len() > 64 * 1024
                                {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let req = String::from_utf8_lossy(&head).into_owned();
                    let path = req.split_whitespace().nth(1).unwrap_or("/");
                    serve(&mut stream, &dir, path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    TestServer {
        port,
        stop,
        handle: Some(handle),
    }
}

fn respond(stream: &mut TcpStream, status: u16, reason: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn serve(stream: &mut TcpStream, dir: &Path, path: &str) {
    if path == "/secret.json" {
        respond(stream, 401, "Unauthorized", b"{\"error\":\"forbidden\"}");
        return;
    }
    let stripped = path.strip_prefix('/').unwrap_or(path);
    let file = dir.join(stripped);
    if path.contains("..") || !file.is_file() {
        respond(stream, 404, "Not Found", b"not found");
        return;
    }
    match fs::read(&file) {
        Ok(bytes) => respond(stream, 200, "OK", &bytes),
        Err(_) => respond(stream, 404, "Not Found", b"not found"),
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// A port with nothing listening on it (bound, then released).
fn dead_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

/// Consumer fixture with one remote source root and a local schema.
fn write_consumer(root: &Path, name: &str, url: &str) {
    let body = r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "URL_PLACEHOLDER"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
    .replace("URL_PLACEHOLDER", url);
    write(&root.join(format!("{name}/cage.toml")), &body);
    write(&root.join(format!("{name}/schema.yaml")), SCHEMA);
}

#[test]
fn remote_source_fetch_build_and_rotate() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let srv_dir = root.join("srv");
    write(&srv_dir.join("data.json"), DATA_V1);
    let server = start_server(&srv_dir);
    let url = format!("http://127.0.0.1:{}/data.json", server.port);

    let proj = root.join("proj").to_str().unwrap().to_string();
    write_consumer(root, "proj", &url);

    let out = run_cage(&["check", &proj]);
    assert_code(&out, 0, "remote source check");
    assert!(stdout(&out).contains("OK (1 tables"), "{}", stdout(&out));

    let out = run_cage(&["build", &proj, "--profile", "client"]);
    assert_code(&out, 0, "remote source build");
    let artifact = fs::read_to_string(root.join("proj/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Sword"),
        "remote data must land: {artifact}"
    );

    // The response bytes are materialized project-locally, keyed by the
    // URL (same cache_key scheme as the registry cache).
    let key = cage_core::remote::cache_key(&url);
    let cached = root
        .join("proj/.cage-cache/source")
        .join(&key)
        .join(format!("{key}.json"));
    assert!(cached.is_file(), "cache file must exist");
    assert_eq!(
        fs::read_to_string(&cached).unwrap(),
        DATA_V1,
        "cache holds the response bytes verbatim"
    );

    // Determinism: identical remote bytes rebuild to identical manifest
    // bytes (the golden contract covers remote sources too).
    let m1 = fs::read(root.join("proj/build/manifest.json")).unwrap();
    let out = run_cage(&["build", &proj, "--profile", "client"]);
    assert_code(&out, 0, "identical rebuild");
    let m2 = fs::read(root.join("proj/build/manifest.json")).unwrap();
    assert_eq!(m1, m2, "same remote bytes → same manifest bytes");

    // Rotation: the remote data changes, the next build refetches and
    // the change is visible in the manifest — no silent data swap.
    write(&srv_dir.join("data.json"), DATA_V2);
    let out = run_cage(&["build", &proj, "--profile", "client"]);
    assert_code(&out, 0, "build after remote change");
    let artifact = fs::read_to_string(root.join("proj/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Shield"),
        "new data must land: {artifact}"
    );
    let m1: serde_json::Value = serde_json::from_slice(&m1).unwrap();
    let m3: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("proj/build/manifest.json")).unwrap()).unwrap();
    assert_ne!(
        m1["source_hash"], m3["source_hash"],
        "remote data change must rotate source_hash"
    );
    assert_ne!(
        m1["build_id"], m3["build_id"],
        "remote data change must rotate build_id"
    );
}

#[test]
fn remote_source_failures_report_their_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let srv_dir = root.join("srv");
    write(&srv_dir.join("data.json"), DATA_V1);
    let server = start_server(&srv_dir);
    let base = format!("http://127.0.0.1:{}", server.port);

    // 401 → E1902
    let proj = root.join("auth").to_str().unwrap().to_string();
    write_consumer(root, "auth", &format!("{base}/secret.json"));
    let out = run_cage(&["check", &proj]);
    assert_code(&out, 2, "auth rejected");
    assert!(stderr(&out).contains("E1902"), "{}", stderr(&out));

    // 404 → E1901
    let proj = root.join("missing").to_str().unwrap().to_string();
    write_consumer(root, "missing", &format!("{base}/absent.json"));
    let out = run_cage(&["check", &proj]);
    assert_code(&out, 2, "missing remote source");
    assert!(stderr(&out).contains("E1901"), "{}", stderr(&out));

    // Unreachable root → E1901 after bounded retries
    let proj = root.join("dead").to_str().unwrap().to_string();
    write_consumer(
        root,
        "dead",
        &format!("http://127.0.0.1:{}/x.json", dead_port()),
    );
    let out = run_cage(&["check", &proj]);
    assert_code(&out, 2, "unreachable remote source");
    assert!(stderr(&out).contains("E1901"), "{}", stderr(&out));

    // Malformed body → the same E0001 a local file reports, nothing loads
    write(&srv_dir.join("broken.json"), "{ not json");
    let proj = root.join("broken").to_str().unwrap().to_string();
    write_consumer(root, "broken", &format!("{base}/broken.json"));
    let out = run_cage(&["check", &proj]);
    assert_code(&out, 2, "malformed remote body");
    assert!(stderr(&out).contains("E0001"), "{}", stderr(&out));
    assert!(stderr(&out).contains("failed to parse"), "{}", stderr(&out));
}

/// S4 offline semantics, end to end: once a first build has materialized
/// the remote bytes, killing the server no longer fails the build — the
/// cached copy is served behind an E1906 WARNING — while `--no-cache`
/// turns the same scenario back into the hard E1901.
#[test]
fn offline_build_falls_back_to_the_cache_and_no_cache_refuses() {
    let tmp = tempfile::tempdir().unwrap();
    let srv_dir = tmp.path().join("srv");
    fs::create_dir_all(&srv_dir).unwrap();
    write(&srv_dir.join("data.json"), DATA_V1);
    let server = start_server(&srv_dir);
    let base = format!("http://127.0.0.1:{}", server.port);

    let proj = tmp.path().join("consumer").to_str().unwrap().to_string();
    write_consumer(tmp.path(), "consumer", &format!("{base}/data.json"));

    // First build: online, materializes the cache copy.
    let out = run_cage(&["build", &proj]);
    assert_code(&out, 0, "online build");
    drop(server);

    // Offline rebuild: served from the cache, WARNING on stderr.
    let out = run_cage(&["build", &proj]);
    assert_code(&out, 0, "offline build falls back to the cache");
    assert!(
        stderr(&out).contains("E1906"),
        "expected the E1906 fallback warning\nstderr:\n{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("warning:"),
        "fallback must be a warning, not silent\nstderr:\n{}",
        stderr(&out)
    );

    // check agrees (the fallback lives in load_project, shared).
    let out = run_cage(&["check", &proj]);
    assert_code(&out, 0, "offline check falls back too");

    // --no-cache refuses the fallback: hard E1901, exit 2.
    let out = run_cage(&["build", &proj, "--no-cache"]);
    assert_code(&out, 2, "no-cache build refuses the fallback");
    assert!(
        stderr(&out).contains("E1901"),
        "expected E1901\nstderr:\n{}",
        stderr(&out)
    );
    let out = run_cage(&["check", &proj, "--no-cache"]);
    assert_code(&out, 2, "no-cache check refuses the fallback");
    assert!(stderr(&out).contains("E1901"), "{}", stderr(&out));
}

/// RFC 8288 pagination, end to end: a source whose first response
/// carries `Link: rel="next"` walks both pages, merges the row arrays,
/// builds through the standard pipeline (bare array → `Data` table),
/// and caches the merged document — identical server state rebuilds to
/// identical manifest bytes.
#[test]
fn paginated_remote_source_builds_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop2.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    loop {
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                head.extend_from_slice(&buf[..n]);
                                if head.windows(4).any(|w| w == b"\r\n\r\n")
                                    || head.len() > 64 * 1024
                                {
                                    break;
                                }
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&head).into_owned();
                    let path = req.split_whitespace().nth(1).unwrap_or("/");
                    let (link, body): (&str, &[u8]) = if path == "/items.json" {
                        (
                            "</items.json?page=2>; rel=\"next\"",
                            br#"[{ "id": 1, "name": "Sword" }]"#,
                        )
                    } else if path == "/items.json?page=2" {
                        ("", br#"[{ "id": 2, "name": "Shield" }]"#)
                    } else {
                        ("", b"not found")
                    };
                    let mut head_out =
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n".to_string();
                    if !link.is_empty() {
                        head_out.push_str(&format!("Link: {link}\r\n"));
                    }
                    head_out.push_str(&format!(
                        "Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    ));
                    let _ = stream.write_all(head_out.as_bytes());
                    let _ = stream.write_all(body);
                    let _ = stream.flush();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    struct Stop {
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Stop {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
    let _guard = Stop {
        stop,
        handle: Some(handle),
    };
    let url = format!("http://127.0.0.1:{port}/items.json");

    let schema = r#"tables:
  Data:
    name: Data
    description: Rows served across pages
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
    let body = r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "paged"
version = "0.1.0"

[source_roots]
main = "URL_PLACEHOLDER"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
    .replace("URL_PLACEHOLDER", &url);
    write(&root.join("paged/cage.toml"), &body);
    write(&root.join("paged/schema.yaml"), schema);

    let proj = root.join("paged").to_str().unwrap().to_string();
    let out = run_cage(&["build", &proj, "--profile", "client"]);
    assert_code(&out, 0, "paginated build");
    let artifact = fs::read_to_string(root.join("paged/build/client/json/Data.json")).unwrap();
    assert!(
        artifact.contains("Sword") && artifact.contains("Shield"),
        "rows from both pages must land: {artifact}"
    );

    // The cache holds the merged array, not page 1 alone.
    let key = cage_core::remote::cache_key(&url);
    let cached = fs::read_to_string(
        root.join("paged/.cage-cache/source")
            .join(&key)
            .join(format!("{key}.json")),
    )
    .unwrap();
    let merged: serde_json::Value = serde_json::from_str(&cached).unwrap();
    assert_eq!(
        merged.as_array().unwrap().len(),
        2,
        "merged cache: {cached}"
    );

    // Determinism: same server state → same manifest bytes.
    let m1 = fs::read(root.join("paged/build/manifest.json")).unwrap();
    let out = run_cage(&["build", &proj, "--profile", "client"]);
    assert_code(&out, 0, "identical rebuild");
    assert_eq!(
        m1,
        fs::read(root.join("paged/build/manifest.json")).unwrap(),
        "same pages → same merged bytes → same manifest"
    );
}
