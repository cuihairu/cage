//! Process-level coverage of env-ized packaging (design §48): `cage
//! snapshot --env` and `cage registry publish --env` resolve
//! `env_overrides` before validating, tag the packed bytes, and keep the
//! one-version-one-pack discipline — a different environment packs
//! differently, so the same version under another environment is an E1801
//! conflict, exactly like any other byte change.

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

/// Hero table with two declared environments: `dev` relaxes the base
/// schema (`hp` becomes optional), `prod` tightens it (`hp` required and
/// `min: 100`, `rarity` also accepts `epic`).
fn write_project(root: &Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();

    write(
        &root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "envs"
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
    );

    write(
        &root.join("schemas/hero.yaml"),
        r#"tables:
  Hero:
    name: Hero
    description: Env-tuned hero table
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      hp: { name: hp, type: { kind: Int32 }, required: true, min: 1 }
      rarity: { name: rarity, type: { kind: String }, enum_values: [common, rare] }
    env_overrides:
      dev:
        hp:
          required: false
      prod:
        hp:
          required: true
          min: 100
        rarity:
          enum_values: [common, rare, epic]
enums: {}
"#,
    );
}

/// Data valid under every rule set (hp 150 clears base `min: 1` and prod
/// `min: 100`; `common` is in both enums) — so the same source can be
/// packed for base and prod in one test.
const LENIENT_SOURCE: &str = r#"{ "Hero": [ { "id": 1, "hp": 150, "rarity": "common" } ] }"#;

/// Snapshot directory names under `<root>/build/snapshot`.
fn snapshot_names(root: &Path) -> Vec<String> {
    let dir = root.join("build/snapshot");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

fn snapshot_dir(root: &Path, name: &str) -> PathBuf {
    root.join("build/snapshot").join(name)
}

/// `snapshot --env prod` resolves the prod rules before validating, tags
/// the directory name, packs the resolved schema, and self-verifies; the
/// base snapshot next to it keeps the env-less name and manifest.
#[test]
fn snapshot_env_packs_resolved_rules_and_tags_the_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    // hp 150 + epic: illegal in base (E1204), legal in prod.
    write(
        &root.join("config/hero.json"),
        r#"{ "Hero": [ { "id": 1, "hp": 150, "rarity": "epic" } ] }"#,
    );
    let proj = root.to_str().unwrap();

    // --env names must be declared; --verify and --env do not compose.
    let out = run_cage(&["snapshot", proj, "--verify", "--env", "dev"]);
    assert_eq!(out.status.code(), Some(2), "--env conflicts with --verify");

    let out = run_cage(&["snapshot", proj, "--env", "prod"]);
    assert_code(&out, 0, "prod snapshot");
    assert!(
        stdout(&out).contains("env 'prod'"),
        "OK line names the environment\n{}",
        stdout(&out)
    );
    let names = snapshot_names(root);
    assert_eq!(names.len(), 1, "one snapshot so far: {names:?}");
    assert!(
        names[0].starts_with("client-prod-"),
        "directory name carries profile and env: {}",
        names[0]
    );
    let prod_dir = snapshot_dir(root, &names[0]);

    // The packed manifest names the environment; the packed schema.json
    // carries the resolved prod constraints (hp min 100, epic legal).
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(prod_dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["environment"], "prod");
    let schema = fs::read_to_string(prod_dir.join("schema.json")).unwrap();
    assert!(
        schema.contains("min: 100") || schema.contains("\"min\": 100"),
        "packed schema carries the prod floor\n{schema}"
    );
    assert!(
        schema.contains("epic"),
        "packed schema carries the prod enum\n{schema}"
    );

    // Load-time check on the packed directory — the server's gate.
    let out = run_cage(&["snapshot", prod_dir.to_str().unwrap(), "--verify"]);
    assert_code(&out, 0, "packed prod snapshot verifies");

    // The base build would reject this source (epic): after switching to
    // data valid everywhere, the base snapshot keeps the env-less shape.
    fs::write(root.join("config/hero.json"), LENIENT_SOURCE).unwrap();
    let out = run_cage(&["snapshot", proj]);
    assert_code(&out, 0, "base snapshot");
    assert!(
        !stdout(&out).contains("env '"),
        "base snapshot line names no environment\n{}",
        stdout(&out)
    );
    let names = snapshot_names(root);
    let base_name = names
        .iter()
        .find(|n| !n.contains("-prod-"))
        .unwrap_or_else(|| panic!("base snapshot dir missing among {names:?}"));
    assert!(
        base_name.starts_with("client-"),
        "base directory name has no env segment: {base_name}"
    );
    let base_dir = snapshot_dir(root, base_name);
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(base_dir.join("manifest.json")).unwrap()).unwrap();
    assert!(
        manifest.get("environment").is_none(),
        "base snapshot records no environment: {manifest}"
    );
}

/// `registry publish --env` keeps one version = one pack: the env pack at
/// an already-published version conflicts (E1801), a fresh version carries
/// the environment in its packed manifest, and an identical re-publish is
/// still a no-op.
#[test]
fn publish_env_conflict_guards_one_version_one_pack() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    fs::write(root.join("config/hero.json"), LENIENT_SOURCE).unwrap();
    let proj = root.to_str().unwrap();
    let reg = root.join("reg");
    let reg_s = reg.to_str().unwrap();

    let out = run_cage(&[
        "registry",
        "publish",
        proj,
        "--registry",
        reg_s,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "base publish");
    assert!(
        !stdout(&out).contains("env '"),
        "base publish line names no environment\n{}",
        stdout(&out)
    );
    let base_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(reg.join("envs/1.0.0/manifest.json")).unwrap())
            .unwrap();
    assert!(base_manifest.get("environment").is_none());

    // Same version, different environment → different packed bytes (the
    // resolved schema rotates build_id/schema.json) → E1801.
    let out = run_cage(&[
        "registry",
        "publish",
        proj,
        "--env",
        "prod",
        "--registry",
        reg_s,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 1, "env pack at a taken version");
    assert!(stderr(&out).contains("E1801"), "{}", stderr(&out));

    // A fresh version takes the env pack; the packed manifest names it.
    let out = run_cage(&[
        "registry",
        "publish",
        proj,
        "--env",
        "prod",
        "--registry",
        reg_s,
        "--version",
        "2.0.0",
    ]);
    assert_code(&out, 0, "env publish at a fresh version");
    assert!(
        stdout(&out).contains("published envs/2.0.0") && stdout(&out).contains("env 'prod'"),
        "{}",
        stdout(&out)
    );
    let env_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(reg.join("envs/2.0.0/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(env_manifest["environment"], "prod");
    // The env pack's schema.json carries the resolved prod floor.
    let schema = fs::read_to_string(reg.join("envs/2.0.0/schema.json")).unwrap();
    assert!(
        schema.contains("\"min\": 100"),
        "entry schema carries the prod floor\n{schema}"
    );

    // Re-publishing the same environment at the same version is still
    // byte-identical → no-op, and the full registry verifies clean.
    let out = run_cage(&[
        "registry",
        "publish",
        proj,
        "--env",
        "prod",
        "--registry",
        reg_s,
        "--version",
        "2.0.0",
    ]);
    assert_code(&out, 0, "identical env re-publish");
    assert!(
        stdout(&out).contains("identical, no-op"),
        "{}",
        stdout(&out)
    );
    let out = run_cage(&["registry", "verify", "--registry", reg_s]);
    assert_code(&out, 0, "registry verifies with both packs");
    assert!(stdout(&out).contains("2 entry"), "{}", stdout(&out));
}
