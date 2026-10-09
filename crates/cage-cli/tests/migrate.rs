//! Process-level coverage of `cage migrate` (M3, design §46): the chain in
//! `migrations/` applies to the loaded document, reverifies under the
//! current schema, and `--write` rewrites the local text sources in place.
//! Dry-run is the default and touches no bytes; a re-run over already
//! migrated data reports zero changed rows and unchanged files; Excel
//! sources stay report-only; a failed reverification (E2004) fails the
//! command without writing anything.

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

const JSON_TOML: &str = r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "migrateproj"
version = "1.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#;

/// The current (post-migration) schema: the CLI holds this version —
/// migration rules move the data, the schema was already edited.
const MIGRATED_SCHEMA: &str = r#"tables:
  Item:
    name: Item
    description: An inventory item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
        required: true
      title:
        name: title
        type: { kind: String }
        required: true
      rarity:
        name: rarity
        type: { kind: String }
        required: true
      grade:
        name: grade
        type: { kind: String }
      level:
        name: level
        type: { kind: Int64 }
enums: {}
"#;

const OLD_ITEM_DATA: &str = r#"{
  "Item": [
    { "id": 1, "name": "Sword", "grade": "S", "level": 10 },
    { "id": 2, "name": "Shield", "grade": "A", "level": 20, "rarity": "rare" }
  ]
}
"#;

const FIRST_SEGMENT: &str = r#"from: "1.0.0"
to: "1.1.0"
steps:
  - rename_field:
      table: Item
      from: name
      to: title
  - set_default:
      table: Item
      field: rarity
      value: "common"
  - widen_type:
      table: Item
      field: level
      to: { kind: Int64 }
"#;

fn write_json_project(root: &Path) -> PathBuf {
    let proj = root.join("proj");
    write(&proj.join("cage.toml"), JSON_TOML);
    write(&proj.join("schema.yaml"), MIGRATED_SCHEMA);
    write(&proj.join("config/item.json"), OLD_ITEM_DATA);
    write(
        &proj.join("migrations/0001-rename-item.yaml"),
        FIRST_SEGMENT,
    );
    proj
}

/// Dry-run reports the plan (segments, per-step row counts, the files
/// --write would touch) and leaves every byte untouched.
#[test]
fn json_dry_run_reports_plan_without_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = write_json_project(tmp.path());
    let data_path = proj.join("config/item.json");
    let before = fs::read(&data_path).unwrap();

    let out = run_cage(&["migrate", proj.to_str().unwrap()]);
    assert_code(&out, 0, "json dry run");
    let out_str = stdout(&out);
    for line in [
        "cage migrate: segment 0001-rename-item.yaml (1.0.0 → 1.1.0)",
        "rename_field(Item.name → title): 2 row(s)",
        // only row 1 lacks `rarity`
        "set_default(Item.rarity): 1 row(s)",
        // canonical Int carries no width: Int32 → Int64 rewrites nothing
        "widen_type(Item.level → Int64): 0 row(s)",
        "would write",
        "config/item.json (1 table(s))",
        "cage migrate: OK (1 segment(s), 3 row(s) migrated",
        "dry run, nothing written",
    ] {
        assert!(out_str.contains(line), "missing '{line}' in:\n{out_str}");
    }
    assert_eq!(
        fs::read(&data_path).unwrap(),
        before,
        "dry run must not touch source bytes"
    );
}

/// --write rewrites the source in place; `cage check` passes on the
/// migrated data under the current schema; a second --write run is a
/// no-op (zero changed rows, all files unchanged).
#[test]
fn json_write_then_check_then_rewrite_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = write_json_project(tmp.path());
    let data_path = proj.join("config/item.json");

    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--write"]);
    assert_code(&out, 0, "first write");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("wrote"),
        "expected a wrote line:\n{out_str}"
    );

    let migrated = fs::read_to_string(&data_path).unwrap();
    assert!(migrated.contains("\"title\""), "renamed field:\n{migrated}");
    assert!(
        !migrated.contains("\"name\""),
        "old field gone:\n{migrated}"
    );
    assert!(
        migrated.contains("\"rarity\": \"common\""),
        "default filled:\n{migrated}"
    );

    // The migrated product validates clean under the current schema.
    let out = run_cage(&["check", proj.to_str().unwrap()]);
    assert_code(&out, 0, "check after migrate");
    assert!(stdout(&out).contains("cage check: OK"));

    // Second run: every step finds its target in place — 0 rows, and the
    // file renders byte-identically so nothing is rewritten.
    let after_first = fs::read(&data_path).unwrap();
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--write"]);
    assert_code(&out, 0, "second write");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("cage migrate: OK (1 segment(s), 0 row(s) migrated"),
        "second run migrates nothing:\n{out_str}"
    );
    assert!(
        out_str.contains("unchanged"),
        "no file rewritten:\n{out_str}"
    );
    assert!(!out_str.contains("wrote "), "nothing rewritten:\n{out_str}");
    assert_eq!(
        fs::read(&data_path).unwrap(),
        after_first,
        "second run must leave bytes identical"
    );
}

/// A missing migrations/ directory is a clean nothing-to-migrate, not an
/// error; a malformed rule file is E2001 and fails.
#[test]
fn missing_dir_is_noop_and_bad_rule_is_e2001() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("plain");
    write(&proj.join("cage.toml"), JSON_TOML);
    write(&proj.join("schema.yaml"), MIGRATED_SCHEMA);
    write(&proj.join("config/item.json"), OLD_ITEM_DATA);

    let out = run_cage(&["migrate", proj.to_str().unwrap()]);
    assert_code(&out, 0, "no migrations dir");
    assert!(
        stdout(&out).contains("nothing to migrate"),
        "{}",
        stdout(&out)
    );

    write(
        &proj.join("migrations/0001-bad.yaml"),
        "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps: []\n",
    );
    let out = run_cage(&["migrate", proj.to_str().unwrap()]);
    assert_code(&out, 1, "empty steps");
    assert!(stderr(&out).contains("E2001"), "{}", stderr(&out));
}

/// Segment selection: the chain runs whole with --all, a prefix with
/// --to, and an off-chain version is a usage error. Every intermediate
/// state must itself satisfy the current schema — the reverification
/// always runs against the one schema on disk.
#[test]
fn segment_selection_all_to_and_offchain() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("proj");
    write(&proj.join("cage.toml"), JSON_TOML);
    write(&proj.join("schema.yaml"), MIGRATED_SCHEMA);
    write(&proj.join("config/item.json"), OLD_ITEM_DATA);
    write(
        &proj.join("migrations/0001-rename-item.yaml"),
        FIRST_SEGMENT,
    );
    // Second segment stays schema-neutral (an enum value rewrite), so the
    // 1.1.0 intermediate state still reverifies clean.
    write(
        &proj.join("migrations/0002-regrade.yaml"),
        "from: \"1.1.0\"\nto: \"1.2.0\"\nsteps:\n  - remap_values:\n      table: Item\n      field: grade\n      map:\n        \"S\": \"legendary\"\n",
    );

    // --to 1.1.0: only the first segment.
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--to", "1.1.0"]);
    assert_code(&out, 0, "--to 1.1.0");
    let out_str = stdout(&out);
    assert!(out_str.contains("0001-rename-item.yaml"), "{out_str}");
    assert!(!out_str.contains("0002-regrade.yaml"), "{out_str}");
    assert!(out_str.contains("1 segment(s)"), "{out_str}");

    // --to a version no segment ends at is a usage error.
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--to", "9.9.9"]);
    assert_code(&out, 2, "--to off-chain");
    assert!(
        stderr(&out).contains("no segment on the migration chain"),
        "{}",
        stderr(&out)
    );

    // --all: the whole chain, both segments in order.
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--all"]);
    assert_code(&out, 0, "--all");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("0001-rename-item.yaml") && out_str.contains("0002-regrade.yaml"),
        "{out_str}"
    );
    assert!(out_str.contains("2 segment(s)"), "{out_str}");
    assert!(
        out_str.contains("remap_values(Item.grade, 1 entry): 1 row(s)"),
        "{out_str}"
    );

    // --to latest resolves to the whole chain — byte-identical output to
    // --all (the symbol carries no other effect on the run).
    let all_out = run_cage(&["migrate", proj.to_str().unwrap(), "--all"]);
    let latest = run_cage(&["migrate", proj.to_str().unwrap(), "--to", "latest"]);
    assert_code(&latest, 0, "--to latest");
    assert_eq!(
        stdout(&latest),
        stdout(&all_out),
        "--to latest must select exactly what --all selects"
    );
}

// ---------------------------------------------------------------- Excel ---

const EXCEL_TOML: &str = r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "excelproj"
version = "1.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#;

/// Schema covering the shared merged_cells.xlsx fixture (id/type/name/
/// note/price) plus `rarity` — required, and only a migration rule can
/// fill it in.
const EXCEL_SCHEMA: &str = r#"tables:
  Items:
    name: Items
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
        required: true
      type:
        name: type
        type: { kind: String }
      name:
        name: name
        type: { kind: String }
      note:
        name: note
        type: { kind: String }
      price:
        name: price
        type: { kind: Int32 }
      rarity:
        name: rarity
        type: { kind: String }
        required: true
enums: {}
"#;

fn write_excel_project(root: &Path, rule: &str) -> PathBuf {
    let proj = root.join("proj");
    write(&proj.join("cage.toml"), EXCEL_TOML);
    write(&proj.join("schema.yaml"), EXCEL_SCHEMA);
    let xlsx = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../cage-source-excel/tests/fixtures/merged_cells.xlsx"
    );
    fs::create_dir_all(proj.join("config")).unwrap();
    fs::copy(xlsx, proj.join("config/Items.xlsx")).unwrap();
    write(&proj.join("migrations/0001-fill-rarity.yaml"), rule);
    proj
}

/// A rule that leaves a required field unfilled reverified-fails as
/// E2004 — the segment report is on stdout, nothing is written (Excel
/// sources are report-only anyway, and a failed verification writes
/// nothing at all).
#[test]
fn excel_failed_reverification_is_e2004_with_report() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = write_excel_project(
        tmp.path(),
        "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps:\n  - rename_field:\n      table: Items\n      from: note\n      to: remark\n",
    );
    let xlsx_path = proj.join("config/Items.xlsx");
    let before = fs::read(&xlsx_path).unwrap();

    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--write"]);
    assert_code(&out, 1, "E2004");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("cage migrate: segment 0001-fill-rarity.yaml (1.0.0 → 1.1.0)"),
        "{out_str}"
    );
    assert!(
        out_str.contains("rename_field(Items.note → remark): 3 row(s)"),
        "{out_str}"
    );
    let err_str = stderr(&out);
    assert!(err_str.contains("E2004"), "{err_str}");
    assert!(err_str.contains("nothing written"), "{err_str}");
    assert_eq!(
        fs::read(&xlsx_path).unwrap(),
        before,
        "a failed migration writes nothing"
    );
}

/// A satisfiable migration over an Excel source: the rule also fills the
/// fixture's merged-cell null note (L2 has no nullable notion), the
/// report carries the Excel skip line, and even `--write` leaves the
/// workbook untouched.
#[test]
fn excel_success_reports_skip_and_never_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = write_excel_project(
        tmp.path(),
        "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps:\n  - set_default:\n      table: Items\n      field: rarity\n      value: \"common\"\n  - set_default:\n      table: Items\n      field: note\n      value: \"\"\n",
    );
    let xlsx_path = proj.join("config/Items.xlsx");
    let before = fs::read(&xlsx_path).unwrap();

    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--write"]);
    assert_code(&out, 0, "excel write run");
    let out_str = stdout(&out);
    assert!(
        out_str.contains("set_default(Items.rarity): 3 row(s)"),
        "{out_str}"
    );
    // the fixture's two merged-null note cells
    assert!(
        out_str.contains("set_default(Items.note): 2 row(s)"),
        "{out_str}"
    );
    assert!(
        out_str.contains("skip") && out_str.contains("Excel source: report only"),
        "{out_str}"
    );
    assert!(
        out_str.contains("cage migrate: OK (1 segment(s), 5 row(s) migrated"),
        "{out_str}"
    );
    assert!(!out_str.contains("wrote "), "nothing written:\n{out_str}");
    assert_eq!(
        fs::read(&xlsx_path).unwrap(),
        before,
        "Excel sources are never rewritten"
    );
}

/// Excel-served tables are never written back, so the report carries the
/// affected rows' cell-level addresses — `file | Sheet: <name> | Row: <n>`
/// — under each step's row count, capped deterministically when there are
/// many. Rows are 1-indexed on the sheet grid (the fixture's data starts
/// at sheet row 3, header on row 2).
#[test]
fn excel_report_carries_cell_level_locations() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = write_excel_project(
        tmp.path(),
        "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps:\n  - set_default:\n      table: Items\n      field: rarity\n      value: \"common\"\n  - set_default:\n      table: Items\n      field: note\n      value: \"\"\n",
    );
    let xlsx = proj.join("config/Items.xlsx").display().to_string();

    let out = run_cage(&["migrate", proj.to_str().unwrap()]);
    assert_code(&out, 0, "excel location report");
    let out_str = stdout(&out);

    assert!(
        out_str.contains("  set_default(Items.rarity): 3 row(s)"),
        "{out_str}"
    );
    // The three fixture data rows, sheet-grid addressed (header on row 2,
    // data on rows 3-5); merged-cell rows still carry their own location.
    for row in [3, 4, 5] {
        let loc = format!("{xlsx} | Sheet: Items | Row: {row}");
        assert!(
            out_str.lines().any(|l| l.trim() == loc),
            "missing '{loc}' in:\n{out_str}"
        );
    }
    // The note fill touches only the two merged-null rows (row 5 carries
    // a note already) — only those two addresses render under its line.
    let note_block = out_str
        .lines()
        .skip_while(|l| !l.contains("set_default(Items.note): 2 row(s)"))
        .skip(1)
        .take_while(|l| l.trim_start().starts_with(&xlsx))
        .collect::<Vec<_>>();
    assert_eq!(note_block.len(), 2, "{out_str}");
    for row in [3, 4] {
        let loc = format!("{xlsx} | Sheet: Items | Row: {row}");
        assert!(
            note_block.iter().any(|l| l.trim() == loc),
            "missing '{loc}' under the note step:\n{out_str}"
        );
    }
    assert!(
        note_block.iter().all(|l| !l.contains("Row: 5")),
        "row 5 has a note, it must not render:\n{out_str}"
    );
    // Plain-text tables never gain per-row lines — only Excel rows are
    // addressed here (the fixture is Excel-only, so nothing else renders).
    assert!(
        !out_str.contains("… +"),
        "three rows must fit under the cap:\n{out_str}"
    );
}

/// A chain that literally contains a segment ending at a version named
/// `latest` resolves `--to latest` as that version first — the concrete
/// target wins over the symbol, selecting the prefix that ends there, not
/// the whole chain.
#[test]
fn to_latest_resolves_the_literal_chain_version_before_the_symbol() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("proj");
    write(&proj.join("cage.toml"), JSON_TOML);
    write(&proj.join("schema.yaml"), MIGRATED_SCHEMA);
    write(&proj.join("config/item.json"), OLD_ITEM_DATA);
    // First segment ends at a version literally named `latest`.
    write(
        &proj.join("migrations/0001-a.yaml"),
        "from: \"1.0.0\"\nto: \"latest\"\nsteps:\n  - rename_field:\n      table: Item\n      from: name\n      to: title\n  - set_default:\n      table: Item\n      field: rarity\n      value: \"common\"\n",
    );
    // Second segment continues past it.
    write(
        &proj.join("migrations/0002-b.yaml"),
        "from: \"latest\"\nto: \"1.2.0\"\nsteps:\n  - remap_values:\n      table: Item\n      field: grade\n      map:\n        \"S\": \"legendary\"\n",
    );

    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--to", "latest"]);
    assert_code(&out, 0, "--to literal latest");
    let out_str = stdout(&out);
    assert!(out_str.contains("0001-a.yaml"), "{out_str}");
    assert!(
        !out_str.contains("0002-b.yaml"),
        "the literal version must win over the chain symbol:\n{out_str}"
    );
    assert!(out_str.contains("1 segment(s)"), "{out_str}");

    // Only the explicit whole-chain selectors reach the second segment.
    let out = run_cage(&["migrate", proj.to_str().unwrap(), "--all"]);
    assert_code(&out, 0, "--all over literal latest chain");
    assert!(stdout(&out).contains("2 segment(s)"), "{}", stdout(&out));
}
