use super::*;

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
