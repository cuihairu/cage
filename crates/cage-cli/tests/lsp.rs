//! `cage lsp` process-level coverage (design §49 I 系列 IDE 立项): the
//! diagnostics-only Language Server speaks hand-rolled JSON-RPC 2.0 over
//! stdio — Content-Length framing on both sides — and pushes the check
//! pipeline's diagnostics onto every open document. These tests drive the
//! real binary as a subprocess: request in, frame out, assertions both
//! ways (initialize/shutdown replies and didOpen/didClose publishes).

use std::io::{Read, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

use serde_json::json;

/// The shared example project, absolute (cargo runs test binaries from the
/// package directory).
fn example_root() -> String {
    format!("{}/../../examples/game-config", env!("CARGO_MANIFEST_DIR"))
}

/// One framed LSP message written to the server's stdin.
fn write_message(stdin: &mut impl Write, body: &str) {
    write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    stdin.flush().unwrap();
}

/// One LSP frame read from the server's stdout, parsed as JSON.
fn read_message(stdout: &mut impl Read) -> serde_json::Value {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            stdout.read_exact(&mut byte).unwrap();
            line.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        let header = String::from_utf8_lossy(&line);
        let header = header.trim_end_matches(['\n', '\r']);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse().unwrap());
        }
    }
    let mut body = vec![0_u8; content_length.expect("server sent Content-Length")];
    stdout.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// A live `cage lsp` session: the parent writes the session, reads the
/// expected replies, and reaps the child on scope exit.
struct Session {
    child: Child,
    stdout: ChildStdout,
}

impl Session {
    fn new(root: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cage"))
            .args(["lsp", root])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn `cage lsp {root}`: {e}"));
        let stdout = child.stdout.take().expect("stdout piped");
        Self { child, stdout }
    }

    fn stdin(&mut self) -> &mut std::process::ChildStdin {
        self.child.stdin.as_mut().expect("stdin piped")
    }

    /// Send a request, read its reply.
    fn request(&mut self, body: &str) -> serde_json::Value {
        write_message(self.stdin(), body);
        read_message(&mut self.stdout)
    }

    /// Send a notification (no reply expected).
    fn notify(&mut self, body: &str) {
        write_message(self.stdin(), body);
    }

    /// Close stdin and return the child's exit code.
    fn wait(mut self) -> i32 {
        drop(self.child.stdin.take());
        let status = self.child.wait().expect("reap child");
        status.code().expect("child exited with a code")
    }
}

fn did_open(uri: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{{"textDocument":{{"uri":"{uri}","languageId":"yaml","version":1,"text":""}}}}}}"#
    )
}

fn did_close(uri: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didClose",
            "params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
    )
}

fn did_change(uri: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{{"textDocument":{{"uri":"{uri}","version":2}},
                       "contentChanges":[{{"text":"tables: {{}}"}}]}}}}"#
    )
}

#[test]
fn initialize_shows_capabilities_and_clean_projects_publish_an_empty_panel() {
    let root = example_root();
    let uri = format!("file://{}/schemas/item.yaml", root);
    let mut session = Session::new(&root);

    let reply = session.request(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"processId":null,"capabilities":{},
                      "initializationOptions":{}}}"#,
    );
    assert_eq!(reply["id"], 1, "the request id is echoed");
    assert_eq!(reply["result"]["serverInfo"]["name"], "cage-lsp");
    assert!(reply["result"]["serverInfo"]["version"].is_string());
    assert_eq!(
        reply["result"]["capabilities"]["textDocumentSync"]["openClose"], true,
        "open/close is advertised"
    );

    // Nothing is published before a document is opened; opening one gets
    // the whole project's panel (empty — the example project is clean).
    session.notify(r#"{"jsonrpc":"2.0","method":"initialized"}"#);
    session.notify(&did_open(&uri));
    let publish = read_message(&mut session.stdout);
    assert_eq!(publish["method"], "textDocument/publishDiagnostics");
    assert_eq!(
        publish["params"]["uri"], uri,
        "panel filed under the opened URI"
    );
    assert_eq!(
        publish["params"]["diagnostics"],
        json!([]),
        "clean project, empty panel"
    );

    let shutdown = session.request(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#);
    assert!(shutdown["result"].is_null());
    session.notify(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(session.wait(), 0, "shutdown then exit is the clean end");
}

#[test]
fn broken_projects_publish_error_diagnostics_with_location_and_code() {
    let root = format!("{}/bad", example_root());
    let uri = format!("file://{}/schemas/item.yaml", root);
    let mut session = Session::new(&root);
    session.request(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    session.notify(&did_open(&uri));
    let publish = read_message(&mut session.stdout);

    let diagnostics = publish["params"]["diagnostics"].as_array().expect("array");
    assert!(!diagnostics.is_empty(), "{}", publish);
    let codes: Vec<&str> = diagnostics
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"E1501"),
        "E1501 from the bad example must reach the editor panel: {codes:?}"
    );
    for diagnostic in diagnostics {
        // File-level zero-width range, cage as the source, the E code
        // leading a message that carries the real location.
        assert_eq!(
            diagnostic["range"]["start"],
            json!({"line": 0, "character": 0}),
            "{diagnostic}"
        );
        assert_eq!(
            diagnostic["range"]["end"],
            json!({"line": 0, "character": 0}),
            "{diagnostic}"
        );
        assert_eq!(diagnostic["source"], "cage", "{diagnostic}");
        let message = diagnostic["message"].as_str().expect("message");
        assert!(
            message.starts_with(&format!("{}: ", diagnostic["code"].as_str().unwrap())),
            "{message}"
        );
        assert!(
            message.contains('.'),
            "the message carries a file location: {message}"
        );
    }
    session.request(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#);
    session.notify(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(session.wait(), 0);
}

#[test]
fn did_close_clears_only_that_document_and_changes_republish() {
    let root = example_root();
    let uri = format!("file://{}/schemas/item.yaml", root);
    let other = format!("file://{}/schemas/quest.yaml", root);
    let mut session = Session::new(&root);
    session.request(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);

    // Every open document gets the panel on each validation burst (sorted
    // URI order): open item → {item}; open quest → {item, quest}.
    session.notify(&did_open(&uri));
    assert_eq!(read_message(&mut session.stdout)["params"]["uri"], uri);
    session.notify(&did_open(&other));
    assert_eq!(read_message(&mut session.stdout)["params"]["uri"], uri);
    assert_eq!(read_message(&mut session.stdout)["params"]["uri"], other);

    // didChange revalidates from disk and pushes both panels again.
    session.notify(&did_change(&uri));
    assert_eq!(read_message(&mut session.stdout)["params"]["uri"], uri);
    let republish = read_message(&mut session.stdout);
    assert_eq!(republish["params"]["uri"], other);
    assert!(republish["params"]["diagnostics"].as_array().is_some());

    // didClose clears exactly the closed document.
    session.notify(&did_close(&uri));
    let clear = read_message(&mut session.stdout);
    assert_eq!(clear["params"]["uri"], uri);
    assert_eq!(
        clear["params"]["diagnostics"],
        json!([]),
        "closed document cleared"
    );

    session.request(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#);
    session.notify(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(session.wait(), 0);
}

#[test]
fn bad_level_from_initialization_options_becomes_a_visible_error() {
    let root = example_root();
    let uri = format!("file://{}/schemas/item.yaml", root);
    let mut session = Session::new(&root);
    session.request(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"initializationOptions":{"level":"nonsense"}}}"#,
    );
    session.notify(&did_open(&uri));
    let publish = read_message(&mut session.stdout);
    let panel = publish["params"]["diagnostics"].as_array().expect("array");
    assert_eq!(panel.len(), 1, "{publish}");
    assert_eq!(panel[0]["severity"], 1, "a bad knob is an error diagnostic");
    assert!(
        panel[0]["message"].as_str().unwrap().contains("nonsense"),
        "{}",
        panel[0]
    );
    session.request(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#);
    session.notify(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(session.wait(), 0);
}

#[test]
fn unknown_requests_and_garbage_bodies_get_json_rpc_errors() {
    let root = example_root();
    let mut session = Session::new(&root);
    // Unknown request → -32601, id echoed.
    let reply = session
        .request(r#"{"jsonrpc":"2.0","id":"hover-1","method":"textDocument/hover","params":{}}"#);
    assert_eq!(reply["id"], "hover-1");
    assert_eq!(reply["error"]["code"], -32601);
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("textDocument/hover"));
    // Malformed body → -32700, null id — and the session keeps serving.
    write_message(session.stdin(), "{not json");
    let reply = read_message(&mut session.stdout);
    assert!(reply["id"].is_null());
    assert_eq!(reply["error"]["code"], -32700);
    let reply = session.request(r#"{"jsonrpc":"2.0","id":9,"method":"shutdown"}"#);
    assert!(reply["result"].is_null());
    session.notify(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(session.wait(), 0);
}

#[test]
fn bare_exit_without_shutdown_is_a_misuse_signal() {
    let root = example_root();
    let mut session = Session::new(&root);
    session.request(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    session.notify(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert_eq!(session.wait(), 1, "the LSP spec's error exit code");
}

#[test]
fn help_documents_the_protocol_knobs() {
    let output = Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(["lsp", "--help"])
        .output()
        .expect("lsp --help");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("initializationOptions"), "{help}");
    assert!(help.contains("Language Server"), "{help}");
}
