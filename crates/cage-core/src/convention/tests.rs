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
        &HashSet::new(),
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

/// A row whose first marker cell is `+` declares sub-columns (design §51
/// Q6): each non-marker cell becomes a child of the header column at its
/// own column, and later keyword rows descend into the deepest sub-column.
#[test]
fn sub_column_row_builds_nested_columns() {
    // The parent name repeats at C and D: in a real workbook a merged span
    // makes the covered cell take the anchor's name, so the decoder sees the
    // same text at both columns and groups the two sub-columns under it.
    let rows = [
        &["##var", "id", "weapon", "weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "Int32", "Int32"][..],
    ];
    let meta = meta_of(&rows, 0, &[0, 1, 2]);
    let weapon = meta.columns.get("weapon").expect("weapon declared");
    assert_eq!(weapon.cell, "C2", "the `##+` cell is the parent anchor");
    assert_eq!(
        weapon.sub_columns.keys().collect::<Vec<_>>(),
        ["atk", "def"],
        "sub-columns in declaration order"
    );
    assert_eq!(
        weapon.sub_columns["atk"].field_type,
        Some(FieldType::Int32),
        "the `##type` row descends into the sub-column scope"
    );
    assert_eq!(weapon.sub_columns["def"].field_type, Some(FieldType::Int32));
}

/// A `##+` row whose cell carries a second marker is two markers in one row,
/// which the sub-column grammar forbids (E2201, anchor reported).
#[test]
fn sub_column_row_with_a_second_marker_is_e2201() {
    let rows = [
        &["##var", "id", "weapon"][..],
        &["##+", "", "##type Int32"][..],
        &["id", "name", "weapon"][..],
    ];
    let diags = decode(
        &grid(&rows),
        0,
        &[0, 1][..],
        0,
        0,
        DEFAULT_MARKER,
        &HashSet::new(),
        "config/Skill.xlsx",
        "Skill",
    )
    .expect_err("a second marker cell is E2201");
    assert!(diags
        .errors()
        .iter()
        .any(|d| { d.code == E2201 && d.message.contains("second marker cell") }));
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
        &HashSet::new(),
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
        &HashSet::new(),
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
        &HashSet::new(),
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
        &HashSet::new(),
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

/// Multi-row records (回流-5, design §51 Q6): a top-level column named
/// `*waves` in the `##var` row declares a multi-row list — the field name
/// is the name without the `*`, and the flag lands on the column whose
/// sub-columns the `##+` row declares.
#[test]
fn multi_row_var_column_marks_the_column_and_strips_the_star() {
    let rows = [
        &["##var", "id", "name", "*waves", "*waves"][..],
        &["##+", "", "", "index", "groups"][..],
        &["##type", "Int32", "String", "Int32", "String"][..],
    ];
    let meta = meta_of(&rows, 0, &[0, 1, 2]);
    assert_eq!(
        meta.columns.get("waves").map(|c| c.multi_row),
        Some(true),
        "the `*` prefix marks waves as a multi-row column"
    );
    assert!(
        meta.columns.get("name").is_some_and(|c| !c.multi_row),
        "a plain header column is not multi-row"
    );
    assert_eq!(
        meta.columns["waves"].sub_columns.keys().collect::<Vec<_>>(),
        ["index", "groups"],
        "the `##+` row declares the list element shape"
    );
    assert_eq!(
        meta.columns["waves"].cell, "D1",
        "anchored at its header cell"
    );
}

/// A `*name` column with no `##+` sub-column row cannot declare the element
/// shape — E2201, anchored at the column.
#[test]
fn multi_row_column_without_sub_columns_is_e2201() {
    let rows = [
        &["##var", "id", "*waves"][..],
        &["##type", "Int32", "Int32"][..],
    ];
    let diags = decode(
        &grid(&rows),
        0,
        &[0, 1][..],
        0,
        0,
        DEFAULT_MARKER,
        &HashSet::new(),
        "config/DungeonStage.xlsx",
        "DungeonStage",
    )
    .expect_err("a `*name` column without `##+` is E2201");
    assert!(diags.errors().iter().any(|d| {
        d.code == E2201 && d.message.contains("multi-row column") && d.message.contains("waves")
    }));
}

/// `[source.sheets]` is one flat table: the shared §51 fields plus the
/// `header_row` knob, deserialized from the same flat shape `[source.excel]`
/// uses (Q7). A typo'd key is rejected like it is for Excel.
#[test]
fn sheets_convention_flatten_roundtrip_and_unknown_key() {
    let cfg: SheetsConvention =
        serde_json::from_str(r###"{"header_row": 2, "preset": "meta_sheet", "marker": "##"}"###)
            .expect("flat [source.sheets] table deserializes");
    assert_eq!(cfg.header_row, 2);
    assert!(matches!(cfg.preset, Preset::MetaSheet));
    assert_eq!(cfg.marker, "##");
    assert!(!cfg.uses_script());
    assert!(matches!(
        cfg.as_excel_convention().preset,
        Preset::MetaSheet
    ));

    // An unknown key is a config typo — rejected, not silently ignored.
    let err = serde_json::from_str::<SheetsConvention>(r#"{"presett": "data"}"#);
    assert!(err.is_err(), "unknown key is rejected: {err:?}");
}

// ------------------------------------------------------------------
// X2: the decoded TableMeta as a schema, and the drift check (derive.rs)
// ------------------------------------------------------------------

use crate::schema::{FieldSchema, FieldType, ReferenceSchema, Schema, TableSchema};

fn metas_of(
    rows: &[&[&str]],
    header_row: usize,
    territory: &[usize],
) -> IndexMap<String, TableMeta> {
    let mut metas = IndexMap::new();
    metas.insert("Skill".to_string(), meta_of(rows, header_row, territory));
    metas
}

fn adopt_one_table(fields: IndexMap<String, FieldSchema>, primary_key: Vec<String>) -> Schema {
    let mut tables = IndexMap::new();
    tables.insert(
        "Skill".to_string(),
        TableSchema {
            name: "Skill".to_string(),
            description: None,
            primary_key,
            fields,
            unique_constraints: Vec::new(),
            order_by: None,
            targets: Vec::new(),
            env_overrides: IndexMap::new(),
        },
    );
    Schema {
        tables,
        enums: IndexMap::new(),
        metadata: None,
    }
}

fn declared_field(name: &str, t: FieldType) -> FieldSchema {
    FieldSchema {
        name: name.to_string(),
        field_type: t,
        description: None,
        required: false,
        nullable: false,
        default: None,
        min: None,
        max: None,
        min_length: None,
        max_length: None,
        pattern: None,
        enum_values: None,
        min_items: None,
        max_items: None,
        items: None,
        separator: None,
        properties: None,
        additional_properties: None,
        reference: None,
        targets: Vec::new(),
        rules: Vec::new(),
        metadata: IndexMap::new(),
    }
}

/// Declared columns map field-for-field into the Schema DSL, and a named
/// enum (`Enum<Rarity>` + `##enum` values) is registered once into
/// `Schema.enums` — the derived chain's `Schema` is valid as a whole.
#[test]
fn derive_schema_maps_declared_columns_and_registers_enums() {
    let rows = [
        &["##type Int32", "String", "Enum<Rarity>"][..],
        &["##enum", "", "common,rare,epic"][..],
        &["id", "name", "rarity"][..],
    ];
    let schema = derive::derive_schema(&metas_of(&rows, 2, &[0, 1]));
    let skill = schema.tables.get("Skill").expect("table");
    assert_eq!(skill.name, "Skill");
    assert_eq!(skill.fields.len(), 3);
    assert_eq!(skill.fields["id"].field_type, FieldType::Int32);
    assert_eq!(skill.fields["name"].field_type, FieldType::String);
    assert_eq!(
        skill.fields["rarity"].field_type,
        FieldType::Enum("Rarity".to_string())
    );
    let rarity = schema.enums.get("Rarity").expect("enum registered");
    assert_eq!(
        rarity
            .values
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["common", "rare", "epic"]
    );
    // No primary key declared → the derived table names none.
    assert_eq!(skill.primary_key, Vec::<String>::new());
}

/// A `multi_row` column with `##+` sub-columns derives to an `Array` of a
/// nested `Object` whose `properties` carry the sub-columns (回流-5/6).
#[test]
fn derive_multi_row_column_is_array_of_nested_object() {
    let rows = [
        &["##var", "id", "*weapon", "*weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "Int32", "Int32"][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    let weapon = metas["Skill"]
        .columns
        .get("weapon")
        .expect("weapon declared");
    assert!(weapon.multi_row);

    let field = derive::field_schema("weapon", weapon);
    assert!(
        matches!(field.field_type, FieldType::Array(_)),
        "a multi_row column is an Array"
    );
    let items = field.items.as_ref().expect("items for the Array");
    assert!(matches!(items.field_type, FieldType::Object(_)));
    let props = items.properties.as_ref().expect("properties of the bean");
    assert_eq!(
        props.keys().collect::<Vec<_>>(),
        ["atk", "def"],
        "sub-columns become the bean's properties"
    );
    assert_eq!(props["atk"].field_type, FieldType::Int32);
}

/// A nested/multi-row parent keeps its own declared field attributes —
/// `required`/`default`/`targets` are not swallowed by the Object/Array
/// element structure (review finding).
#[test]
fn derive_nested_column_keeps_parent_attributes() {
    let mut weapon = ColumnMeta {
        multi_row: true,
        cell: "C2".to_string(),
        required: Some(true),
        default: Some(serde_json::Value::Null),
        targets: vec!["client".to_string()],
        ..ColumnMeta::default()
    };
    weapon.sub_columns.insert(
        "atk".to_string(),
        ColumnMeta {
            cell: "C2".to_string(),
            field_type: Some(FieldType::Int32),
            ..ColumnMeta::default()
        },
    );
    weapon.sub_columns.insert(
        "def".to_string(),
        ColumnMeta {
            cell: "D2".to_string(),
            field_type: Some(FieldType::Int32),
            ..ColumnMeta::default()
        },
    );
    assert!(weapon.multi_row);

    let field = derive::field_schema("weapon", &weapon);
    assert!(field.required, "required from the parent declaration");
    assert_eq!(field.default, Some(serde_json::Value::Null));
    assert_eq!(field.targets, ["client"].map(String::from).to_vec());

    let items = field.items.as_ref().expect("items");
    assert!(items.required);
    assert_eq!(items.targets, ["client"].map(String::from).to_vec());
}

/// A declared primary key becomes the table-level list.
#[test]
fn derive_collects_declared_primary_key_columns() {
    let rows = [
        &["##type Int32", "String"][..],
        &["##primary_key true", ""][..],
        &["id", "name"][..],
    ];
    let schema = derive::derive_schema(&metas_of(&rows, 2, &[0, 1]));
    assert_eq!(
        schema.tables["Skill"].primary_key,
        ["id"].map(String::from).to_vec()
    );
}

/// `render_draft` output re-parses to the exact derived schema (the
/// round-trip acceptance): the body is the Schema's own serde, the header
/// is comments only.
#[test]
fn draft_render_round_trips_to_the_derived_schema() {
    let rows = [
        &["##type Int32", "String", "Enum<Rarity>"][..],
        &["##enum", "", "common,rare"][..],
        &["##min 0", "", ""][..],
        &["id", "name", "rarity"][..],
    ];
    let derived = derive::derive_schema(&metas_of(&rows, 3, &[0, 1, 2]));
    let draft = derive::render_draft("excel:config/Skill.xlsx", &derived);

    assert!(draft.starts_with("# schema draft"), "comment header first");
    let body: Schema =
        serde_yaml::from_str(&draft).expect("the draft body is a valid schema document");
    assert_eq!(body.tables.keys().collect::<Vec<_>>(), ["Skill"]);
    assert_eq!(
        body.tables["Skill"].fields["id"].field_type,
        FieldType::Int32
    );
    assert_eq!(body.enums["Rarity"].values.len(), 2);
    assert_eq!(body.tables["Skill"].fields["id"].min, Some(0.0));
}

/// `fill_derived` inserts only tables the schema does not already have —
/// an adopted partial schema keeps its own table untouched (the derived
/// remainder fills the rest).
#[test]
fn fill_derived_leaves_adopted_tables_alone() {
    let rows = [&["##type Int32", "String"][..], &["id", "name"][..]];
    let mut metas = metas_of(&rows, 1, &[0]);
    metas.insert("Item".to_string(), metas["Skill"].clone());

    // "Skill" is adopted with a different field order; "Item" is not.
    let mut adopted = adopt_one_table(
        [("id".to_string(), declared_field("id", FieldType::Int32))]
            .into_iter()
            .collect(),
        vec!["id".to_string()],
    );
    adopted.tables.get_mut("Skill").expect("skill").description = Some("adopted".to_string());

    derive::fill_derived(&mut adopted, &metas);
    assert_eq!(
        adopted.tables["Skill"].description.as_deref(),
        Some("adopted"),
        "an adopted table is not overwritten"
    );
    assert_eq!(
        adopted.tables["Skill"].fields.len(),
        1,
        "id only, as adopted"
    );
    assert_eq!(
        adopted.tables["Item"].fields["name"].field_type,
        FieldType::String,
        "the missing table is filled from the workbook"
    );
}

/// A column the meta declares as `Int32` but the adopted schema has as
/// `String` is drift: E2208, anchored at the exact declaration cell.
#[test]
fn drift_type_mismatch_is_e2208_at_the_declaration_cell() {
    let rows = [&["##type Int32", "String"][..], &["id", "name"][..]];
    let metas = metas_of(&rows, 1, &[0]);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::String)),
            (
                "name".to_string(),
                declared_field("name", FieldType::String),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert_eq!(diags.errors().len(), 1, "{diags:?}");
    let err = diags.errors()[0];
    assert_eq!(err.code, crate::error::codes::source_convention::E2208);
    assert!(err.message.contains("declares type"), "{}", err.message);
    let loc = err.location.as_ref().expect("cell location");
    assert_eq!(
        loc.column.as_deref(),
        Some("A"),
        "the `##type` column letter"
    );
    assert_eq!(loc.row, Some(1), "the `##type` row");
}

/// An adopted field the workbook never mentions is the author's own and is
/// not drift — only the declaration-carried subset is compared.
#[test]
fn drift_ignores_author_added_constraints_and_extra_fields() {
    let rows = [&["##type Int32", "String"][..], &["id", "name"][..]];
    let metas = metas_of(&rows, 1, &[0]);
    let mut name = declared_field("name", FieldType::String);
    name.pattern = Some("^[a-z]+$".to_string());
    name.min_length = Some(1);
    let extra = declared_field("level", FieldType::Int32);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            ("name".to_string(), name),
            ("level".to_string(), extra),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert!(diags.is_empty(), "no drift: {diags:?}");
}

/// A meta-declared field the adopted schema lacks is drift (the workbook
/// grew a column the schema never adopted), and so is a table the workbook
/// declares but the schema has no entry for.
#[test]
fn drift_missing_field_and_missing_table_are_e2208() {
    let rows = [&["##type Int32", "String"][..], &["id", "name"][..]];
    let mut metas = metas_of(&rows, 1, &[0]);
    metas.insert("Item".to_string(), metas["Skill"].clone());

    let adopted = adopt_one_table(
        [("id".to_string(), declared_field("id", FieldType::Int32))]
            .into_iter()
            .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    let errors: Vec<&Diagnostic> = diags.errors();
    assert_eq!(
        errors.len(),
        2,
        "the missing field + the missing table: {errors:?}"
    );
    assert!(errors
        .iter()
        .all(|e| e.code == crate::error::codes::source_convention::E2208));
    let field_errs: Vec<&&Diagnostic> = errors
        .iter()
        .filter(|e| e.message.contains("declares field 'name'"))
        .collect();
    assert_eq!(field_errs.len(), 1);
    let loc = field_errs[0].location.as_ref().expect("cell location");
    assert_eq!(loc.column.as_deref(), Some("B"), "the name column letter");
}

/// A primary key the workbook declares that differs from the adopted one is
/// drift (declaration-bearing on the schema side too).
#[test]
fn drift_primary_key_mismatch_is_e2208() {
    let rows = [
        &["##type Int32", "String"][..],
        &["##primary_key true", ""][..],
        &["id", "name"][..],
    ];
    let metas = metas_of(&rows, 2, &[0, 1]);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            (
                "name".to_string(),
                declared_field("name", FieldType::String),
            ),
        ]
        .into_iter()
        .collect(),
        vec!["name".to_string()],
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert_eq!(diags.errors().len(), 1, "{diags:?}");
    let err = diags.errors()[0];
    assert!(
        err.message.contains("declares primary key"),
        "{}",
        err.message
    );
}

/// A reference the workbook declares must match the adopted table+field
/// (the predicate and cardinality are schema-side and not compared).
#[test]
fn drift_reference_table_mismatch_is_e2208() {
    let rows = [
        &["##type Int32", "String"][..],
        &["##reference Item.id", ""][..],
        &["id", "name"][..],
    ];
    let metas = metas_of(&rows, 2, &[0, 1]);
    // The meta marks the id column (cell 0) as referencing Item.id; the
    // adopted schema points the same field at a different target.
    let mut adopted_id = declared_field("id", FieldType::Int32);
    adopted_id.reference = Some(ReferenceSchema {
        table: "Item".to_string(),
        field: "other".to_string(),
        predicate: None,
        cardinality: "one".to_string(),
        compatible_with: None,
    });
    let adopted = adopt_one_table(
        [
            ("id".to_string(), adopted_id),
            (
                "name".to_string(),
                declared_field("name", FieldType::String),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert_eq!(diags.errors().len(), 1, "{diags:?}");
    assert!(
        diags.errors()[0].message.contains("reference"),
        "{}",
        diags.errors()[0].message
    );
}

/// A workbook that declares `bean`/`multi_row` while the adopted schema
/// carries a scalar (or a non-object array) is structural drift — the
/// implied Object/Array shape is compared even when no `##type` was written
/// (review finding: nested shape drift silently passed E2208).
#[test]
fn drift_structural_shape_mismatch_is_e2208() {
    // `weapon` declares a bean (sub-columns atk/def, no `##type` written);
    // the adopted schema says it is a plain Int32 scalar.
    let rows = [
        &["##var", "id", "weapon", "weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "", ""][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            (
                "weapon".to_string(),
                declared_field("weapon", FieldType::Int32),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert_eq!(diags.errors().len(), 1, "{diags:?}");
    let err = &diags.errors()[0];
    assert_eq!(err.code, crate::error::codes::source_convention::E2208);
    assert!(
        err.message.contains("declares a bean structure"),
        "{}",
        err.message
    );
}

/// A `multi_row` column whose adopted schema is a scalar (not an array of
/// objects) is structural drift too, and the mismatch is anchored at the
/// column's declaration cell.
#[test]
fn drift_multi_row_shape_mismatch_is_e2208() {
    let rows = [
        &["##var", "id", "*weapon", "*weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "", ""][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            (
                "weapon".to_string(),
                declared_field("weapon", FieldType::Int32),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert_eq!(diags.errors().len(), 1, "{diags:?}");
    let err = &diags.errors()[0];
    assert_eq!(err.code, crate::error::codes::source_convention::E2208);
    assert!(
        err.message.contains("declares a multi_row structure"),
        "{}",
        err.message
    );
}

/// A nested child whose adopted schema says `atk` is a String (not the
/// declared Int32) is reported — the recursion descends into the bean's
/// properties, which the old code skipped for nested columns.
#[test]
fn drift_nested_child_mismatch_is_e2208() {
    let rows = [
        &["##var", "id", "weapon", "weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "Int32", "Int32"][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    let mut props = IndexMap::new();
    props.insert("atk".to_string(), declared_field("atk", FieldType::String));
    props.insert("def".to_string(), declared_field("def", FieldType::Int32));
    let mut weapon = declared_field("weapon", FieldType::Object(IndexMap::new()));
    weapon.properties = Some(props);
    weapon.additional_properties = Some(false);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            ("weapon".to_string(), weapon),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert_eq!(diags.errors().len(), 1, "{diags:?}");
    let err = &diags.errors()[0];
    assert!(err.message.contains("weapon.atk"), "{}", err.message);
}

/// The documented `[source.excel]` / `[source.sheets]` tables (design §51)
/// map to the `source` container — the nested shape is the contract, not the
/// flat `source_excel` spelling, and a config typo inside a declared table is
/// rejected rather than silently meaning "no convention".
#[test]
fn project_config_source_tables_map_to_the_container() {
    use crate::manifest::ProjectConfig;

    let cfg: ProjectConfig = serde_yaml::from_str(
        "project:\n  name: demo\nprofiles: {}\nsource:\n  excel:\n    preset: marker_rows\n    marker: \"##\"\n  sheets:\n    preset: meta_sheet\n    header_row: 2\n",
    )
    .expect("[source.excel]/[source.sheets] shape deserializes");
    let excel = cfg.source_excel().expect("excel convention present");
    assert!(matches!(excel.preset, Preset::MarkerRows));
    assert_eq!(excel.marker, "##");
    let sheets = cfg.source_sheets().expect("sheets convention present");
    assert_eq!(sheets.header_row, 2);
    assert!(matches!(sheets.preset, Preset::MetaSheet));

    // A typo inside a declared table is a hard parse error, not "no convention".
    let err = serde_yaml::from_str::<ProjectConfig>(
        "project:\n  name: demo\nprofiles: {}\nsource:\n  excel:\n    presett: data\n",
    );
    assert!(err.is_err(), "typo'd key is rejected: {err:?}");
}
