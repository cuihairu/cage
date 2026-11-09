use super::super::*;
use super::{grid, meta_of};
use crate::diagnostics::Diagnostic;
use std::collections::HashSet;

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

/// A `##unique true` declaration becomes a single-field table-level
/// `unique_constraints` entry (the Schema DSL has no per-field unique slot),
/// so the keyword is no longer accepted-and-dropped.
#[test]
fn derive_unique_column_becomes_table_unique_constraint() {
    let rows = [
        &["##type Int32", "##type String"][..],
        &["##unique true", ""][..],
        &["id", "name"][..],
    ];
    let schema = derive::derive_schema(&metas_of(&rows, 2, &[0, 1]));
    let skill = schema.tables.get("Skill").expect("table");
    assert_eq!(skill.unique_constraints.len(), 1);
    assert_eq!(skill.unique_constraints[0].name, "uniq_id");
    assert_eq!(skill.unique_constraints[0].fields, ["id"]);
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

/// A `##unique true` column's table-level unique constraint is part of the
/// drift contract: a schema missing or naming a different column is E2208.
#[test]
fn drift_unique_mismatch_is_e2208() {
    let rows = [
        &["##type Int32", "##type String"][..],
        &["##unique true", ""][..],
        &["id", "name"][..],
    ];
    let metas = metas_of(&rows, 2, &[0, 1]);
    let mut adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            (
                "name".to_string(),
                declared_field("name", FieldType::String),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    // Missing → drift.
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert!(
        diags.errors().iter().any(|d| d.message.contains("unique")),
        "a missing unique constraint is drift: {diags:?}"
    );

    // A matching constraint clears it.
    adopted
        .tables
        .get_mut("Skill")
        .expect("skill")
        .unique_constraints
        .push(crate::schema::UniqueConstraint {
            name: "uniq_id".to_string(),
            fields: vec!["id".to_string()],
        });
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert!(
        !diags
            .errors()
            .iter()
            .any(|d| d.message.contains("unique columns")),
        "a matching unique constraint is not drift: {diags:?}"
    );
}

/// An explicit `##unique false` is the author opting out, not a
/// declaration: the drift check must not enter the unique branch (which
/// would derive an empty `unique_constraints` and misreport a workbook
/// that never claimed the column unique as
/// "declares unique columns [] but the schema has [...]"). The adopted
/// schema holds a unique constraint on a column the workbook never marked
/// — that is the author's own schema-side constraint, not drift.
#[test]
fn drift_unique_false_is_not_a_declaration() {
    let rows = [
        &["##type Int32", "##type String"][..],
        &["##unique false", ""][..],
        &["id", "name"][..],
    ];
    let metas = metas_of(&rows, 2, &[0, 1]);
    let mut adopted = adopt_one_table(
        [
            ("id".to_string(), declared_field("id", FieldType::Int32)),
            (
                "name".to_string(),
                declared_field("name", FieldType::String),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    // The schema carries a unique constraint the workbook never declared.
    adopted
        .tables
        .get_mut("Skill")
        .expect("skill")
        .unique_constraints
        .push(crate::schema::UniqueConstraint {
            name: "uniq_id".to_string(),
            fields: vec!["id".to_string()],
        });
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert!(
        !diags.errors().iter().any(|d| d.message.contains("unique")),
        "##unique false is an opt-out, not a declaration: {diags:?}"
    );
}

/// The same opt-out rule applies to the primary key: an explicit
/// `##primary_key false` must not enter the primary-key drift branch
/// either, so a schema-side primary key the workbook never claimed is the
/// author's own and is not drift.
#[test]
fn drift_primary_key_false_is_not_a_declaration() {
    let rows = [
        &["##type Int32", "String"][..],
        &["##primary_key false", ""][..],
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
        vec!["id".to_string()],
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    assert!(
        !diags
            .errors()
            .iter()
            .any(|d| d.message.contains("primary key")),
        "##primary_key false is an opt-out, not a declaration: {diags:?}"
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

/// `script_fingerprint` mixes the script bytes + resolved path + canonical
/// `script_config` + contract version (design §51 Q2). The two review
/// findings: a raw `root.join(script)` path (never canonicalized) and a
/// non-order-normalized `script_config` both broke the "same input → same
/// `build_id`" contract — same project spelled differently, or the same config
/// with reordered keys, produced different fingerprints.
#[test]
fn script_fingerprint_is_path_and_config_canonical() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("tools")).expect("mkdir tools");
    std::fs::write(root.join("tools/echo.py"), b"print('hello')\n").expect("write script");

    let mut cfg = ExcelConvention {
        script: Some("tools/echo.py".to_string()),
        script_config: serde_json::json!({"b": 2, "a": {"d": 4, "c": 3}}),
        ..Default::default()
    };
    let fp1 = script_fingerprint(root, &cfg)
        .expect("fingerprint")
        .expect("Some");

    // Same project, same script, same config but with reordered object keys
    // (config semantically equal) → identical fingerprint (canonical order).
    cfg.script_config = serde_json::json!({"a": {"c": 3, "d": 4}, "b": 2});
    let fp2 = script_fingerprint(root, &cfg)
        .expect("fingerprint")
        .expect("Some");
    assert_eq!(
        fp1, fp2,
        "reordered config keys must not change the fingerprint"
    );

    // Same config, different config values → different fingerprint.
    cfg.script_config = serde_json::json!({"b": 99, "a": {"d": 4, "c": 3}});
    let fp3 = script_fingerprint(root, &cfg)
        .expect("fingerprint")
        .expect("Some");
    assert_ne!(
        fp1, fp3,
        "a different config value must rotate the fingerprint"
    );

    // Same project spelled with a relative root vs the same absolute root:
    // the raw `root.join` spelling would differ, the canonical path must not.
    // `root.join("tools/echo.py")` from an uncanonicalized root spelling
    // differs, but `resolve_script` returns the canonicalized join — so a
    // root spelled as `root` and as `root/../<tmpname>` (both canonicalize
    // to the same dir) must produce the same fingerprint.
    let dotdot = root.join("..").join(root.file_name().expect("tmp name"));
    let rel_fp = script_fingerprint(&dotdot, &cfg)
        .expect("fingerprint")
        .expect("Some");
    let abs_fp = script_fingerprint(root, &cfg)
        .expect("fingerprint")
        .expect("Some");
    assert_eq!(rel_fp, abs_fp, "equivalent root spellings must match");
}

/// `script_fingerprint` returns `None` when no script is declared, and the
/// contract version prefix is stable.
#[test]
fn script_fingerprint_none_without_script() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg = ExcelConvention::default();
    assert!(script_fingerprint(tmp.path(), &cfg)
        .expect("no script")
        .is_none());
}

/// A regular `type: Object` schema carries its children in the `FieldType`
/// value map, not in `FieldSchema.properties` — the drift recursion must
/// still descend into them (review finding: only `properties`-bearing
/// adopted fields were recursed, so a plain `Object` skipped every sub-column
/// existence and type check).
#[test]
fn drift_descends_into_regular_object_type_schema() {
    let rows = [
        &["##var", "id", "weapon", "weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "Int32", "Int32"][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    // The adopted schema spells the bean as a plain `type: Object` with the
    // children in the `FieldType` map and no `properties`/`items`.
    let mut props = IndexMap::new();
    props.insert("atk".to_string(), FieldType::String); // mismatched: declared Int32
    props.insert("def".to_string(), FieldType::Int32);
    let weapon = declared_field("weapon", FieldType::Object(props));
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

/// A regular `type: Array<Object>` `multi_row` schema likewise carries its
/// element children in the `FieldType` value map — the drift recursion must
/// unwrap the array element and descend (review finding).
#[test]
fn drift_descends_into_regular_array_of_object_type_schema() {
    let rows = [
        &["##var", "id", "*weapon", "*weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "Int32", "Int32"][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    let mut props = IndexMap::new();
    props.insert("atk".to_string(), FieldType::Int32);
    props.insert("def".to_string(), FieldType::String); // mismatched: declared Int32
    let weapon = declared_field(
        "weapon",
        FieldType::Array(Box::new(FieldType::Object(props))),
    );
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
    assert!(err.message.contains("weapon.def"), "{}", err.message);
}

/// A sub-field the regular `type: Object` schema lacks is drift too — the
/// recursion reports the declared-but-unadopted child rather than skipping.
#[test]
fn drift_reports_missing_child_in_regular_object_type_schema() {
    let rows = [
        &["##var", "id", "weapon", "weapon"][..],
        &["##+", "", "atk", "def"][..],
        &["##type", "Int32", "Int32", "Int32"][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2]);
    // The schema only adopts `atk` — `def` is missing.
    let mut props = IndexMap::new();
    props.insert("atk".to_string(), FieldType::Int32);
    let weapon = declared_field("weapon", FieldType::Object(props));
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
    assert!(
        err.message.contains("declares sub-field 'weapon.def'"),
        "{}",
        err.message
    );
}

/// Territory selection for each preset: `marker_rows` with a `##var` header
/// scans from row 0 to the first pure-data row; without one it scans the
/// rows above the header; `header_block` walks up through the contiguous
/// non-empty block; `data`/`meta_sheet` carry no in-grid territory.
#[test]
fn convention_territory_preset_shapes() {
    let var_rows = [
        &["##type Int32", "String"][..],
        &["##var", "id", "name"][..],
        &["##description", "武器名", "备注"][..],
        &["1", "sword", "x"][..],
    ];
    let (rows, data_start) =
        convention_territory(&grid(&var_rows), 1, DEFAULT_MARKER, Preset::MarkerRows);
    assert_eq!(rows, [0, 1, 2]);
    assert_eq!(data_start, 3);

    let plain_rows = [
        &["##type Int32", "String"][..],
        &["id", "name"][..],
        &["1", "x"][..],
    ];
    let (rows, data_start) =
        convention_territory(&grid(&plain_rows), 1, DEFAULT_MARKER, Preset::MarkerRows);
    assert_eq!(rows, [0]);
    assert_eq!(data_start, 2);

    let block_rows = [
        &["", ""][..],
        &["##type Int32", "String"][..],
        &["id", "name"][..],
        &["1", "x"][..],
    ];
    let (rows, data_start) =
        convention_territory(&grid(&block_rows), 2, DEFAULT_MARKER, Preset::HeaderBlock);
    assert_eq!(rows, [1]);
    assert_eq!(data_start, 3);

    let (rows, data_start) =
        convention_territory(&grid(&plain_rows), 1, DEFAULT_MARKER, Preset::Data);
    assert_eq!(rows, [] as [usize; 0]);
    assert_eq!(data_start, 2);
    let (rows, data_start) =
        convention_territory(&grid(&plain_rows), 1, DEFAULT_MARKER, Preset::MetaSheet);
    assert_eq!(rows, [] as [usize; 0]);
    assert_eq!(data_start, 2);
}

/// A `##+` cell with no column in the header row at that column is E2201
/// (anchored), and a repeated sub-column name under the same parent is
/// E2201 (anchored).
#[test]
fn sub_column_no_parent_and_duplicate_are_e2201() {
    let no_parent = [
        &["##var", "id"][..],
        &["##+", "", "atk"][..],
        &["id", "name"][..],
    ];
    let diags = decode(
        &grid(&no_parent),
        0,
        &[1],
        0,
        0,
        DEFAULT_MARKER,
        &HashSet::new(),
        "config/Skill.xlsx",
        "Skill",
    )
    .expect_err("no parent is E2201");
    assert!(
        diags
            .errors()
            .iter()
            .any(|d| d.code == E2201 && d.message.contains("has no column in the header row")),
        "{diags:?}"
    );

    let dup = [
        &["##var", "weapon", "weapon"][..],
        &["##+", "atk", "atk"][..],
        &["id", "weapon"][..],
    ];
    let diags = decode(
        &grid(&dup),
        0,
        &[1],
        0,
        0,
        DEFAULT_MARKER,
        &HashSet::new(),
        "config/Skill.xlsx",
        "Skill",
    )
    .expect_err("duplicate sub-column is E2201");
    assert!(
        diags
            .errors()
            .iter()
            .any(|d| d.code == E2201 && d.message.contains("declared twice")),
        "{diags:?}"
    );
}

/// The drift check reports every declared-vs-adopted mismatch arm:
/// default, enum values, min, max, and a declared reference the schema
/// does not carry.
#[test]
fn drift_reports_default_enum_min_max_and_missing_reference() {
    // Declared: default 10, enum [a,b], min 0, max 999, reference Item.id.
    // score / level declare Int32 to match the adopted type — a type
    // mismatch short-circuits the per-field comparison and would mask the
    // min / max arms under test.
    let rows = [
        &["##type Int32", "String", "Int32", "Int32", "String"][..],
        &[
            "##default 10",
            "##enum a,b",
            "##min 0",
            "##max 999",
            "##reference Item.id",
        ][..],
        &["id", "name", "score", "level", "icon"][..],
    ];
    let metas = metas_of(&rows, 2, &[0, 1]);
    // Adopted schema matches nothing the workbook declares (different
    // default, no enum, different min/max, no reference).
    let mut adopted_id = declared_field("id", FieldType::Int32);
    adopted_id.default = Some(serde_json::json!(99));
    let mut adopted_name = declared_field("name", FieldType::String);
    adopted_name.enum_values = Some(vec!["x".to_string()]);
    let mut adopted_score = declared_field("score", FieldType::Int32);
    adopted_score.min = Some(1.0);
    let mut adopted_level = declared_field("level", FieldType::Int32);
    adopted_level.max = Some(1.0);
    let adopted = adopt_one_table(
        [
            ("id".to_string(), adopted_id),
            ("name".to_string(), adopted_name),
            ("score".to_string(), adopted_score),
            ("level".to_string(), adopted_level),
            (
                "icon".to_string(),
                declared_field("icon", FieldType::String),
            ),
        ]
        .into_iter()
        .collect(),
        Vec::new(),
    );
    let diags = derive::drift(&metas, &adopted, "config/Skill.xlsx");
    let errs = diags.errors();
    let msgs: Vec<&str> = errs.iter().map(|d| d.message.as_str()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("declares a default")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("declares enum values")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("declares a min")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("declares a max")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("declares a reference")),
        "{msgs:?}"
    );
}

/// The drift check descends into sub-columns and compares each child
/// against the adopted `Object` properties, including the `items`
/// spelling for `multi_row` arrays.
#[test]
fn drift_compares_sub_column_children_and_items_shape() {
    use crate::schema::FieldType;

    // A `*weapon` multi-row bean with sub-columns whose declared type and
    // default drift. The parent's shape comes from the `*bean` marker —
    // `##type Object` is not declarable in a cell (E2202), so the fixture
    // uses the `##var` header + `##+` sub-column idiom.
    let rows = [
        &["##var", "id", "*weapon", "*weapon"][..],
        &["##+", "", "id", "name"][..],
        &["##type", "Int32", "Int32", "String"][..],
        &["##default", "", "5", ""][..],
    ];
    let metas = metas_of(&rows, 0, &[0, 1, 2, 3]);
    // The `*weapon` marker derives a `multi_row` bean: the adopted shape
    // is `Array<Object>`, children compared under the items object.
    let mut props = IndexMap::new();
    props.insert("id".to_string(), FieldType::Int32);
    props.insert("name".to_string(), FieldType::Int32); // mismatched: declared String
    let weapon = declared_field(
        "weapon",
        FieldType::Array(Box::new(FieldType::Object(props))),
    );
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
    let msgs: Vec<&str> = diags.errors().iter().map(|d| d.message.as_str()).collect();
    assert!(
        msgs.iter()
            .any(|m| m.contains("field 'weapon.name'") && m.contains("declares type")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("field 'weapon.id'") && m.contains("declares a default")),
        "{msgs:?}"
    );
}
