//! Process-level coverage of `cage schema-draft` (design §45 deferred
//! item): the introspection discipline surfaces through the CLI before
//! any wire is touched — the spec must be a `mysql:` / `pg:` source
//! spec (E1905), the DSN resolves from the env var named in
//! `[remote.<scheme>].dsn_env` (E1904 when undeclared or unset), and an
//! unreachable server reports E1901. No database server participates
//! here: the structural half of the draft (type classification, YAML
//! rendering, round-trip parse) is covered by the adapter's own tests,
//! and this suite pins the boundary the CLI owns.

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

/// The draft command needs cage.toml alone — no schema, no sources, no
/// build targets: `remote_section` carries the `[remote.*]` settings.
/// A profile is the one thing the config parser insists on.
fn write_project(dir: &Path, remote_section: &str) {
    let body = format!(
        r#"output_dir = "build"

[project]
name = "drafter"
version = "0.1.0"

{remote_section}
[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{{table}}.json"
"#
    );
    write(&dir.join("cage.toml"), &body);
}

#[test]
fn schema_draft_reports_spec_dsn_and_lookup_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // No [remote.mysql] at all → no dsn_env declared (E1904).
    let proj = root.join("no_config");
    write_project(&proj, "# (no remote section)\n");
    let out = run_cage(&["schema-draft", proj.to_str().unwrap(), "mysql:items"]);
    assert_code(&out, 2, "undeclared dsn_env");
    let err = stderr(&out);
    assert!(err.contains("E1904"), "{err}");
    assert!(err.contains("dsn_env"), "{err}");

    // dsn_env declared but the env var unset → E1904, and the message
    // names the env var, never a value.
    let proj = root.join("unset_env");
    write_project(
        &proj,
        "[remote.mysql]\ndsn_env = \"CAGE_TEST_DRAFT_DSN_UNSET\"\n",
    );
    let out = run_cage_without_env(
        &["schema-draft", proj.to_str().unwrap(), "mysql:items"],
        "CAGE_TEST_DRAFT_DSN_UNSET",
    );
    assert_code(&out, 2, "unset dsn env");
    let err = stderr(&out);
    assert!(err.contains("E1904"), "{err}");
    assert!(err.contains("CAGE_TEST_DRAFT_DSN_UNSET"), "{err}");

    // A spec that is not a mysql:/pg: source spec is refused before any
    // DSN work (E1905) — even with no [remote] section declared at all.
    let proj = root.join("bad_spec");
    write_project(&proj, "# (no remote section)\n");
    let out = run_cage(&["schema-draft", proj.to_str().unwrap(), "sqlite:items"]);
    assert_code(&out, 2, "not a db spec");
    let err = stderr(&out);
    assert!(err.contains("E1905"), "{err}");

    // A whitelisted spec with a set env var reaches the connect step:
    // an unreachable server reports E1901 (mysql).
    let proj = root.join("dead_mysql");
    write_project(
        &proj,
        "[remote.mysql]\ndsn_env = \"CAGE_TEST_DRAFT_DSN_DEAD\"\n",
    );
    let dsn = format!("mysql://127.0.0.1:{}/cage_test", dead_port());
    let out = run_cage_with_env(
        &["schema-draft", proj.to_str().unwrap(), "mysql:items"],
        "CAGE_TEST_DRAFT_DSN_DEAD",
        &dsn,
    );
    assert_code(&out, 2, "unreachable mysql");
    assert!(stderr(&out).contains("E1901"), "{}", stderr(&out));

    // Same on pg: over postgresql:// DSNs.
    let proj = root.join("dead_pg");
    write_project(
        &proj,
        "[remote.pg]\ndsn_env = \"CAGE_TEST_DRAFT_DSN_DEAD\"\n",
    );
    let dsn = format!("postgresql://127.0.0.1:{}/cage_test", dead_port());
    let out = run_cage_with_env(
        &["schema-draft", proj.to_str().unwrap(), "pg:items"],
        "CAGE_TEST_DRAFT_DSN_DEAD",
        &dsn,
    );
    assert_code(&out, 2, "unreachable postgresql");
    assert!(stderr(&out).contains("E1901"), "{}", stderr(&out));
}

/// The command is a project-rooted tool: no cage.toml is a configuration
/// error, and at least one spec is required by the parser.
#[test]
fn schema_draft_needs_a_project_and_at_least_one_spec() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let out = run_cage(&["schema-draft", root.to_str().unwrap(), "mysql:items"]);
    assert_code(&out, 2, "no cage.toml");
    assert!(!stderr(&out).is_empty(), "the error names the missing file");

    let proj = root.join("proj");
    write_project(&proj, "[remote.mysql]\ndsn_env = \"X\"\n");
    let out = run_cage(&["schema-draft", proj.to_str().unwrap()]);
    assert_code(&out, 2, "specs are required");
    let err = stderr(&out);
    assert!(
        err.contains("specs") || err.contains("required"),
        "clap usage error names the missing argument\n{err}"
    );
}
