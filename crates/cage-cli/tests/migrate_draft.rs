//! Process-level coverage of `cage migrate-draft` (design §46): two schema
//! versions in, a reviewable rule draft out — mechanically safe transforms
//! become steps, anything a structural diff cannot know stays a `# TODO` —
//! and the drafted file then runs through `cage migrate --all --write` to
//! migrate real data. An all-TODO draft renders `steps: []`, which the
//! rule parser refuses (E2001) until the author fills it in.

use std::fs;
use std::path::{Path, PathBuf};
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

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

const PROJECT_TOML: &str = r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "draftproj"
version = "1.0.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
"#;

const SCHEMA_V1: &str = r#"metadata:
  version: 1.0.0
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      hp:
        name: hp
        type: { kind: Int32 }
      name:
        name: name
        type: { kind: String }
        required: true
  Boss:
    name: Boss
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
"#;

const SCHEMA_V2: &str = r#"metadata:
  version: 2.0.0
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      hp:
        name: hp
        type: { kind: Int64 }
      rarity:
        name: rarity
        type: { kind: String }
        default: "common"
enums: {}
"#;

const DATA: &str = r#"{
  "Item": [
    { "id": 1, "hp": 10, "name": "sword" },
    { "id": 2, "hp": 20, "name": "shield" }
  ]
}
"#;

fn setup_project(base: &Path) -> PathBuf {
    let proj = base.join("proj");
    write(&proj.join("cage.toml"), PROJECT_TOML);
    write(&proj.join("schema.yaml"), SCHEMA_V1);
    write(&proj.join("config/item.json"), DATA);
    write(&proj.join("schema.v2.yaml"), SCHEMA_V2);
    proj
}

#[test]
fn draft_runs_through_migrate_write() {
    let dir = tempfile::tempdir().unwrap();
    let proj = setup_project(dir.path());
    let draft_path = proj.join("migrations/0001.yaml");
    let schema_v1 = proj.join("schema.yaml");
    let schema_v2 = proj.join("schema.v2.yaml");

    // 1. Draft from the two schema files.
    let out = run_cage(&[
        "migrate-draft",
        schema_v1.to_str().unwrap(),
        schema_v2.to_str().unwrap(),
        "--from",
        "1.0.0",
        "--to",
        "2.0.0",
        "-o",
        draft_path.to_str().unwrap(),
    ]);
    assert_code(&out, 0, "migrate-draft");
    assert!(
        stdout(&out).contains("3 step(s)"),
        "expected 3 steps in summary, got: {}",
        stdout(&out)
    );
    let draft = fs::read_to_string(&draft_path).unwrap();
    assert!(
        draft.contains("from: '1.0.0'") && draft.contains("to: '2.0.0'"),
        "version headers must be quoted and present:\n{draft}"
    );
    for transform in ["remove_field", "widen_type", "set_default"] {
        assert!(
            draft.contains(transform),
            "expected a {transform} step in the draft:\n{draft}"
        );
    }
    // The dropped table is a TODO, not a guess — renames are never
    // invented by a structural diff.
    assert!(
        draft.contains("# TODO") && draft.contains("Boss"),
        "expected a TODO for the dropped Boss table:\n{draft}"
    );

    // 2. Swap the project schema to v2 and migrate for real.
    write(&proj.join("schema.yaml"), SCHEMA_V2);
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--all", "--write"]);
    assert_code(&out, 0, "migrate --all --write");

    // 3. The data moved: `name` is gone, `rarity` is backfilled, `hp`
    // survives the Int32 → Int64 widening. (Plain string checks — the
    // JSON writer's exact layout is covered by its own tests.)
    let data = fs::read_to_string(proj.join("config/item.json")).unwrap();
    assert!(!data.contains("\"name\""), "name must be gone:\n{data}");
    assert!(data.contains("\"common\""), "default backfilled:\n{data}");
    assert!(
        data.contains("10") && data.contains("20"),
        "hp values survive the widening:\n{data}"
    );
}

#[test]
fn todo_only_draft_refuses_to_parse_until_filled() {
    let dir = tempfile::tempdir().unwrap();
    let proj = setup_project(dir.path());
    // v2 identical to v1 except the Boss table is dropped: nothing a
    // structural diff can safely transform — an all-TODO draft.
    let v2 = r#"metadata:
  version: 2.0.0
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      hp:
        name: hp
        type: { kind: Int32 }
      name:
        name: name
        type: { kind: String }
        required: true
enums: {}
"#;
    let schema_v1 = proj.join("schema.yaml");
    let schema_v2 = proj.join("schema.v2.yaml");
    write(&schema_v2, v2);
    let draft_path = proj.join("migrations/0001.yaml");

    let out = run_cage(&[
        "migrate-draft",
        schema_v1.to_str().unwrap(),
        schema_v2.to_str().unwrap(),
        "--from",
        "1.0.0",
        "--to",
        "2.0.0",
        "-o",
        draft_path.to_str().unwrap(),
    ]);
    assert_code(&out, 0, "all-TODO migrate-draft still exits 0");
    assert!(
        stdout(&out).contains("0 step(s)") && stdout(&out).contains("steps: [] must be filled"),
        "expected the empty-steps warning, got: {}",
        stdout(&out)
    );
    let draft = fs::read_to_string(proj.join("migrations/0001.yaml")).unwrap();
    assert!(
        draft.contains("steps:\n  []\n"),
        "all-TODO draft renders the empty-steps marker:\n{draft}"
    );

    // And the rule parser refuses it (E2001) until steps are written.
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--all"]);
    assert_code(&out, 1, "migrate over an all-TODO draft fails");
    assert!(
        stderr(&out).contains("E2001"),
        "expected the no-steps diagnostic, got: {}",
        stderr(&out)
    );
}
