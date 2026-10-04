//! Process-level coverage of the remote Configuration Registry (R3,
//! read-only): `[registry] path = "http://127.0.0.1:<port>"` resolves
//! `registry:<pkg>[@<ver>]` over HTTP — source roots AND schema_path —
//! through the project-local cache, keeping every R2 pin rule; downloads
//! are hash-gated (tamper → E1803), dead/unknown roots are E1802, and
//! publish/list stay local-only (usage refusals). A warm cache that
//! re-verifies clean resolves with the server already down (offline reuse).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
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

/// One Item row per version, so a consumer's built artifact proves which
/// entry version it resolved.
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

fn write_publisher(root: &Path) {
    write(
        &root.join("pub/cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "common"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"

[[profiles.client.targets]]
format = "csv"
output_dir = "build/client/csv"
file_template = "{table}.csv"
"#,
    );
    write(&root.join("pub/schema.yaml"), SCHEMA);
    write(&root.join("pub/config/item.json"), &item_rows("Sword"));
}

/// Consumer fixture pointing `[registry].path` at the remote root URL.
fn write_consumer_remote(
    root: &Path,
    name: &str,
    url: &str,
    source_root: &str,
    deps: Option<&str>,
    schema_path: &str,
) {
    let deps_block = deps
        .map(|d| format!("\n[dependencies]\ncommon = \"{d}\"\n"))
        .unwrap_or_default();
    let body = r#"output_dir = "build"
schema_path = "SCHEMA_PATH"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "REG_PLACEHOLDER"

[registry]
path = "URL_PLACEHOLDER"
DEPS_LINE
[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
    .replace("REG_PLACEHOLDER", source_root)
    .replace("SCHEMA_PATH", schema_path)
    .replace("URL_PLACEHOLDER", url)
    .replace("DEPS_LINE\n", &deps_block);
    write(&root.join(format!("{name}/cage.toml")), &body);
    if !schema_path.starts_with("registry:") {
        write(&root.join(format!("{name}/schema.yaml")), SCHEMA);
    }
}

/// Static-file HTTP server over std::net — the remote registry stand-in.
/// Serves GET /<path> from `dir` (404 for anything else or traversal
/// attempts); the stop flag lets a test kill it mid-suite to prove offline
/// cache reuse.
struct TestServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

fn start_registry_server(dir: &Path) -> TestServer {
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
                                if head.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                                if head.len() > 64 * 1024 {
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

fn serve(stream: &mut TcpStream, dir: &Path, path: &str) {
    let (status, body) = match resolve_path(dir, path) {
        Some(file) => match fs::read(&file) {
            Ok(bytes) => (200, bytes),
            Err(_) => (404, b"not found".to_vec()),
        },
        None => (404, b"not found".to_vec()),
    };
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/octet-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

fn resolve_path(dir: &Path, path: &str) -> Option<PathBuf> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path.contains("..") {
        return None;
    }
    let file = dir.join(path);
    file.is_file().then_some(file)
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
    l.local_addr().unwrap().port()
}

fn publish_versions(root: &Path, reg: &Path) {
    for (v, marker) in [("1.0.0", "Sword"), ("1.9.0", "Shield"), ("2.0.0", "Lance")] {
        write(&root.join("pub/config/item.json"), &item_rows(marker));
        let out = run_cage(&[
            "registry",
            "publish",
            root.join("pub").to_str().unwrap(),
            "--registry",
            reg.to_str().unwrap(),
            "--version",
            v,
        ]);
        assert_code(&out, 0, v);
    }
}

#[test]
fn remote_resolve_build_pin_and_offline_reuse() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg");
    publish_versions(root, &reg);

    let server = start_registry_server(&reg);
    let url = format!("http://127.0.0.1:{}", server.port);

    // Bare `registry:common` + `[dependencies]` pin + entry schema_path:
    // everything resolves through the remote root — the pin picks 1.9.0,
    // the schema comes from the entry (the consumer ships no schema file).
    write_consumer_remote(
        root,
        "depcon",
        &url,
        "registry:common",
        Some(">=1.0, <2.0"),
        "registry:common",
    );
    let depcon = root.join("depcon").to_str().unwrap().to_string();
    assert!(!root.join("depcon/schema.yaml").exists());
    let out = run_cage(&["check", &depcon]);
    assert_code(&out, 0, "remote pinned check");
    assert!(stdout(&out).contains("OK (1 tables"), "{}", stdout(&out));
    let out = run_cage(&["build", &depcon, "--profile", "client"]);
    assert_code(&out, 0, "remote pinned build");
    let artifact = fs::read_to_string(root.join("depcon/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Shield"),
        "remote pin must resolve 1.9.0, not the 2.0.0 latest: {artifact}"
    );

    // The cache is materialized project-locally, keyed by the URL, and
    // self-verifying.
    let cached = root
        .join("depcon/.cage-cache/registry")
        .join(cage_core::registry::cache_key(&url));
    assert!(cached.join("common/1.9.0/HASHES.json").is_file());
    assert!(cached
        .join("common/1.9.0/data/client/json/Item.json")
        .is_file());

    // Explicit @1.0.0 through the remote root too.
    write_consumer_remote(
        root,
        "pinnedcon",
        &url,
        "registry:common@1.0.0",
        None,
        "schema.yaml",
    );
    let pinnedcon = root.join("pinnedcon").to_str().unwrap().to_string();
    let out = run_cage(&["build", &pinnedcon, "--profile", "client"]);
    assert_code(&out, 0, "remote explicit pin build");
    let artifact = fs::read_to_string(root.join("pinnedcon/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Sword"),
        "explicit pin must stay on 1.0.0: {artifact}"
    );

    // Offline reuse: kill the server, warm caches must keep resolving.
    drop(server);
    let out = run_cage(&["check", &depcon]);
    assert_code(&out, 0, "offline reuse check");
    let out = run_cage(&["build", &depcon, "--profile", "client"]);
    assert_code(&out, 0, "offline reuse build");
    let out = run_cage(&["check", &pinnedcon]);
    assert_code(&out, 0, "offline reuse check (pinned)");
}

#[test]
fn remote_errors_and_readonly_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg");
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        reg.to_str().unwrap(),
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "initial publish");

    let server = start_registry_server(&reg);
    let url = format!("http://127.0.0.1:{}", server.port);

    // Unknown package (404 on index.json) → E1802.
    write_consumer_remote(root, "ghost", &url, "registry:ghost", None, "schema.yaml");
    let out = run_cage(&["check", root.join("ghost").to_str().unwrap()]);
    assert_code(&out, 2, "unknown package");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));

    // Unknown version → E1802.
    write_consumer_remote(
        root,
        "badver",
        &url,
        "registry:common@9.9.9",
        None,
        "schema.yaml",
    );
    let out = run_cage(&["check", root.join("badver").to_str().unwrap()]);
    assert_code(&out, 2, "unknown version");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));

    // Unsatisfiable pin → E1802 with the published list.
    write_consumer_remote(
        root,
        "impossible",
        &url,
        "registry:common",
        Some(">=3.0"),
        "schema.yaml",
    );
    let out = run_cage(&["check", root.join("impossible").to_str().unwrap()]);
    assert_code(&out, 2, "unsatisfiable pin");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("satisfies requirement"),
        "{}",
        stderr(&out)
    );

    // publish/list against a remote root are read-only refusals (exit 2).
    let out = run_cage(&["registry", "publish", &pub_root, "--registry", &url]);
    assert_code(&out, 2, "remote publish refusal");
    assert!(stderr(&out).contains("read-only"), "{}", stderr(&out));
    let out = run_cage(&["registry", "list", "--registry", &url]);
    assert_code(&out, 2, "remote list refusal");
    assert!(stderr(&out).contains("read-only"), "{}", stderr(&out));

    // The R4 administration commands (verify/gc/remove) refuse remote roots
    // the same way — the read-only protocol serves resolution only.
    for (args, what) in [
        (
            vec!["registry", "verify", "--registry", url.as_str()],
            "remote verify refusal",
        ),
        (
            vec!["registry", "gc", "--registry", url.as_str()],
            "remote gc refusal",
        ),
        (
            vec![
                "registry",
                "remove",
                "common",
                "1.0.0",
                "--registry",
                url.as_str(),
            ],
            "remote remove refusal",
        ),
    ] {
        let out = run_cage(&args);
        assert_code(&out, 2, what);
        assert!(
            stderr(&out).contains("read-only"),
            "{what}: {}",
            stderr(&out)
        );
    }

    // Tampered server-side bytes → E1803: the download hash gate rejects
    // them before they can enter the cache.
    fs::write(
        reg.join("common/1.0.0/data/client/json/Item.json"),
        r#"[{"id": 1, "name": "Forged"}]"#,
    )
    .unwrap();
    write_consumer_remote(
        root,
        "tampered",
        &url,
        "registry:common@1.0.0",
        None,
        "schema.yaml",
    );
    let out = run_cage(&["check", root.join("tampered").to_str().unwrap()]);
    assert_code(&out, 2, "tampered remote entry");
    assert!(stderr(&out).contains("E1803"), "{}", stderr(&out));

    // Dead root → E1802 transport failure after bounded retries.
    let dead_url = format!("http://127.0.0.1:{}", dead_port());
    write_consumer_remote(
        root,
        "dead",
        &dead_url,
        "registry:common",
        None,
        "schema.yaml",
    );
    let out = run_cage(&["check", root.join("dead").to_str().unwrap()]);
    assert_code(&out, 2, "dead root");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(stderr(&out).contains("cannot reach"), "{}", stderr(&out));
}
