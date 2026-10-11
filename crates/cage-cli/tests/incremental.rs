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

/// Project with json + msgpack data targets; the json target's `pretty`
/// option visibly changes its bytes (pretty vs compact).
fn write_project_two_targets(root: &Path) {
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

[[profiles.client.targets]]
format = "msgpack"
output_dir = "build/client2"
file_template = "{table}.msgpack"
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

    fs::write(
        root.join("config/item.json"),
        r#"{"Item": [{"name": "Sword", "id": 1, "price": 100}]}"#,
    )
    .unwrap();
}

fn mtime(root: &Path, rel: &str) -> std::time::SystemTime {
    fs::metadata(root.join(rel)).unwrap().modified().unwrap()
}

/// Layer 3: a target's option change regenerates only that target's
/// artifacts (from the full document); the untouched target's artifacts are
/// carried over byte-for-byte, and the manifest converges to a full build.
#[test]
fn target_option_change_regenerates_only_that_target() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_two_targets(root);

    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json_before = fs::read(root.join("build/client/Item.json")).unwrap();
    let msgpack_mtime = mtime(root, "build/client2/Item.msgpack");

    // Flip json pretty: compact output, bytes change.
    let toml = fs::read_to_string(root.join("cage.toml")).unwrap();
    let toml = toml.replace(
        "file_template = \"{table}.json\"\n",
        "file_template = \"{table}.json\"\n\n[profiles.client.targets.options]\npretty = false\n",
    );
    fs::write(root.join("cage.toml"), toml).unwrap();

    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("up to date"),
        "target change must rebuild: {stdout}"
    );

    // json regenerated (compact); msgpack carried untouched.
    let json_after = fs::read(root.join("build/client/Item.json")).unwrap();
    assert_ne!(
        json_before, json_after,
        "json bytes must change with pretty"
    );
    assert!(
        !json_after.windows(2).any(|w| w == b"\n  "),
        "compact json must not be pretty-printed: {json_after:?}"
    );
    assert_eq!(
        mtime(root, "build/client2/Item.msgpack"),
        msgpack_mtime,
        "unchanged target's artifact must be carried, not rewritten"
    );

    // Convergence: the incremental manifest equals a fresh full build's.
    let incremental_manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    let out = build(root, false);
    assert!(out.status.success());
    assert_eq!(
        incremental_manifest,
        fs::read_to_string(root.join("build/manifest.json")).unwrap()
    );
}

/// Layer 3: removing a target deletes its stale artifacts from disk and
/// drops them from the manifest; the remaining target carries untouched.
#[test]
fn target_removal_deletes_stale_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_two_targets(root);

    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json_mtime = mtime(root, "build/client/Item.json");
    assert!(root.join("build/client2/Item.msgpack").is_file());

    // Drop the msgpack target from the profile.
    let toml = fs::read_to_string(root.join("cage.toml")).unwrap();
    let toml = toml.replace(
        r#"
[[profiles.client.targets]]
format = "msgpack"
output_dir = "build/client2"
file_template = "{table}.msgpack"
"#,
        "",
    );
    fs::write(root.join("cage.toml"), toml).unwrap();

    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("up to date"),
        "target removal must rebuild: {stdout}"
    );

    // Stale artifacts deleted; the remaining target carried untouched.
    assert!(
        !root.join("build/client2/Item.msgpack").exists(),
        "removed target's artifact must be deleted"
    );
    assert_eq!(
        mtime(root, "build/client/Item.json"),
        json_mtime,
        "unchanged target's artifact must be carried, not rewritten"
    );
    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(
        !manifest.contains("client2"),
        "manifest must not record the removed target: {manifest}"
    );

    // Convergence: a fresh full build after the removal matches.
    let incremental_manifest = manifest;
    let out = build(root, false);
    assert!(out.status.success());
    assert_eq!(
        incremental_manifest,
        fs::read_to_string(root.join("build/manifest.json")).unwrap()
    );
}

/// Layer 3 stale cleanup: a stale artifact path occupied by a directory
/// cannot be removed — the build warns on stderr but still succeeds, and
/// the manifest drops the removed target regardless.
#[test]
fn stale_artifact_directory_warns_but_build_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_two_targets(root);

    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("build/client2/Item.msgpack").is_file());

    // Occupy the future-stale artifact path with a directory.
    let stale = root.join("build/client2/Item.msgpack");
    fs::remove_file(&stale).unwrap();
    fs::create_dir(&stale).unwrap();

    // Drop the msgpack target from the profile.
    let toml = fs::read_to_string(root.join("cage.toml")).unwrap();
    let toml = toml.replace(
        r#"
[[profiles.client.targets]]
format = "msgpack"
output_dir = "build/client2"
file_template = "{table}.msgpack"
"#,
        "",
    );
    fs::write(root.join("cage.toml"), toml).unwrap();

    let out = build(root, true);
    assert!(
        out.status.success(),
        "undremovable stale artifact must not fail the build: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("could not remove stale artifact 'build/client2/Item.msgpack'"),
        "must warn about the directory in the way: {stderr}"
    );
    assert!(stale.is_dir(), "the occupying directory must survive");
    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(
        !manifest.contains("client2"),
        "manifest must not record the removed target: {manifest}"
    );
}

/// Layer 3: adding a target generates its artifacts while the existing
/// target's artifacts carry untouched.
#[test]
fn target_addition_generates_only_new_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root); // json-only project

    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json_mtime = mtime(root, "build/client/Item.json");

    // Add a msgpack target.
    let toml = fs::read_to_string(root.join("cage.toml")).unwrap();
    let toml = toml.replace(
        "file_template = \"{table}.json\"\n",
        "file_template = \"{table}.json\"\n\n[[profiles.client.targets]]\nformat = \"msgpack\"\noutput_dir = \"build/client2\"\nfile_template = \"{table}.msgpack\"\n",
    );
    fs::write(root.join("cage.toml"), toml).unwrap();

    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("up to date"),
        "target addition must rebuild: {stdout}"
    );

    // New artifacts exist; the existing target carried untouched.
    assert!(root.join("build/client2/Item.msgpack").is_file());
    assert_eq!(
        mtime(root, "build/client/Item.json"),
        json_mtime,
        "unchanged target's artifact must be carried, not rewritten"
    );

    // Convergence: the incremental manifest equals a fresh full build's.
    let incremental_manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    let out = build(root, false);
    assert!(out.status.success());
    assert_eq!(
        incremental_manifest,
        fs::read_to_string(root.join("build/manifest.json")).unwrap()
    );
}

/// Legacy manifests (no target records) take one full build to migrate,
/// then target propagation participates — no silent skip on a target edit.
#[test]
fn legacy_manifest_migrates_with_one_full_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_two_targets(root);

    let out = build(root, false);
    assert!(
        out.status.success(),
        "seed build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Strip the target records: a pre-1.1.0 manifest.
    let manifest_path = root.join("build/manifest.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replace(
        r#""targets": [
    {
      "format": "json",
      "output_dir": "build/client",
      "hash": ""#,
        r#""targets": [
    {
      "format": "json",
      "output_dir": "build/client",
      "hash_legacy": ""#,
    );
    // Simpler: drop the whole "targets" array via serde round-trip.
    let mut value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    value.as_object_mut().unwrap().remove("targets");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&value).unwrap(),
    )
    .unwrap();

    // A target edit on the legacy manifest must NOT be skipped: the full
    // build migrates the manifest (records targets again).
    let toml = fs::read_to_string(root.join("cage.toml")).unwrap();
    let toml = toml.replace(
        "file_template = \"{table}.msgpack\"\n",
        "file_template = \"{table}.msgpack\"\n\n[profiles.client.targets.options]\nsort_keys = false\n",
    );
    fs::write(root.join("cage.toml"), toml).unwrap();

    let out = build(root, true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("up to date"),
        "legacy manifest must not skip a target edit: {stdout}"
    );
    let migrated = fs::read_to_string(&manifest_path).unwrap();
    assert!(
        migrated.contains("\"targets\""),
        "migrated manifest must record targets: {migrated}"
    );
    let value: serde_json::Value = serde_json::from_str(&migrated).unwrap();
    assert!(
        value["targets"].as_array().is_some_and(|t| !t.is_empty()),
        "migrated manifest must carry target records: {migrated}"
    );
}
