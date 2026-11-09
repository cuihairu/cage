use super::*;

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
