//! `cage lsp` — a diagnostics-only Language Server over stdio (design §49,
//! I-series IDE ruling). Transport, framing, and the JSON-RPC envelope come
//! from the `lsp-server` crate (the scaffold rust-analyzer and kode-server
//! sit on): frames travel via [`lsp_server::Message::read`]/[`lsp_server::Message::write`],
//! envelopes are `lsp_server::{Request, Response, Notification}`, and the
//! payload shapes are `lsp-types` structs. Dispatch stays a plain
//! synchronous loop — the library owns every protocol byte.
//!
//! Protocol subset (the ruling's list): `initialize` / `initialized` /
//! `textDocument/didOpen` / `didChange` / `didClose` / `shutdown` / `exit`,
//! with `textDocument/publishDiagnostics` as the push channel (not the
//! pull-based `textDocument/diagnostic`). Anything else: requests get
//! -32601, notifications are ignored — both LSP-legal.
//!
//! Adopted library semantics (approved deviation from the hand-rolled
//! era): a frame whose body fails to deserialize makes `Message::read`
//! fail and ends the session (the library reader cannot resync
//! mid-stream), and there is no per-frame size cap anymore — the library
//! trusts the declared body length. Both match kode-server's behavior.
//!
//! Diagnostics are the check pipeline's own output — [`crate::validate_project`],
//! the single entry `cage check` also runs — mapped onto every open
//! document as file-level zero-width ranges. A (0,0)-(0,0) range points at
//! nothing on purpose: the message body carries the cage E code + real
//! location + hint, and every diagnostic the project produces is on the
//! panel (per-file anchoring would hide the cross-file errors — references,
//! dependencies — the panel exists to surface). didOpen / didChange trigger
//! a full validation from disk (the ruling's no-incremental boundary:
//! config scale is hundreds of tables, a full pass is milliseconds); the
//! editor buffer is not virtualized, so what gets diagnosed is the saved
//! state and Full sync says so honestly. didClose clears the closed
//! document's panel.
//!
//! `env` / `level` ride the initialize request's `initializationOptions`
//! (`{"env": "prod", "level": "semantic"}`); defaults match `check` (base
//! environment, semantic level, client profile). A project that fails to
//! load (bad cage.toml, unresolvable registry reference) surfaces as one
//! error-severity diagnostic instead of a silent server.

use cage_core::{Diagnostic, Severity};
use lsp_server::{ErrorCode, Message, Notification, Response};
use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticSeverity, InitializeResult, NumberOrString, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, Url,
};
use std::collections::BTreeSet;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// The server session: project root + the two check-aligned knobs + the
/// URIs the editor currently holds open (each receives the panel).
struct Server {
    root: PathBuf,
    env: Option<String>,
    level: String,
    open_documents: BTreeSet<Url>,
}

impl Server {
    /// Validate the whole project once and publish the same panel onto
    /// every open document. A load failure (bad config, unresolvable
    /// registry reference) becomes one error diagnostic instead of
    /// silence — a broken project must be visible in the editor.
    fn publish_all(&self, output: &mut impl Write) {
        let diagnostics: Vec<LspDiagnostic> = match crate::validate_project(
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
            Err(load_error) => vec![LspDiagnostic {
                range: zero_range(),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(String::from("cage")),
                message: load_error,
                ..LspDiagnostic::default()
            }],
        };
        for uri in &self.open_documents {
            let notification = Notification::new(
                String::from("textDocument/publishDiagnostics"),
                PublishDiagnosticsParams {
                    uri: uri.clone(),
                    diagnostics: diagnostics.clone(),
                    version: None,
                },
            );
            let _ = Message::from(notification).write(output);
        }
    }
}

/// The zero-width file-level range every cage diagnostic is anchored to.
fn zero_range() -> Range {
    Range::new(Position::new(0, 0), Position::new(0, 0))
}

/// Map one cage diagnostic onto the LSP shape: severity 1/2/3 = error /
/// warning / info, zero-width range, the E code + real location + hint
/// all in the message (the editor sidebar reads like `cage check` output).
fn lsp_diagnostic(diag: &Diagnostic) -> LspDiagnostic {
    let severity = match diag.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    };
    let mut message = format!("{}: {}", diag.code, diag.message);
    if let Some(location) = &diag.location {
        use std::fmt::Write as _;
        // Editor-facing text keeps the portable `/` separators even when
        // the location's file path is Windows-native.
        let _ = write!(
            message,
            " ({})",
            location.display().replace(std::path::MAIN_SEPARATOR, "/")
        );
    }
    if let Some(hint) = &diag.hint {
        use std::fmt::Write as _;
        let _ = write!(message, " [hint: {hint}]");
    }
    LspDiagnostic {
        range: zero_range(),
        severity: Some(severity),
        code: Some(NumberOrString::String(diag.code.clone())),
        source: Some(String::from("cage")),
        message,
        ..LspDiagnostic::default()
    }
}

/// Send one response frame.
fn send_response(output: &mut impl Write, response: Response) {
    let _ = Message::from(response).write(output);
}

/// Read a URI out of a notification's params and parse it. Params stay
/// loosely read on purpose: the pinned contract accepts documents opened
/// without the full typed item (no `languageId`/`version`/`text`), which
/// the strict typed params would reject. Unparseable params or URIs drop
/// the notification.
fn parse_uri(params: &serde_json::Value, pointer: &str) -> Option<Url> {
    params
        .pointer(pointer)
        .and_then(|value| value.as_str())
        .and_then(|uri| Url::parse(uri).ok())
}

/// `cage lsp <root>`: serve the diagnostics subset over stdio until the
/// client sends `exit` or closes the stream. Exit code: 0 after a proper
/// `shutdown` → `exit` handshake, on EOF, or when the reader gives up on
/// an unreadable frame; 1 on a bare `exit` (the LSP spec's "client
/// misbehaved" signal).
pub(crate) fn run_lsp(root: &Path, input: impl Read, mut output: impl Write) -> i32 {
    let mut server = Server {
        root: root.to_path_buf(),
        env: None,
        level: String::from("semantic"),
        open_documents: BTreeSet::new(),
    };
    let mut reader = BufReader::new(input);
    let mut shutdown = false;
    loop {
        // EOF is the ordinary session end; an unreadable frame (bad
        // header, bad body) also ends it — the library reader cannot
        // resync, so neither can the session (adopted semantics).
        let Ok(Some(message)) = Message::read(&mut reader) else {
            return 0;
        };
        match message {
            Message::Request(request) => match request.method.as_str() {
                "initialize" => {
                    // The two check-aligned knobs ride `initializationOptions`;
                    // anything else there is ignored (forward compatible).
                    // The params stay loosely read (see `parse_uri`): the
                    // pinned contract accepts initialize requests without a
                    // capabilities object, which the typed `InitializeParams`
                    // (strict `capabilities` field) would reject.
                    if let Some(options) = request.params.pointer("/initializationOptions") {
                        if let Some(env) = options.get("env").and_then(|v| v.as_str()) {
                            server.env = Some(env.to_string());
                        }
                        if let Some(level) = options.get("level").and_then(|v| v.as_str()) {
                            server.level = level.to_string();
                        }
                    }
                    let result = InitializeResult {
                        capabilities: ServerCapabilities {
                            text_document_sync: Some(TextDocumentSyncCapability::Options(
                                TextDocumentSyncOptions {
                                    open_close: Some(true),
                                    change: Some(TextDocumentSyncKind::FULL),
                                    ..TextDocumentSyncOptions::default()
                                },
                            )),
                            ..ServerCapabilities::default()
                        },
                        server_info: Some(ServerInfo {
                            name: String::from("cage-lsp"),
                            version: Some(env!("CARGO_PKG_VERSION").to_string()),
                        }),
                    };
                    send_response(&mut output, Response::new_ok(request.id, result));
                }
                "shutdown" => {
                    shutdown = true;
                    send_response(&mut output, Response::new_ok(request.id, ()));
                }
                unknown => {
                    send_response(
                        &mut output,
                        Response::new_err(
                            request.id,
                            ErrorCode::MethodNotFound as i32,
                            format!("method not found: {unknown}"),
                        ),
                    );
                }
            },
            Message::Notification(notification) => match notification.method.as_str() {
                "exit" => return i32::from(!shutdown),
                "textDocument/didOpen" => {
                    if let Some(uri) = parse_uri(&notification.params, "/textDocument/uri") {
                        server.open_documents.insert(uri);
                        server.publish_all(&mut output);
                    }
                }
                // Full revalidation from disk on every change (the ruling's
                // no-incremental boundary); the buffer is not consumed —
                // the editor's saved state is what gets diagnosed.
                "textDocument/didChange" => server.publish_all(&mut output),
                "textDocument/didClose" => {
                    if let Some(uri) = parse_uri(&notification.params, "/textDocument/uri") {
                        server.open_documents.remove(&uri);
                        // Clear the closed document's panel; other documents
                        // keep theirs.
                        let notification = Notification::new(
                            String::from("textDocument/publishDiagnostics"),
                            PublishDiagnosticsParams {
                                uri,
                                diagnostics: Vec::new(),
                                version: None,
                            },
                        );
                        let _ = Message::from(notification).write(&mut output);
                    }
                }
                // Unknown notifications and client responses need no action.
                _ => {}
            },
            // Responses to server-initiated requests; the server never
            // sends one, so a stray response is ignored.
            Message::Response(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_server::Request;
    use serde_json::Value;
    use std::io::Cursor;

    /// Encode one client message through the library's writer — the same
    /// framing the real stdin side speaks.
    fn frame(body: &str) -> Vec<u8> {
        let message: Message = serde_json::from_str(body).expect("test message parses");
        let mut bytes = Vec::new();
        message.write(&mut bytes).expect("frame writes");
        bytes
    }

    /// Decode every server frame back into JSON for the assertions.
    fn messages(output: &[u8]) -> Vec<Value> {
        let mut cursor = Cursor::new(output.to_vec());
        let mut out = Vec::new();
        while let Ok(Some(message)) = Message::read(&mut cursor) {
            out.push(serde_json::to_value(&message).expect("server frames are serializable"));
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
    fn library_frames_round_trip_and_tolerate_extra_headers() {
        // A well-formed frame carrying a legal extra header decodes intact.
        let mut wire = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n".to_vec();
        wire.extend_from_slice(frame(r#"{"jsonrpc":"2.0","method":"initialized"}"#).as_slice());
        let decoded = Message::read(&mut Cursor::new(wire))
            .expect("frame reads")
            .expect("one frame");
        match decoded {
            Message::Notification(notification) => {
                assert_eq!(notification.method, "initialized", "body survives");
            }
            other => panic!("expected a notification, got {other:?}"),
        }
        // No length header at all → the library rejects the frame.
        let mut raw = Cursor::new(b"Content-Type: text/plain\r\n\r\n{}".to_vec());
        assert!(
            Message::read(&mut raw).is_err(),
            "missing length is a read error"
        );
        // Clean EOF between frames is the ordinary session end.
        let mut empty = Cursor::new(Vec::new());
        assert!(
            matches!(Message::read(&mut empty), Ok(None)),
            "empty stream ends cleanly"
        );
        // A written frame reads back identically (write/read symmetry).
        let sent = Message::Request(Request {
            id: 7.into(),
            method: String::from("initialize"),
            params: serde_json::json!({ "a": 1 }),
        });
        let mut buffer = Vec::new();
        sent.write(&mut buffer).unwrap();
        let read_back = Message::read(&mut Cursor::new(buffer))
            .unwrap()
            .expect("one frame");
        assert_eq!(
            serde_json::to_value(&sent).unwrap(),
            serde_json::to_value(&read_back).unwrap(),
            "a written frame reads back identically"
        );
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
    fn unknown_requests_get_32601_and_notifications_stay_quiet() {
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
