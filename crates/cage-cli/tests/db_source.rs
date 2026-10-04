//! Process-level coverage of the MySQL / PostgreSQL source (S2, design
//! §45): the load-time discipline surfaces through the CLI before any
//! wire is touched — statements and names pass the static read-only
//! whitelist (E1905) first, then the DSN resolves from the env var named
//! in `[remote.<scheme>].dsn_env` (E1904 when undeclared or unset), and
//! only a whitelisted SELECT with a set env var reaches the connect step,
//! where an unreachable server reports E1901. No database server
//! participates here: everything up to the socket is observable at the
//! process boundary, and the bytes-onward half (canonical JSON → cache →
//! standard parse) is covered by the adapter's own tests and the S1 CLI
//! suite that exercises the same cache-and-parse path.

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

/// Run `cage` with one env var explicitly removed — proves the DSN env is
/// genuinely absent for the child, regardless of the developer shell.
fn run_cage_without_env(args: &[&str], removed: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .env_remove(removed)
        .output()
        .unwrap()
}

/// Run `cage` with one env var set just for the child process.
fn run_cage_with_env(args: &[&str], name: &str, value: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .env(name, value)
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

/// A port with nothing listening on it (bound, then released).
fn dead_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
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

/// Consumer fixture: one `[source_roots]` entry pointing at `source`,
/// plus the verbatim `remote_section` TOML for `[remote.*]` settings.
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
fn db_source_load_time_failures_report_their_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // No [remote.mysql] at all → no dsn_env declared (E1904)
    let proj = root.join("no_config");
    write_project(&proj, "mysql:items", "# (no remote section)\n");
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "undeclared dsn_env");
    let err = stderr(&out);
    assert!(err.contains("E1904"), "{err}");
    assert!(err.contains("dsn_env"), "{err}");

    // dsn_env declared but the env var unset → E1904; the env var NAME
    // lives in cage.toml, the DSN itself never does.
    let proj = root.join("env_unset");
    write_project(
        &proj,
        "mysql:items",
        "[remote.mysql]\ndsn_env = \"CAGE_TEST_DB_DSN_ABSENT\"\n",
    );
    let out = run_cage_without_env(
        &["check", proj.to_str().unwrap()],
        "CAGE_TEST_DB_DSN_ABSENT",
    );
    assert_code(&out, 2, "unset dsn env");
    let err = stderr(&out);
    assert!(err.contains("E1904"), "{err}");
    assert!(err.contains("CAGE_TEST_DB_DSN_ABSENT"), "{err}");

    // Whitelist fires before credentials: an UPDATE named query fails
    // E1905 even though no DSN is declared at all.
    let proj = root.join("bad_query");
    write_project(
        &proj,
        "mysql:bad",
        "[remote.mysql]\n[remote.mysql.queries]\nbad = \"UPDATE items SET price = 0\"\n",
    );
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "non-SELECT named query");
    let err = stderr(&out);
    assert!(err.contains("E1905"), "{err}");

    // A name that is neither a named query nor a safe table → E1905,
    // again before any credential or connection work.
    let proj = root.join("bad_name");
    write_project(
        &proj,
        "mysql:items; DROP TABLE items",
        "[remote.mysql]\ndsn_env = \"CAGE_TEST_DB_DSN_ABSENT\"\n",
    );
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "unsafe table name");
    let err = stderr(&out);
    assert!(err.contains("E1905"), "{err}");

    // Same discipline on the pg: scheme.
    let proj = root.join("bad_pg");
    write_project(
        &proj,
        "pg:wipe",
        "[remote.pg]\n[remote.pg.queries]\nwipe = \"DELETE FROM items\"\n",
    );
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 2, "non-SELECT pg named query");
    assert!(stderr(&out).contains("E1905"), "{}", stderr(&out));
}

#[test]
fn db_source_unreachable_servers_report_e1901() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Whitelist-clean query, DSN set, server unreachable → E1901 (mysql)
    let proj = root.join("dead_mysql");
    write_project(
        &proj,
        "mysql:items",
        "[remote.mysql]\ndsn_env = \"CAGE_TEST_DB_DSN_DEAD\"\n",
    );
    let dsn = format!("mysql://127.0.0.1:{}/cage_test", dead_port());
    let out = run_cage_with_env(
        &["check", proj.to_str().unwrap()],
        "CAGE_TEST_DB_DSN_DEAD",
        &dsn,
    );
    assert_code(&out, 2, "unreachable mysql");
    let err = stderr(&out);
    assert!(err.contains("E1901"), "{err}");

    // Same on pg: over postgresql:// DSNs.
    let proj = root.join("dead_pg");
    write_project(
        &proj,
        "pg:items",
        "[remote.pg]\ndsn_env = \"CAGE_TEST_DB_DSN_DEAD\"\n",
    );
    let dsn = format!("postgresql://127.0.0.1:{}/cage_test", dead_port());
    let out = run_cage_with_env(
        &["check", proj.to_str().unwrap()],
        "CAGE_TEST_DB_DSN_DEAD",
        &dsn,
    );
    assert_code(&out, 2, "unreachable postgresql");
    let err = stderr(&out);
    assert!(err.contains("E1901"), "{err}");

    // A DSN the driver cannot even parse never gets to the wire (mysql).
    let proj = root.join("bad_dsn");
    write_project(
        &proj,
        "mysql:items",
        "[remote.mysql]\ndsn_env = \"CAGE_TEST_DB_DSN_BAD\"\n",
    );
    let out = run_cage_with_env(
        &["check", proj.to_str().unwrap()],
        "CAGE_TEST_DB_DSN_BAD",
        "not-a-dsn",
    );
    assert_code(&out, 2, "malformed mysql DSN");
    let err = stderr(&out);
    assert!(err.contains("E1901"), "{err}");
}
