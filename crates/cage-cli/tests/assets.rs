//! Process-level coverage of `cage assets` (design §50 B1): the text report,
//! the JSON baseline format, and the missing / changed / unlisted verify
//! triad against the real binary.

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

const CAGE_TOML: &str = r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "assets-demo"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#;

const ITEM_SCHEMA: &str = r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
enums: {}
"#;

const ITEM_DATA: &str = r#"{"Item": [{"id": 1, "name": "Sword"}]}"#;

/// cage.toml + schemas/item.yaml + config/item.json, plus one file each in
/// the two directories the inventory must never touch (build/, .cage-cache/).
fn write_project(root: &std::path::Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::create_dir_all(root.join("build")).unwrap();
    fs::create_dir_all(root.join(".cage-cache")).unwrap();
    fs::write(root.join("cage.toml"), CAGE_TOML).unwrap();
    fs::write(root.join("schemas/item.yaml"), ITEM_SCHEMA).unwrap();
    fs::write(root.join("config/item.json"), ITEM_DATA).unwrap();
    fs::write(root.join("build/Item.json"), "{}").unwrap();
    fs::write(root.join(".cage-cache/blob.bin"), "cache").unwrap();
}

// ---------------------------------------------------------------- report ---

#[test]
fn text_report_covers_all_three_classes() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["assets", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "text report");
    let text = stdout(&out);
    for needle in [
        "project: assets-demo",
        "assets: 3",
        "build",
        "rule",
        "source",
        "cage.toml",
        "schemas/item.yaml",
        "config/item.json",
        "blake3:",
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
    }
    // Never-inventoried directories stay out of the report.
    assert!(
        !text.contains("build/Item.json"),
        "leaked build output:\n{text}"
    );
    assert!(!text.contains(".cage-cache"), "leaked cache file:\n{text}");
}

#[test]
fn json_report_is_parseable_and_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let first = run_cage(&["assets", tmp.path().to_str().unwrap(), "--json"]);
    assert_code(&first, 0, "json report");
    let second = run_cage(&["assets", tmp.path().to_str().unwrap(), "--json"]);
    assert_eq!(
        stdout(&first),
        stdout(&second),
        "same tree must produce identical bytes"
    );
    let value: serde_json::Value =
        serde_json::from_str(&stdout(&first)).expect("baseline is valid JSON");
    assert_eq!(value["project"], "assets-demo");
    assert!(value["cage_version"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
    let assets = value["assets"].as_array().expect("assets is an array");
    assert_eq!(assets.len(), 3, "{assets:?}");
    let mut classes: Vec<&str> = assets
        .iter()
        .map(|a| a["class"].as_str().expect("class string"))
        .collect();
    classes.sort_unstable();
    assert_eq!(classes, ["build", "rule", "source"]);
    // Every record carries path + hash + size (the --check contract).
    for asset in assets {
        assert!(asset["path"].as_str().is_some_and(|p| !p.is_empty()));
        assert!(
            asset["hash"].as_str().is_some_and(|h| h.len() == 64),
            "hash is blake3 hex: {asset:?}"
        );
        assert!(asset["size"].as_u64().is_some_and(|s| s > 0));
    }
}

// --------------------------------------------------------------- baseline ---

#[test]
fn write_then_check_roundtrips_clean() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let baseline = tmp.path().join("assets.json");
    let wrote = run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--write",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&wrote, 0, "write");
    assert!(
        stdout(&wrote).contains("wrote 3 asset(s)"),
        "stdout:\n{}",
        stdout(&wrote)
    );

    let checked = run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&checked, 0, "clean check");
    assert!(
        stdout(&checked).contains("3 asset(s) verified"),
        "stdout:\n{}",
        stdout(&checked)
    );
}

#[test]
fn check_names_changed_deleted_and_unlisted_assets() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let baseline = tmp.path().join("assets.json");
    run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--write",
        baseline.to_str().unwrap(),
    ]);

    // Tamper: rewrite a source file (changed), drop the schema (missing),
    // and leave a stray file behind (unlisted).
    fs::write(tmp.path().join("config/item.json"), r#"{"Item": []}"#).unwrap();
    fs::remove_file(tmp.path().join("schemas/item.yaml")).unwrap();
    fs::write(tmp.path().join("config/stray.txt"), "x").unwrap();

    let checked = run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&checked, 1, "dirty check");
    let text = stdout(&checked);
    assert!(text.contains("2 asset(s) verified, 3 problem(s)"), "{text}");
    assert!(text.contains("changed   config/item.json"), "{text}");
    assert!(text.contains("missing   schemas/item.yaml"), "{text}");
    assert!(text.contains("unlisted  config/stray.txt"), "{text}");
}

// ----------------------------------------------------------------- errors ---

#[test]
fn check_rejects_an_unreadable_baseline_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let baseline = tmp.path().join("assets.json");
    fs::write(&baseline, "not json").unwrap();
    let out = run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&out, 2, "garbage baseline");
    assert!(
        stderr(&out).contains("invalid asset inventory"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_rejects_a_missing_baseline_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--check",
        tmp.path().join("nope.json").to_str().unwrap(),
    ]);
    assert_code(&out, 2, "missing baseline");
    assert!(
        stderr(&out).contains("cannot read"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_rejects_a_foreign_baseline() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let baseline = tmp.path().join("assets.json");
    run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--write",
        baseline.to_str().unwrap(),
    ]);
    // The same inventory, attributed to another project.
    let text = fs::read_to_string(&baseline).unwrap();
    let foreign = text.replace("assets-demo", "other-project");
    fs::write(&baseline, foreign).unwrap();
    let out = run_cage(&[
        "assets",
        tmp.path().to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&out, 2, "foreign baseline");
    assert!(
        stderr(&out).contains("baseline is for project 'other-project'"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn assets_exits_2_without_a_project_config() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run_cage(&["assets", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "no config");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}
