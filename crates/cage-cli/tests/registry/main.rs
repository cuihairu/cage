//! Process-level coverage of the local Configuration Registry (R1 + R2):
//! `cage registry publish` (fresh build → self-verifying snapshot → entry,
//! idempotent identical re-publish, E1801 version conflict), `cage registry
//! list`, consumer-side source resolution — `registry:<pkg>[@<ver>]`
//! source roots resolve to the entry's data artifacts (manifest-driven
//! table identity), refusing tampered entries (E1803) and unresolved
//! references (E1802) — and the R2 layer: `[dependencies]` pins (version
//! ranges pick the max satisfying version; an explicit `@<ver>` outside
//! its pin is rejected) and `schema_path: registry:<pkg>` loading the
//! entry's published schema.

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

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// One Item row per version, so a consumer's built artifact proves which
/// entry version it resolved.
fn item_rows(marker: &str) -> String {
    format!(
        r#"{{
  "Item": [
    {{ "id": 1, "name": "{marker}" }}
  ]
}}
"#
    )
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

fn write_publisher(root: &Path) {
    write(
        &root.join("pub/cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "common"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"

[[profiles.client.targets]]
format = "csv"
output_dir = "build/client/csv"
file_template = "{table}.csv"
"#,
    );
    write(&root.join("pub/schema.yaml"), SCHEMA);
    write(&root.join("pub/config/item.json"), &item_rows("Sword"));
}

fn write_consumer(root: &Path, name: &str, source_root: &str) {
    write_consumer_full(root, name, source_root, None, "schema.yaml");
}

/// Full-shape consumer: `source_root` lands in `[source_roots].main`,
/// `deps` (a version-requirement string) becomes a `[dependencies]` pin on
/// the `common` package, and `schema_path` picks between a local
/// `schema.yaml` and the entry schema (`registry:common`, R2 — the
/// consumer then ships no schema file of its own).
fn write_consumer_full(
    root: &Path,
    name: &str,
    source_root: &str,
    deps: Option<&str>,
    schema_path: &str,
) {
    let deps_block = deps
        .map(|d| format!("\n[dependencies]\ncommon = \"{d}\"\n"))
        .unwrap_or_default();
    let body = r#"output_dir = "build"
schema_path = "SCHEMA_PATH"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "registry:PLACEHOLDER"

[registry]
path = "../reg"
DEPS_LINE
[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
    .replace("registry:PLACEHOLDER", source_root)
    .replace("SCHEMA_PATH", schema_path)
    .replace("DEPS_LINE\n", &deps_block);
    write(&root.join(format!("{name}/cage.toml")), &body);
    if !schema_path.starts_with("registry:") {
        write(&root.join(format!("{name}/schema.yaml")), SCHEMA);
    }
}

/// sha256-free determinism probe: sorted (path, size) digest of a tree.
fn tree_fingerprint(root: &Path) -> String {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let meta = fs::metadata(&p).unwrap();
                entries.push(format!(
                    "{}:{}",
                    p.strip_prefix(root).unwrap().display(),
                    meta.len()
                ));
            }
        }
    }
    entries.sort();
    entries.join("\n")
}

mod export;
mod import;
mod publish;
mod resolve;
