//! `cage lsp` — a diagnostics-only Language Server over stdio (design §49
//! I 系列 IDE 立项). Hand-rolled JSON-RPC 2.0 with LSP framing: the whole
//! wire is "headers until a blank line, then a `Content-Length` body" —
//! small enough to test exhaustively over in-memory buffers, so the server
//! stays synchronous and the workspace keeps zero new dependencies and no
//! async runtime (`lsp-types` + `tokio` would bring both for no gain).
//!
//! Protocol subset (the ruling's list): `initialize` / `initialized` /
//! `textDocument/didOpen` / `didChange` / `didClose` / `shutdown` / `exit`,
//! with `textDocument/publishDiagnostics` as the push channel (not the
//! pull-based `textDocument/diagnostic`). Anything else: requests get
//! -32601, notifications are ignored — both LSP-legal.
//!
//! Diagnostics are the check pipeline's own output — [`crate::validate_project`],
//! the single entry `cage check` also runs — mapped onto every open
//! document as file-level zero-width ranges. A (0,0)-(0,0) range points at
//! nothing on purpose: the message body carries the cage E code + real
//! location + hint, and every diagnostic the project produces is on the
//! panel (per-file anchoring would hide the cross-file errors — references,
//! dependencies — the panel exists to surface). didOpen / didChange trigger
//! a full validation from disk (the ruling's no-incremental boundary: 配置
//! 规模是百表量级，全量毫秒级); the editor buffer is not virtualized, so
//! what gets diagnosed is the saved state and Full sync says so honestly.
//! didClose clears the closed document's panel.
//!
//! `env` / `level` ride the initialize request's `initializationOptions`
//! (`{"env": "prod", "level": "semantic"}`); defaults match `check` (base
//! environment, semantic level, client profile). A project that fails to
//! load (bad cage.toml, unresolvable registry reference) surfaces as one
//! error-severity diagnostic instead of a silent server.

use cage_core::{Diagnostic, Severity};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Maximum accepted LSP message body (bounds per-frame allocation).
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Read one `\n`-terminated header line; `None` on EOF with nothing read.
fn read_line(input: &mut impl Read) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match input.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                line.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    if line.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&line).into_owned())
    }
}

/// Read one LSP frame body, or `None` when the stream ends (cleanly or
/// mid-frame — either way the session is over).
fn read_frame(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut content_length: Option<usize> = None;
    loop {
        let header = read_line(input)?;
        let header = header.trim_end_matches(['\n', '\r']);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().ok();
        }
        // Other headers (Content-Type, …) are legal and irrelevant here.
    }
    let content_length = content_length?;
    if content_length > MAX_FRAME_BYTES {
        return None;
    }
    let mut body = vec![0_u8; content_length];
    input.read_exact(&mut body).ok()?;
    Some(body)
}

/// Write one LSP frame.
fn write_frame(output: &mut impl Write, body: &str) -> std::io::Result<()> {
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(body.as_bytes())?;
    output.flush()
}

/// The server session: project root + the two check-aligned knobs + the
/// URIs the editor currently holds open (each receives the panel).
struct Server {
    root: PathBuf,
    env: Option<String>,
    level: String,
    open_documents: BTreeSet<String>,
}

impl Server {
    /// Validate the whole project once and publish the same panel onto
    /// every open document. A load failure (bad config, unresolvable
    /// registry reference) becomes one error diagnostic instead of
    /// silence — a broken project must be visible in the editor.
    fn publish_all(&self, output: &mut impl Write) {
        let diagnostics: Vec<Value> = match crate::validate_project(
            &self.root,
            &self.level,
            self.env.as_deref(),
            "client",
            false,
        ) {
            Ok(outcome) => outcome
                .diagnostics
                .iter()
                .map(lsp_diagnostic)
                .collect::<Vec<_>>(),
            Err(load_error) => vec![json!({
                "range": zero_range(),
                "severity": 1,
                "source": "cage",
                "message": load_error,
            })],
        };
        for uri in &self.open_documents {
            let _ = write_frame(
                output,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": uri, "diagnostics": diagnostics },
                })
                .to_string(),
            );
        }
    }
}

/// The zero-width file-level range every cage diagnostic is anchored to.
fn zero_range() -> Value {
    json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } })
}

/// Map one cage diagnostic onto the LSP shape: severity 1/2/3 = error /
/// warning / info, zero-width range, the E code + real location + hint
/// all in the message (the editor sidebar reads like `cage check` output).
fn lsp_diagnostic(diag: &Diagnostic) -> Value {
    let severity = match diag.severity {
        Severity::Error => 1,
        Severity::Warning => 2,
        Severity::Info => 3,
    };
    let mut message = format!("{}: {}", diag.code, diag.message);
    if let Some(location) = &diag.location {
        use std::fmt::Write as _;
        let _ = write!(message, " ({})", location.display());
    }
    if let Some(hint) = &diag.hint {
        use std::fmt::Write as _;
        let _ = write!(message, " [hint: {hint}]");
    }
    json!({
        "range": zero_range(),
        "severity": severity,
        "code": diag.code,
        "source": "cage",
        "message": message,
    })
}

/// Answer a request with `result` (the id is echoed verbatim — number,
/// string, whatever the client chose).
fn reply(output: &mut impl Write, id: Option<&Value>, result: &Value) {
    let _ = write_frame(
        output,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
    );
}

/// Answer a request with a JSON-RPC error object. `None` id serializes as
/// null — the parse-error reply's shape (the request was unreadable).
fn reply_error(output: &mut impl Write, id: Option<&Value>, code: i64, message: &str) {
    let _ = write_frame(
        output,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        })
        .to_string(),
    );
}

/// `cage lsp <root>`: serve the diagnostics subset over stdio until the
/// client sends `exit` or closes the stream. Exit code: 0 after a proper
/// `shutdown` → `exit` handshake or on EOF, 1 on a bare `exit` (the LSP
/// spec's "client misbehaved" signal).
pub(crate) fn run_lsp(root: &Path, mut input: impl Read, mut output: impl Write) -> i32 {
    let mut server = Server {
        root: root.to_path_buf(),
        env: None,
        level: String::from("semantic"),
        open_documents: BTreeSet::new(),
    };
    let mut shutdown = false;
    while let Some(body) = read_frame(&mut input) {
        let message: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(e) => {
                reply_error(&mut output, None, -32700, &format!("parse error: {e}"));
                continue;
            }
        };
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id");
        match (method, id.is_some()) {
            (Some("initialize"), true) => {
                // The two check-aligned knobs ride initializationOptions;
                // anything else there is ignored (forward compatible).
                if let Some(options) = message.pointer("/params/initializationOptions") {
                    if let Some(env) = options.get("env").and_then(Value::as_str) {
                        server.env = Some(env.to_string());
                    }
                    if let Some(level) = options.get("level").and_then(Value::as_str) {
                        server.level = level.to_string();
                    }
                }
                reply(
                    &mut output,
                    id,
                    &json!({
                        "capabilities": {
                            "textDocumentSync": { "openClose": true, "change": 1 },
                        },
                        "serverInfo": {
                            "name": "cage-lsp",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                );
            }
            (Some("shutdown"), true) => {
                shutdown = true;
                reply(&mut output, id, &Value::Null);
            }
            (Some("exit"), _) => {
                return i32::from(!shutdown);
            }
            (Some("textDocument/didOpen"), false) => {
                if let Some(uri) = message
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                {
                    server.open_documents.insert(uri.to_string());
                    server.publish_all(&mut output);
                }
            }
            (Some("textDocument/didChange"), false) => {
                // Full revalidation from disk on every change (the ruling's
                // no-incremental boundary); the buffer text is not consumed —
                // the editor's saved state is what gets diagnosed.
                server.publish_all(&mut output);
            }
            (Some("textDocument/didClose"), false) => {
                if let Some(uri) = message
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                {
                    server.open_documents.remove(uri);
                    // Clear the closed document's panel; other documents
                    // keep theirs.
                    let _ = write_frame(
                        &mut output,
                        &json!({
                            "jsonrpc": "2.0",
                            "method": "textDocument/publishDiagnostics",
                            "params": { "uri": uri, "diagnostics": [] },
                        })
                        .to_string(),
                    );
                }
            }
            (Some(unknown), true) => {
                reply_error(
                    &mut output,
                    id,
                    -32601,
                    &format!("method not found: {unknown}"),
                );
            }
            (Some(_), false) | (None, _) => {
                // Unknown notifications and client responses need no action.
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Wrap a JSON body into an LSP frame.
    fn frame(body: &str) -> Vec<u8> {
        let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body.as_bytes());
        bytes
    }

    /// Collect every JSON-RPC message the server wrote.
    fn messages(output: &[u8]) -> Vec<Value> {
        let mut cursor = Cursor::new(output.to_vec());
        let mut out = Vec::new();
        while let Some(body) = read_frame(&mut cursor) {
            out.push(serde_json::from_slice(&body).expect("server wrote valid json"));
        }
        out
    }

    /// Drive one session: framed bytes in → (exit code, parsed messages).
    fn session(root: &Path, input: &[Vec<u8>]) -> (i32, Vec<Value>) {
        let mut wire = Vec::new();
        for frame in input {
            wire.extend_from_slice(frame);
        }
        let mut output = Vec::new();
        let code = run_lsp(root, Cursor::new(wire), &mut output);
        (code, messages(&output))
    }

    /// A one-table project; `broken` drops the required `hp` from the row
    /// so validation produces E1001.
    fn write_project(root: &Path, broken: bool) {
        std::fs::create_dir_all(root.join("schemas")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("cage.toml"),
            "output_dir = \"build\"\nschema_path = \"schemas\"\n\n\
             [project]\nname = \"lsp-demo\"\nversion = \"0.1.0\"\n\n\
             [source_roots]\nmain = \"config\"\n\n\
             [profiles.client]\nname = \"client\"\n\n\
             [[profiles.client.targets]]\nformat = \"json\"\n\
             output_dir = \"build/client\"\nfile_template = \"{table}.json\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("schemas/hero.yaml"),
            "tables:\n  Hero:\n    name: Hero\n    primary_key: [id]\n    fields:\n\
             \x20     id: { name: id, type: { kind: Int32 }, required: true }\n\
             \x20     hp: { name: hp, type: { kind: Int32 }, required: true, min: 1 }\n\
             enums: {}\n",
        )
        .unwrap();
        let hero = if broken {
            r#"{ "Hero": [ { "id": 1 } ] }"#
        } else {
            r#"{ "Hero": [ { "id": 1, "hp": 100 } ] }"#
        };
        std::fs::write(root.join("config/hero.json"), hero).unwrap();
    }

    #[test]
    fn frames_round_trip_and_tolerate_extra_headers() {
        // A well-formed frame with a legal extra header decodes intact.
        let mut wire = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n".to_vec();
        wire.extend_from_slice(frame("{\"a\":1}").as_slice());
        let mut cursor = Cursor::new(wire);
        assert_eq!(
            read_frame(&mut cursor),
            Some(b"{\"a\":1}".to_vec()),
            "body survives an extra header"
        );
        // No Content-Length → no body length → no frame.
        let mut cursor = Cursor::new(b"Content-Type: text/plain\r\n\r\n{}".to_vec());
        assert_eq!(read_frame(&mut cursor), None, "missing length is no frame");
        // Clean EOF between frames is the ordinary session end.
        let mut cursor = Cursor::new(Vec::new());
        assert_eq!(read_frame(&mut cursor), None, "empty stream is no frame");
        // A written frame reads back byte-identically (write/read symmetry).
        let mut buffer = Vec::new();
        write_frame(&mut buffer, "{\"b\":2}").unwrap();
        let mut cursor = Cursor::new(buffer);
        assert_eq!(read_frame(&mut cursor), Some(b"{\"b\":2}".to_vec()));
    }

    #[test]
    fn framing_rejects_content_length_over_limit() {
        let input = format!("Content-Length: {}\r\n\r\n", MAX_FRAME_BYTES + 1);
        let mut cursor = Cursor::new(input.into_bytes());
        assert_eq!(read_frame(&mut cursor), None);
    }

    #[test]
    fn initialize_replies_capabilities_and_exit_codes_follow_the_spec() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), false);
        let init = frame(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"initializationOptions":{}}}"#,
        );
        // Bare `exit` without a prior `shutdown` is the error signal.
        let (code, _) = session(
            tmp.path(),
            &[init.clone(), frame(r#"{"jsonrpc":"2.0","method":"exit"}"#)],
        );
        assert_eq!(code, 1, "exit before shutdown must signal misuse");
        // Proper handshake → exit 0, capabilities + serverInfo on the reply.
        let (code, msgs) = session(
            tmp.path(),
            &[
                init.clone(),
                frame(r#"{"jsonrpc":"2.0","method":"initialized"}"#),
                frame(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#),
                frame(r#"{"jsonrpc":"2.0","method":"exit"}"#),
            ],
        );
        assert_eq!(code, 0, "shutdown then exit is the clean end");
        assert_eq!(msgs.len(), 2, "one initialize reply, one shutdown reply");
        assert_eq!(msgs[0]["id"], 1);
        assert_eq!(
            msgs[0]["result"]["capabilities"]["textDocumentSync"]["openClose"],
            true
        );
        assert_eq!(msgs[0]["result"]["serverInfo"]["name"], "cage-lsp");
        assert_eq!(msgs[1]["id"], 2);
        assert!(msgs[1]["result"].is_null(), "shutdown replies null");
        // EOF without exit also ends cleanly (the client closed the pipe).
        let (code, _) = session(tmp.path(), &[init]);
        assert_eq!(code, 0, "eof after handshake is a clean end");
    }

    #[test]
    fn did_open_publishes_the_panel_and_knobs_reach_the_pipeline() {
        let clean = tempfile::tempdir().unwrap();
        write_project(clean.path(), false);
        let open = |uri: &str| {
            frame(&format!(
                r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen",
                    "params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
            ))
        };
        // A clean project publishes an empty panel.
        let (_, msgs) = session(
            clean.path(),
            &[
                frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#),
                open("file:///w/hero.yaml"),
            ],
        );
        assert_eq!(msgs.len(), 2, "initialize reply + one publish");
        assert_eq!(msgs[1]["method"], "textDocument/publishDiagnostics");
        assert_eq!(msgs[1]["params"]["uri"], "file:///w/hero.yaml");
        assert_eq!(
            msgs[1]["params"]["diagnostics"],
            serde_json::json!([]),
            "clean project, empty panel"
        );

        // A broken project publishes E1001 with the zero-width range and
        // the mapped severity; a knob (level) reaches the pipeline.
        let broken = tempfile::tempdir().unwrap();
        write_project(broken.path(), true);
        let (_, msgs) = session(
            broken.path(),
            &[
                frame(
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize",
                        "params":{"initializationOptions":{"level":"semantic"}}}"#,
                ),
                open("file:///w/hero.yaml"),
            ],
        );
        let panel = &msgs[1]["params"]["diagnostics"];
        assert_eq!(panel.as_array().map(Vec::len), Some(1), "{msgs:?}");
        assert_eq!(panel[0]["severity"], 1, "error maps to severity 1");
        assert_eq!(panel[0]["code"], "E1001");
        assert_eq!(panel[0]["source"], "cage");
        assert_eq!(
            panel[0]["range"]["start"],
            serde_json::json!({"line": 0, "character": 0}),
            "file-level zero-width range"
        );
        let message = panel[0]["message"].as_str().unwrap();
        assert!(message.starts_with("E1001: "), "{message}");
        assert!(
            message.contains("hero.json"),
            "message carries the location"
        );
        assert!(message.contains("config/hero.json"), "{message}");

        // An invalid level from initializationOptions surfaces as a load
        // error diagnostic instead of a silent (or crashing) server.
        let (_, msgs) = session(
            broken.path(),
            &[
                frame(
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize",
                        "params":{"initializationOptions":{"level":"nonsense"}}}"#,
                ),
                open("file:///w/hero.yaml"),
            ],
        );
        let panel = &msgs[1]["params"]["diagnostics"];
        assert_eq!(panel[0]["severity"], 1, "bad knob is an error diagnostic");
        assert!(
            panel[0]["message"].as_str().unwrap().contains("nonsense"),
            "{panel:?}"
        );
    }

    #[test]
    fn did_change_republishes_and_did_close_clears_only_that_document() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), true);
        let open = |uri: &str| {
            frame(&format!(
                r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen",
                    "params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
            ))
        };
        let close = |uri: &str| {
            frame(&format!(
                r#"{{"jsonrpc":"2.0","method":"textDocument/didClose",
                    "params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
            ))
        };
        let change = frame(
            r#"{"jsonrpc":"2.0","method":"textDocument/didChange",
                "params":{"textDocument":{"uri":"file:///w/a.yaml"},
                          "contentChanges":[]}}"#,
        );
        let (_, msgs) = session(
            tmp.path(),
            &[
                frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#),
                open("file:///w/a.yaml"),
                open("file:///w/b.yaml"),
                change,
                close("file:///w/a.yaml"),
            ],
        );
        // Every open document gets the panel on each validation burst, in
        // sorted-URI order: open a → {a}; open b → {a,b}; change → {a,b};
        // close a → the cleared panel for a only.
        assert_eq!(msgs.len(), 2 + 2 + 2 + 1);
        assert_eq!(msgs[1]["params"]["uri"], "file:///w/a.yaml");
        assert_eq!(
            msgs[1]["params"]["diagnostics"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(msgs[2]["params"]["uri"], "file:///w/a.yaml");
        assert_eq!(msgs[3]["params"]["uri"], "file:///w/b.yaml");
        assert_eq!(msgs[4]["params"]["uri"], "file:///w/a.yaml");
        assert_eq!(msgs[5]["params"]["uri"], "file:///w/b.yaml");
        assert_eq!(msgs[6]["params"]["uri"], "file:///w/a.yaml");
        assert_eq!(
            msgs[6]["params"]["diagnostics"],
            serde_json::json!([]),
            "closed document's panel is cleared"
        );
        // After the close, b is no longer open → it gets no more pushes.
        let (_, msgs) = session(
            tmp.path(),
            &[
                frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#),
                open("file:///w/b.yaml"),
                close("file:///w/b.yaml"),
            ],
        );
        assert_eq!(msgs.len(), 3, "open panel + close clear, nothing else");
        assert_eq!(msgs[2]["params"]["uri"], "file:///w/b.yaml");
    }

    #[test]
    fn unknown_requests_get_32601_garbage_gets_32700_notifications_stay_quiet() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), false);
        // Unknown REQUEST → -32601 with the id echoed.
        let (code, msgs) = session(
            tmp.path(),
            &[frame(
                r#"{"jsonrpc":"2.0","id":"abc","method":"textDocument/hover","params":{}}"#,
            )],
        );
        assert_eq!(code, 0, "eof ends the session cleanly");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["id"], "abc");
        assert_eq!(msgs[0]["error"]["code"], -32601);
        // Unparseable body → -32700 with a null id.
        let (_, msgs) = session(tmp.path(), &[frame("{not json")]);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0]["id"].is_null());
        assert_eq!(msgs[0]["error"]["code"], -32700);
        // Unknown NOTIFICATION (no id) → no reply at all.
        let (_, msgs) = session(
            tmp.path(),
            &[frame(
                r#"{"jsonrpc":"2.0","method":"$/cancelRequest","params":{}}"#,
            )],
        );
        assert!(msgs.is_empty(), "notifications must stay silent");
    }
}
