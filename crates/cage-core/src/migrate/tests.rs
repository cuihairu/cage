use super::apply::{is_widening, widen_value};
use super::parse::validate_step;
use super::*;
use crate::schema::{FieldType, Schema};
use indexmap::IndexMap;

#[test]
fn yaml_to_value_decodes_the_canonical_tagged_form() {
    // A bare unsigned scalar reads back Int; the tagged form keeps
    // the family, including nested inside plain structures.
    let tagged: serde_yaml::Value = serde_yaml::from_str("type: UInt\nvalue: 5").unwrap();
    assert_eq!(yaml_to_value(tagged).unwrap(), Value::UInt(5));

    let nested: serde_yaml::Value = serde_yaml::from_str("- 1\n- type: UInt\n  value: 7").unwrap();
    assert_eq!(
        yaml_to_value(nested).unwrap(),
        Value::Array(vec![Value::Int(1), Value::UInt(7)])
    );

    // A mapping that is not a valid tagged value keeps the plain
    // reading (Object), even with type/value-looking keys.
    let plain: serde_yaml::Value = serde_yaml::from_str("type: NotAVariant\nvalue: 5").unwrap();
    let mut expected = IndexMap::new();
    expected.insert("type".to_string(), Value::String("NotAVariant".into()));
    expected.insert("value".to_string(), Value::Int(5));
    assert_eq!(yaml_to_value(plain).unwrap(), Value::Object(expected));

    // Plain scalars keep the historical i64-first reading.
    let bare: serde_yaml::Value = serde_yaml::from_str("5").unwrap();
    assert_eq!(yaml_to_value(bare).unwrap(), Value::Int(5));
}

fn schema_with_table(name: &str, fields: &[&str]) -> Schema {
    let mut field_map = IndexMap::new();
    for field in fields {
        field_map.insert(
            (*field).to_string(),
            crate::schema::FieldSchema {
                name: (*field).to_string(),
                field_type: FieldType::String,
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
            },
        );
    }
    let mut tables = IndexMap::new();
    tables.insert(
        name.to_string(),
        crate::schema::TableSchema {
            name: name.to_string(),
            description: None,
            primary_key: vec![fields.first().unwrap_or(&"id").to_string()],
            fields: field_map,
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

const SPEC_YAML: &str = r#"
from: "1.0.0"
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
  - remove_field:
      table: Item
      field: legacy_id
  - widen_type:
      table: Item
      field: level
      to: { kind: Int64 }
  - remap_values:
      table: Item
      field: grade
      map:
        "S": "legendary"
        "A": "epic"
  - rename_table:
      from: Mob
      to: Monster
"#;

#[test]
fn parse_spec_reads_all_six_step_kinds_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("0001-rename.yaml");
    std::fs::write(&file, SPEC_YAML).unwrap();
    let spec = parse_spec(&file).unwrap();
    assert_eq!(spec.from, "1.0.0");
    assert_eq!(spec.to, "1.1.0");
    assert_eq!(spec.steps.len(), 6);
    assert_eq!(
        spec.steps[0],
        Step::RenameField {
            table: "Item".into(),
            from: "name".into(),
            to: "title".into()
        }
    );
    assert_eq!(
        spec.steps[1],
        Step::SetDefault {
            table: "Item".into(),
            field: "rarity".into(),
            value: Value::String("common".into())
        }
    );
    assert_eq!(
        spec.steps[2],
        Step::RemoveField {
            table: "Item".into(),
            field: "legacy_id".into()
        }
    );
    assert_eq!(
        spec.steps[3],
        Step::WidenType {
            table: "Item".into(),
            field: "level".into(),
            to: FieldType::Int64
        }
    );
    assert_eq!(
        spec.steps[4],
        Step::RemapValues {
            table: "Item".into(),
            field: "grade".into(),
            map: IndexMap::from([
                ("S".to_string(), "legendary".to_string()),
                ("A".to_string(), "epic".to_string()),
            ])
        }
    );
    assert_eq!(
        spec.steps[5],
        Step::RenameTable {
            from: "Mob".into(),
            to: "Monster".into()
        }
    );
}

#[test]
fn parse_migration_dir_orders_segments_by_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let later = r#"
from: "1.1.0"
to: "2.0.0"
steps:
  - remove_field:
      table: Item
      field: legacy_id
"#;
    std::fs::write(dir.path().join("0002-second.yaml"), later).unwrap();
    std::fs::write(dir.path().join("0001-first.yaml"), SPEC_YAML).unwrap();
    std::fs::write(dir.path().join("README.md"), "not a migration").unwrap();
    let chain = parse_migration_dir(dir.path()).unwrap();
    let names: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["0001-first.yaml", "0002-second.yaml"]);
    assert_eq!(chain[0].1.from, "1.0.0");
    assert_eq!(chain[1].1.from, "1.1.0");
}

#[test]
fn parse_spec_reports_structural_problems_as_e2001() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let out = parse_spec(&root.join("missing.yaml")).unwrap_err();
    assert!(out.starts_with("E2001"), "{out}");

    let bad_yaml = root.join("bad.yaml");
    std::fs::write(&bad_yaml, "from: [unclosed").unwrap();
    let out = parse_spec(&bad_yaml).unwrap_err();
    assert!(out.starts_with("E2001"), "{out}");

    let no_steps = root.join("no-steps.yaml");
    std::fs::write(&no_steps, "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps: []\n").unwrap();
    let out = parse_spec(&no_steps).unwrap_err();
    assert!(out.starts_with("E2001"), "{out}");

    let unknown = root.join("unknown-step.yaml");
    std::fs::write(
        &unknown,
        "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps:\n  - drop_database: { table: Item }\n",
    )
    .unwrap();
    let out = parse_spec(&unknown).unwrap_err();
    assert!(out.starts_with("E2001"), "{out}");

    let no_to = root.join("no-to.yaml");
    std::fs::write(
        &no_to,
        "from: \"1.0.0\"\nsteps:\n  - remove_field: { table: Item, field: x }\n",
    )
    .unwrap();
    let out = parse_spec(&no_to).unwrap_err();
    assert!(out.starts_with("E2001"), "{out}");

    let self_loop = root.join("self-loop.yaml");
    std::fs::write(
        &self_loop,
        "from: \"1.0.0\"\nto: \"1.0.0\"\nsteps:\n  - remove_field: { table: Item, field: x }\n",
    )
    .unwrap();
    let out = parse_spec(&self_loop).unwrap_err();
    assert!(out.starts_with("E2001"), "{out}");
}

#[test]
fn validate_spec_checks_table_and_field_references_as_e2002() {
    let schema = schema_with_table("Item", &["id", "name"]);

    // Table does not exist.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RemoveField {
            table: "Weapon".into(),
            field: "id".into(),
        }],
    };
    let out = validate_spec(&spec, &schema).unwrap_err();
    assert!(out.starts_with("E2002"), "{out}");
    assert!(out.contains("Weapon"), "{out}");

    // Field does not exist.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::SetDefault {
            table: "Item".into(),
            field: "rarity".into(),
            value: Value::String("common".into()),
        }],
    };
    let out = validate_spec(&spec, &schema).unwrap_err();
    assert!(out.starts_with("E2002"), "{out}");
    assert!(out.contains("rarity"), "{out}");

    // Rename collides with an existing field.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameField {
            table: "Item".into(),
            from: "name".into(),
            to: "id".into(),
        }],
    };
    let out = validate_spec(&spec, &schema).unwrap_err();
    assert!(out.starts_with("E2002"), "{out}");

    // rename_table to a free name is fine; to an existing one is not.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameTable {
            from: "Item".into(),
            to: "Weapon".into(),
        }],
    };
    assert!(validate_spec(&spec, &schema).is_ok());
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameTable {
            from: "Item".into(),
            to: "Item".into(),
        }],
    };
    let out = validate_spec(&spec, &schema).unwrap_err();
    assert!(out.starts_with("E2002"), "{out}");

    // A clean spec passes.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![
            Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "title".into(),
            },
            Step::RenameTable {
                from: "Item".into(),
                to: "Artifact".into(),
            },
        ],
    };
    assert!(validate_spec(&spec, &schema).is_ok());
}

use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value as V};

fn tv(value: V) -> TypedValue {
    TypedValue {
        value,
        location: SourceLocation::new("config/item.json"),
        schema_type: None,
    }
}

fn make_row(index: usize, fields: Vec<(&str, V)>) -> Row {
    Row {
        primary_key: vec![],
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), tv(value)))
            .collect(),
        location: SourceLocation::new("config/item.json").with_row(index + 1),
        index,
    }
}

fn make_document() -> Document {
    let items = Table {
        name: "Item".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: vec![
            make_row(
                0,
                vec![
                    ("id", V::Int(1)),
                    ("name", V::String("Sword".to_string())),
                    ("legacy_id", V::String("SW-1".to_string())),
                    ("level", V::Int(10)),
                    ("grade", V::String("S".to_string())),
                ],
            ),
            make_row(
                1,
                vec![
                    ("id", V::Int(2)),
                    ("name", V::String("Shield".to_string())),
                    ("legacy_id", V::String("SH-1".to_string())),
                    ("level", V::Int(20)),
                    ("grade", V::String("A".to_string())),
                    ("rarity", V::String("rare".to_string())),
                ],
            ),
        ],
        source_file: "config/item.json".to_string(),
        sheet: None,
    };
    let mobs = Table {
        name: "Mob".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: vec![make_row(
            0,
            vec![("id", V::Int(1)), ("name", V::String("Slime".to_string()))],
        )],
        source_file: "config/mob.json".to_string(),
        sheet: None,
    };
    let mut tables = IndexMap::new();
    tables.insert("Item".to_string(), items);
    tables.insert("Mob".to_string(), mobs);
    Document {
        tables,
        source_files: vec!["config/item.json".to_string()],
        metadata: crate::value::DocumentMetadata::default(),
    }
}

fn full_spec() -> MigrationSpec {
    MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![
            Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "title".into(),
            },
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: V::String("common".into()),
            },
            Step::RemoveField {
                table: "Item".into(),
                field: "legacy_id".into(),
            },
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64,
            },
            Step::RemapValues {
                table: "Item".into(),
                field: "grade".into(),
                map: IndexMap::from([("S".to_string(), "legendary".to_string())]),
            },
            Step::RenameTable {
                from: "Mob".into(),
                to: "Monster".into(),
            },
        ],
    }
}

fn schema_for_document(version: &str) -> Schema {
    let mut schema = schema_with_table("Item", &[]);
    // Rebuild Item with the full from-schema field set.
    let fields: Vec<(&str, FieldType)> = vec![
        ("id", FieldType::Int32),
        ("name", FieldType::String),
        ("legacy_id", FieldType::String),
        ("level", FieldType::Int32),
        ("grade", FieldType::String),
    ];
    let mut field_map = IndexMap::new();
    for (name, field_type) in fields {
        field_map.insert(
            name.to_string(),
            crate::schema::FieldSchema {
                name: name.to_string(),
                field_type,
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
            },
        );
    }
    schema.tables.get_mut("Item").unwrap().fields = field_map;
    schema.tables.insert(
        "Mob".to_string(),
        crate::schema::TableSchema {
            name: "Mob".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: {
                let mut m = IndexMap::new();
                for name in ["id", "name"] {
                    m.insert(
                        name.to_string(),
                        crate::schema::FieldSchema {
                            name: name.to_string(),
                            field_type: if name == "id" {
                                FieldType::Int32
                            } else {
                                FieldType::String
                            },
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
                        },
                    );
                }
                m
            },
            unique_constraints: Vec::new(),
            order_by: None,
            targets: Vec::new(),
            env_overrides: IndexMap::new(),
        },
    );
    schema.metadata = Some(crate::schema::SchemaMetadata {
        version: version.to_string(),
        description: None,
        author: None,
    });
    schema
}

#[test]
fn apply_runs_all_six_step_semantics() {
    let mut doc = make_document();
    let from_schema = schema_for_document("1.0.0");
    let report = apply(&full_spec(), &mut doc, &from_schema).unwrap();

    // rename_field: every row renamed, order preserved (title stays
    // where name was).
    let items = &doc.tables["Item"];
    for row in &items.rows {
        let keys: Vec<&String> = row.fields.keys().collect();
        assert!(!keys.iter().any(|k| k.as_str() == "name"), "{keys:?}");
    }
    assert_eq!(
        items.rows[0].fields.keys().nth(1).unwrap(),
        "title",
        "renamed field keeps its position"
    );
    assert_eq!(
        items.rows[0].fields["title"].value,
        V::String("Sword".to_string())
    );

    // set_default: fills only rows where the field is absent; an
    // existing non-null value survives.
    assert_eq!(
        items.rows[0].fields["rarity"].value,
        V::String("common".to_string())
    );
    assert_eq!(
        items.rows[1].fields["rarity"].value,
        V::String("rare".to_string()),
        "existing value is kept"
    );

    // remove_field: gone from every row.
    assert!(items
        .rows
        .iter()
        .all(|r| !r.fields.contains_key("legacy_id")));

    // widen_type: values still present, representation unchanged.
    assert_eq!(items.rows[0].fields["level"].value, V::Int(10));

    // remap_values: mapped values rewritten, unmapped untouched.
    assert_eq!(
        items.rows[0].fields["grade"].value,
        V::String("legendary".to_string())
    );
    assert_eq!(
        items.rows[1].fields["grade"].value,
        V::String("A".to_string()),
        "unmapped value passes through"
    );

    // rename_table: new name carries the same rows, old name gone.
    assert!(!doc.tables.contains_key("Mob"));
    assert_eq!(doc.tables["Monster"].rows.len(), 1);
    assert_eq!(
        doc.tables.keys().take(2).cloned().collect::<Vec<_>>(),
        vec!["Item", "Monster"],
        "renamed table keeps its position"
    );

    // Report: one entry per step in order.
    assert_eq!(report.from, "1.0.0");
    assert_eq!(report.to, "1.1.0");
    assert_eq!(report.steps.len(), 6);
    assert_eq!(report.steps[0].rows_changed, 2, "both items renamed");
    assert_eq!(
        report.steps[1].rows_changed, 1,
        "only the row missing rarity"
    );
    assert_eq!(report.steps[2].rows_changed, 2);
    assert_eq!(report.steps[4].rows_changed, 1, "only S is mapped");
    assert_eq!(report.steps[5].rows_changed, 1, "the single mob row");

    // The document's tables are text-sourced: no per-row locations are
    // recorded for them (their rewritten file is the change record).
    assert!(report.steps.iter().all(|s| s.affected_locations.is_empty()));
}

/// An Item table served from an Excel workbook — sheet set, row
/// locations carrying sheet-grid row numbers (header on row 1).
fn excel_document() -> Document {
    let mut items = Table {
        name: "Item".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: Vec::new(),
        source_file: "config/items.xlsx".to_string(),
        sheet: Some("Items".to_string()),
    };
    for (index, (id, grade)) in [(1, "S"), (2, "A"), (3, "B")].iter().enumerate() {
        items.rows.push(Row {
            primary_key: vec![V::Int(*id)],
            fields: [
                ("id", V::Int(*id)),
                ("grade", V::String((*grade).to_string())),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_string(), tv(value)))
            .collect(),
            location: SourceLocation::new("config/items.xlsx")
                .with_sheet("Items")
                .with_row(index + 2),
            index,
        });
    }
    let mut tables = IndexMap::new();
    tables.insert("Item".to_string(), items);
    Document {
        tables,
        source_files: vec!["config/items.xlsx".to_string()],
        metadata: crate::value::DocumentMetadata::default(),
    }
}

#[test]
fn excel_sourced_steps_record_affected_row_locations() {
    let mut doc = excel_document();
    let from_schema = schema_for_document("1.0.0");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![
            Step::RemapValues {
                table: "Item".into(),
                field: "grade".into(),
                map: IndexMap::from([
                    ("S".to_string(), "legendary".to_string()),
                    ("A".to_string(), "epic".to_string()),
                ]),
            },
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: V::String("common".into()),
            },
        ],
    };
    let report = apply(&spec, &mut doc, &from_schema).unwrap();

    // Remap touched rows 1 and 2 (sheet rows 2 and 3) — in source
    // order, sheet-grid addressed, the step's field carried by the
    // step name.
    assert_eq!(report.steps[0].rows_changed, 2);
    assert_eq!(
        report.steps[0]
            .affected_locations
            .iter()
            .map(SourceLocation::to_string)
            .collect::<Vec<_>>(),
        vec![
            "config/items.xlsx | Sheet: Items | Row: 2",
            "config/items.xlsx | Sheet: Items | Row: 3",
        ]
    );
    // set_default filled all three rows.
    assert_eq!(report.steps[1].rows_changed, 3);
    assert_eq!(
        report.steps[1]
            .affected_locations
            .iter()
            .map(SourceLocation::to_string)
            .collect::<Vec<_>>(),
        vec![
            "config/items.xlsx | Sheet: Items | Row: 2",
            "config/items.xlsx | Sheet: Items | Row: 3",
            "config/items.xlsx | Sheet: Items | Row: 4",
        ]
    );
}

#[test]
fn excel_remap_run_twice_records_no_locations_and_no_rows() {
    let mut doc = excel_document();
    let from_schema = schema_for_document("1.0.0");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RemapValues {
            table: "Item".into(),
            field: "grade".into(),
            map: IndexMap::from([("S".to_string(), "legendary".to_string())]),
        }],
    };
    let first = apply(&spec, &mut doc, &from_schema).unwrap();
    assert_eq!(first.steps[0].rows_changed, 1);
    assert_eq!(first.steps[0].affected_locations.len(), 1);

    // Second run: nothing changed, so no rows and no locations — the
    // no-op semantics cover the location record too.
    let again = apply(&spec, &mut doc, &from_schema).unwrap();
    assert_eq!(again.steps[0].rows_changed, 0);
    assert_eq!(again.steps[0].affected_locations, [] as [SourceLocation; 0]);
}

#[test]
fn rename_table_reports_whole_excel_table_and_text_tables_stay_location_free() {
    let mut doc = excel_document();
    // A second, text-sourced table sharing the workbook migration.
    let mut logs = Table {
        name: "Log".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: vec![make_row(0, vec![("id", V::Int(1))])],
        source_file: "config/logs.json".to_string(),
        sheet: None,
    };
    logs.rows[0].location = SourceLocation::new("config/logs.json").with_row(1);
    doc.tables.insert("Log".to_string(), logs);

    let from_schema = schema_for_document("1.0.0");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![
            Step::RenameTable {
                from: "Item".into(),
                to: "Goods".into(),
            },
            Step::RenameTable {
                from: "Log".into(),
                to: "Event".into(),
            },
        ],
    };
    let report = apply(&spec, &mut doc, &from_schema).unwrap();

    // Table-level steps count every row and locate every row — for
    // the Excel table only; the text-sourced table stays file-level.
    assert_eq!(report.steps[0].rows_changed, 3);
    assert_eq!(
        report.steps[0]
            .affected_locations
            .iter()
            .map(SourceLocation::to_string)
            .collect::<Vec<_>>(),
        vec![
            "config/items.xlsx | Sheet: Items | Row: 2",
            "config/items.xlsx | Sheet: Items | Row: 3",
            "config/items.xlsx | Sheet: Items | Row: 4",
        ]
    );
    assert_eq!(report.steps[1].rows_changed, 1);
    assert_eq!(
        report.steps[1].affected_locations,
        [] as [SourceLocation; 0]
    );
}

#[test]
fn widen_type_rejects_unsafe_directions_and_unfit_values_as_e2003() {
    let mut doc = make_document();
    let from_schema = schema_for_document("1.0.0");

    // Int64 → Float32 is not a widening.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "level".into(),
            to: FieldType::Float32,
        }],
    };
    let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
    assert!(out.contains("not a safe widening"), "{out}");

    // UInt8 → Int8 loses half the range — refused.
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "id".into(),
            to: FieldType::Int8,
        }],
    };
    let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");

    // A value outside the widened domain fails the segment without
    // touching the document: a rogue Int16 column carrying a value
    // beyond Int32's range (the schema type has been narrowed here to
    // make the row value provably out-of-domain for the target).
    let mut bad = make_document();
    bad.tables["Item"].rows[0].fields["level"].value = V::Int(5_000_000_000);
    let mut int16_schema = from_schema.clone();
    int16_schema.tables["Item"].fields["level"].field_type = FieldType::Int16;
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "level".into(),
            to: FieldType::Int32,
        }],
    };
    let out = apply(&spec, &mut bad, &int16_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
    assert!(out.contains("does not fit"), "{out}");

    // Missing field on a row.
    let mut short = make_document();
    short.tables["Item"].rows[1].fields.shift_remove("level");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "level".into(),
            to: FieldType::Int64,
        }],
    };
    let out = apply(&spec, &mut short, &from_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
    assert!(out.contains("has no `level`"), "{out}");

    // The safe direction on clean data succeeds: legal widenings run.
    let mut doc2 = make_document();
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "id".into(),
            to: FieldType::Int64,
        }],
    };
    let report = apply(&spec, &mut doc2, &from_schema).unwrap();
    // rows_changed counts rows whose bytes moved: canonical Int has no
    // width, so Int32 → Int64 rewrites no value — 0 changed, value
    // intact.
    assert_eq!(report.steps[0].rows_changed, 0);
    assert_eq!(doc2.tables["Item"].rows[0].fields["id"].value, V::Int(1));

    // A document without the table fails as E2003 too.
    let mut empty = Document::new();
    let out = apply(&full_spec(), &mut empty, &from_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
}

#[test]
fn reverify_passes_clean_and_fails_as_e2004() {
    let from_schema = schema_for_document("1.0.0");

    // Migrate a document, then build the to-schema (the migrated
    // shape: title / rarity / no legacy_id / level Int64) and check
    // it comes back clean.
    let mut doc = make_document();
    apply(&full_spec(), &mut doc, &from_schema).unwrap();
    let mut to_schema = schema_for_document("1.1.0");
    {
        let item = to_schema.tables.get_mut("Item").unwrap();
        // Rebuild fields to the migrated shape; `rarity` is required —
        // set_default in the spec is what makes the migrated rows
        // satisfy it, so the migrated document passes while the
        // pre-migration one (row 0 lacks rarity) must fail.
        let mut fields = IndexMap::new();
        for (name, field_type, required) in [
            ("id", FieldType::Int32, false),
            ("title", FieldType::String, false),
            ("level", FieldType::Int64, false),
            ("grade", FieldType::String, false),
            ("rarity", FieldType::String, true),
        ] {
            fields.insert(
                name.to_string(),
                crate::schema::FieldSchema {
                    name: name.to_string(),
                    field_type,
                    description: None,
                    required,
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
                },
            );
        }
        item.fields = fields;
    }
    to_schema
        .tables
        .insert("Monster".to_string(), to_schema.tables["Mob"].clone());
    to_schema.tables.shift_remove("Mob");
    assert!(reverify(&doc, &to_schema).is_ok());

    // Re-verifying the ORIGINAL document under the NEW schema fails
    // as E2004 — rows still carry `name`, which the new schema does
    // not know, and the old Mob table still exists.
    let original = make_document();
    let out = reverify(&original, &to_schema).unwrap_err();
    assert!(out.starts_with("E2004"), "{out}");
}

#[test]
fn apply_is_deterministic_two_runs_identical() {
    let from_schema = schema_for_document("1.0.0");
    let mut first = make_document();
    let mut second = make_document();
    apply(&full_spec(), &mut first, &from_schema).unwrap();
    apply(&full_spec(), &mut second, &from_schema).unwrap();
    let a = serde_json::to_string(&first).unwrap();
    let b = serde_json::to_string(&second).unwrap();
    assert_eq!(a, b, "same rules over same data = same document");
}

#[test]
fn apply_over_migrated_document_is_a_zero_change_noop() {
    // Second run over already-migrated data: every step finds its
    // target already in place (renamed fields, filled defaults, removed
    // fields, remapped values, renamed table) — 0 rows changed, bytes
    // untouched. This is the guarantee `cage migrate --write` leans on
    // for idempotence.
    let from_schema = schema_for_document("1.0.0");
    let mut doc = make_document();
    apply(&full_spec(), &mut doc, &from_schema).unwrap();
    let before = serde_json::to_string(&doc).unwrap();

    let again = apply(&full_spec(), &mut doc, &from_schema).unwrap();
    assert_eq!(
        again.total_rows_changed(),
        0,
        "every step is a no-op over migrated data: {:?}",
        again.steps
    );
    let after = serde_json::to_string(&doc).unwrap();
    assert_eq!(before, after, "second run writes nothing");
}

#[test]
fn rename_collisions_fail_as_e2003_instead_of_collapsing() {
    // The CLI validates against the post-migration schema only (no
    // historical from-schema on disk), so `validate_spec` cannot run
    // there — the apply layer refuses a rename whose target is already
    // taken rather than collapsing two values into one key.
    let from_schema = schema_for_document("1.0.0");

    // Field collision: both `name` and `title` present on a row.
    let mut doc = make_document();
    doc.tables["Item"].rows[0]
        .fields
        .insert("title".into(), tv(V::String("Already".into())));
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameField {
            table: "Item".into(),
            from: "name".into(),
            to: "title".into(),
        }],
    };
    let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
    assert!(out.contains("already has target field `title`"), "{out}");

    // Table collision: `Monster` exists alongside `Mob`.
    let mut doc = make_document();
    doc.tables
        .insert("Monster".into(), doc.tables["Mob"].clone());
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameTable {
            from: "Mob".into(),
            to: "Monster".into(),
        }],
    };
    let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
    assert!(out.contains("already has table `Monster`"), "{out}");
}

#[test]
fn widen_type_current_equals_to_skips_direction_table_keeps_domain_check() {
    // CLI shape: the loaded schema is already the post-migration one,
    // so the schema's type equals the step's target (`current == to`).
    // The direction table would refuse X → X as a non-widening; the
    // dual path skips it and still runs the value-domain net.
    let mut new_schema = schema_for_document("1.0.0");
    new_schema.tables["Item"].fields["level"].field_type = FieldType::Int64;
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "level".into(),
            to: FieldType::Int64,
        }],
    };

    // Clean values: direction not consulted, domain holds, bytes
    // unchanged → 0 rows changed.
    let mut doc = make_document();
    let report = apply(&spec, &mut doc, &new_schema).unwrap();
    assert_eq!(report.steps[0].rows_changed, 0);

    // A value the target type cannot hold still fails as E2003 — the
    // domain check is the safety net on this path too.
    let mut bad = make_document();
    bad.tables["Item"].rows[0].fields["level"].value = V::String("ten".into());
    let out = apply(&spec, &mut bad, &new_schema).unwrap_err();
    assert!(out.starts_with("E2003"), "{out}");
    assert!(out.contains("does not fit"), "{out}");
}

#[test]
fn yaml_to_value_maps_every_scalar_family_and_refuses_tags() {
    assert_eq!(
        yaml_to_value(serde_yaml::from_str("null").unwrap()).unwrap(),
        Value::Null
    );
    assert_eq!(
        yaml_to_value(serde_yaml::from_str("true").unwrap()).unwrap(),
        Value::Bool(true)
    );
    // a u64 that cannot fit i64 keeps its family; a float reads float
    assert_eq!(
        yaml_to_value(serde_yaml::from_str("18446744073709551615").unwrap()).unwrap(),
        Value::UInt(u64::MAX)
    );
    assert_eq!(
        yaml_to_value(serde_yaml::from_str("1.25").unwrap()).unwrap(),
        Value::Float(1.25)
    );

    // number and bool mapping keys are stringified into the object
    let keyed: serde_yaml::Value = serde_yaml::from_str("1: x\ntrue: y").unwrap();
    let mut expected = IndexMap::new();
    expected.insert("1".to_string(), Value::String("x".into()));
    expected.insert("true".to_string(), Value::String("y".into()));
    assert_eq!(yaml_to_value(keyed).unwrap(), Value::Object(expected));

    // a composite key cannot name a field
    let bad: serde_yaml::Value = serde_yaml::from_str("[1]: one").unwrap();
    let err = yaml_to_value(bad).unwrap_err();
    assert!(err.contains("unsupported mapping key"), "{err}");

    // explicit YAML tags are refused — plain YAML cannot carry them
    let tagged = serde_yaml::Value::Tagged(Box::new(serde_yaml::value::TaggedValue {
        tag: serde_yaml::value::Tag::new("!custom"),
        value: serde_yaml::Value::Number(5.into()),
    }));
    let err = yaml_to_value(tagged).unwrap_err();
    assert!(err.contains("YAML tags are not supported"), "{err}");
}

#[test]
fn a_step_must_carry_exactly_one_transform() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("0001-two.yaml");
    std::fs::write(
        &file,
        r#"
from: "1.0.0"
to: "1.1.0"
steps:
  - rename_field:
      table: Item
      from: name
      to: title
    set_default:
      table: Item
      field: rarity
      value: "common"
"#,
    )
    .unwrap();
    let err = parse_spec(&file).unwrap_err();
    assert!(
        err.contains("exactly one transform mapping, got 2 keys"),
        "{err}"
    );
}

#[test]
fn validate_structure_refuses_empty_version_strings() {
    let dir = tempfile::tempdir().unwrap();
    let from_empty = dir.path().join("0001-from.yaml");
    std::fs::write(
            &from_empty,
            "from: \"\"\nto: \"1.1.0\"\nsteps:\n  - rename_field:\n      table: Item\n      from: name\n      to: title\n",
        )
        .unwrap();
    let err = parse_spec(&from_empty).unwrap_err();
    assert!(err.contains("empty `from` version"), "{err}");

    let to_empty = dir.path().join("0002-to.yaml");
    std::fs::write(
            &to_empty,
            "from: \"1.0.0\"\nto: \"\"\nsteps:\n  - rename_field:\n      table: Item\n      from: name\n      to: title\n",
        )
        .unwrap();
    let err = parse_spec(&to_empty).unwrap_err();
    assert!(err.contains("empty `to` version"), "{err}");
}

#[test]
fn validate_step_rejects_widen_type_on_an_unknown_field() {
    let schema = schema_with_table("Item", &["id", "name"]);
    let step = Step::WidenType {
        table: "Item".into(),
        field: "ghost".into(),
        to: FieldType::Int64,
    };
    let err = validate_step(&step, &schema).unwrap_err();
    assert!(
        err.contains("E2002") && err.contains("has no field `ghost`"),
        "{err}"
    );
}

#[test]
fn parse_migration_dir_reports_an_unreadable_directory() {
    let dir = tempfile::tempdir().unwrap();
    let err = parse_migration_dir(&dir.path().join("absent")).unwrap_err();
    assert!(
        err.contains("E2001") && err.contains("cannot read migrations directory"),
        "{err}"
    );
}

#[test]
fn widen_type_without_a_from_schema_type_is_e2003() {
    let mut doc = make_document();
    let schema = schema_with_table("Item", &["id", "name"]);
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::WidenType {
            table: "Item".into(),
            field: "level".into(),
            to: FieldType::Int64,
        }],
    };
    let err = apply(&spec, &mut doc, &schema).unwrap_err();
    assert!(err.contains("has no type for `Item.level`"), "{err}");
}

#[test]
fn excel_remove_field_and_widen_type_record_affected_rows() {
    let mut doc = excel_document();
    for row in &mut doc.tables["Item"].rows {
        row.fields
            .insert("note".to_string(), tv(V::String("x".into())));
        row.fields.insert("level".to_string(), tv(V::UInt(5)));
    }
    // the from-schema says level is UInt8; widening into Int16 converts
    // every value's representation (UInt → Int), so every row reports
    let mut from_schema = schema_for_document("1.0.0");
    from_schema.tables["Item"]
        .fields
        .get_mut("level")
        .unwrap()
        .field_type = FieldType::UInt8;
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![
            Step::RemoveField {
                table: "Item".into(),
                field: "note".into(),
            },
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int16,
            },
        ],
    };
    let report = apply(&spec, &mut doc, &from_schema).unwrap();

    // remove_field: every workbook row that carried the field reports
    // its sheet-grid location
    assert_eq!(report.steps[0].rows_changed, 3);
    assert_eq!(
        report.steps[0]
            .affected_locations
            .iter()
            .map(SourceLocation::to_string)
            .collect::<Vec<_>>(),
        vec![
            "config/items.xlsx | Sheet: Items | Row: 2",
            "config/items.xlsx | Sheet: Items | Row: 3",
            "config/items.xlsx | Sheet: Items | Row: 4",
        ]
    );

    // widen_type: UInt → Int is a real representation change, so all
    // three rows report too
    assert_eq!(report.steps[1].rows_changed, 3);
    for row in &doc.tables["Item"].rows {
        assert_eq!(row.fields["level"].value, V::Int(5));
    }
    assert_eq!(report.steps[1].affected_locations.len(), 3);
}

#[test]
fn excel_rename_records_rows_and_null_defaults_fill_in_place() {
    // rename_field on a workbook table records every renamed row
    let mut doc = excel_document();
    let from_schema = schema_for_document("1.0.0");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameField {
            table: "Item".into(),
            from: "grade".into(),
            to: "tier".into(),
        }],
    };
    let report = apply(&spec, &mut doc, &from_schema).unwrap();
    assert_eq!(report.steps[0].rows_changed, 3);
    assert_eq!(report.steps[0].affected_locations.len(), 3);
    let fields: Vec<&str> = doc.tables["Item"].rows[0]
        .fields
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(fields, ["id", "tier"]);

    // set_default over an explicit null fills in place — the location
    // rides on the existing field slot, not the row
    let mut doc = make_document();
    doc.tables["Item"].rows[0].fields["grade"].value = V::Null;
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::SetDefault {
            table: "Item".into(),
            field: "grade".into(),
            value: V::String("standard".into()),
        }],
    };
    let report = apply(&spec, &mut doc, &from_schema).unwrap();
    assert_eq!(report.steps[0].rows_changed, 1);
    assert_eq!(
        doc.tables["Item"].rows[0].fields["grade"].value,
        V::String("standard".into())
    );
}

#[test]
fn remap_values_ignores_non_string_values() {
    let mut doc = excel_document();
    // only the first two rows carry the field — the third walks past
    for row in doc.tables["Item"].rows.iter_mut().take(2) {
        row.fields.insert("qty".to_string(), tv(V::Int(2)));
    }
    let from_schema = schema_for_document("1.0.0");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RemapValues {
            table: "Item".into(),
            field: "qty".into(),
            map: IndexMap::from([("2".to_string(), "two".to_string())]),
        }],
    };
    let report = apply(&spec, &mut doc, &from_schema).unwrap();
    assert_eq!(report.steps[0].rows_changed, 0);
    assert_eq!(report.steps[0].affected_locations.len(), 0);
    assert_eq!(doc.tables["Item"].rows[0].fields["qty"].value, V::Int(2));
}

#[test]
fn rename_table_without_either_name_is_e2003() {
    let mut doc = make_document();
    let from_schema = schema_for_document("1.0.0");
    let spec = MigrationSpec {
        from: "1.0.0".into(),
        to: "1.1.0".into(),
        steps: vec![Step::RenameTable {
            from: "Ghost".into(),
            to: "Phantom".into(),
        }],
    };
    let err = apply(&spec, &mut doc, &from_schema).unwrap_err();
    assert!(
        err.contains("E2003") && err.contains("document has no table `Ghost`"),
        "{err}"
    );
}

#[test]
fn is_widening_and_widen_value_cover_every_rank_and_family() {
    let u8 = FieldType::UInt8;
    let u16 = FieldType::UInt16;
    let u32 = FieldType::UInt32;
    let u64 = FieldType::UInt64;
    let i8 = FieldType::Int8;
    let i16 = FieldType::Int16;
    let i32 = FieldType::Int32;
    let i64 = FieldType::Int64;
    let f32 = FieldType::Float32;
    let f64 = FieldType::Float64;

    // unsigned rank ladders grow; equal or narrower is refused
    assert!(is_widening(&u8, &u16));
    assert!(is_widening(&u16, &u32));
    assert!(is_widening(&u32, &u64));
    assert!(!is_widening(&u8, &u8));
    assert!(!is_widening(&u16, &u8));

    // unsigned → signed: one signed step per rank
    assert!(is_widening(&u8, &i16));
    assert!(is_widening(&u16, &i32));
    assert!(is_widening(&u32, &i64));
    assert!(!is_widening(&u8, &i8));
    assert!(!is_widening(&u64, &i64));

    // the ≤32-bit ladder and Float32 step exactly into Float64
    assert!(is_widening(&i32, &f64));
    assert!(is_widening(&f32, &f64));
    assert!(!is_widening(&i64, &f64));
    assert!(!is_widening(&FieldType::String, &f64));

    // null widens to anything; integers fit each signed width
    assert_eq!(widen_value(&V::Null, &i8), Some(V::Null));
    assert_eq!(widen_value(&V::Int(7), &i8), Some(V::Int(7)));
    assert_eq!(widen_value(&V::Int(7), &i16), Some(V::Int(7)));
    assert_eq!(widen_value(&V::Int(7), &i32), Some(V::Int(7)));
    assert_eq!(widen_value(&V::Int(7), &i64), Some(V::Int(7)));
    assert_eq!(widen_value(&V::Int(7), &f64), Some(V::Int(7)));
    assert_eq!(widen_value(&V::Int(200), &i8), None);

    // unsigned into signed converts to Int; into its own rank ladder it
    // keeps the family, and overflow is refused
    assert_eq!(widen_value(&V::UInt(7), &i16), Some(V::Int(7)));
    assert_eq!(widen_value(&V::UInt(7), &u8), Some(V::UInt(7)));
    assert_eq!(widen_value(&V::UInt(7), &u16), Some(V::UInt(7)));
    assert_eq!(widen_value(&V::UInt(7), &u32), Some(V::UInt(7)));
    assert_eq!(widen_value(&V::UInt(7), &u64), Some(V::UInt(7)));
    assert_eq!(widen_value(&V::UInt(300), &u8), None);

    // floats step into Float64; a Float32 target holds no integer
    // family, and non-numbers never widen
    assert_eq!(widen_value(&V::Float(1.5), &f64), Some(V::Float(1.5)));
    assert_eq!(widen_value(&V::UInt(1), &f32), None);
    assert_eq!(widen_value(&V::String("x".into()), &i8), None);
}
