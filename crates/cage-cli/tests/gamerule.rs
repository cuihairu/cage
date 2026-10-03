//! `cage check --level gamerule`: L7 game rule validators run end to end —
//! project load → built-in plugin registry → `E1601` diagnostics rendered
//! with row locations; lower levels stay untouched by the rule.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Monster table with level/attack — the two fields the built-in
/// `power_curve` rule keys on. `rows` is `(id, level, attack)` tuples.
fn write_project(root: &Path, rows: &[(i32, i32, i32)]) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();

    fs::write(
        root.join("cage.toml"),
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
"#,
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
