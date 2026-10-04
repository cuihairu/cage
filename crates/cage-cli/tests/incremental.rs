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

/// Three-table project for layer-2 tests: `Drop` references `Item`
/// (so a change to `Item` must propagate to `Drop`), `Skill` stands alone
/// (so it must be carried over untouched).
fn write_project_three_tables(root: &Path) {
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
        root.join("schemas/main.yaml"),
        r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
      price: { name: price, type: { kind: Int32 }, min: 0 }
  Drop:
    name: Drop
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      amount: { name: amount, type: { kind: Int32 }, required: true }
      item: { name: item, type: { kind: Int32 }, required: true,
              reference: { table: Item, field: id } }
  Skill:
    name: Skill
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
enums: {}
"#,
    )
    .unwrap();

    write_table(
        root,
        "Item",
        r#"[{"id": 1, "name": "Sword", "price": 100}]"#,
    );
    write_table(root, "Drop", r#"[{"id": 1, "amount": 2, "item": 1}]"#);
    write_table(root, "Skill", r#"[{"id": 1, "name": "Slash"}]"#);
}

fn write_table(root: &Path, table: &str, rows: &str) {
    let content = format!("{{\"{table}\": {rows}}}");
    fs::write(root.join(format!("config/{table}.json")), content).unwrap();
}

#[test]
fn layer2_rebuilds_affected_tables_and_carries_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_three_tables(root);

    // Seed full build captures every artifact's bytes.
    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let seed_drop = fs::read(root.join("build/client/Drop.json")).unwrap();
    let seed_skill = fs::read(root.join("build/client/Skill.json")).unwrap();

    // Change one row of Item only → Item and its dependent Drop must
    // regenerate; standalone Skill must be carried over from disk.
    write_table(root, "Item", r#"[{"id": 1, "name": "Sword", "price": 50}]"#);
    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("unchanged via dependency graph"),
        "expected layer-2 report, got: {stdout}"
    );
    assert!(
        stdout.contains("2 regenerated, 1 unchanged"),
        "expected 2 regenerated + 1 carried, got: {stdout}"
    );

    assert!(
        fs::read_to_string(root.join("build/client/Item.json"))
            .unwrap()
            .contains("\"price\": 50"),
        "touched table not regenerated"
    );
    assert_eq!(
        fs::read(root.join("build/client/Skill.json")).unwrap(),
        seed_skill,
        "untouched standalone table must be carried byte-identical"
    );
    // Drop depends on Item → recomputed (deterministic output keeps bytes).
    assert_eq!(
        fs::read(root.join("build/client/Drop.json")).unwrap(),
        seed_drop,
        "referrer of a changed table must be in the affected set"
    );

    // Convergence: the incremental manifest must equal a fresh full build's
    // manifest byte-for-byte (same inputs → same manifest bytes).
    let inc_manifest = fs::read(root.join("build/manifest.json")).unwrap();
    let out = build(root, false);
    assert!(out.status.success());
    let full_manifest = fs::read(root.join("build/manifest.json")).unwrap();
    assert_eq!(
        inc_manifest, full_manifest,
        "incremental manifest must converge to the full-build manifest"
    );
    assert_eq!(
        fs::read(root.join("build/client/Skill.json")).unwrap(),
        seed_skill,
        "carried bytes must match a full build's bytes"
    );
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
