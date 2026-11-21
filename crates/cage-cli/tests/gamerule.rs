//! `cage check --level gamerule`: L7 game rule validators run end to end —
//! project load → built-in plugin registry → `E1601` diagnostics rendered
//! with row locations; lower levels stay untouched by the rule.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Monster table with level/attack — the two fields the built-in
/// `power_curve` rule keys on. `rows` is `(id, level, attack)` tuples.
fn write_project(root: &Path, rows: &[(i32, i32, i32)]) {
    write_project_with_policy(root, rows, "");
}

/// Same fixture plus a `[validation]` policy section (review P2-5); an empty
/// `validation` string writes the plain project.
fn write_project_with_policy(root: &Path, rows: &[(i32, i32, i32)], validation: &str) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();

    fs::write(
        root.join("cage.toml"),
        format!(
            r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "gamerule"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
{validation}
"#,
        ),
    )
    .unwrap();

    fs::write(
        root.join("schemas/monster.yaml"),
        r#"tables:
  Monster:
    name: Monster
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
      level: { name: level, type: { kind: Int32 }, required: true }
      attack: { name: attack, type: { kind: Int32 }, required: true }
enums: {}
"#,
    )
    .unwrap();

    let rows_json: Vec<String> = rows
        .iter()
        .map(|(id, level, attack)| {
            format!(r#"{{"id": {id}, "name": "m{id}", "level": {level}, "attack": {attack}}}"#)
        })
        .collect();
    fs::write(
        root.join("config/monster.json"),
        format!(r#"{{"Monster": [{}]}}"#, rows_json.join(",")),
    )
    .unwrap();
}

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn gamerule_level_reports_power_curve_violation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // level 1 caps attack at 150 — 500 violates; level 5 caps at 550 — fine.
    write_project(root, &[(1, 1, 500), (2, 5, 400)]);

    let out = run_cage(&["check", root.to_str().unwrap(), "--level", "gamerule"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "violating row must fail the check"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("E1601"), "diagnostics:\n{stdout}");
    assert!(stdout.contains("power curve"), "diagnostics:\n{stdout}");
    assert!(
        stdout.contains("power_curve: attack 500"),
        "diagnostics:\n{stdout}"
    );
    assert!(stdout.contains("Monster"), "diagnostics:\n{stdout}");
    assert!(
        stdout.contains("cage check: FAILED (1 errors"),
        "diagnostics:\n{stdout}"
    );
}

#[test]
fn gamerule_level_passes_clean_data() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root, &[(1, 1, 150), (2, 5, 400)]);

    let out = run_cage(&["check", root.to_str().unwrap(), "--level", "gamerule"]);
    assert!(
        out.status.success(),
        "boundary-level data must pass: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cage check: OK"), "stdout:\n{stdout}");
}

#[test]
fn default_level_does_not_run_game_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root, &[(1, 1, 500)]);

    // Default level is semantic (L6) — the L7 plugin must not fire, the
    // data itself is schema/type/value-clean so the check passes.
    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "L7 stays out of lower levels: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("E1601"), "stdout:\n{stdout}");
}

// ---- review P2-5: the project policy raises the CLI default level ----

/// `required_level = "gamerule"` escalates the default (semantic) check: the
/// violating row fails without any `--level` flag, and a clean run reports the
/// level that actually ran.
#[test]
fn required_level_raises_the_cli_default_to_gamerule() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 500)],
        "\n[validation]\nrequired_level = \"gamerule\"\n",
    );

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the policy must run L7 even at the default level"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("E1601"), "stdout:\n{stdout}");
    assert!(stdout.contains("power curve"), "stdout:\n{stdout}");

    // Clean data passes and the summary names the effective level.
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrequired_level = \"gamerule\"\n",
    );
    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "clean data must pass: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("level <= gamerule"),
        "the summary must report the effective level: {stdout}"
    );
}

/// A required plugin that is not under `plugins/` fails the check (E1602)
/// instead of passing silently — the review §六.1 scenario.
#[test]
fn required_plugins_missing_fails_the_check() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrequired_plugins = [\"drop_rewards\"]\n",
    );

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a missing required plugin must fail validation"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("E1602"), "stdout:\n{stdout}");
    assert!(stdout.contains("drop_rewards"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("required validator plugin"),
        "stdout:\n{stdout}"
    );
}

/// An unparseable `required_level` is a hard usage error — never a silent
/// downgrade to a level that skips the declared rules.
#[test]
fn an_invalid_required_level_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrequired_level = \"banana\"\n",
    );

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("required_level"), "stderr:\n{stderr}");
    assert!(stderr.contains("banana"), "stderr:\n{stderr}");
}

/// The same policy gates `cage build` — CI cannot pass without the plugins
/// the project declared.
#[test]
fn required_level_gates_the_build_too() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 500)],
        "\n[validation]\nrequired_level = \"gamerule\"\n",
    );

    let out = run_cage(&["build", root.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the policy must gate the build pipeline as well"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("E1601"), "stdout:\n{stdout}");
}
