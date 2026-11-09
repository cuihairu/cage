use super::*;

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

/// Two tables referencing each other: L5 emits an E1403 cycle warning, the
/// check still passes (cycles do not block builds), and
/// `warnings_as_errors = true` escalates it to a validation failure.
#[test]
fn reference_cycle_warns_e1403_and_escalates() {
    let tmp = tempfile::tempdir().unwrap();
    let schema = r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      kit: { name: kit, type: { kind: Int32 }, reference: { table: Kit, field: id } }
  Kit:
    name: Kit
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      item: { name: item, type: { kind: Int32 }, reference: { table: Item, field: id } }
enums: {}
"#;
    fs::create_dir_all(tmp.path().join("schemas")).unwrap();
    fs::create_dir_all(tmp.path().join("config")).unwrap();
    fs::write(tmp.path().join("schemas/cycle.yaml"), schema).unwrap();
    fs::write(
        tmp.path().join("config/Item.json"),
        r#"{"Item": [{"id": 1, "kit": 1}]}"#,
    )
    .unwrap();
    fs::write(
        tmp.path().join("config/Kit.json"),
        r#"{"Kit": [{"id": 1, "item": 1}]}"#,
    )
    .unwrap();

    let cage_toml = |wae: bool| {
        format!(
            r#"output_dir = "build"
schema_path = "schemas"
warnings_as_errors = {wae}

[project]
name = "cycle"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
"#
        )
    };

    fs::write(tmp.path().join("cage.toml"), cage_toml(false)).unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "cycle warns without failing");
    let out_str = stdout(&out);
    assert!(out_str.contains("E1403"), "stdout:\n{out_str}");
    assert!(
        out_str.contains("Circular reference: Item → Kit → Item"),
        "stdout:\n{out_str}"
    );
    assert!(out_str.contains("cage check: OK"), "stdout:\n{out_str}");

    fs::write(tmp.path().join("cage.toml"), cage_toml(true)).unwrap();
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_code(&out, 1, "warnings_as_errors escalates the cycle");
    assert!(stdout(&out).contains("E1403"), "stdout:\n{}", stdout(&out));
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
fn build_msgpack_target_writes_deterministic_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let toml = format!(
        "{CAGE_TOML}\n[[profiles.client.targets]]\nformat = \"msgpack\"\noutput_dir = \"build/msgpack\"\n"
    );
    fs::write(tmp.path().join("cage.toml"), toml).unwrap();
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert_code(&out, 0, "build with msgpack target");

    let artifact_path = tmp.path().join("build/msgpack/Item.msgpack");
    let bytes1 = fs::read(&artifact_path).unwrap();
    // Golden bytes: fixarray(1) of fixmap(2) with sorted row fields
    // (id, name) — same shape the msgpack crate's unit golden locks.
    let expected: Vec<u8> = [
        0x91, 0x82, 0xa2, b'i', b'd', 0x01, 0xa4, b'n', b'a', b'm', b'e', 0xa5, b'S', b'w', b'o',
        b'r', b'd',
    ]
    .to_vec();
    assert_eq!(
        bytes1, expected,
        "msgpack bytes not deterministic-canonical"
    );

    // A fresh full build over the same inputs reproduces the same bytes.
    let out = run_cage(&["build", tmp.path().to_str().unwrap()]);
    assert!(
        stdout(&out).contains("cage build: OK"),
        "stdout:\n{}",
        stdout(&out)
    );
    let bytes2 = fs::read(&artifact_path).unwrap();
    assert_eq!(bytes1, bytes2);

    // The manifest records the artifact under the msgpack format.
    let manifest = fs::read_to_string(tmp.path().join("build/manifest.json")).unwrap();
    assert!(manifest.contains("msgpack"), "manifest:\n{manifest}");
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
