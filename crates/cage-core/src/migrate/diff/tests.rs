//! Migration draft rendering tests (split out of diff.rs, LAY-025).

use super::values::{
    has_unsigned, json_to_value, quote_string, typed_json_value, value_to_yaml_block,
};
use super::*;
use crate::schema::{FieldType, Schema};
use crate::value::Value as V;
use indexmap::IndexMap;

fn schema(yaml: &str) -> Schema {
    serde_yaml::from_str(yaml).expect("parse schema")
}

#[test]
fn draft_covers_remove_widen_add_default_and_todo_paths() {
    let from = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      name:
        name: name
        type: { kind: String }
        required: true
      level:
        name: level
        type: { kind: Int32 }
  Boss:
    name: Boss
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
",
    );
    let to = schema(
        r#"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      level:
        name: level
        type: { kind: Int64 }
      rarity:
        name: rarity
        type: { kind: String }
        default: "common"
  NewTab:
    name: NewTab
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
"#,
    );
    let draft = draft_migration(&diff_schemas(&from, &to));
    assert_eq!(
        draft.steps,
        vec![
            Step::RemoveField {
                table: "Item".into(),
                field: "name".into(),
            },
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64,
            },
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: V::String("common".into()),
            },
        ]
    );
    // The dropped table and the new table surface as TODOs, not steps.
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("table `Boss` was removed")));
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("table `NewTab` is new")));
}

#[test]
fn typed_defaults_land_in_the_field_family() {
    let from = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
",
    );
    let to = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      weight:
        name: weight
        type: { kind: UInt32 }
        default: 5
      mana:
        name: mana
        type: { kind: Int32 }
        default: 5
      ratio:
        name: ratio
        type: { kind: Float64 }
        default: 5
      label:
        name: label
        type: { kind: String }
        required: true
        default: 7
enums: {}
",
    );
    let draft = draft_migration(&diff_schemas(&from, &to));
    let find_default = |field: &str| {
        draft.steps.iter().find_map(|s| match s {
            Step::SetDefault {
                field: f, value, ..
            } if f == field => Some(value.clone()),
            _ => None,
        })
    };
    assert_eq!(find_default("weight"), Some(V::UInt(5)));
    assert_eq!(find_default("mana"), Some(V::Int(5)));
    assert_eq!(find_default("ratio"), Some(V::Float(5.0)));
    // A number default on a String field is not representable as a
    // value of the field's own type: TODO, no broken step.
    assert!(find_default("label").is_none());
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("field `Item.label`") && t.contains("not representable")));
}

#[test]
fn enum_member_changes_and_non_widenings_stay_todo() {
    let from = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      grade:
        name: grade
        type: { kind: String }
        enum_values: [S, A, B]
      code:
        name: code
        type: { kind: Int32 }
enums: {}
",
    );
    let to = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      grade:
        name: grade
        type: { kind: String }
        enum_values: [S, A, C]
      code:
        name: code
        type: { kind: String }
enums: {}
",
    );
    let draft = draft_migration(&diff_schemas(&from, &to));
    assert_eq!(draft.steps, vec![]);
    assert!(draft.todos.iter().any(|t| t.contains("`Item.grade`")
        && t.contains("removed: [B]")
        && t.contains("added: [C]")));
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("`Item.code` type changed Int32 → String")));
}

#[test]
fn required_flip_backfills_from_default_or_todo() {
    let from = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      a:
        name: a
        type: { kind: String }
      b:
        name: b
        type: { kind: String }
enums: {}
",
    );
    let to = schema(
        r#"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      a:
        name: a
        type: { kind: String }
        required: true
        default: "x"
      b:
        name: b
        type: { kind: String }
        required: true
enums: {}
"#,
    );
    let draft = draft_migration(&diff_schemas(&from, &to));
    assert_eq!(
        draft.steps,
        vec![Step::SetDefault {
            table: "Item".into(),
            field: "a".into(),
            value: V::String("x".into()),
        }]
    );
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("`Item.b` became required with no default")));
}

#[test]
fn render_round_trips_through_parse_spec() {
    let draft = MigrationDraft {
        steps: vec![
            Step::RemoveField {
                table: "Item".into(),
                field: "name".into(),
            },
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64,
            },
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: V::String("yes".into()),
            },
            Step::SetDefault {
                table: "Item".into(),
                field: "caps".into(),
                // UInt rides the canonical tagged form: a bare `1`
                // would read back as `Int` and trip E1101 at reverify.
                value: V::Array(vec![V::UInt(1), V::UInt(2)]),
            },
        ],
        todos: vec!["# TODO rename name → title".into()],
    };
    let text = format!("from: \"1.0.0\"\nto: \"2.0.0\"\n{}", render_draft(&draft));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("0001-draft.yaml");
    std::fs::write(&path, &text).unwrap();
    let parsed = super::super::parse_spec(&path).unwrap();
    assert_eq!(parsed.from, "1.0.0");
    assert_eq!(parsed.to, "2.0.0");
    assert_eq!(parsed.steps, draft.steps);
    assert!(text.contains("type: UInt"));
}

#[test]
fn draft_render_is_deterministic_and_todo_only_drafts_render_empty_steps() {
    let from = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
",
    );
    let diff = diff_schemas(&from, &from);
    let draft = draft_migration(&diff);
    assert_eq!(render_draft(&draft), render_draft(&draft));
    // An all-quiet diff drafts nothing: the empty-steps marker, which
    // parse_spec refuses (E2001) until the author adds steps.
    assert!(render_draft(&draft).contains("steps:\n  []\n"));
    let empty = MigrationDraft::default();
    assert!(render_draft(&empty).contains("steps:\n  []\n"));
}

#[test]
fn typed_json_value_shapes_defaults_into_their_family() {
    use serde_json::json;
    // Unsigned family demands a non-negative integer literal.
    assert_eq!(
        typed_json_value(&json!(5), &FieldType::UInt32),
        Some(V::UInt(5))
    );
    assert_eq!(typed_json_value(&json!(-1), &FieldType::UInt32), None);
    assert_eq!(typed_json_value(&json!("x"), &FieldType::UInt8), None);
    // Signed family demands an integer literal.
    assert_eq!(
        typed_json_value(&json!(-3), &FieldType::Int32),
        Some(V::Int(-3))
    );
    assert_eq!(typed_json_value(&json!(1.5), &FieldType::Int64), None);
    // Float, bool and string families.
    assert_eq!(
        typed_json_value(&json!(2.5), &FieldType::Float64),
        Some(V::Float(2.5))
    );
    assert_eq!(typed_json_value(&json!("x"), &FieldType::Float32), None);
    assert_eq!(
        typed_json_value(&json!(true), &FieldType::Bool),
        Some(V::Bool(true))
    );
    assert_eq!(typed_json_value(&json!(1), &FieldType::Bool), None);
    assert_eq!(
        typed_json_value(&json!("mage"), &FieldType::String),
        Some(V::String("mage".into()))
    );
    assert_eq!(typed_json_value(&json!(1), &FieldType::String), None);
    // Null accepts only a JSON null.
    assert_eq!(
        typed_json_value(&json!(null), &FieldType::Null),
        Some(V::Null)
    );
    assert_eq!(typed_json_value(&json!(1), &FieldType::Null), None);
    // Arrays recurse per element; one bad element kills the default.
    assert_eq!(
        typed_json_value(
            &json!([1, 2]),
            &FieldType::Array(Box::new(FieldType::UInt32))
        ),
        Some(V::Array(vec![V::UInt(1), V::UInt(2)]))
    );
    assert_eq!(
        typed_json_value(
            &json!([1, -1]),
            &FieldType::Array(Box::new(FieldType::UInt32))
        ),
        None
    );
    assert_eq!(
        typed_json_value(&json!("x"), &FieldType::Array(Box::new(FieldType::Int32))),
        None
    );
    // Everything else (bytes included) falls back to the generic bridge.
    assert_eq!(
        typed_json_value(&json!(5), &FieldType::Bytes),
        Some(V::Int(5))
    );
}

#[test]
fn json_to_value_maps_every_json_shape() {
    use serde_json::json;
    assert_eq!(json_to_value(&json!(null)), Some(V::Null));
    assert_eq!(json_to_value(&json!(true)), Some(V::Bool(true)));
    assert_eq!(json_to_value(&json!(5)), Some(V::Int(5)));
    assert_eq!(json_to_value(&json!(u64::MAX)), Some(V::UInt(u64::MAX)));
    assert_eq!(json_to_value(&json!(1.5)), Some(V::Float(1.5)));
    assert_eq!(json_to_value(&json!("s")), Some(V::String("s".to_string())));
    assert_eq!(
        json_to_value(&json!([1, "a"])),
        Some(V::Array(vec![V::Int(1), V::String("a".into())]))
    );
    let obj = json_to_value(&json!({"k": [1]}));
    let V::Object(map) = obj.unwrap() else {
        panic!("expected an object value");
    };
    assert_eq!(map.get("k"), Some(&V::Array(vec![V::Int(1)])));
}

#[test]
fn quote_string_always_reparses_as_a_string_or_refuses() {
    assert_eq!(quote_string("yes"), Some("'yes'".to_string()));
    assert_eq!(quote_string("5"), Some("'5'".to_string()));
    assert_eq!(quote_string("it's"), Some("'it''s'".to_string()));
    assert_eq!(quote_string("a\tb"), Some("'a\tb'".to_string()));
    // No inline form exists for multi-line or control-character text.
    assert_eq!(quote_string("a\nb"), None);
    assert_eq!(quote_string("a\rb"), None);
    assert_eq!(quote_string("a\u{1}b"), None);
}

#[test]
fn has_unsigned_spots_the_family_through_containers() {
    assert!(has_unsigned(&V::UInt(1)));
    assert!(!has_unsigned(&V::Int(-1)));
    assert!(has_unsigned(&V::Array(vec![V::Int(1), V::UInt(2)])));
    assert!(!has_unsigned(&V::Array(vec![V::Int(1)])));
    let mut map = IndexMap::new();
    map.insert("k".to_string(), V::UInt(2));
    assert!(has_unsigned(&V::Object(map)));
}

#[test]
fn render_step_forms_cover_every_variant_and_todo_fallback() {
    let mut remap = IndexMap::new();
    remap.insert("yes".to_string(), "true".to_string());
    let draft = MigrationDraft {
        todos: vec!["# TODO fix by hand".to_string()],
        steps: vec![
            Step::RenameField {
                table: "Item".into(),
                from: "desc".into(),
                to: "description".into(),
            },
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: V::String("common".into()),
            },
            // Unsigned rides the canonical tagged form.
            Step::SetDefault {
                table: "Item".into(),
                field: "stack".into(),
                value: V::UInt(99),
            },
            // Sequences need the block form.
            Step::SetDefault {
                table: "Item".into(),
                field: "ids".into(),
                value: V::Array(vec![V::Int(1), V::Int(2)]),
            },
            Step::RemoveField {
                table: "Item".into(),
                field: "ghost".into(),
            },
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64,
            },
            Step::RemapValues {
                table: "Item".into(),
                field: "rarity".into(),
                map: remap,
            },
            Step::RenameTable {
                from: "Mob".into(),
                to: "Enemy".into(),
            },
            // Bytes have no rule-file form → rendered as a TODO line.
            Step::SetDefault {
                table: "Item".into(),
                field: "blob".into(),
                value: V::Bytes(vec![1]),
            },
        ],
    };
    let text = render_draft(&draft);
    assert!(text.starts_with("# --- review before use ---\n# TODO fix by hand\n\nsteps:\n"));
    assert!(text
        .contains("rename_field:\n      table: Item\n      from: desc\n      to: description\n"));
    assert!(text.contains("value: 'common'\n"), "inline scalar form");
    assert!(
        text.contains("value:\n") && text.contains("99"),
        "unsigned default rides the tagged block form: {text}"
    );
    assert!(text.contains("- 1\n"), "sequence default in block form");
    assert!(text.contains("remove_field:\n      table: Item\n      field: ghost\n"));
    assert!(text.contains("widen_type:\n      table: Item\n      field: level\n"));
    assert!(text.contains("remap_values:\n      table: Item\n      field: rarity\n"));
    assert!(text.contains("'yes': 'true'\n"), "map keys stay quoted");
    assert!(text.contains("rename_table:\n      from: Mob\n      to: Enemy\n"));
    assert!(
        text.contains("# TODO step set_default(Item.blob) carries a value"),
        "bytes default degrades to a TODO: {text}"
    );
}

#[test]
fn added_and_required_flip_todo_arms() {
    let from = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      old_req:
        name: old_req
        type: { kind: String }
        required: true
      bad_default:
        name: bad_default
        type: { kind: Int32 }
        default: true
enums: {}
",
    );
    let to = schema(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      old_req:
        name: old_req
        type: { kind: String }
      new_req:
        name: new_req
        type: { kind: String }
        required: true
      new_opt:
        name: new_opt
        type: { kind: String }
      bad_default:
        name: bad_default
        type: { kind: Int32 }
        required: true
        default: true
enums: {}
",
    );
    let draft = draft_migration(&diff_schemas(&from, &to));
    // New + required + no default → TODO telling the author to backfill.
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("`Item.new_req` is new and required with no default")));
    // New + optional + no default → nothing to say.
    assert!(!draft.todos.iter().any(|t| t.contains("`Item.new_opt`")));
    // Became required with a default no rule can express → TODO.
    assert!(draft
        .todos
        .iter()
        .any(|t| t.contains("`Item.bad_default` became required and its default `true`")));
    // required → optional needs no data change: no step, no TODO.
    assert!(!draft.steps.iter().any(|s| matches!(
        s,
        Step::SetDefault { field, .. } if field == "old_req"
    )));
    assert!(!draft.todos.iter().any(|t| t.contains("`Item.old_req`")));
}

/// The YAML block renderer walks every value family: scalars render
/// bare, containers nest two columns deeper, and a value that cannot
/// survive quoting (control characters) refuses the whole block.
#[test]
fn value_to_yaml_block_renders_every_family() {
    assert_eq!(value_to_yaml_block(&V::Null, 0).as_deref(), Some("null\n"));
    assert_eq!(
        value_to_yaml_block(&V::Bool(true), 0).as_deref(),
        Some("true\n")
    );
    assert_eq!(
        value_to_yaml_block(&V::Bool(false), 0).as_deref(),
        Some("false\n")
    );
    assert_eq!(value_to_yaml_block(&V::Int(-5), 0).as_deref(), Some("-5\n"));
    assert_eq!(value_to_yaml_block(&V::UInt(7), 0).as_deref(), Some("7\n"));
    let float = value_to_yaml_block(&V::Float(0.5), 0).expect("float renders");
    assert!(float.starts_with("0.5") && float.ends_with('\n'), "{float}");
    assert_eq!(
        value_to_yaml_block(&V::String("it's".into()), 0).as_deref(),
        Some("'it''s'\n")
    );
    // Bytes have no YAML scalar form; control characters cannot quote.
    assert_eq!(value_to_yaml_block(&V::Bytes(vec![1]), 0), None);
    assert_eq!(value_to_yaml_block(&V::String("a\nb".into()), 0), None);

    // Array of scalars keeps `- item` on one line.
    let arr = V::Array(vec![V::Int(1), V::String("x".into())]);
    assert_eq!(
        value_to_yaml_block(&arr, 0).as_deref(),
        Some("- 1\n- 'x'\n")
    );
    // Nested containers take the two-space `-\n` form.
    let nested = V::Array(vec![V::Array(vec![V::Int(2)])]);
    assert_eq!(
        value_to_yaml_block(&nested, 0).as_deref(),
        Some("-\n  - 2\n")
    );
    // Object: scalar members inline, container members nest.
    let mut map = indexmap::IndexMap::new();
    map.insert("plain".to_string(), V::String("v".into()));
    map.insert("deep".to_string(), V::Array(vec![V::Int(3)]));
    let obj = V::Object(map);
    assert_eq!(
        value_to_yaml_block(&obj, 2).as_deref(),
        Some("  'plain': 'v'\n  'deep':\n    - 3\n")
    );
    // Padding shifts the container keys, not the first scalar level.
    assert_eq!(
        value_to_yaml_block(&V::Array(vec![V::Int(1)]), 2).as_deref(),
        Some("  - 1\n")
    );
}
