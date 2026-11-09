//! Process-level coverage of the `cage` CLI surface: every subcommand's
//! happy path plus every error path that should exit 2 (bad project root,
//! missing cage.toml, invalid schema, unknown profile, unsupported target
//! format, invalid --level, unreadable manifests, unwritable outputs) and
//! every validation failure that should exit 1 (type errors,
//! warnings-as-errors).

use std::fs;
use std::path::Path;
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
name = "clipaths"
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
    description: An inventory item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
enums: {}
"#;

const ITEM_DATA: &str = r#"{"Item": [{"id": 1, "name": "Sword"}]}"#;

/// Minimal valid project: cage.toml + schemas/item.yaml + config/item.json.
fn write_project(root: &Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(root.join("cage.toml"), CAGE_TOML).unwrap();
    fs::write(root.join("schemas/item.yaml"), ITEM_SCHEMA).unwrap();
    fs::write(root.join("config/item.json"), ITEM_DATA).unwrap();
}

/// Project with no schema and no sources: validation is trivially clean.
fn write_empty_project(root: &Path, cage_toml: &str) {
    fs::write(root.join("cage.toml"), cage_toml).unwrap();
}

mod build;
mod check;
mod diff;
mod gen;
mod inspect;
mod scenario;
