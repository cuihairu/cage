//! `cage build --incremental`: when the previous manifest recorded the same
//! schema/source hashes (same profile) and every artifact is still on disk,
//! regeneration is skipped; any input change or missing artifact rebuilds.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write_project(root: &Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();

    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "incremental"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client"
file_template = "{table}.json"
"#,
    )
    .unwrap();

    fs::write(
        root.join("schemas/item.yaml"),
        r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
      price: { name: price, type: { kind: Int32 }, min: 0 }
enums: {}
"#,
    )
    .unwrap();

    write_source(
        root,
        r#"{"Item": [{"id": 1, "name": "Sword", "price": 100}]}"#,
    );
}

fn write_source(root: &Path, content: &str) {
    fs::write(root.join("config/item.json"), content).unwrap();
}

fn build(root: &Path, incremental: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cage"));
    cmd.args(["build", root.to_str().unwrap(), "--profile", "client"]);
    if incremental {
        cmd.arg("--incremental");
    }
    cmd.output().unwrap()
}

fn artifact_mtime(root: &Path) -> std::time::SystemTime {
    fs::metadata(root.join("build/client/Item.json"))
        .unwrap()
        .modified()
        .unwrap()
}

#[test]
fn incremental_skips_rebuild_until_inputs_change() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    // Full build seeds the manifest.
    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let seed_mtime = artifact_mtime(root);

    // Unchanged inputs + intact artifacts → skip (no rewrite, "up to date").
    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("up to date"),
        "expected skip, got: {stdout}"
    );
    assert_eq!(
        artifact_mtime(root),
        seed_mtime,
        "artifact was rewritten on skip"
    );

    // Changed source → rebuild despite --incremental.
    write_source(
        root,
        r#"{"Item": [{"id": 1, "name": "Sword", "price": 50}]}"#,
    );
    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("up to date"),
        "expected rebuild, got: {stdout}"
    );
    let rebuilt = fs::read_to_string(root.join("build/client/Item.json")).unwrap();
    assert!(
        rebuilt.contains("50"),
        "artifact content not refreshed: {rebuilt}"
    );

    // Missing artifact (deleted) → rebuild restores it.
    let before_delete = fs::read_to_string(root.join("build/client/Item.json")).unwrap();
    fs::remove_file(root.join("build/client/Item.json")).unwrap();
    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("up to date"),
        "deleted artifact must trigger rebuild"
    );
    assert_eq!(
        fs::read_to_string(root.join("build/client/Item.json")).unwrap(),
        before_delete
    );
}
