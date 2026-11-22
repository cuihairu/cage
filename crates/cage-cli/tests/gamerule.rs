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
    // Review P3-1: the power_curve sample is an opt-in package; selecting it
    // is what makes the L7 pass run the game-specific formula.
    write_project_with_policy(
        root,
        &[(1, 1, 500), (2, 5, 400)],
        "\n[validation]\nrule_packages = [\"power_curve\"]\n",
    );

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
    write_project_with_policy(
        root,
        &[(1, 1, 150), (2, 5, 400)],
        "\n[validation]\nrule_packages = [\"power_curve\"]\n",
    );

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
        "\n[validation]\nrequired_level = \"gamerule\"\n\
         rule_packages = [\"power_curve\"]\n",
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
        "\n[validation]\nrequired_level = \"gamerule\"\n\
         rule_packages = [\"power_curve\"]\n",
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
        "\n[validation]\nrequired_level = \"gamerule\"\n\
         rule_packages = [\"power_curve\"]\n",
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

// ---- review P3-1: game-specific sample rules are opt-in packages ----

/// A plain project at `--level gamerule` (no `[validation]` policy) does NOT
/// run the power_curve sample — Cage is a generic tool and must not assume
/// every game shares the same numeric curve.
#[test]
fn plain_project_at_gamerule_runs_no_sample_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // level 1 → cap 150; 500 would violate the sample's formula.
    write_project(root, &[(1, 1, 500)]);

    let out = run_cage(&["check", root.to_str().unwrap(), "--level", "gamerule"]);
    assert!(
        out.status.success(),
        "no package selected = no game-specific rule fires: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("E1601"), "stdout:\n{stdout}");
    assert!(stdout.contains("cage check: OK"), "stdout:\n{stdout}");
}

/// A package name no built-in package carries is a hard usage error (exit 2),
/// never a silent "select nothing" that lets the L7 pass pass vacuously.
#[test]
fn an_unknown_rule_package_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrule_packages = [\"banana\"]\n",
    );

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("rule_packages"), "stderr:\n{stderr}");
    assert!(stderr.contains("banana"), "stderr:\n{stderr}");
    assert!(stderr.contains("power_curve"), "stderr:\n{stderr}");
}

// ---- review P3-2: versioning / parameterization / capability declaration ----

/// Package parameters ride in `[validation.rule_packages_config.<name>]` and
/// edit the sample's curve: `slope`/`intercept` replace the hard-coded
/// 100/50, and the hint names the configured formula.
#[test]
fn rule_package_params_configure_the_curve_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // slope 50 / intercept 0 → level 1 caps at 50: attack 100 violates the
    // configured curve (the default curve would have passed it), attack 50
    // sits exactly on the boundary and passes.
    write_project_with_policy(
        root,
        &[(1, 1, 100), (2, 1, 50)],
        "\n[validation]\nrule_packages = [\"power_curve\"]\n\
         \n[validation.rule_packages_config.power_curve]\n\
         params = { slope = 50, intercept = 0 }\n",
    );

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the configured curve must fail the violating row"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("E1601"), "diagnostics:\n{stdout}");
    assert!(stdout.contains("level * 50 + 0"), "diagnostics:\n{stdout}");
    assert!(
        stdout.contains("cage check: FAILED (1 errors"),
        "only the one violating row fails: {stdout}"
    );
}

/// A version pin on a different major than the shipped version is a hard
/// usage error (exit 2) — the project must migrate, never run a rule of
/// unknown behavior. A pin on the shipped major passes the same project.
#[test]
fn an_incompatible_rule_package_version_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrule_packages = [\"power_curve\"]\n\
         \n[validation.rule_packages_config.power_curve]\n\
         version = \"2\"\n",
    );

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("power_curve"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("1.0.0"),
        "names the shipped version: {stderr}"
    );
    assert!(stderr.contains("2"), "stderr:\n{stderr}");

    // The same project pinning the shipped major (1.0.0) is compatible and
    // passes its clean data.
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrule_packages = [\"power_curve\"]\n\
         \n[validation.rule_packages_config.power_curve]\n\
         version = \"1.0.0\"\n",
    );
    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "a pin on the shipped major is compatible: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A selected package whose declared capability the schema does not satisfy
/// reports E1609 and fails the check — the run is vacuous and that must be
/// visible, never a silent pass.
#[test]
fn a_selected_package_without_its_fields_reports_e1609() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project_with_policy(
        root,
        &[(1, 1, 150)],
        "\n[validation]\nrule_packages = [\"power_curve\"]\n",
    );
    // Replace the fixture with a schema/data pair that carries no level or
    // attack fields anywhere.
    fs::write(
        root.join("schemas/monster.yaml"),
        r#"tables:
  Monster:
    name: Monster
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
enums: {}
"#,
    )
    .unwrap();
    fs::write(
        root.join("config/monster.json"),
        r#"{"Monster": [{"id": 1, "name": "slime"}]}"#,
    )
    .unwrap();

    let out = run_cage(&["check", root.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a vacuous package run must fail the check"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("E1609"), "diagnostics:\n{stdout}");
    assert!(stdout.contains("power_curve"), "diagnostics:\n{stdout}");
    assert!(stdout.contains("level"), "diagnostics:\n{stdout}");
}
