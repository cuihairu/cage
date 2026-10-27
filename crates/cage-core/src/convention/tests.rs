//! Convention engine tests (X1, design §51): grammar, vocabulary, anchors,
//! the config gate, and the A1 helpers. The script runner and response gate
//! run against a real subprocess in the cage-source-excel adapter tests.

use super::*;

fn grid(rows: &[&[&str]]) -> Grid {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    if c.is_empty() {
                        CellValue::Null
                    } else {
                        CellValue::Str((*c).to_string())
                    }
                })
                .collect()
        })
        .collect()
}

fn meta_of(rows: &[&[&str]], header_row: usize, territory: &[usize]) -> TableMeta {
    decode(
        &grid(rows),
        header_row,
        territory,
        0,
        0,
        DEFAULT_MARKER,
        "config/Skill.xlsx",
        "Skill",
    )
    .expect("decode must succeed")
}

/// The Luban-style row shape the 2026-10-26 sourcing directive points at:
/// keyword cell in the first column, per-column values across the row.
#[test]
fn marker_rows_type_comment_enum_rows() {
    let rows = [
        &["##type Int32", "String", "Enum<Rarity>"][..],
        &["##description 描述词", "武器名", "稀有度"][..],
        &["##enum", "", "common,rare,epic"][..],
        &["id", "name", "rarity"][..],
        &["1", "sword", "common"][..],
    ];
    let meta = meta_of(&rows, 3, &[0, 1, 2]);
    assert_eq!(meta.header_row, 3);

    let id = meta.columns.get("id").expect("id declared");
    assert_eq!(id.cell, "A1");
    assert_eq!(id.field_type, Some(FieldType::Int32));
    assert_eq!(id.description.as_deref(), Some("描述词"));
    assert_eq!(id.cell, "A1", "the keyword cell is the anchor");

    let name = meta.columns.get("name").expect("name declared");
    assert_eq!(name.cell, "B1");
    assert_eq!(name.field_type, Some(FieldType::String));
    assert_eq!(name.description.as_deref(), Some("武器名"));

    let rarity = meta.columns.get("rarity").expect("rarity declared");
    assert_eq!(rarity.cell, "C1");
    assert_eq!(
        rarity.field_type,
        Some(FieldType::Enum("Rarity".to_string()))
    );
    assert_eq!(
        rarity.enum_values.as_deref(),
        Some(["common", "rare", "epic"].map(String::from).as_slice())
    );
    assert_eq!(rarity.description.as_deref(), Some("稀有度"));
}

/// Per-cell inline form: the keyword and its own column's value share one
/// cell, several keywords share one row.
#[test]
fn inline_keyword_values_and_shared_rows() {
    let rows = [
        &["##required true", "##required false", "##unique true"][..],
        &["##primary_key true", "##min 0", "##max 999"][..],
        &["id", "dmg", "score"][..],
    ];
    let meta = meta_of(&rows, 2, &[0, 1]);
    assert_eq!(meta.columns["id"].required, Some(true));
    assert_eq!(meta.columns["id"].primary_key, Some(true));
    assert_eq!(meta.columns["dmg"].required, Some(false));
    assert_eq!(meta.columns["dmg"].min, Some(0.0));
    assert_eq!(meta.columns["score"].unique, Some(true));
    assert_eq!(meta.columns["score"].max, Some(999.0));
}

/// `default` accepts a JSON literal; `targets` and `enum` are comma lists;
/// `reference` is Table.field with an optional cardinality suffix.
#[test]
fn defaults_targets_and_references() {
    let rows = [
        &[
            "##default 10",
            "##targets client,server",
            "##reference Icon.id:many",
        ][..],
        &["##description 备注", "##default null", "##min 0"][..],
        &["id", "scope", "icon"][..],
    ];
    let meta = meta_of(&rows, 2, &[0, 1]);
    assert_eq!(meta.columns["id"].default, Some(serde_json::json!(10)));
    assert_eq!(meta.columns["id"].description.as_deref(), Some("备注"));
    assert_eq!(
        meta.columns["scope"].targets,
        ["client".to_string(), "server".to_string()]
    );
    assert_eq!(meta.columns["scope"].default, Some(serde_json::json!(null)));
    assert_eq!(meta.columns["icon"].min, Some(0.0));
    let reference = meta.columns["icon"].reference.as_ref().expect("reference");
    assert_eq!(reference.table, "Icon");
    assert_eq!(reference.field, "id");
    assert_eq!(reference.cardinality, "many");
}

/// Unknown keywords, values before any keyword, bare markers, duplicates —
/// all E2201, all with the cell anchor in the message and location. Every
/// cell that claims an unknown keyword is its own violation: three cells
/// saying `##var` are three authoring mistakes to fix.
#[test]
fn syntax_violations_are_e2201_with_anchors() {
    let rows = [
        &["##var", "String", "Int32"][..], // unknown keyword on all three columns
        &["name", "##description 描述", "x"][..], // a value before any keyword in its row
        &["##type Int32", "##type String", "##"][..], // the third cell is a bare marker
        &["##type Float64", "##type Float64", ""][..], // duplicate keyword on column "dup"
        &["id", "name", "age", "dup"][..],
    ];
    let diags = decode(
        &grid(&rows),
        4,
        &[0, 1, 2, 3][..],
        0,
        0,
        DEFAULT_MARKER,
        "config/Skill.xlsx",
        "Skill",
    )
    .expect_err("must fail");
    let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes.len(), 6, "one diagnostic per violation: {codes:?}");
    assert!(codes.iter().all(|c| *c == "E2201"), "{codes:?}");

    let messages: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("\"var\"") && m.contains("B1")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("\"var\"") && m.contains("C1")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("value \"name\" at A2") && m.contains("before any keyword")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("C3") && m.contains("carries no keyword")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("declares \"type\" twice")),
        "{messages:?}"
    );

    // Every violation carries the sheet + cell location
    for d in &diags {
        let location = d.location.as_ref().expect("location on every violation");
        assert_eq!(location.sheet.as_deref(), Some("Skill"));
        assert!(location.column.is_some() && location.row.is_some());
    }
}

fn messages_preview(diags: &Diagnostics) -> String {
    diags
        .iter()
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Values the vocabulary cannot resolve are E2202 with the column named.
#[test]
fn vocabulary_violations_are_e2202() {
    let rows = [
        &["##type integer", "##required yes", "##primary_key maybe"][..],
        &["##min zero", "##reference nopoint", "##type Object"][..],
        &["id", "flag", "tag"][..],
    ];
    let diags = decode(
        &grid(&rows),
        2,
        &[0, 1][..],
        0,
        0,
        DEFAULT_MARKER,
        "config/Skill.xlsx",
        "Skill",
    )
    .expect_err("must fail");
    let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes.len(), 6, "{}", messages_preview(&diags));
    assert!(
        codes.iter().all(|c| *c == "E2202"),
        "{}",
        messages_preview(&diags)
    );
    let messages: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("not a Schema type name")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("expected true or false")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("not a number")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("Table.field")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("cannot be declared in a cell")),
        "{messages:?}"
    );
}

/// A declaration cell to the right of the header row's last column cannot
/// attach to any column — E2201, anchored.
#[test]
fn declaration_beyond_the_header_row_is_e2201() {
    let rows = [
        &["##type Int32", "String", "Float64"][..],
        &["id", "name"][..],
    ];
    let diags = decode(
        &grid(&rows),
        1,
        &[0],
        0,
        0,
        DEFAULT_MARKER,
        "config/Skill.xlsx",
        "Skill",
    )
    .expect_err("must fail");
    assert!(diags.iter().all(|d| d.code == "E2201"));
    assert!(
        diags.iter().any(|d| d.message.contains("C1")),
        "{:?}",
        diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn type_cell_notations() {
    // Cell shorthand...
    assert_eq!(parse_type_text("Int32").unwrap(), FieldType::Int32);
    assert_eq!(parse_type_text("Any").unwrap(), FieldType::Any);
    assert_eq!(
        parse_type_text("Array<Int32>").unwrap(),
        FieldType::Array(Box::new(FieldType::Int32))
    );
    assert_eq!(
        parse_type_text("Array<Array<String>>").unwrap(),
        FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::String))))
    );
    assert_eq!(
        parse_type_text("Enum<Rarity>").unwrap(),
        FieldType::Enum("Rarity".to_string())
    );
    assert_eq!(
        parse_type_text("Map<string, Int32>").unwrap(),
        FieldType::Map(MapField {
            key_type: MapKeyType::String,
            value_type: Box::new(FieldType::Int32),
        })
    );
    // ...and the Schema's own expression form, parsed by the Schema's own
    // deserializer (no second vocabulary table).
    assert_eq!(
        parse_type_text("{ kind: Int32 }").unwrap(),
        FieldType::Int32
    );
    assert_eq!(
        parse_type_text("{ kind: Array, value: { kind: Int32 } }").unwrap(),
        FieldType::Array(Box::new(FieldType::Int32))
    );
    assert_eq!(
        parse_type_text("{ kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }")
            .unwrap(),
        FieldType::Map(MapField {
            key_type: MapKeyType::String,
            value_type: Box::new(FieldType::Int32),
        })
    );

    // Objects cannot be declared in a cell; unknown spellings are refused.
    assert!(parse_type_text("Object").is_err());
    assert!(parse_type_text("Object<hp: Int32>").is_err());
    assert!(parse_type_text("Array").is_err());
    assert!(parse_type_text("Map<string>").is_err());
    assert!(parse_type_text("Map<bool, Int32>").is_err());
    assert!(parse_type_text("Enum<9bad>").is_err());
    assert!(parse_type_text("int").is_err());
    assert!(parse_type_text("").is_err());
}

#[test]
fn col_letters_and_anchors() {
    assert_eq!(col_index_to_letter(0), "A");
    assert_eq!(col_index_to_letter(25), "Z");
    assert_eq!(col_index_to_letter(26), "AA");
    assert_eq!(col_index_to_letter(702), "AAA");
    assert_eq!(anchor(26, 6), "G27");
}

/// A used range not starting at A1: anchors must still be real sheet
/// addresses (row0/col0 shift).
#[test]
fn anchors_account_for_range_offset() {
    let rows = [&["##type Int32", "String"][..], &["id", "name"][..]];
    let meta = decode(
        &grid(&rows),
        1,
        &[0],
        1, // range starts at sheet row 2
        1, // ...and column B
        DEFAULT_MARKER,
        "config/Skill.xlsx",
        "Skill",
    )
    .expect("decode");
    // Sheet B2 is the type cell for column B (id), C2 for column C (name)
    assert_eq!(meta.columns["id"].cell, "B2");
    assert_eq!(meta.columns["name"].cell, "C2");
}

#[test]
fn config_gate_and_workbook_overrides() {
    let mut cfg = ExcelConvention::default();
    assert!(cfg.validate().is_ok(), "default is the data baseline");
    assert!(cfg.preset_for("Skill.xlsx").is_data());
    assert!(!cfg.uses_script());

    cfg.preset = Preset::MarkerRows;
    cfg.workbooks
        .insert("Skill.xlsx".to_string(), Preset::HeaderBlock);
    assert!(cfg.validate().is_ok());
    assert_eq!(cfg.preset_for("Skill.xlsx"), Preset::HeaderBlock);
    assert_eq!(cfg.preset_for("Item.xlsx"), Preset::MarkerRows);

    let bad_marker = ExcelConvention {
        marker: "//".to_string(),
        ..Default::default()
    };
    assert!(bad_marker.validate().unwrap_err().starts_with("E2201"));

    let both = ExcelConvention {
        preset: Preset::MarkerRows,
        script: Some("tools/x.py".to_string()),
        ..Default::default()
    };
    assert!(both.validate().unwrap_err().contains("mutually exclusive"));

    let overrides_with_script = ExcelConvention {
        script: Some("tools/x.py".to_string()),
        workbooks: IndexMap::from([("Skill.xlsx".to_string(), Preset::Data)]),
        ..Default::default()
    };
    assert!(overrides_with_script
        .validate()
        .unwrap_err()
        .contains("do not apply"));
}

/// `ColumnMeta` is the vocabulary gate: unknown keys must not deserialize.
#[test]
fn column_meta_rejects_invented_keywords() {
    let json = serde_json::json!({
        "cell": "B2",
        "type": {"kind": "Int32"},
        "comment": "not a cage keyword"
    });
    assert!(serde_json::from_value::<ColumnMeta>(json).is_err());

    let ok = serde_json::from_value::<ColumnMeta>(serde_json::json!({
        "cell": "B2",
        "type": {"kind": "Int32"},
        "enum": ["a", "b"],
        "reference": {"table": "Icon", "field": "id", "cardinality": "many"}
    }))
    .expect("vocabulary deserializes");
    assert_eq!(ok.field_type, Some(FieldType::Int32));
    assert_eq!(
        ok.enum_values.as_deref(),
        Some(["a", "b"].map(String::from).as_slice())
    );
    assert_eq!(ok.reference.as_ref().unwrap().cardinality, "many");
}

/// A meta sheet decodes with the same grammar keyed by its own name row.
#[test]
fn meta_sheet_grid_decodes_by_its_name_row() {
    // The meta sheet's first row is the column-name row; territory is the
    // rows below it. The name row must match the main sheet's header (the
    // adapter checks that; here the grammar is what is under test).
    let rows = [
        &["id", "name", "rarity"][..],
        &["##type Int32", "String", "Enum<Rarity>"][..],
        &["##description 列注释", "武器名", "稀有度"][..],
    ];
    let meta = meta_of(&rows, 0, &[1, 2]);
    assert_eq!(meta.columns["id"].field_type, Some(FieldType::Int32));
    assert_eq!(meta.columns["name"].description.as_deref(), Some("武器名"));
    assert_eq!(
        meta.columns["rarity"].field_type,
        Some(FieldType::Enum("Rarity".to_string()))
    );
}
