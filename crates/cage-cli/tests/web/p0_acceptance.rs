//! P0-6 acceptance: the P0 edit closed-loop over a real project with Excel
//! data — open Monster → add a field → rename it → relate Items → a range
//! rule → Excel data validation with an accurate error row → post-fix diff
//! preview → save back to the source files. No YAML is hand-edited during
//! the loop (the editor document is a JSON value; every write is the tool's
//! canonical path), and the CLI and the Web report the same violation.

use super::*;
use std::path::Path;

/// Directory-schema project: `Items` is a plain-data Excel workbook (the
/// `merged_cells` fixture, sheet `Items`, header + three rows) and `Monster`
/// is a JSON-backed table. Both source files live under `schemas/` so the
/// save goes through the per-source-file incremental path.
fn write_p0_project(root: &Path) {
    fs::write(root.join("cage.toml"), P0_TOML).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::create_dir_all(root.join("config")).unwrap();

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../cage-source-excel/tests/fixtures/merged_cells.xlsx");
    fs::copy(&fixture, root.join("config/Items.xlsx")).unwrap();
    fs::write(
        root.join("config/monster.json"),
        r#"{"Monster": [{"id": 1, "name": "Goblin"}]}"#,
    )
    .unwrap();

    fs::write(root.join("schemas/item.yaml"), P0_ITEM).unwrap();
    fs::write(root.join("schemas/monster.yaml"), P0_MONSTER).unwrap();
}

const P0_TOML: &str = r#"schema_path = "schemas"

[project]
name = "p0-acceptance"
version = "0.1.0"

[source_roots]
main = "config"

[source.excel]
preset = "data"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#;

/// The `Items` table mirrors the workbook: `type` and `note` have empty
/// cells in some rows, so they are nullable; `price` is present in every row.
const P0_ITEM: &str = r#"tables:
  Items:
    name: Items
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
      type: { name: type, type: { kind: String }, nullable: true }
      price: { name: price, type: { kind: Int32 } }
      note: { name: note, type: { kind: String }, nullable: true }
enums: {}
"#;

const P0_MONSTER: &str = r#"tables:
  Monster:
    name: Monster
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
enums: {}
"#;

#[test]
fn p0_edit_closed_loop_over_real_excel_data() {
    let tmp = tempfile::tempdir().unwrap();
    write_p0_project(tmp.path());
    let server = start_server(tmp.path());

    // The baseline project is clean before any edit.
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "baseline must check clean\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 1. Open the editor: the merged document carries both tables and the
    //    per-source-file index for the incremental save.
    let (status, body) = request(server.port, "GET", "/api/schema", None);
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    assert!(v["schema"]["tables"]["Monster"].is_object(), "{body}");
    assert!(v["schema"]["tables"]["Items"].is_object(), "{body}");
    let sources = v["sources"].clone();
    assert_eq!(sources["files"].as_array().unwrap().len(), 2, "{body}");
    let mut doc = v["schema"].clone();

    // 2. Add a field to Monster — a document edit, never a YAML edit.
    doc["tables"]["Monster"]["fields"]["favorite_item"] =
        serde_json::json!({"name": "favorite_item", "type": {"kind": "Int32"}});

    // 3. Rename the new field (P0-4): every site the rename owns patches.
    let rename = serde_json::json!({
        "schema": doc,
        "target": {"kind": "field", "table": "Monster", "field": "favorite_item"},
        "new_name": "best_item"
    });
    let (status, body) = request(
        server.port,
        "POST",
        "/api/schema/rename",
        Some(&serde_json::to_string(&rename).unwrap()),
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "body: {body}");
    assert_eq!(v["notes"].as_array().unwrap().len(), 0, "{body}");
    assert!(
        v["schema"]["tables"]["Monster"]["fields"]
            .get("favorite_item")
            .is_none(),
        "the old key is gone\n{body}"
    );
    assert_eq!(
        v["schema"]["tables"]["Monster"]["fields"]["best_item"]["name"], "best_item",
        "{body}"
    );
    doc = v["schema"].clone();

    // 4. Relate Items: the renamed field references the Excel table's key.
    doc["tables"]["Monster"]["fields"]["best_item"]["reference"] = serde_json::json!({
        "table": "Items", "field": "id", "cardinality": "one"
    });

    // 5. A range rule the real workbook violates (Shield costs 250 > 200).
    doc["tables"]["Items"]["fields"]["price"]["rules"] = serde_json::json!([
        {"name": "price_cap", "assert": "price <= 200",
         "message": "price must be <= 200", "warning_only": false}
    ]);

    // 6. Excel data validation: the draft against the real data, and the
    //    violation lands on the exact row (Excel row 4 = Shield, price 250).
    let body = serde_json::to_string(&doc).unwrap();
    let (status, body) = request(server.port, "POST", "/api/validate/data", Some(&body));
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], false, "body: {body}");
    let diags = v["diagnostics"].as_array().expect("merged list");
    assert_eq!(diags.len(), 1, "only the range rule fails\n{v}");
    let violation = &diags[0];
    assert_eq!(violation["code"], "E1501", "{v}");
    assert_eq!(violation["scope"], "data", "{v}");
    assert_eq!(
        violation["row"], "1",
        "the second data row of the table\n{v}"
    );
    assert_eq!(violation["location"]["row"], 4, "the exact Excel row\n{v}");
    assert_eq!(violation["location"]["sheet"], "Items", "{v}");
    assert_eq!(violation["hint"], "price must be <= 200", "{v}");

    // The CLI sees the same single violation from the same document: the
    // editor value serialized to YAML (not hand-written) and `cage check`ed
    // against the real data.
    let cli_tmp = tempfile::tempdir().unwrap();
    write_p0_project(cli_tmp.path());
    let cli_doc = serde_yaml::to_string(&doc).expect("draft to yaml");
    fs::write(cli_tmp.path().join("schemas/single.yaml"), cli_doc).unwrap();
    let mut cli_toml = fs::read_to_string(cli_tmp.path().join("cage.toml")).unwrap();
    cli_toml = cli_toml.replace(
        "schema_path = \"schemas\"",
        "schema_path = \"schemas/single.yaml\"",
    );
    fs::write(cli_tmp.path().join("cage.toml"), cli_toml).unwrap();
    let out = run_cage(&["check", cli_tmp.path().to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the CLI must refuse the same draft\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(out.contains("E1501"), "{out}");
    assert!(
        out.contains("Row: 4"),
        "CLI names the same Excel row\n{out}"
    );
    assert!(out.contains("Sheet: Items"), "{out}");

    // 7. Post-fix diff preview (P0-2): fix the rule, and the preview names
    //    exactly the two source files the save would rewrite.
    doc["tables"]["Items"]["fields"]["price"]["rules"] = serde_json::json!([
        {"name": "price_cap", "assert": "price <= 300",
         "message": "price must be <= 300", "warning_only": false}
    ]);
    let (_, body) = request(
        server.port,
        "POST",
        "/api/validate/data",
        Some(&serde_json::to_string(&doc).unwrap()),
    );
    assert_eq!(
        json(&body)["ok"],
        true,
        "fixed draft validates clean\n{body}"
    );

    let preview = serde_json::json!({"schema": doc, "sources": sources});
    let (status, body) = request(
        server.port,
        "POST",
        "/api/schema/preview",
        Some(&serde_json::to_string(&preview).unwrap()),
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "{body}");
    assert_eq!(v["unchanged"], 0, "{body}");
    let files = v["files"].as_array().expect("preview files");
    assert_eq!(files.len(), 2, "both source files would be rewritten\n{v}");
    for f in files {
        assert!(!f["diff"].as_str().unwrap().is_empty(), "{v}");
    }

    // 8. Save back to the source files — the canonical path, no hand edits.
    let (status, body) = request(
        server.port,
        "POST",
        "/api/schema",
        Some(&serde_json::to_string(&preview).unwrap()),
    );
    assert_eq!(status, 200, "body: {body}");
    let v = json(&body);
    assert_eq!(v["ok"], true, "{body}");
    assert_eq!(v["unchanged"], 0, "{body}");
    let written: Vec<&str> = v["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    for file in ["schemas/item.yaml", "schemas/monster.yaml"] {
        assert!(written.contains(&file), "missing {file}\n{v}");
    }

    // The saved fragments carry the edits in canonical form.
    let monster = fs::read_to_string(tmp.path().join("schemas/monster.yaml")).unwrap();
    assert!(monster.contains("best_item:"), "{monster}");
    assert!(monster.contains("table: Items"), "{monster}");
    let item = fs::read_to_string(tmp.path().join("schemas/item.yaml")).unwrap();
    assert!(item.contains("price <= 300"), "{item}");
    assert!(!item.contains("price <= 200"), "{item}");

    // Atomic saves leave no temp litter.
    let litter = fs::read_dir(tmp.path().join("schemas"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .count();
    assert_eq!(litter, 0, "no leftover temp files");

    // 9. The saved project is what the real build path reads: the CLI checks
    //    it clean, agreeing with the Web's post-fix validation.
    let out = run_cage(&["check", tmp.path().to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
