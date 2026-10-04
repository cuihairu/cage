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

// ---------------------------------------------------------------- check ---

#[test]
fn check_invalid_level_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["check", tmp.path().to_str().unwrap(), "--level", "bogus"]);
    assert_code(&out, 2, "invalid --level");
    assert!(
        stderr(&out).contains("Unknown validation level: bogus"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_missing_config_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "missing cage.toml");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_nonexistent_root_exits_2() {
    let ghost = std::env::temp_dir().join("cage-cli-ghost-root");
    let out = run_cage(&["check", ghost.to_str().unwrap()]);
    assert_code(&out, 2, "nonexistent project root");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_invalid_yaml_schema_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::write(
        tmp.path().join("schemas/item.yaml"),
        "tables: [unterminated",
    )
    .unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "invalid schema yaml");
    assert!(
        stderr(&out).contains("invalid schema"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_invalid_json_schema_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::remove_file(tmp.path().join("schemas/item.yaml")).unwrap();
    fs::write(tmp.path().join("schemas/schema.json"), "{not json").unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "invalid schema json");
    let err = stderr(&out);
    assert!(err.contains("invalid schema"), "stderr:\n{err}");
    assert!(err.contains("schema.json"), "stderr:\n{err}");
}

#[test]
fn check_json_schema_ok() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::remove_file(tmp.path().join("schemas/item.yaml")).unwrap();
    fs::write(
        tmp.path().join("schemas/schema.json"),
        r#"{"tables":{"Item":{"name":"Item","primary_key":["id"],"fields":{"id":{"name":"id","type":{"kind":"Int32"},"required":true},"name":{"name":"name","type":{"kind":"String"}}}}},"enums":{}}"#,
    )
    .unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "schema in JSON format");
    assert!(
        stdout(&out).contains("cage check: OK"),
        "stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn check_missing_schema_dir_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let toml = CAGE_TOML.replace("schema_path = \"schemas\"", "schema_path = \"ghost\"");
    fs::write(tmp.path().join("cage.toml"), toml).unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "schema_path pointing nowhere");
    assert!(
        stderr(&out).contains("path not found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_schema_path_pointing_at_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let toml = CAGE_TOML.replace(
        "schema_path = \"schemas\"",
        "schema_path = \"schemas/item.yaml\"",
    );
    fs::write(tmp.path().join("cage.toml"), toml).unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "schema_path as a single file");
    assert!(
        stdout(&out).contains("cage check: OK"),
        "stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn check_nested_schema_dir_is_merged() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::create_dir_all(tmp.path().join("schemas/nested")).unwrap();
    fs::rename(
        tmp.path().join("schemas/item.yaml"),
        tmp.path().join("schemas/nested/item.yaml"),
    )
    .unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "schema in a nested directory");
    assert!(
        stdout(&out).contains("cage check: OK (1 tables"),
        "stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn check_malformed_source_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::write(tmp.path().join("config/item.json"), "{ not json").unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "malformed source file");
    let err = stderr(&out);
    assert!(err.contains("JSON syntax error"), "stderr:\n{err}");
    assert!(err.contains("failed to parse"), "stderr:\n{err}");
}

#[test]
fn check_cage_json_config_ok() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("cage.json"),
        r#"{"project":{"name":"jsonproj"},"profiles":{"client":{"name":"client","targets":[{"format":"json","output_dir":"build/json"}]}}}"#,
    )
    .unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "cage.json config");
    assert!(
        stdout(&out).contains("cage check: OK"),
        "stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn check_cage_yaml_config_ok() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("cage.yaml"),
        "project:\n  name: yamlproj\nprofiles:\n  client:\n    name: client\n    targets:\n      - format: json\n        output_dir: build/json\n",
    )
    .unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "cage.yaml config");
    assert!(
        stdout(&out).contains("cage check: OK"),
        "stdout:\n{}",
        stdout(&out)
    );
}

/// Each config format has its own parse-error message.
#[test]
fn check_invalid_toml_config_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("cage.toml"),
        "output_dir = \"build\"\n[project\n",
    )
    .unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "invalid cage.toml");
    assert!(
        stderr(&out).contains("invalid cage.toml"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_invalid_json_config_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("cage.json"), "{not json").unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "invalid cage.json");
    assert!(
        stderr(&out).contains("invalid cage.json"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn check_invalid_yaml_config_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("cage.yaml"), "profiles: [\n  name: x\n").unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "invalid cage.yaml");
    assert!(
        stderr(&out).contains("invalid cage.yaml"),
        "stderr:\n{}",
        stderr(&out)
    );
}

/// A declared source root that does not exist is a load error, not an
/// empty document.
#[test]
fn check_missing_source_root_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_empty_project(
        tmp.path(),
        r#"output_dir = "build"

[project]
name = "srcmissing"

[source_roots]
main = "ghost_dir"

[profiles.client]
name = "client"
targets = []
"#,
    );
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "source root pointing nowhere");
    assert!(
        stderr(&out).contains("path not found"),
        "stderr:\n{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("ghost_dir"),
        "stderr:\n{}",
        stderr(&out)
    );
}

/// The source walker accepts the legacy `.xls` extension but the Excel
/// adapter rejects it: the parse failure must surface as a load error.
#[test]
fn check_xls_extension_rejected_by_adapter_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::write(tmp.path().join("config/legacy.xls"), "pretend xls bytes").unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "legacy .xls source");
    let err = stderr(&out);
    assert!(
        err.contains("Unsupported Excel format: .xls"),
        "stderr:\n{err}"
    );
    assert!(err.contains("failed to parse"), "stderr:\n{err}");
}

// ---------------------------------------------------------------- build ---

#[test]
fn build_invalid_level_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["build", tmp.path().to_str().unwrap(), "--level", "nope"]);
    assert_code(&out, 2, "invalid --level");
    assert!(
        stderr(&out).contains("Unknown validation level: nope"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn build_nonexistent_root_exits_2() {
    let ghost = std::env::temp_dir().join("cage-cli-ghost-build");
    let out = run_cage(&["build", ghost.to_str().unwrap()]);
    assert_code(&out, 2, "nonexistent project root");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn build_unknown_profile_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["build", tmp.path().to_str().unwrap(), "--profile", "server"]);
    assert_code(&out, 2, "unknown profile");
    let err = stderr(&out);
    assert!(
        err.contains("unknown profile 'server' (available: client)"),
        "stderr:\n{err}"
    );
}

#[test]
fn build_type_error_fails_validation() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::write(
        tmp.path().join("config/item.json"),
        r#"{"Item": [{"id": "abc", "name": "Broken"}]}"#,
    )
    .unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 1, "type mismatch");
    let out_str = stdout(&out);
    assert!(out_str.contains("Type mismatch"), "stdout:\n{out_str}");
    assert!(
        out_str.contains("cage build: FAILED validation"),
        "stdout:\n{out_str}"
    );
}

/// Schema declares a table the sources never provide: L1 emits a warning,
/// the build still succeeds and the warning is rendered.
#[test]
fn build_warnings_only_still_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let cage_toml = r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "ghost"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
"#;
    fs::write(tmp.path().join("cage.toml"), cage_toml).unwrap();
    fs::create_dir_all(tmp.path().join("schemas")).unwrap();
    fs::write(
        tmp.path().join("schemas/ghost.yaml"),
        "tables:\n  Ghost:\n    name: Ghost\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
    )
    .unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "warning-only build");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("Expected table 'Ghost' not found"),
        "stdout:\n{out_str}"
    );
    assert!(out_str.contains("cage build: OK"), "stdout:\n{out_str}");
}

/// Same warning with `warnings_as_errors = true` escalates to an error.
#[test]
fn build_warnings_as_errors_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let cage_toml = r#"output_dir = "build"
schema_path = "schemas"
warnings_as_errors = true

[project]
name = "ghost"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
"#;
    fs::write(tmp.path().join("cage.toml"), cage_toml).unwrap();
    fs::create_dir_all(tmp.path().join("schemas")).unwrap();
    fs::write(
        tmp.path().join("schemas/ghost.yaml"),
        "tables:\n  Ghost:\n    name: Ghost\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
    )
    .unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 1, "warnings_as_errors escalation");
    assert!(
        stdout(&out).contains("cage build: FAILED validation"),
        "stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn build_csv_target_writes_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let toml = format!(
        "{CAGE_TOML}\n[[profiles.client.targets]]\nformat = \"csv\"\noutput_dir = \"build/csv\"\n"
    );
    fs::write(tmp.path().join("cage.toml"), toml).unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "build with csv target");
    let csv = fs::read_to_string(tmp.path().join("build/csv/Item.csv")).unwrap();
    assert!(csv.contains("Sword"), "csv:\n{csv}");
    assert!(csv.contains("id"), "csv:\n{csv}");
}

#[test]
fn build_unsupported_format_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_empty_project(
        tmp.path(),
        r#"output_dir = "build"

[project]
name = "xmlproj"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "xml"
output_dir = "build/xml"
"#,
    );
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "unsupported target format");
    assert!(
        stderr(&out).contains("unsupported target format 'xml'"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn build_artifact_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    // The artifact's parent directory is a regular file → create_dir_all fails.
    fs::create_dir_all(tmp.path().join("build")).unwrap();
    fs::write(tmp.path().join("build/json"), "blocker").unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "artifact parent blocked");
    assert!(
        stderr(&out).contains("cannot create"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn build_manifest_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    // Manifest lands in output_dir = "mout", which is a regular file.
    let toml = CAGE_TOML.replace("output_dir = \"build\"", "output_dir = \"mout\"");
    fs::write(tmp.path().join("cage.toml"), toml).unwrap();
    fs::create_dir_all(tmp.path().join("config")).unwrap();
    fs::create_dir_all(tmp.path().join("schemas")).unwrap();
    fs::write(tmp.path().join("schemas/item.yaml"), ITEM_SCHEMA).unwrap();
    fs::write(tmp.path().join("config/item.json"), ITEM_DATA).unwrap();
    fs::write(tmp.path().join("mout"), "blocker").unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "manifest dir blocked");
    assert!(
        stderr(&out).contains("cannot create"),
        "stderr:\n{}",
        stderr(&out)
    );
}

/// `--incremental` with no previous manifest must fall back to a full build.
#[test]
fn build_incremental_without_manifest_runs_full_build() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["build", tmp.path().to_str().unwrap(), "--incremental"]);
    assert_code(&out, 0, "incremental on fresh project");
    let out_str = stdout(&out);
    assert!(out_str.contains("cage build: OK"), "stdout:\n{out_str}");
    assert!(!out_str.contains("up to date"), "stdout:\n{out_str}");
    assert!(tmp.path().join("build/manifest.json").is_file());
}

// ------------------------------------------------------------------ gen ---

#[test]
fn gen_nonexistent_root_exits_2() {
    let ghost = std::env::temp_dir().join("cage-cli-ghost-gen");
    let out = run_cage(&["gen", ghost.to_str().unwrap()]);
    assert_code(&out, 2, "nonexistent project root");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn gen_unknown_profile_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["gen", tmp.path().to_str().unwrap(), "--profile", "server"]);
    assert_code(&out, 2, "unknown profile");
    assert!(
        stderr(&out).contains("unknown profile 'server' (available: client)"),
        "stderr:\n{}",
        stderr(&out)
    );
}

fn write_codegen_project(root: &Path, cage_toml: &str) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(root.join("cage.toml"), cage_toml).unwrap();
    fs::write(root.join("schemas/item.yaml"), ITEM_SCHEMA).unwrap();
    fs::write(root.join("config/item.json"), ITEM_DATA).unwrap();
}

#[test]
fn gen_artifact_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_codegen_project(
        tmp.path(),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "genfail"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "cs"
output_dir = "gout"
"#,
    );
    // The code artifact's output dir is a regular file → create_dir_all fails.
    fs::write(tmp.path().join("gout"), "blocker").unwrap();
    let out = run_cage(&["gen", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "gen artifact parent blocked");
    assert!(
        stderr(&out).contains("cannot create"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn gen_manifest_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    // Manifest lands in output_dir = "gmout", which is a regular file, while
    // the code artifacts go to "gout" and succeed.
    write_codegen_project(
        tmp.path(),
        r#"output_dir = "gmout"
schema_path = "schemas"

[project]
name = "genfail"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "cs"
output_dir = "gout"
"#,
    );
    fs::write(tmp.path().join("gmout"), "blocker").unwrap();
    let out = run_cage(&["gen", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "gen manifest dir blocked");
    assert!(
        stderr(&out).contains("cannot create"),
        "stderr:\n{}",
        stderr(&out)
    );
}

// -------------------------------------------------------------- inspect ---

#[test]
fn inspect_lists_project_tables_and_schemas() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["inspect", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "inspect listing");
    let out_str = stdout(&out);
    assert!(out_str.contains("project: clipaths"), "stdout:\n{out_str}");
    assert!(out_str.contains("version: 0.1.0"), "stdout:\n{out_str}");
    assert!(out_str.contains("tables:"), "stdout:\n{out_str}");
    assert!(out_str.contains("  Item (1 rows)"), "stdout:\n{out_str}");
    assert!(out_str.contains("schemas:"), "stdout:\n{out_str}");
    assert!(
        out_str.contains("  Item (2 fields, primary key: id)"),
        "stdout:\n{out_str}"
    );
}

#[test]
fn inspect_table_detail() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["inspect", tmp.path().to_str().unwrap(), "Item"]);
    assert_code(&out, 0, "inspect table");
    let out_str = stdout(&out);
    assert!(out_str.contains("table: Item"), "stdout:\n{out_str}");
    assert!(
        out_str.contains("description: An inventory item"),
        "stdout:\n{out_str}"
    );
    assert!(out_str.contains("primary key: id"), "stdout:\n{out_str}");
    assert!(out_str.contains("rows: 1"), "stdout:\n{out_str}");
    assert!(out_str.contains("fields:"), "stdout:\n{out_str}");
    assert!(
        out_str.contains("  id: Int32 (required)"),
        "stdout:\n{out_str}"
    );
    // Optional field: no "(required)" suffix.
    assert!(out_str.contains("  name: String\n"), "stdout:\n{out_str}");
    assert!(!out_str.contains("name: String (required)"), "{out_str}");
}

/// A project without a `[project].version` omits the version line from the
/// listing.
#[test]
fn inspect_listing_without_version() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let toml = CAGE_TOML.replace("\nversion = \"0.1.0\"\n", "\n");
    fs::write(tmp.path().join("cage.toml"), toml).unwrap();
    let out = run_cage(&["inspect", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "inspect without version");
    let out_str = stdout(&out);
    assert!(out_str.contains("project: clipaths"), "stdout:\n{out_str}");
    assert!(
        !out_str.contains("version:"),
        "no version line expected:\n{out_str}"
    );
}

/// A table schema without a `description` omits the description line from
/// the table detail view.
#[test]
fn inspect_table_without_description() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    fs::write(
        tmp.path().join("schemas/item.yaml"),
        r"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
    )
    .unwrap();
    let out = run_cage(&["inspect", tmp.path().to_str().unwrap(), "Item"]);
    assert_code(&out, 0, "inspect table without description");
    let out_str = stdout(&out);
    assert!(out_str.contains("table: Item"), "stdout:\n{out_str}");
    assert!(
        !out_str.contains("description:"),
        "no description line expected:\n{out_str}"
    );
    assert!(out_str.contains("primary key: id"), "stdout:\n{out_str}");
}

#[test]
fn inspect_unknown_table_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["inspect", tmp.path().to_str().unwrap(), "Ghost"]);
    assert_code(&out, 2, "unknown table");
    assert!(
        stderr(&out).contains("unknown table 'Ghost'"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn inspect_nonexistent_root_exits_2() {
    let ghost = std::env::temp_dir().join("cage-cli-ghost-inspect");
    let out = run_cage(&["inspect", ghost.to_str().unwrap()]);
    assert_code(&out, 2, "nonexistent project root");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn inspect_merges_json_yaml_csv_and_xlsx_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("config")).unwrap();
    let toml = CAGE_TOML.replace("schema_path = \"schemas\"", "");
    fs::write(root.join("cage.toml"), toml).unwrap();
    // JSON source adapter.
    fs::write(root.join("config/item.json"), ITEM_DATA).unwrap();
    // YAML source adapter (multi-table format: top-level key = table).
    fs::write(
        root.join("config/skills.yaml"),
        "Skills:\n  - id: 1\n    name: fire\n",
    )
    .unwrap();
    // CSV source adapter (table named after the file stem).
    fs::write(root.join("config/units.csv"), "id,name\n1,Arrow\n").unwrap();
    // Excel source adapter: shared fixture from cage-source-excel.
    let xlsx = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../cage-source-excel/tests/fixtures/merged_cells.xlsx"
    );
    fs::copy(xlsx, root.join("config/Items.xlsx")).expect("xlsx fixture");

    let out = run_cage(&["inspect", root.to_str().unwrap()]);
    assert_code(&out, 0, "inspect mixed sources");
    let out_str = stdout(&out);
    for line in [
        "  Item (1 rows)",
        "  Skills (1 rows)",
        "  units (1 rows)",
        "  Items (3 rows)",
    ] {
        assert!(out_str.contains(line), "missing '{line}' in:\n{out_str}");
    }
}

// ----------------------------------------------------------------- diff ---

/// Build, park the output aside as `name`, optionally mutate the project,
/// then build again and park that output too.
fn build_twice(root: &Path, mutate: impl FnOnce()) {
    let out = run_cage(&["build", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "seed build failed: {}{}",
        stdout(&out),
        stderr(&out)
    );
    fs::rename(root.join("build"), root.join("buildA")).unwrap();
    mutate();
    let out = run_cage(&["build", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "second build failed: {}{}",
        stdout(&out),
        stderr(&out)
    );
    fs::rename(root.join("build"), root.join("buildB")).unwrap();
}

const DIFF_TOML: &str = r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "diffproj"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#;

fn write_diff_tables(root: &Path, tables: &[(&str, &str)]) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(root.join("cage.toml"), DIFF_TOML).unwrap();
    for (name, schema) in tables {
        fs::write(root.join(format!("schemas/{name}.yaml")), schema).unwrap();
    }
}

#[test]
fn diff_reports_added_removed_changed_and_content_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_diff_tables(root, &[("item", "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\n      qty: { name: qty, type: { kind: Int32 }, required: true }\nenums: {}\n"), ("stale", "tables:\n  Stale:\n    name: Stale\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n"), ("keep", "tables:\n  Keep:\n    name: Keep\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\n      label: { name: label, type: { kind: String }, required: true }\nenums: {}\n")]);
    fs::write(
        root.join("config/item.json"),
        r#"{"Item": [{"id": 1, "qty": 100}]}"#,
    )
    .unwrap();
    fs::write(root.join("config/stale.json"), r#"{"Stale": [{"id": 9}]}"#).unwrap();
    fs::write(
        root.join("config/keep.json"),
        r#"{"Keep": [{"id": 5, "label": "same"}]}"#,
    )
    .unwrap();

    build_twice(root, || {
        // Drop Stale, add Extra, change Item's quantity; Keep stays identical.
        fs::remove_file(root.join("schemas/stale.yaml")).unwrap();
        fs::remove_file(root.join("config/stale.json")).unwrap();
        fs::write(
            root.join("schemas/extra.yaml"),
            "tables:\n  Extra:\n    name: Extra\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
        )
        .unwrap();
        fs::write(root.join("config/extra.json"), r#"{"Extra": [{"id": 7}]}"#).unwrap();
        fs::write(
            root.join("config/item.json"),
            r#"{"Item": [{"id": 1, "qty": 200}]}"#,
        )
        .unwrap();
    });

    let out = run_cage(&[
        "diff",
        root.join("buildA").to_str().unwrap(),
        root.join("buildB").to_str().unwrap(),
    ]);
    assert_code(&out, 0, "diff");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("+ build/json/Extra.json"),
        "stdout:\n{out_str}"
    );
    assert!(
        out_str.contains("- build/json/Stale.json"),
        "stdout:\n{out_str}"
    );
    assert!(
        out_str.contains("~ build/json/Item.json"),
        "stdout:\n{out_str}"
    );
    assert!(
        out_str.contains("cage diff: 1 added, 1 removed, 1 changed, 1 unchanged"),
        "stdout:\n{out_str}"
    );
    assert!(out_str.contains("content_hash: "), "stdout:\n{out_str}");
}

#[test]
fn diff_identical_builds_report_all_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    build_twice(root, || {});

    let out = run_cage(&[
        "diff",
        root.join("buildA").to_str().unwrap(),
        root.join("buildB").to_str().unwrap(),
    ]);
    assert_code(&out, 0, "diff of identical builds");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("cage diff: 0 added, 0 removed, 0 changed, 1 unchanged"),
        "stdout:\n{out_str}"
    );
    assert!(!out_str.contains("content_hash:"), "stdout:\n{out_str}");
}

#[test]
fn diff_accepts_manifest_json_file_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    build_twice(root, || {
        fs::write(
            root.join("config/item.json"),
            r#"{"Item": [{"id": 2, "name": "Shield"}]}"#,
        )
        .unwrap();
    });

    let out = run_cage(&[
        "diff",
        root.join("buildA/manifest.json").to_str().unwrap(),
        root.join("buildB/manifest.json").to_str().unwrap(),
    ]);
    assert_code(&out, 0, "diff of manifest.json files");
    assert!(
        stdout(&out).contains("cage diff: 0 added, 0 removed, 1 changed, 0 unchanged"),
        "stdout:\n{}",
        stdout(&out)
    );
}

#[test]
fn diff_missing_manifests_exit_2() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let out = run_cage(&[
        "diff",
        root.join("ghostA").to_str().unwrap(),
        root.join("ghostB").to_str().unwrap(),
    ]);
    assert_code(&out, 2, "both manifests missing");
    assert!(
        stderr(&out).contains("cannot read"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn diff_missing_target_manifest_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    let out = run_cage(&["build", root.to_str().unwrap()]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));

    let out = run_cage(&[
        "diff",
        root.join("build").to_str().unwrap(),
        root.join("ghost").to_str().unwrap(),
    ]);
    assert_code(&out, 2, "target manifest missing");
    assert!(
        stderr(&out).contains("cannot read"),
        "stderr:\n{}",
        stderr(&out)
    );
}

/// E9006 (Profile 语义化): a projection that would silently strip a
/// structurally required / key / unique / reference-critical field is a
/// conflict reported as E9006; optional server-only fields stay a
/// legitimate client view. Validation covers the full corpus regardless of
/// profile (profiles gate runtime views, not data quality).
#[test]
fn e9006_visibility_conflict_blocks_client_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "visibility"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client"
file_template = "{table}.json"

[profiles.server]
name = "server"

[[profiles.server.targets]]
format = "json"
output_dir = "build/server"
file_template = "{table}.json"
"#,
    )
    .unwrap();
    fs::write(
        root.join("schemas/account.yaml"),
        r#"tables:
  Account:
    name: Account
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      email: { name: email, type: { kind: String }, required: true }
      secret: { name: secret, type: { kind: String }, required: true,
                targets: [server] }
enums: {}
"#,
    )
    .unwrap();
    fs::write(
        root.join("config/account.json"),
        r#"{"Account": [{"id": 1, "email": "a@b.c", "secret": "s3cr3t"}]}"#,
    )
    .unwrap();

    // client projection would lose the required server-only field → E9006.
    let out = run_cage(&["check", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 1, "check client with hidden required field");
    let stdout = stdout(&out);
    assert!(stdout.contains("E9006"), "expected E9006, got:\n{stdout}");
    assert!(
        stdout.contains("Required field hidden by profile") && stdout.contains("secret"),
        "expected field-level conflict, got:\n{stdout}"
    );

    // build shares the conflict.
    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 1, "build client with hidden required field");

    // server profile sees the full schema → clean.
    let out = run_cage(&["check", root.to_str().unwrap(), "--profile", "server"]);
    assert_code(&out, 0, "check server profile");

    // Optional server-only fields remain a legitimate client view: relax the
    // field and the client build passes, with the field stripped from output.
    let schema_text = fs::read_to_string(root.join("schemas/account.yaml")).unwrap();
    let relaxed = schema_text.replace(
        "secret: { name: secret, type: { kind: String }, required: true,\n                targets: [server] }",
        "secret: { name: secret, type: { kind: String }, targets: [server] }",
    );
    fs::write(root.join("schemas/account.yaml"), relaxed).unwrap();
    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 0, "client build with optional server-only field");
    let artifact = fs::read_to_string(root.join("build/client/Account.json")).unwrap();
    assert!(!artifact.contains("secret"), "leak: {artifact}");
}
