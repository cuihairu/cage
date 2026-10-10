//! Process-level coverage of `--env` validation (design §48): a schema's
//! `env_overrides` re-tune field constraints per environment before any
//! validation runs — dev can relax what the base schema demands, prod can
//! tighten it further. The base schema is one environment among many:
//! `--env` names a declared environment, unknown names are usage errors
//! (exit 2), and the environment rotates the manifest (`environment`
//! field, `schema_hash`/`build_id`) so incremental builds never reuse one
//! environment's artifacts for another.

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

/// Hero table with two declared environments: `dev` relaxes the base
/// schema (`hp` becomes optional), `prod` tightens it (`hp` required and
/// `min: 100`, `rarity` also accepts `epic`). One table, three rule sets.
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

fn write_source(root: &Path, content: &str) {
    fs::write(root.join("config/hero.json"), content).unwrap();
}

fn manifest(root: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(root.join("build/manifest.json")).unwrap()).unwrap()
}

/// The same table validates three ways: base (E1001 — `hp` required, the
/// row lacks it), `dev` (relaxed — the same row passes), and `prod`
/// (tightened — present values must clear `min: 100`, `epic` becomes a
/// legal rarity). Constraint overrides only move the bars; the shape
/// (types, table structure) never changes.
#[test]
fn env_overrides_retarget_constraint_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    // No hp in the row: base E1001, dev relaxes it away.
    write_source(root, r#"{ "Hero": [ { "id": 1, "rarity": "common" } ] }"#);
    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_code(&out, 1, "base requires hp");
    assert!(stdout(&out).contains("E1001"), "{}", stdout(&out));
    let out = run_cage(&["check", root.to_str().unwrap(), "--env", "dev"]);
    assert_code(&out, 0, "dev makes hp optional");
    assert!(stdout(&out).contains("OK (1 tables"), "{}", stdout(&out));

    // hp = 150, rarity = epic: illegal in base (E1204), legal in prod.
    write_source(
        root,
        r#"{ "Hero": [ { "id": 1, "hp": 150, "rarity": "epic" } ] }"#,
    );
    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_code(&out, 1, "base rejects epic");
    assert!(stdout(&out).contains("E1204"), "{}", stdout(&out));
    let out = run_cage(&["check", root.to_str().unwrap(), "--env", "prod"]);
    assert_code(&out, 0, "prod extends the rarity enum");

    // hp = 50 in prod: the tightened floor fires (E1201), base is fine.
    write_source(
        root,
        r#"{ "Hero": [ { "id": 1, "hp": 50, "rarity": "common" } ] }"#,
    );
    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_code(&out, 0, "base accepts hp 50");
    let out = run_cage(&["check", root.to_str().unwrap(), "--env", "prod"]);
    assert_code(&out, 1, "prod demands hp >= 100");
    assert!(stdout(&out).contains("E1201"), "{}", stdout(&out));
}

/// `--env` names must be declared by the schema, and a schema without
/// `env_overrides` has no environments at all — both are usage errors
/// (exit 2) naming what is declared, before any validation runs.
#[test]
fn unknown_or_undeclared_environments_are_usage_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    write_source(root, r#"{ "Hero": [ { "id": 1, "hp": 10 } ] }"#);

    let out = run_cage(&["check", root.to_str().unwrap(), "--env", "staging"]);
    assert_code(&out, 2, "undeclared environment");
    assert!(
        stderr(&out).contains("unknown environment 'staging' (declared: dev, prod)"),
        "{}",
        stderr(&out)
    );

    // A schema without env_overrides declares nothing.
    let bare = tmp.path().join("bare");
    write_project(&bare);
    fs::write(
        bare.join("schemas/hero.yaml"),
        r#"tables:
  Hero:
    name: Hero
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
"#,
    )
    .unwrap();
    write_source(&bare, r#"{ "Hero": [ { "id": 1 } ] }"#);
    let out = run_cage(&["check", bare.to_str().unwrap(), "--env", "dev"]);
    assert_code(&out, 2, "schema declares no environments");
    assert!(
        stderr(&out).contains("schema declares no environments"),
        "{}",
        stderr(&out)
    );
}

/// The build manifest names its environment, and switching environments
/// rebuilds: same inputs but a different `--env` is a different rule set
/// (different `schema_hash`), never an incremental hit for the wrong env.
#[test]
fn environment_is_recorded_and_rotates_the_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    write_source(
        root,
        r#"{ "Hero": [ { "id": 1, "hp": 150, "rarity": "common" } ] }"#,
    );
    let proj = root.to_str().unwrap();

    let out = run_cage(&["build", proj, "--env", "prod"]);
    assert_code(&out, 0, "prod build");
    let m = manifest(root);
    assert_eq!(m["environment"], "prod", "manifest names the environment");
    let prod_build_id = m["build_id"].as_str().unwrap().to_string();

    // Same command again: a clean incremental hit.
    let out = run_cage(&["build", proj, "--env", "prod", "--incremental"]);
    assert_code(&out, 0, "prod rebuild is up to date");
    assert!(stdout(&out).contains("up to date"), "{}", stdout(&out));

    // Switching to base: different rules, different build_id, no
    // environment field (the base schema is not an environment).
    let out = run_cage(&["build", proj, "--incremental"]);
    assert_code(&out, 0, "base build after prod");
    assert!(
        !stdout(&out).contains("up to date"),
        "environment switch must rebuild\n{}",
        stdout(&out)
    );
    let m = manifest(root);
    assert!(
        m.get("environment").is_none(),
        "base build records no environment: {m}"
    );
    assert_ne!(
        m["build_id"].as_str().unwrap(),
        prod_build_id,
        "environment switch rotates build_id"
    );
}

/// An incremental `--env` build converges to the full build's bytes: the
/// manifest is identical either way (determinism contract holds per
/// environment, not just per project).
#[test]
fn incremental_env_build_matches_full_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    write_source(
        root,
        r#"{ "Hero": [ { "id": 1, "hp": 150, "rarity": "common" } ] }"#,
    );
    let proj = root.to_str().unwrap();

    let out = run_cage(&["build", proj, "--env", "prod"]);
    assert_code(&out, 0, "full prod build");
    let full_manifest = fs::read(root.join("build/manifest.json")).unwrap();
    let full_artifact = fs::read(root.join("build/client/Hero.json")).unwrap();

    // A second environment first, then the incremental prod run.
    let out = run_cage(&["build", proj, "--env", "dev"]);
    assert_code(&out, 0, "dev build");
    let out = run_cage(&["build", proj, "--env", "prod", "--incremental"]);
    assert_code(&out, 0, "incremental prod build after dev");
    assert!(
        !stdout(&out).contains("up to date"),
        "dev's manifest must not satisfy a prod build\n{}",
        stdout(&out)
    );
    assert_eq!(
        full_manifest,
        fs::read(root.join("build/manifest.json")).unwrap(),
        "incremental prod manifest == full prod manifest"
    );
    assert_eq!(
        full_artifact,
        fs::read(root.join("build/client/Hero.json")).unwrap(),
        "incremental prod artifact == full prod artifact"
    );
}
