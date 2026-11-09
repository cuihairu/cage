use super::*;

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
