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

mod data;
mod editor;
mod snapshots;
mod status;
