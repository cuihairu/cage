//! Cross-language interop: the Rust protocol types are exercised against the
//! real Python SDK over an actual subprocess stdin/stdout exchange.
//!
//! This is the P2-1 proof that both sides of the versioned JSON IPC speak
//! the same wire — the host builds a `Request` with Rust types, the Python
//! `cage_sdk` decodes it, runs a plugin, and returns a `Response` the Rust
//! side decodes back. (Same python3 gate as `convention/tests/script.rs`.)

use cage_plugin_protocol::{
    PluginInfo, Request, Response, RowPayload, RunParams, TablePayload,
};
use serde_json::json;
use std::io::Write;
use std::process::{Command, Stdio};

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Repo root for a test running inside `crates/cage-plugin-protocol`.
fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

const SDK_PLUGIN: &str = r#"
import sys, os
sys.path.insert(0, os.environ["CAGE_SDK_PATH"])
from cage_sdk import Plugin, run

class Demo(Plugin):
    id = "demo"
    version = "1.0.0"
    def validate(self, ctx):
        table = ctx.table("Monster")
        if table is None:
            ctx.warning("no Monster table")
            return
        for row in table["rows"]:
            level = row["fields"].get("level")
            if isinstance(level, (int, float)) and level > 10:
                ctx.error("level too high", table="Monster", row=row["index"], field="level")
            else:
                ctx.warning("checked", table="Monster", row=row["index"], field="level")

run(Demo())
"#;

#[test]
#[cfg(unix)]
fn rust_host_and_python_sdk_agree_on_the_wire() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("plugin.py");
    std::fs::write(&script, SDK_PLUGIN).expect("write plugin");

    let mut fields = serde_json::Map::new();
    fields.insert("id".to_string(), json!(1));
    fields.insert("level".to_string(), json!(12));
    let request = Request::run(
        "interop-1",
        RunParams {
            plugin: PluginInfo {
                id: "demo".to_string(),
                version: "1.0.0".to_string(),
            },
            schema: json!({"name": "Monster", "tables": []}),
            tables: vec![TablePayload {
                name: "Monster".to_string(),
                primary_key: vec!["id".to_string()],
                rows: vec![RowPayload {
                    index: 0,
                    location: Some("Monster.xlsx#Sheet1!A2".to_string()),
                    fields,
                }],
            }],
            options: json!({}),
        },
    );

    let mut child = Command::new("python3")
        .arg(&script)
        .env("CAGE_SDK_PATH", repo_root().join("sdk").join("python"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python3");

    let mut stdin = child.stdin.take().expect("stdin");
    let body = serde_json::to_vec(&request).expect("serialize request");
    stdin.write_all(&body).expect("write request");
    drop(stdin); // close stdin: the MVP worker reads until EOF

    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "worker exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let response: Response = serde_json::from_str(stdout.trim()).expect("decode response");
    assert!(response.ok, "expected ok response, got: {response:?}");
    assert_eq!(response.id, "interop-1");
    assert_eq!(response.protocol, cage_plugin_protocol::PROTOCOL_VERSION);

    let diagnostics = response.diagnostics.expect("diagnostics present");
    assert_eq!(diagnostics.len(), 1);
    let diag = &diagnostics[0];
    assert_eq!(diag.severity, cage_plugin_protocol::Severity::Error);
    assert_eq!(diag.code, "E1601");
    assert_eq!(diag.message, "level too high");
    assert_eq!(diag.table.as_deref(), Some("Monster"));
    assert_eq!(diag.row, Some(0));
    assert_eq!(diag.field.as_deref(), Some("level"));
    assert_eq!(diag.hint, None);
}

#[test]
#[cfg(unix)]
fn a_plugin_error_surfaces_as_a_stable_protocol_error() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("boom.py");
    std::fs::write(
        &script,
        r#"
import sys, os
sys.path.insert(0, os.environ["CAGE_SDK_PATH"])
from cage_sdk import Plugin, run

class Boom(Plugin):
    id = "boom"
    version = "1.0.0"
    def validate(self, ctx):
        raise RuntimeError("kaboom")

run(Boom())
"#,
    )
    .expect("write plugin");

    let request = Request::run(
        "interop-2",
        RunParams {
            plugin: PluginInfo {
                id: "boom".into(),
                version: "1.0.0".into(),
            },
            schema: json!({}),
            tables: vec![],
            options: json!({}),
        },
    );

    let mut child = Command::new("python3")
        .arg(&script)
        .env("CAGE_SDK_PATH", repo_root().join("sdk").join("python"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python3");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(&request).expect("serialize"))
        .expect("write request");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code(),
        Some(1),
        "worker must exit 1 on plugin error"
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let response: Response = serde_json::from_str(stdout.trim()).expect("decode response");
    assert!(!response.ok);
    let error = response.error.expect("error present");
    assert_eq!(error.code, cage_plugin_protocol::ERR_PLUGIN_ERROR);
    assert!(
        error.message.contains("RuntimeError"),
        "message was: {}",
        error.message
    );
}

#[test]
#[cfg(unix)]
fn a_version_mismatch_is_refused_by_the_sdk() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("v.py");
    std::fs::write(
        &script,
        r#"
import sys, os
sys.path.insert(0, os.environ["CAGE_SDK_PATH"])
from cage_sdk import Plugin, run
class P(Plugin):
    id = "p"
    version = "1.0.0"
    def validate(self, ctx):
        pass
run(P())
"#,
    )
    .expect("write plugin");

    let request = Request::run(
        "interop-3",
        RunParams {
            plugin: PluginInfo {
                id: "p".into(),
                version: "1.0.0".into(),
            },
            schema: json!({}),
            tables: vec![],
            options: json!({}),
        },
    );
    // Declare a protocol version the SDK does not speak.
    let mut value = serde_json::to_value(&request).expect("to value");
    value["protocol"] = json!(99);

    let mut child = Command::new("python3")
        .arg(&script)
        .env("CAGE_SDK_PATH", repo_root().join("sdk").join("python"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python3");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(&value).expect("serialize"))
        .expect("write request");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let response: Response = serde_json::from_str(stdout.trim()).expect("decode response");
    assert!(!response.ok);
    let error = response.error.expect("error present");
    assert_eq!(
        error.code,
        cage_plugin_protocol::ERR_PROTOCOL_VERSION_MISMATCH
    );
    assert!(
        error.message.contains("99"),
        "message was: {}",
        error.message
    );
}
