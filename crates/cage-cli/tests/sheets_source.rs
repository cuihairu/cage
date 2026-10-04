//! Process-level coverage of the Google Sheets source (S3, design §45):
//! the load-time discipline surfaces through the CLI before any network
//! touch — malformed or traversal-shaped specs are refused (E1901), and
//! a missing `credential_env` (or an unset env var) is E1904. The
//! fetch → shape gate → canonical JSON → cache → parse half runs against
//! a local stand-in server inside the adapter's own suite; the CLI always
//! talks to the production endpoint, so no network participates here.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

fn run_cage_without_env(args: &[&str], removed: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .env_remove(removed)
        .output()
        .unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_code(out: &std::process::Output, want: i32, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(want),
        "{what}: expected exit {want}\nstderr:\n{}",
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
  Levels:
    name: Levels
    description: A game level
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

fn write_project(dir: &Path, source: &str, remote_section: &str) {
    let body = format!(
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "consumer"
version = "0.1.0"

{remote_section}
[source_roots]
main = "{source}"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{{table}}.json"
"#
    );
    write(&dir.join("cage.toml"), &body);
    write(&dir.join("schema.yaml"), SCHEMA);
}

#[test]
fn sheets_source_load_time_failures_report_their_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // No [remote.gsheets] at all → no credential_env declared (E1904),
    // before any network touch.
    let proj = root.join("no_config");
    write_project(&proj, "gsheet:1AbC/Levels", "# (no remote section)\n");
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "undeclared credential_env");
    let err = stderr(&out);
    assert!(err.contains("E1904"), "{err}");
    assert!(err.contains("credential_env"), "{err}");

    // credential_env declared but the env var unset → E1904; the env var
    // NAME lives in cage.toml, the key itself never does.
    let proj = root.join("env_unset");
    write_project(
        &proj,
        "gsheet:1AbC/Levels",
        "[remote.gsheets]\ncredential_env = \"CAGE_TEST_SHEETS_KEY_ABSENT\"\n",
    );
    let out = run_cage_without_env(
        &["check", proj.to_str().unwrap()],
        "CAGE_TEST_SHEETS_KEY_ABSENT",
    );
    assert_code(&out, 2, "unset credential env");
    let err = stderr(&out);
    assert!(err.contains("E1904"), "{err}");
    assert!(err.contains("CAGE_TEST_SHEETS_KEY_ABSENT"), "{err}");

    // Not a Sheets spec at all → refused before credential resolution.
    let proj = root.join("bad_spec");
    write_project(&proj, "gsheet:noslash", "[remote.gsheets]\n");
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "malformed gsheet spec");
    let err = stderr(&out);
    assert!(err.contains("E1901"), "{err}");

    // A traversal-shaped spreadsheet id never reaches a URL.
    let proj = root.join("traversal");
    write_project(&proj, "gsheet:../etc/passwd", "[remote.gsheets]\n");
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "traversal-shaped id");
    let err = stderr(&out);
    assert!(err.contains("E1901"), "{err}");
}
