//! Process-level coverage of `cage init`: the flag-driven scaffold must
//! produce a project that checks and builds with no further edits, the
//! re-run must refuse to overwrite, and unknown flag values must be a
//! usage error.

use std::fs;
use std::process::Command;

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

#[test]
fn flag_mode_scaffold_checks_and_builds() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo");
    let out = run_cage(&[
        "init",
        dir.to_str().unwrap(),
        "--name",
        "demo",
        "--source",
        "csv",
        "--envs",
        "four",
        "--targets",
        "json,csv",
    ]);
    assert_code(&out, 0, "flag-mode scaffold");

    assert!(dir.join("cage.toml").is_file());
    assert!(dir.join("schemas/item.yaml").is_file());
    assert!(dir.join("config/Item.csv").is_file());
    assert!(dir.join("README.md").is_file());
    assert!(dir.join(".gitignore").is_file());

    // Zero edits from the user: the scaffold checks clean.
    assert_code(&run_cage(&["check", dir.to_str().unwrap()]), 0, "check");
    // And builds under a declared environment.
    assert_code(
        &run_cage(&["build", dir.to_str().unwrap(), "--env", "dev"]),
        0,
        "build --env dev",
    );
    assert!(dir.join("build/client/json").is_dir());
    // An undeclared environment is a usage error, same as the examples.
    assert_code(
        &run_cage(&["build", dir.to_str().unwrap(), "--env", "qa"]),
        2,
        "build --env qa",
    );
}

#[test]
fn rerun_refuses_and_preserves_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo");
    assert_code(
        &run_cage(&["init", dir.to_str().unwrap(), "--name", "demo"]),
        0,
        "first scaffold",
    );
    let before = fs::read(dir.join("cage.toml")).unwrap();

    let out = run_cage(&["init", dir.to_str().unwrap(), "--name", "demo-again"]);
    assert_code(&out, 1, "second scaffold in the same directory");
    assert!(
        stderr(&out).contains("拒绝覆盖"),
        "stderr: {}",
        stderr(&out)
    );
    // Nothing was rewritten.
    assert_eq!(fs::read(dir.join("cage.toml")).unwrap(), before);
    assert!(
        !fs::read_to_string(dir.join("cage.toml"))
            .unwrap()
            .contains("demo-again"),
        "cage.toml must not be rewritten"
    );
}

#[test]
fn no_envs_project_builds_without_env() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("plain");
    assert_code(
        &run_cage(&[
            "init",
            dir.to_str().unwrap(),
            "--name",
            "plain",
            "--envs",
            "none",
        ]),
        0,
        "scaffold without envs",
    );
    assert!(!fs::read_to_string(dir.join("schemas/item.yaml"))
        .unwrap()
        .contains("env_overrides"));
    assert_code(&run_cage(&["check", dir.to_str().unwrap()]), 0, "check");
    assert_code(
        &run_cage(&["build", dir.to_str().unwrap()]),
        0,
        "build without --env",
    );
    // No declared environments, so --env is a usage error.
    assert_code(
        &run_cage(&["build", dir.to_str().unwrap(), "--env", "dev"]),
        2,
        "build --env dev on a project without envs",
    );
}

#[test]
fn piped_stdin_without_flags_uses_defaults() {
    // No TTY and no flags: defaults apply, the command never hangs on a
    // prompt.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("defaults");
    let out = Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(["init", dir.to_str().unwrap()])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_code(&out, 0, "init with piped stdin");
    assert!(
        dir.join("config/Item.json").is_file(),
        "json sample expected"
    );
    assert!(
        fs::read_to_string(dir.join("schemas/item.yaml"))
            .unwrap()
            .contains("env_overrides"),
        "four-env set expected"
    );
    assert_code(&run_cage(&["check", dir.to_str().unwrap()]), 0, "check");
}

#[test]
fn no_examples_writes_a_skeleton() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("skeleton");
    assert_code(
        &run_cage(&[
            "init",
            dir.to_str().unwrap(),
            "--name",
            "skeleton",
            "--no-examples",
        ]),
        0,
        "skeleton scaffold",
    );
    assert!(dir.join("cage.toml").is_file());
    assert!(dir.join("README.md").is_file());
    assert!(!dir.join("config/Item.json").exists());
    assert!(!dir.join("config/Item.csv").exists());
    // The empty skeleton still loads and checks.
    assert_code(
        &run_cage(&["check", dir.to_str().unwrap()]),
        0,
        "check skeleton",
    );
}

#[test]
fn remote_source_kinds_keep_a_local_sample() {
    for kind in ["excel", "sheets", "db", "http"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("remote");
        let out = run_cage(&["init", dir.to_str().unwrap(), "--source", kind]);
        assert_code(&out, 0, &format!("scaffold --source {kind}"));
        // A local text sample so the project checks clean out of the box.
        assert!(dir.join("config/Item.csv").is_file());
        assert_code(
            &run_cage(&["check", dir.to_str().unwrap()]),
            0,
            &format!("check --source {kind}"),
        );
    }
}

#[test]
fn yaml_source_sample_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("yaml");
    assert_code(
        &run_cage(&[
            "init",
            dir.to_str().unwrap(),
            "--name",
            "yaml",
            "--source",
            "yaml",
        ]),
        0,
        "yaml scaffold",
    );
    assert!(dir.join("config/Item.yaml").is_file());
    assert_code(
        &run_cage(&["check", dir.to_str().unwrap()]),
        0,
        "check yaml",
    );
    assert_code(
        &run_cage(&["build", dir.to_str().unwrap(), "--env", "dev"]),
        0,
        "build yaml",
    );
}

#[test]
fn unknown_flag_values_are_usage_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("bad");
    for (flag, value) in [
        ("--source", "parquet"),
        ("--envs", "five"),
        ("--targets", "json,parquet"),
        ("--template", "rust"),
    ] {
        let out = run_cage(&["init", dir.to_str().unwrap(), flag, value]);
        assert_code(&out, 2, &format!("init {flag} {value}"));
        assert!(stderr(&out).contains("用法"), "stderr: {}", stderr(&out));
    }
    // Nothing was scaffolded by the failing attempts.
    assert!(!dir.join("cage.toml").exists());
}

#[test]
fn git_flag_initializes_a_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    let out = run_cage(&["init", dir.to_str().unwrap(), "--name", "repo", "--git"]);
    assert_code(&out, 0, "init --git");
    if Command::new("git").arg("--version").output().is_ok() {
        assert!(
            dir.join(".git").exists(),
            "git init should have created .git"
        );
    }
}
