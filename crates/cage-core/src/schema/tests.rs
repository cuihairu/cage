use super::field_type::scalar_wire;
use super::field_type_serde::{BoundedFieldTypeDe, FieldTypeDe, MapFieldDe, TaggedUnionDe};
use super::*;
use std::collections::HashSet;

/// `map<key_type, value_type>` — shared by the tests below.
fn map_type(key_type: MapKeyType, value_type: FieldType) -> FieldType {
    FieldType::Map(MapField {
        key_type,
        value_type: Box::new(value_type),
    })
}

#[test]
fn test_schema_validation() {
    let mut schema = Schema::new();
    let mut table = TableSchema {
        name: "Item".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    table.fields.insert(
        "id".to_string(),
        FieldSchema {
            name: "id".to_string(),
            field_type: FieldType::UInt32,
            description: None,
            required: true,
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
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        },
    );
    schema.add_table(table);

    let diags = schema.validate();
    assert!(!diags.has_errors());
}

#[test]
fn test_dependency_graph() {
    let mut schema = Schema::new();
    let mut item = TableSchema {
        name: "Item".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    item.fields.insert(
        "id".to_string(),
        FieldSchema {
            name: "id".to_string(),
            field_type: FieldType::UInt32,
            description: None,
            required: true,
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
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        },
    );
    schema.add_table(item);

    let mut monster = TableSchema {
        name: "Monster".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    monster.fields.insert(
        "id".to_string(),
        FieldSchema {
            name: "id".to_string(),
            field_type: FieldType::UInt32,
            description: None,
            required: true,
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
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        },
    );
    monster.fields.insert(
        "drop_item_id".to_string(),
        FieldSchema {
            name: "drop_item_id".to_string(),
            field_type: FieldType::UInt32,
            description: None,
            required: true,
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
            reference: Some(ReferenceSchema {
                table: "Item".to_string(),
                field: "id".to_string(),
                predicate: None,
                cardinality: "many".to_string(),
                compatible_with: None,
            }),
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        },
    );
    schema.add_table(monster);

    let graph = crate::reference::DependencyGraph::from_schema(&schema);
    let deps = graph.dependencies("Monster");
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], "Item");

    let order = graph.topological_sort().unwrap();
    assert_eq!(order[0], "Item");
    assert_eq!(order[1], "Monster");
}

#[test]
fn field_type_classification_helpers() {
    for t in [
        FieldType::Int8,
        FieldType::Int16,
        FieldType::Int32,
        FieldType::Int64,
        FieldType::UInt8,
        FieldType::UInt16,
        FieldType::UInt32,
        FieldType::UInt64,
        FieldType::Float32,
        FieldType::Float64,
    ] {
        assert!(t.is_numeric());
    }
    for t in [
        FieldType::Int8,
        FieldType::Int16,
        FieldType::Int32,
        FieldType::Int64,
        FieldType::UInt8,
        FieldType::UInt16,
        FieldType::UInt32,
        FieldType::UInt64,
    ] {
        assert!(t.is_integer());
        assert!(!t.is_float());
    }
    assert!(FieldType::Float32.is_float());
    assert!(FieldType::Float64.is_float());
    assert!(FieldType::Float32.is_numeric());
    assert!(FieldType::Float64.is_numeric());
    assert!(!FieldType::Float32.is_integer());
    assert!(!FieldType::Float64.is_integer());
    for t in [
        FieldType::Null,
        FieldType::Bool,
        FieldType::String,
        FieldType::Bytes,
        FieldType::Any,
        FieldType::Enum("E".to_string()),
        FieldType::Array(Box::new(FieldType::Int32)),
        FieldType::Object(IndexMap::new()),
        map_type(MapKeyType::String, FieldType::Int32),
        map_type(MapKeyType::Int, FieldType::Int32),
    ] {
        assert!(!t.is_numeric());
        assert!(!t.is_integer());
        assert!(!t.is_float());
    }
}

#[test]
fn rust_type_names_for_every_variant() {
    assert_eq!(FieldType::Null.rust_type(), "()");
    assert_eq!(FieldType::Bool.rust_type(), "bool");
    assert_eq!(FieldType::Int8.rust_type(), "i8");
    assert_eq!(FieldType::Int16.rust_type(), "i16");
    assert_eq!(FieldType::Int32.rust_type(), "i32");
    assert_eq!(FieldType::Int64.rust_type(), "i64");
    assert_eq!(FieldType::UInt8.rust_type(), "u8");
    assert_eq!(FieldType::UInt16.rust_type(), "u16");
    assert_eq!(FieldType::UInt32.rust_type(), "u32");
    assert_eq!(FieldType::UInt64.rust_type(), "u64");
    assert_eq!(FieldType::Float32.rust_type(), "f32");
    assert_eq!(FieldType::Float64.rust_type(), "f64");
    assert_eq!(FieldType::String.rust_type(), "String");
    assert_eq!(FieldType::Bytes.rust_type(), "Vec<u8>");
    assert_eq!(
        FieldType::Array(Box::new(FieldType::Int32)).rust_type(),
        "Vec<_>"
    );
    assert_eq!(
        FieldType::Object(IndexMap::new()).rust_type(),
        "IndexMap<String, _>"
    );
    assert_eq!(
        map_type(MapKeyType::String, FieldType::Int32).rust_type(),
        "HashMap<String, _>"
    );
    assert_eq!(
        map_type(MapKeyType::Int, FieldType::Int32).rust_type(),
        "HashMap<i64, _>"
    );
    assert_eq!(FieldType::Enum("Rarity".to_string()).rust_type(), "Enum");
    assert_eq!(FieldType::Any.rust_type(), "serde_json::Value");
}

#[test]
fn field_type_yaml_parsing_all_kinds_and_edges() {
    // Every one of the 19 FieldType kinds must round-trip through YAML.
    let cases = [
            ("{ kind: Null }", FieldType::Null),
            ("{ kind: Bool }", FieldType::Bool),
            ("{ kind: Int8 }", FieldType::Int8),
            ("{ kind: Int16 }", FieldType::Int16),
            ("{ kind: Int32 }", FieldType::Int32),
            ("{ kind: Int64 }", FieldType::Int64),
            ("{ kind: UInt8 }", FieldType::UInt8),
            ("{ kind: UInt16 }", FieldType::UInt16),
            ("{ kind: UInt32 }", FieldType::UInt32),
            ("{ kind: UInt64 }", FieldType::UInt64),
            ("{ kind: Float32 }", FieldType::Float32),
            ("{ kind: Float64 }", FieldType::Float64),
            ("{ kind: String }", FieldType::String),
            ("{ kind: Bytes }", FieldType::Bytes),
            ("{ kind: Any }", FieldType::Any),
            (
                "{ kind: Array, value: { kind: Int32 } }",
                FieldType::Array(Box::new(FieldType::Int32)),
            ),
            (
                "{ kind: Object, value: { hp: { kind: Int32 }, tag: { kind: String } } }",
                FieldType::Object(IndexMap::from([
                    ("hp".to_string(), FieldType::Int32),
                    ("tag".to_string(), FieldType::String),
                ])),
            ),
            (
                "{ kind: Enum, value: Rarity }",
                FieldType::Enum("Rarity".to_string()),
            ),
            // The 19th kind: map<K, V> — string keys ...
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }",
                map_type(MapKeyType::String, FieldType::Int32),
            ),
            // ... signed-integer keys ...
            (
                "{ kind: Map, value: { key_type: int, value_type: { kind: String } } }",
                map_type(MapKeyType::Int, FieldType::String),
            ),
            // ... and arbitrary nesting in both directions.
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } }",
                map_type(
                    MapKeyType::String,
                    map_type(MapKeyType::String, FieldType::Int32),
                ),
            ),
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }",
                map_type(MapKeyType::String, FieldType::Array(Box::new(FieldType::Int32))),
            ),
        ];
    let mut kinds = HashSet::new();
    for (yaml, expected) in cases {
        let parsed: FieldType =
            serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("parse {yaml}: {e}"));
        assert_eq!(parsed, expected, "yaml: {yaml}");
        // serializing and re-parsing keeps the same shape
        let emitted = serde_yaml::to_string(&parsed).expect("serialize kind");
        let back: FieldType = serde_yaml::from_str(&emitted).expect("re-parse kind");
        assert_eq!(back, expected, "roundtrip: {emitted}");
        // Debug starts with the variant name (e.g. `Map(MapField { ... })`)
        let debug = format!("{expected:?}");
        kinds.insert(
            debug
                .split('(')
                .next()
                .expect("Debug rendering starts with the variant name")
                .to_string(),
        );
    }
    assert_eq!(kinds.len(), 19, "kinds covered: {kinds:?}");

    // unknown kind, missing content for data variants, missing tag, not a map
    assert!(serde_yaml::from_str::<FieldType>("{ kind: Quadruple }").is_err());
    assert!(serde_yaml::from_str::<FieldType>("{ kind: Array }").is_err());
    assert!(serde_yaml::from_str::<FieldType>("{ kind: Enum }").is_err());
    assert!(serde_yaml::from_str::<FieldType>("{ kind: Map }").is_err());
    assert!(
        serde_yaml::from_str::<FieldType>(
            "{ kind: Map, value: { key_type: bool, value_type: { kind: Int32 } } }"
        )
        .is_err(),
        "unknown key_type must fail"
    );
    assert!(
        serde_yaml::from_str::<FieldType>("{ kind: Map, value: { key_type: string } }").is_err(),
        "missing value_type must fail"
    );
    assert!(serde_yaml::from_str::<FieldType>("{}").is_err());
    assert!(serde_yaml::from_str::<FieldType>("just-a-string").is_err());
}

#[test]
fn map_key_type_wire_format_is_lowercase() {
    // Wire format is the lowercase rename — "string" / "int", nothing else.
    assert_eq!(
        serde_yaml::to_string(&MapKeyType::String).expect("yaml string"),
        "string\n"
    );
    assert_eq!(
        serde_yaml::to_string(&MapKeyType::Int).expect("yaml int"),
        "int\n"
    );
    assert_eq!(
        serde_json::to_string(&MapKeyType::String).expect("json string"),
        "\"string\""
    );
    assert_eq!(
        serde_json::to_string(&MapKeyType::Int).expect("json int"),
        "\"int\""
    );
    for (wire, expected) in [
        ("string", MapKeyType::String),
        ("int", MapKeyType::Int),
        ("\"string\"", MapKeyType::String),
        ("\"int\"", MapKeyType::Int),
    ] {
        let key: MapKeyType = if wire.starts_with('"') {
            serde_json::from_str(wire).expect("json key type")
        } else {
            serde_yaml::from_str(wire).expect("yaml key type")
        };
        assert_eq!(key, expected, "wire: {wire}");
    }
    assert!(serde_yaml::from_str::<MapKeyType>("u32").is_err());
    assert!(serde_yaml::from_str::<MapKeyType>("String").is_err());
    assert!(serde_json::from_str::<MapKeyType>("\"u32\"").is_err());
}

#[test]
fn field_schema_yaml_parses_map_field() {
    let yaml = r"
name: drops
type:
  kind: Map
  value:
    key_type: string
    value_type:
      kind: Array
      value: { kind: Int32 }
description: drop table keyed by rarity
";
    let field: FieldSchema = serde_yaml::from_str(yaml).expect("map field parses");
    assert_eq!(field.name, "drops");
    assert_eq!(
        field.field_type,
        map_type(
            MapKeyType::String,
            FieldType::Array(Box::new(FieldType::Int32))
        )
    );
    assert_eq!(
        field.description.as_deref(),
        Some("drop table keyed by rarity")
    );
    // JSON carries the same shape
    let json = serde_json::to_value(&field.field_type).expect("map type to json");
    assert_eq!(json["kind"], "Map");
    assert_eq!(json["value"]["key_type"], "string");
    assert_eq!(json["value"]["value_type"]["kind"], "Array");
    let back: FieldType = serde_json::from_value(json).expect("map type from json");
    assert_eq!(back, field.field_type);
}

#[test]
fn field_schema_yaml_parsing_rules_constraints_and_metadata() {
    let yaml = r#"
name: level
type: { kind: Int32 }
description: skill level
required: true
default: 1
min: 1
max: 100
targets: [server]
rules:
  - name: level_bounds
    assert: "`level` >= 1"
    message: level must be positive
    warning_only: true
  - name: no_message
    assert: "`level` <= 100"
custom_note: keep
"#;
    let field: FieldSchema = serde_yaml::from_str(yaml).expect("field parses");
    assert_eq!(field.name, "level");
    assert_eq!(field.field_type, FieldType::Int32);
    assert_eq!(field.description.as_deref(), Some("skill level"));
    assert!(field.required);
    assert_eq!(field.default, Some(serde_json::json!(1)));
    assert_eq!(field.min, Some(1.0));
    assert_eq!(field.max, Some(100.0));
    assert_eq!(field.targets, ["server"]);
    assert_eq!(field.rules.len(), 2);
    assert_eq!(field.rules[0].name, "level_bounds");
    assert_eq!(field.rules[0].assert, "`level` >= 1");
    assert_eq!(
        field.rules[0].message.as_deref(),
        Some("level must be positive")
    );
    assert!(field.rules[0].warning_only);
    // message absent and warning_only defaults to false
    assert_eq!(field.rules[1].message, None);
    assert!(!field.rules[1].warning_only);
    // unknown keys flatten into the metadata bag
    assert_eq!(
        field.metadata.get("custom_note"),
        Some(&serde_json::json!("keep"))
    );
    assert!(field.min_length.is_none());
    assert!(field.max_length.is_none());
    assert!(field.pattern.is_none());

    // required keys: `name` and `type` must both be present
    assert!(serde_yaml::from_str::<FieldSchema>("type: { kind: String }\n").is_err());
    assert!(serde_yaml::from_str::<FieldSchema>("name: x\n").is_err());
    // an expression rule requires its `assert`
    assert!(serde_yaml::from_str::<ExpressionRule>("name: r\n").is_err());
    let bare: ExpressionRule =
        serde_yaml::from_str("name: r\nassert: 'true'\n").expect("minimal rule parses");
    assert!(!bare.warning_only);
    assert!(bare.message.is_none());
}

#[test]
fn table_schema_yaml_parsing_primary_key_unique_and_order() {
    let yaml = r"
name: Item
description: an item
primary_key: [id, version]
fields:
  id: { name: id, type: { kind: UInt32 } }
  version: { name: version, type: { kind: UInt16 } }
unique_constraints:
  - { name: uniq_pair, fields: [id, version] }
order_by: [id]
targets: [server, client]
";
    let table: TableSchema = serde_yaml::from_str(yaml).expect("table parses");
    assert_eq!(table.name, "Item");
    assert_eq!(table.description.as_deref(), Some("an item"));
    assert_eq!(table.primary_key, ["id", "version"]);
    assert_eq!(table.fields.len(), 2);
    assert_eq!(table.unique_constraints.len(), 1);
    assert_eq!(table.unique_constraints[0].name, "uniq_pair");
    assert_eq!(table.unique_constraints[0].fields, ["id", "version"]);
    assert_eq!(table.order_by, Some(vec!["id".to_string()]));
    assert_eq!(table.targets, ["server", "client"]);

    // primary_key has no default → required
    assert!(
        serde_yaml::from_str::<TableSchema>("name: T\nfields: {}\n").is_err(),
        "missing primary_key must fail"
    );
    // a unique constraint must name its fields
    assert!(serde_yaml::from_str::<TableSchema>(
        "name: T\nprimary_key: [a]\nfields: {}\nunique_constraints: [{ name: c }]\n"
    )
    .is_err());
}

#[test]
fn reference_schema_cardinality_serde_edges() {
    let bare: ReferenceSchema =
        serde_yaml::from_str("table: Item\nfield: id\n").expect("minimal reference");
    assert_eq!(bare.cardinality, "one");
    assert!(bare.predicate.is_none());
    assert!(bare.compatible_with.is_none());
    // cardinality "one" is the default and is skipped when serializing
    let emitted = serde_yaml::to_string(&bare).expect("serialize reference");
    assert!(!emitted.contains("cardinality"));

    let many: ReferenceSchema = serde_yaml::from_str(
        "table: Item\nfield: id\ncardinality: many\ncompatible_with: [server]\n",
    )
    .expect("many reference");
    assert_eq!(many.cardinality, "many");
    assert_eq!(many.compatible_with, Some(vec!["server".to_string()]));
    let emitted = serde_yaml::to_string(&many).expect("serialize many");
    assert!(emitted.contains("cardinality"));
    let back: ReferenceSchema = serde_yaml::from_str(&emitted).expect("re-parse reference");
    assert_eq!(back.cardinality, "many");

    // required keys: table and field
    assert!(serde_yaml::from_str::<ReferenceSchema>("table: Item\n").is_err());
    assert!(serde_yaml::from_str::<ReferenceSchema>("field: id\n").is_err());
}

#[test]
fn schema_default_and_enum_registration() {
    let mut schema = Schema::default();
    assert!(schema.tables.is_empty());
    assert!(schema.enums.is_empty());
    assert!(schema.metadata.is_none());
    assert!(schema.get_table("Item").is_none());

    schema.add_enum(EnumSchema {
        name: "Rarity".to_string(),
        values: vec![EnumValue {
            name: "Common".to_string(),
            value: None,
            description: None,
        }],
        description: None,
    });
    assert!(schema.enums.contains_key("Rarity"));
    // adding the same name replaces the previous definition
    schema.add_enum(EnumSchema {
        name: "Rarity".to_string(),
        values: vec![],
        description: None,
    });
    assert!(schema.enums["Rarity"].values.is_empty());
}

#[test]
fn schema_lookup_helpers_hit_and_miss() {
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 } }
enums:
  Rarity:
    name: Rarity
    values:
      - { name: Common }
",
    )
    .expect("schema parses");

    let item = schema.get_table("Item").expect("Item exists");
    assert_eq!(item.primary_key, ["id"]);
    assert!(schema.get_table("Ghost").is_none());

    let id = schema.get_field("Item", "id").expect("id exists");
    assert_eq!(id.field_type, FieldType::UInt32);
    assert!(schema.get_field("Item", "ghost").is_none());
    assert!(schema.get_field("Ghost", "id").is_none());

    let rarity = schema.get_enum("Rarity").expect("Rarity exists");
    assert_eq!(rarity.values.len(), 1);
    assert_eq!(rarity.values[0].name, "Common");
    assert!(schema.get_enum("Ghost").is_none());
}

/// A one-field table whose sole field carries the given type.
fn schema_with_one_field(t: FieldType) -> Schema {
    let field = FieldSchema {
        name: "deep".to_string(),
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
    };
    let mut fields = IndexMap::new();
    fields.insert("deep".to_string(), field);
    let mut tables = IndexMap::new();
    tables.insert(
        "T".to_string(),
        TableSchema {
            name: "T".to_string(),
            description: None,
            primary_key: Vec::new(),
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

/// A `FieldType` nested deeper than `MAX_FIELD_TYPE_DEPTH` is refused by
/// `validate` with E1004 — the recursive schema/generator walks never
/// see an unbounded nesting depth.
#[test]
fn schema_rejects_field_type_deeper_than_the_max() {
    // Build Array<Array<...Array<Int32>...>> with MAX+2 levels.
    let mut t = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 2) {
        t = FieldType::Array(Box::new(t));
    }
    let schema = schema_with_one_field(t);
    let diags = schema.validate();
    let msg = diags
        .errors()
        .iter()
        .find(|d| d.code == "E1004" && d.message.contains("nesting depth"))
        .map(|d| d.message.clone())
        .expect("a too-deep type is E1004");
    // A depth of MAX+2 is still within one probe of the cap (the walk
    // only saturates past MAX+1), so the real depth is reported.
    assert!(
        msg.contains(&format!("depth {}", MAX_FIELD_TYPE_DEPTH + 2)),
        "the real marginal depth is named: {msg}"
    );
    assert!(
        msg.contains(&format!(
            "maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
        )),
        "E1004 names the cap: {msg}"
    );

    // Exactly at the limit is accepted.
    let mut t = FieldType::Int32;
    for _ in 0..MAX_FIELD_TYPE_DEPTH {
        t = FieldType::Array(Box::new(t));
    }
    let schema = schema_with_one_field(t);
    assert!(
        !schema
            .validate()
            .errors()
            .iter()
            .any(|d| d.message.contains("nesting depth")),
        "at the limit is accepted"
    );

    // One level over the cap is a real, non-saturated measurement: the
    // walk reports MAX+1 as the measured depth (distinguishable from a
    // pathologically deep type that saturates).
    let mut t = FieldType::Int32;
    for _ in 0..=MAX_FIELD_TYPE_DEPTH {
        t = FieldType::Array(Box::new(t));
    }
    let schema = schema_with_one_field(t);
    let diags = schema.validate();
    let msg = diags
        .errors()
        .iter()
        .find(|d| d.code == "E1004" && d.message.contains("nesting depth"))
        .map(|d| d.message.clone())
        .expect("one-over-the-cap is E1004");
    assert!(
        msg.contains(&format!("depth {}", MAX_FIELD_TYPE_DEPTH + 1)),
        "one level over the cap reports its real depth: {msg}"
    );
}

/// The depth probe itself must not overflow the stack on a
/// maliciously deep type: the walk short-circuits at
/// `MAX_FIELD_TYPE_DEPTH` instead of recursing with the input depth,
/// so even a type nested thousands of times beyond the cap is refused
/// with E1004 rather than crashing the process. (An unbounded probe
/// would recurse ~200k frames and overflow the test stack before
/// `validate` could report anything.)
#[test]
fn schema_depth_probe_short_circuits_on_a_huge_type() {
    let mut t = FieldType::Int32;
    for _ in 0..200_000 {
        t = FieldType::Array(Box::new(t));
    }
    let schema = schema_with_one_field(t);
    let diags = schema.validate();
    let msg = diags
        .errors()
        .iter()
        .find(|d| d.code == "E1004" && d.message.contains("nesting depth"))
        .expect("a huge type is refused, not a crash")
        .message
        .clone();
    // The diagnostic must not carry the short-circuit sentinel (65) as if
    // it were the real depth — that would mislead an operator debugging a
    // 200k-deep type into thinking the value is only one level over. The
    // saturated probe omits any depth figure entirely.
    assert!(
        !msg.contains("has nesting depth"),
        "a saturated probe must not name a (sentinel) depth: {msg}"
    );
    assert!(
        msg.contains(&format!(
            "maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
        )),
        "E1004 names the cap: {msg}"
    );
    // Drop the (adversarial) schema for real: the iterative `Drop` must
    // release the 200k-deep spine without overflowing the stack. This is
    // the regression the old `mem::forget` masked — a test that leaks the
    // value never proves the lifecycle is safe end to end.
    drop(schema);
}

/// The derived `Clone` / `PartialEq` / `Debug` / `Serialize` recursed
/// through `Array` / `Map` / `Object` / `TaggedUnion` and overflowed on a
/// type nested past the depth cap. `Clone` / `PartialEq` / `Debug` now walk
/// the spine with an explicit worklist, and `Serialize` refuses an
/// over-cap type rather than recursing — so a hand-built 200k-deep type
/// survives a full round of clone, compare, debug, and a *refused*
/// serialize (drop is covered by the probe test above).
#[test]
fn huge_field_type_survives_clone_eq_debug_and_serialize() {
    let mut t = FieldType::Int32;
    for _ in 0..200_000 {
        t = FieldType::Array(Box::new(t));
    }
    let cloned = t.clone();
    assert!(t == cloned, "iterative PartialEq handles a huge type");
    let _ = format!("{t:?}");
    let err = serde_json::to_string(&t)
        .expect_err("Serialize refuses an over-cap type instead of recursing");
    assert!(
        err.to_string().contains("nesting depth limit"),
        "the refusal names the cap: {err}"
    );
    drop(cloned);
    drop(t);
}

/// The bounded-stack `fmt_deep` fallback must cover every `FieldType`
/// variant: it is the only formatter that runs on an over-cap type, so
/// an unreachable arm would make a pathological debug print incomplete.
#[test]
fn fmt_deep_renders_every_variant_without_recursion() {
    // Array(Map(Object(TaggedUnion(...)))) exercises every container
    // arm of the bounded fallback formatter; nest it past the cap so
    // `Debug` takes the `fmt_deep` path.
    let mut deep = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "kind".to_string(),
        branches: {
            let mut b: IndexMap<String, IndexMap<String, FieldType>> = IndexMap::new();
            let mut p = IndexMap::new();
            p.insert("leaf".to_string(), FieldType::Enum("E".to_string()));
            b.insert("arm".to_string(), p);
            b
        },
    }));
    deep = FieldType::Object({
        let mut m = IndexMap::new();
        m.insert("k".to_string(), deep);
        m
    });
    deep = FieldType::Map(MapField {
        key_type: MapKeyType::Int,
        value_type: Box::new(deep),
    });
    deep = FieldType::Array(Box::new(deep));
    for _ in 0..=MAX_FIELD_TYPE_DEPTH {
        deep = FieldType::Array(Box::new(deep));
    }
    let printed = format!("{deep:?}");
    for needle in [
        "Array(",
        "Map(MapField { key_type: Int, value_type: ",
        "Object({",
        "TaggedUnion(TaggedUnion { tag: ",
        "Enum(",
    ] {
        assert!(printed.contains(needle), "missing {needle} in {printed}");
    }
}

/// The custom iterative `Drop` means Rust refuses by-value moves out of
/// the container variants (E0509). `FieldType::take` is the documented
/// migration path for code that needs to *move the whole value out* of a
/// `&mut` slot (e.g. evicting a field from an `IndexMap`) — it swaps the
/// default `Null` in, returning the owned value without a by-value match.
#[test]
fn field_type_take_is_the_migration_path_for_e0509() {
    let mut map = IndexMap::new();
    map.insert(
        "a".to_string(),
        FieldType::Array(Box::new(FieldType::Int32)),
    );
    let owned = map.get_mut("a").expect("present").take();
    assert_eq!(
        map["a"],
        FieldType::Null,
        "the slot is reset to the default"
    );
    assert_eq!(owned, FieldType::Array(Box::new(FieldType::Int32)));
}

/// The iterative `Debug` must reproduce the derived `Debug` byte-for-byte
/// for an at-or-under-cap type — including the compact `{ "a": ... }`
/// spacing (no leading space after `{`), the empty-map `{}` shape, and
/// `{:#?}` pretty alternation. Downstream snapshots and diagnostics that
/// pin `{:?}` / `{:#?}` strings must not drift from the derive.
#[test]
fn debug_matches_the_derived_shape_for_compact_and_pretty() {
    let mut fields = IndexMap::new();
    fields.insert("a".to_string(), FieldType::Int32);
    fields.insert(
        "b".to_string(),
        FieldType::Array(Box::new(FieldType::String)),
    );
    let object = FieldType::Object(fields);
    assert_eq!(
        format!("{object:?}"),
        "Object({\"a\": Int32, \"b\": Array(String)})"
    );
    assert_eq!(
            format!("{object:#?}"),
            "Object(\n    {\n        \"a\": Int32,\n        \"b\": Array(\n            String,\n        ),\n    },\n)"
        );
    assert_eq!(
        format!("{:?}", FieldType::Object(IndexMap::new())),
        "Object({})"
    );
    assert_eq!(
        format!("{:#?}", FieldType::Object(IndexMap::new())),
        "Object(\n    {},\n)"
    );

    let mut branches = IndexMap::new();
    let mut dmg = IndexMap::new();
    dmg.insert("amount".to_string(), FieldType::Int32);
    branches.insert("Damage".to_string(), dmg);
    let tu = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "$type".to_string(),
        branches,
    }));
    assert_eq!(
        format!("{tu:?}"),
        "TaggedUnion(TaggedUnion { tag: \"$type\", branches: {\"Damage\": {\"amount\": Int32}} })"
    );
    // A type nested past the cap still renders (bounded stack) even if it
    // is an over-cap value the validator would refuse; it must not crash.
    let mut t = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        t = FieldType::Array(Box::new(t));
    }
    let _ = format!("{t:?}");
}

/// The iterative `Serialize` must emit `Object` / `TaggedUnion` keys in
/// declaration order and without a `preserve_order` dependency: the crate
/// enables `serde_json/preserve_order` itself, so the wire format is
/// stable even when compiled in isolation (or if a consumer does not
/// re-enable the feature). Also the child values are moved out of the
/// work stack, not deep-cloned (no O(n) copies per node).
#[test]
fn serialize_object_and_union_keys_stay_in_declaration_order() {
    let mut fields = IndexMap::new();
    fields.insert("zz".to_string(), FieldType::Int32);
    fields.insert("aa".to_string(), FieldType::String);
    fields.insert(
        "mm".to_string(),
        FieldType::Array(Box::new(FieldType::Int32)),
    );
    let object = FieldType::Object(fields);
    let wire = serde_json::to_string(&object).expect("serialize object");
    assert_eq!(
        wire,
        r#"{"kind":"Object","value":{"zz":{"kind":"Int32"},"aa":{"kind":"String"},"mm":{"kind":"Array","value":{"kind":"Int32"}}}}"#
    );

    let mut branches = IndexMap::new();
    let mut heal = IndexMap::new();
    heal.insert("duration".to_string(), FieldType::Float32);
    branches.insert("Heal".to_string(), heal);
    let mut dmg = IndexMap::new();
    dmg.insert("amount".to_string(), FieldType::Int32);
    branches.insert("Damage".to_string(), dmg);
    let tu = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "$type".to_string(),
        branches,
    }));
    let wire = serde_json::to_string(&tu).expect("serialize union");
    assert_eq!(
        wire,
        r#"{"kind":"TaggedUnion","value":{"tag":"$type","branches":{"Heal":{"duration":{"kind":"Float32"}},"Damage":{"amount":{"kind":"Int32"}}}}}"#
    );
}

/// A type nested past [`MAX_FIELD_TYPE_DEPTH`] must be refused by both
/// `Serialize` and `Deserialize` — a value either round-trips or is
/// rejected on both sides. Before the bounded `Deserialize`, a depth
/// 65..~127 type could be `from_str`'d (serde's own ~128 recursion guard)
/// yet rejected by `to_string`, an asymmetry.
#[test]
fn deserialize_refuses_a_type_deeper_than_the_cap() {
    // At the cap: accepted on both sides.
    let mut at = FieldType::Int32;
    for _ in 0..MAX_FIELD_TYPE_DEPTH {
        at = FieldType::Array(Box::new(at));
    }
    let wire = serde_yaml::to_string(&at).expect("at-cap serializes");
    let back: FieldType = serde_yaml::from_str(&wire).expect("at-cap parses");
    assert_eq!(back, at);

    // One over the cap: refused by `Serialize` (E1004-style error) and now
    // refused by `Deserialize` too.
    let mut over = FieldType::Int32;
    for _ in 0..=MAX_FIELD_TYPE_DEPTH {
        over = FieldType::Array(Box::new(over));
    }
    assert!(
        serde_yaml::to_string(&over).is_err(),
        "over-cap must not serialize"
    );
    // Build the over-cap YAML shape directly (serialize refuses it): an
    // `Array` spine of `MAX + 1` levels wrapping `Int32`.
    let mut v = serde_yaml::to_value(FieldType::Int32).unwrap();
    for _ in 0..=MAX_FIELD_TYPE_DEPTH {
        v = serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([
            (
                serde_yaml::Value::String("kind".into()),
                serde_yaml::Value::String("Array".into()),
            ),
            (serde_yaml::Value::String("value".into()), v),
        ]));
    }
    let over_wire = serde_yaml::to_string(&v).unwrap();
    let err = serde_yaml::from_str::<FieldType>(&over_wire).expect_err("over-cap refuses");
    assert!(
        err.to_string().contains("nesting depth limit"),
        "the refusal names the cap: {err}"
    );
}

#[test]
fn schema_validate_reports_every_error_kind() {
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    primary_key: [missing_pk]
    fields:
      id: { name: id, type: { kind: UInt32 } }
      rarity: { name: rarity, type: { kind: Enum, value: Rarity } }
      element: { name: element, type: { kind: Enum, value: Present } }
      drop: { name: drop, type: { kind: UInt32 }, reference: { table: Ghost, field: id } }
      sub: { name: sub, type: { kind: UInt32 }, reference: { table: Item, field: nope } }
    unique_constraints:
      - { name: uc_missing, fields: [ghost_field] }
enums:
  Present:
    name: Present
    values:
      - { name: A }
",
    )
    .expect("schema parses");

    let diags = schema.validate();
    assert!(diags.has_errors());
    let messages: Vec<String> = diags.errors().iter().map(|d| d.message.clone()).collect();
    assert_eq!(messages.len(), 5, "messages: {messages:?}");
    let expect = |needle: &str| {
        assert!(
            messages.iter().any(|m| m.contains(needle)),
            "no message contains {needle:?} in {messages:?}"
        );
    };
    expect("Primary key 'missing_pk' not found in table 'Item' fields");
    expect("Unique constraint 'uc_missing' references unknown field 'ghost_field'");
    expect("Field 'rarity' in table 'Item' references unknown enum 'Rarity'");
    expect("Field 'drop' in table 'Item' references unknown table 'Ghost'");
    expect("Field 'sub' in table 'Item' references unknown field 'nope' in table 'Item'");
    for d in &diags {
        assert_eq!(d.code, "E1004");
        assert_eq!(d.source, "schema");
    }
    // a schema with only resolvable references validates cleanly
    let clean: Schema = serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 } }
enums: {}
",
    )
    .expect("clean schema parses");
    assert!(!clean.validate().has_errors());
}

#[test]
fn dependency_graph_sort_handles_revisits_and_cycles() {
    // A → B → C: C is pulled in as a dependency before the loop reaches B,
    // so the second visit of B skips the recursive call entirely.
    let mut graph = crate::reference::DependencyGraph::new();
    graph.add_edge("A", "B");
    graph.add_edge("B", "C");
    assert_eq!(graph.dependencies("A"), ["B"]);
    assert_eq!(graph.dependencies("B"), ["C"]);
    assert_eq!(graph.dependencies("C"), [] as [&String; 0]);
    let order = graph.topological_sort().expect("acyclic");
    assert_eq!(order, ["C", "B", "A"]);

    // circular dependencies are reported instead of looping forever
    let mut cyclic = crate::reference::DependencyGraph::new();
    cyclic.add_edge("X", "Y");
    cyclic.add_edge("Y", "X");
    let err = cyclic.topological_sort().expect_err("cycle detected");
    assert!(
        err.contains("Circular dependency detected involving:"),
        "unexpected error: {err}"
    );
}

/// A two-table fixture: `Item.hp` is required with `min: 1`,
/// `Monster.drop_table` is optional; only `Item` carries env
/// overrides.
fn env_schema() -> Schema {
    let mut schema = Schema::new();
    let field = |name: &str, required: bool| FieldSchema {
        name: name.to_string(),
        field_type: FieldType::Int32,
        nullable: false,
        description: None,
        required,
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
        targets: vec![],
        rules: vec![],
        metadata: IndexMap::new(),
    };
    let mut item = TableSchema {
        name: "Item".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    item.fields.insert("id".to_string(), field("id", true));
    item.fields.insert("hp".to_string(), field("hp", true));
    let mut prod = IndexMap::new();
    let hp = FieldOverride {
        required: Some(false),
        min: Some(100.0),
        ..FieldOverride::default()
    };
    prod.insert("hp".to_string(), hp);
    item.env_overrides.insert("prod".to_string(), prod);
    schema.add_table(item);

    let mut monster = TableSchema {
        name: "Monster".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    monster.fields.insert("id".to_string(), field("id", true));
    monster
        .fields
        .insert("drop_table".to_string(), field("drop_table", false));
    schema.add_table(monster);
    schema
}

#[test]
fn declared_envs_unions_table_override_keys() {
    let schema = env_schema();
    assert_eq!(
        schema.declared_envs(),
        ["prod".to_string()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    );

    let plain = Schema::new();
    assert!(plain.declared_envs().is_empty());
}

#[test]
fn validate_env_overrides_rejects_unknown_fields_and_empty_patches() {
    let mut schema = env_schema();

    // unknown field → named in the error
    let mut prod = IndexMap::new();
    prod.insert("ghost".to_string(), FieldOverride::default());
    schema
        .tables
        .get_mut("Item")
        .unwrap()
        .env_overrides
        .insert("prod".to_string(), prod);
    let err = schema.validate_env_overrides().unwrap_err();
    assert!(
        err.contains("unknown field 'ghost' in table 'Item'"),
        "{err}"
    );

    // empty patch → also rejected
    let mut schema = env_schema();
    let mut prod = IndexMap::new();
    prod.insert(
        "hp".to_string(),
        FieldOverride {
            min: Some(1.0),
            ..FieldOverride::default()
        },
    );
    prod.insert("id".to_string(), FieldOverride::default());
    schema
        .tables
        .get_mut("Item")
        .unwrap()
        .env_overrides
        .insert("prod".to_string(), prod);
    let err = schema.validate_env_overrides().unwrap_err();
    assert!(err.contains("sets no constraint"), "{err}");

    // well-formed overrides pass
    assert!(env_schema().validate_env_overrides().is_ok());
}

/// The lint reports every broken override at once — the base lane can
/// see all unselected-environment breakage, not just the first.
#[test]
fn lint_env_overrides_collects_all_problems() {
    let mut schema = env_schema();
    let mut prod = IndexMap::new();
    prod.insert("ghost".to_string(), FieldOverride::default());
    schema
        .tables
        .get_mut("Item")
        .unwrap()
        .env_overrides
        .insert("prod".to_string(), prod);
    // A second break in another env, on another table.
    let mut dev = IndexMap::new();
    dev.insert("hp".to_string(), FieldOverride::default());
    schema
        .tables
        .get_mut("Item")
        .unwrap()
        .env_overrides
        .insert("dev".to_string(), dev);

    let problems = schema.lint_env_overrides();
    // `ghost` reports twice (unknown field AND empty patch — an empty
    // patch is also a break in its own right), `dev.hp` once.
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(
        problems.iter().any(|p| p.contains("unknown field 'ghost'")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("env_overrides[dev]")
            && p.contains(
                "sets no \
             constraint"
            )),
        "{problems:?}"
    );
    // validate still fails on the first one (the hard path).
    assert!(schema.validate_env_overrides().is_err());
}

#[test]
fn field_type_depth_measures_object_and_union_spines() {
    // Array/Map/object/union spines all measure their real depth and
    // saturate past the cap without recursing the caller's stack.
    let mut obj = IndexMap::new();
    obj.insert(
        "a".to_string(),
        FieldType::Array(Box::new(FieldType::Int32)),
    );
    obj.insert(
        "b".to_string(),
        FieldType::Map(MapField {
            key_type: MapKeyType::String,
            value_type: Box::new(FieldType::String),
        }),
    );
    let object = FieldType::Object(obj);
    let m = field_type_depth(&object).expect("object is a container");
    assert_eq!(m.depth, 2);
    assert!(!m.saturated);

    let mut branches = IndexMap::new();
    let mut leaf = IndexMap::new();
    leaf.insert("x".to_string(), FieldType::Int32);
    branches.insert("A".to_string(), leaf);
    let union = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "$t".to_string(),
        branches,
    }));
    let m = field_type_depth(&union).expect("union is a container");
    assert_eq!(m.depth, 1);
    assert!(!m.saturated);

    // A deeply nested array saturates the probe.
    let mut deep = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        deep = FieldType::Array(Box::new(deep));
    }
    let m = field_type_depth(&deep).expect("deep type");
    assert!(m.saturated);
    assert_eq!(m.depth, MAX_FIELD_TYPE_DEPTH + 1);
}

#[test]
fn from_de_refuses_object_and_union_over_cap() {
    // Object over the cap: the deserializer's own depth seed refuses
    // with the same message as arrays.
    let mut value = serde_json::json!({"kind": "Int32"});
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 2) {
        value = serde_json::json!({"kind": "Object", "value": {"f": value}});
    }
    assert!(
        serde_json::from_value::<FieldType>(value)
            .unwrap_err()
            .to_string()
            .contains("nesting depth"),
        "object over cap refused"
    );

    // TaggedUnion one-over-the-cap: refused too.
    let mut tu = serde_json::json!({"kind": "Int32"});
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 2) {
        tu = serde_json::json!({
            "kind": "TaggedUnion",
            "value": {"tag": "$t", "branches": {"B": {"f": tu}}}
        });
    }
    assert!(
        serde_json::from_value::<FieldType>(tu)
            .unwrap_err()
            .to_string()
            .contains("nesting depth"),
        "union over cap refused"
    );
}

#[test]
fn resolve_env_unknown_field_is_reported() {
    let mut schema = env_schema();
    let mut prod = IndexMap::new();
    prod.insert("ghost".to_string(), FieldOverride::default());
    schema
        .tables
        .get_mut("Item")
        .unwrap()
        .env_overrides
        .insert("prod".to_string(), prod);
    let err = schema.resolve_env("prod").unwrap_err();
    assert!(
        err.contains("unknown field 'ghost' in table 'Item'"),
        "{err}"
    );
}

#[test]
fn resolve_env_patches_fields_and_keeps_base_untouched() {
    let schema = env_schema();

    // base: hp required, no min
    let base = schema.tables["Item"].fields["hp"].clone();
    assert!(base.required);
    assert_eq!(base.min, None);

    // prod: hp optional with min 100 — same shape, tighter/looser rules
    let prod = schema.resolve_env("prod").unwrap();
    let hp = &prod.tables["Item"].fields["hp"];
    assert!(!hp.required, "prod loosens required");
    assert_eq!(hp.min, Some(100.0), "prod tightens min");
    assert!(
        prod.tables["Item"].fields["id"].required,
        "untouched field stays"
    );

    // Monster has no overrides: identical in both
    assert_eq!(
        schema.tables["Monster"].fields["drop_table"].required,
        prod.tables["Monster"].fields["drop_table"].required
    );

    // the base schema is never mutated
    assert!(schema.tables["Item"].fields["hp"].required);
    assert_eq!(schema.tables["Item"].fields["hp"].min, None);

    // an env with no overrides anywhere resolves to the base
    let dev = schema.resolve_env("dev").unwrap();
    assert_eq!(
        serde_yaml::to_string(&dev).unwrap(),
        serde_yaml::to_string(&schema).unwrap()
    );
}

#[test]
fn field_type_depth_saturates_every_container_spine() {
    // Map spine one-over-cap saturates.
    let mut deep = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        deep = FieldType::Map(MapField {
            key_type: MapKeyType::String,
            value_type: Box::new(deep),
        });
    }
    let m = field_type_depth(&deep).expect("map is a container");
    assert!(m.saturated, "{m:?}");

    // Object spine one-over-cap saturates.
    let mut deep = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        let mut fields = IndexMap::new();
        fields.insert("f".to_string(), deep);
        deep = FieldType::Object(fields);
    }
    let m = field_type_depth(&deep).expect("object is a container");
    assert!(m.saturated, "{m:?}");

    // TaggedUnion spine one-over-cap saturates.
    let mut deep = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        let mut branches = IndexMap::new();
        let mut branch = IndexMap::new();
        branch.insert("f".to_string(), deep);
        branches.insert("A".to_string(), branch);
        deep = FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "$t".to_string(),
            branches,
        }));
    }
    let m = field_type_depth(&deep).expect("union is a container");
    assert!(m.saturated, "{m:?}");

    // A shallow container that mixes saturated and unsaturated children
    // reports the saturated spine.
    let mut fields = IndexMap::new();
    fields.insert("leaf".to_string(), FieldType::Int32);
    let mut deep = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        deep = FieldType::Array(Box::new(deep));
    }
    fields.insert("deep".to_string(), deep);
    let m = field_type_depth(&FieldType::Object(fields)).expect("mixed object");
    assert!(m.saturated, "{m:?}");
}

#[test]
fn scalar_wire_maps_every_scalar_variant() {
    assert_eq!(
        scalar_wire(&FieldType::Null),
        serde_json::json!({"kind": "Null"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Bool),
        serde_json::json!({"kind": "Bool"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Int8),
        serde_json::json!({"kind": "Int8"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Int16),
        serde_json::json!({"kind": "Int16"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Int32),
        serde_json::json!({"kind": "Int32"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Int64),
        serde_json::json!({"kind": "Int64"})
    );
    assert_eq!(
        scalar_wire(&FieldType::UInt8),
        serde_json::json!({"kind": "UInt8"})
    );
    assert_eq!(
        scalar_wire(&FieldType::UInt16),
        serde_json::json!({"kind": "UInt16"})
    );
    assert_eq!(
        scalar_wire(&FieldType::UInt32),
        serde_json::json!({"kind": "UInt32"})
    );
    assert_eq!(
        scalar_wire(&FieldType::UInt64),
        serde_json::json!({"kind": "UInt64"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Float32),
        serde_json::json!({"kind": "Float32"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Float64),
        serde_json::json!({"kind": "Float64"})
    );
    assert_eq!(
        scalar_wire(&FieldType::String),
        serde_json::json!({"kind": "String"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Bytes),
        serde_json::json!({"kind": "Bytes"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Any),
        serde_json::json!({"kind": "Any"})
    );
    assert_eq!(
        scalar_wire(&FieldType::Enum("Rarity".to_string())),
        serde_json::json!({"kind": "Enum", "value": "Rarity"})
    );
}

#[test]
fn every_field_type_variant_clone_eq_debug_round_trips() {
    let variants = [
        FieldType::Null,
        FieldType::Bool,
        FieldType::Int8,
        FieldType::Int16,
        FieldType::Int32,
        FieldType::Int64,
        FieldType::UInt8,
        FieldType::UInt16,
        FieldType::UInt32,
        FieldType::UInt64,
        FieldType::Float32,
        FieldType::Float64,
        FieldType::String,
        FieldType::Bytes,
        FieldType::Any,
        FieldType::Enum("Rarity".to_string()),
        FieldType::Array(Box::new(FieldType::Int32)),
        FieldType::Map(MapField {
            key_type: MapKeyType::String,
            value_type: Box::new(FieldType::Int32),
        }),
        FieldType::Object({
            let mut f = IndexMap::new();
            f.insert("x".to_string(), FieldType::Int32);
            f
        }),
        FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "$t".to_string(),
            branches: {
                let mut b = IndexMap::new();
                let mut br = IndexMap::new();
                br.insert("x".to_string(), FieldType::Int32);
                b.insert("A".to_string(), br);
                b
            },
        })),
    ];
    for v in &variants {
        let cloned = v.clone();
        assert_eq!(v, &cloned, "clone/eq for {v:?}");
        let dbg = format!("{v:?}");
        assert!(!dbg.is_empty(), "debug renders");
        let ser = serde_json::to_value(v).expect("serialize");
        let back: FieldType = serde_json::from_value(ser).expect("deserialize");
        assert_eq!(v, &back, "serde round-trip for {v:?}");
    }
}

/// Every `FieldOverride` constraint key is applied by `resolve_env`:
/// the full override surface (`min_length`, `max_length`, `pattern`,
/// `enum_values`, `min_items`, `max_items`) must land on the resolved field.
#[test]
fn resolve_env_applies_every_override_constraint() {
    let mut schema = env_schema();
    let prod = IndexMap::from([(
        "hp".to_string(),
        FieldOverride {
            required: Some(false),
            min: Some(1.0),
            max: Some(999.0),
            min_length: Some(2),
            max_length: Some(8),
            pattern: Some(r"^[A-Z]".to_string()),
            enum_values: Some(vec!["a".to_string(), "b".to_string()]),
            min_items: Some(1),
            max_items: Some(3),
        },
    )]);
    schema
        .tables
        .get_mut("Item")
        .unwrap()
        .env_overrides
        .insert("prod".to_string(), prod);
    let resolved = schema.resolve_env("prod").expect("resolves");
    let hp = &resolved.tables["Item"].fields["hp"];
    assert!(!hp.required);
    assert_eq!(hp.min, Some(1.0));
    assert_eq!(hp.max, Some(999.0));
    assert_eq!(hp.min_length, Some(2));
    assert_eq!(hp.max_length, Some(8));
    assert_eq!(hp.pattern.as_deref(), Some(r"^[A-Z]"));
    assert_eq!(
        hp.enum_values.as_deref(),
        Some(&["a".to_string(), "b".to_string()][..])
    );
    assert_eq!(hp.min_items, Some(1));
    assert_eq!(hp.max_items, Some(3));
}

/// The iterative `PartialEq` must report the false branches of the
/// container frames: enum name mismatch, map key mismatch, object size /
/// missing-name mismatch, union tag / branch-size / missing-branch and
/// missing-field mismatches, and the cross-kind arm.
#[test]
fn field_type_eq_reports_container_mismatches() {
    let arr = |t: FieldType| FieldType::Array(Box::new(t));
    // Enum name mismatch.
    assert_ne!(FieldType::Enum("A".into()), FieldType::Enum("B".into()));
    // Map key mismatch.
    assert_ne!(
        map_type(MapKeyType::String, FieldType::Int32),
        map_type(MapKeyType::Int, FieldType::Int32)
    );
    // Map value mismatch.
    assert_ne!(
        map_type(MapKeyType::String, FieldType::Int32),
        map_type(MapKeyType::String, FieldType::String)
    );
    // Object size mismatch.
    assert_ne!(
        FieldType::Object(IndexMap::new()),
        FieldType::Object(IndexMap::from([("x".into(), FieldType::Int32)]))
    );
    // Object missing key.
    assert_ne!(
        FieldType::Object(IndexMap::from([
            ("x".into(), FieldType::Int32),
            ("y".into(), FieldType::Int32)
        ])),
        FieldType::Object(IndexMap::from([("x".into(), FieldType::Int32)]))
    );
    // Union tag mismatch.
    assert_ne!(
        FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "a".into(),
            branches: IndexMap::new(),
        })),
        FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "b".into(),
            branches: IndexMap::new(),
        })),
    );
    // Union branch-size mismatch.
    assert_ne!(
        FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "a".into(),
            branches: IndexMap::from([("A".into(), IndexMap::new())]),
        })),
        FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "a".into(),
            branches: IndexMap::new(),
        })),
    );
    // Union missing branch / missing field inside a branch.
    let union = |branches: IndexMap<String, IndexMap<String, FieldType>>| {
        FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "a".into(),
            branches,
        }))
    };
    assert_ne!(
        union(IndexMap::from([("A".into(), IndexMap::new())])),
        union(IndexMap::from([("B".into(), IndexMap::new())]))
    );
    assert_ne!(
        union(IndexMap::from([(
            "A".into(),
            IndexMap::from([("x".into(), FieldType::Int32)])
        )])),
        union(IndexMap::from([("A".into(), IndexMap::new())]))
    );
    assert_ne!(
        union(IndexMap::from([(
            "A".into(),
            IndexMap::from([("x".into(), FieldType::Int32)])
        )])),
        union(IndexMap::from([(
            "A".into(),
            IndexMap::from([("y".into(), FieldType::Int32)])
        )]))
    );
    // Cross-kind mismatch.
    assert_ne!(FieldType::Null, FieldType::Bool);
    assert_ne!(arr(FieldType::Int32), FieldType::Int32);
}

/// `rust_type` names the tagged-union wire shape and `field_type_depth`
/// reports the union/map spines exactly; `scalar_wire` panics only for a
/// container (the serializer never sends one — the contract is internal).
#[test]
fn rust_type_and_depth_cover_union_and_map_spines() {
    let tu = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "$t".into(),
        branches: IndexMap::new(),
    }));
    assert_eq!(tu.rust_type(), "IndexMap<String, serde_json::Value>");

    let m = field_type_depth(&map_type(MapKeyType::String, FieldType::Int32)).expect("map");
    assert_eq!(m.depth, 1);
    assert!(!m.saturated);

    let tu_m = field_type_depth(&tu).expect("union");
    assert_eq!(tu_m.depth, 1);
    assert!(!tu_m.saturated);

    // A map spine past the cap saturates.
    let mut deep = FieldType::Int32;
    for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
        deep = map_type(MapKeyType::String, deep);
    }
    let m = field_type_depth(&deep).expect("deep map");
    assert!(m.saturated);

    // scalar_wire refuses a container with the documented internal panic.
    let panic =
        std::panic::catch_unwind(|| scalar_wire(&FieldType::Array(Box::new(FieldType::Int32))));
    assert!(
        panic.is_err(),
        "container variants must not reach scalar_wire"
    );
}

/// `from_de` enforces the nesting cap on every container family
/// independently — a hand-built chain one past the cap is refused by
/// Array, Map, Object and `TaggedUnion` alike (the parse guard is a
/// separate layer; this is the conversion's own contract).
#[test]
fn from_de_enforces_the_cap_on_every_container_independently() {
    let leaf = || BoundedFieldTypeDe(FieldTypeDe::Int32);
    let chains: Vec<FieldTypeDe> = vec![
        {
            let mut v = leaf();
            for _ in 0..=MAX_FIELD_TYPE_DEPTH {
                v = BoundedFieldTypeDe(FieldTypeDe::Array(Box::new(v)));
            }
            v.0
        },
        {
            let mut v = leaf();
            for _ in 0..=MAX_FIELD_TYPE_DEPTH {
                v = BoundedFieldTypeDe(FieldTypeDe::Map(MapFieldDe {
                    key_type: MapKeyType::String,
                    value_type: Box::new(v),
                }));
            }
            v.0
        },
        {
            let mut v = leaf();
            for _ in 0..=MAX_FIELD_TYPE_DEPTH {
                let mut fields = IndexMap::new();
                fields.insert("f".to_string(), v);
                v = BoundedFieldTypeDe(FieldTypeDe::Object(fields));
            }
            v.0
        },
        {
            let mut v = leaf();
            for _ in 0..=MAX_FIELD_TYPE_DEPTH {
                let mut branch = IndexMap::new();
                branch.insert("f".to_string(), v);
                let mut branches = IndexMap::new();
                branches.insert("B".to_string(), branch);
                v = BoundedFieldTypeDe(FieldTypeDe::TaggedUnion(Box::new(TaggedUnionDe {
                    tag: "$t".to_string(),
                    branches,
                })));
            }
            v.0
        },
    ];
    for chain in chains {
        let err = FieldType::from_de(chain, 0).expect_err("over-cap refused");
        assert!(err.contains("nesting depth"), "{err}");
    }
}

/// The iterative `Debug` fallback renders every scalar leaf and folds
/// multi-entry objects and unions with the `", "` separator — the arms
/// the derived builder would never reach on a type past the cap.
#[test]
fn field_type_debug_fallback_renders_scalars_and_separators() {
    let deep_with = |leaf: FieldType| {
        let mut t = leaf;
        for _ in 0..MAX_FIELD_TYPE_DEPTH + 5 {
            t = FieldType::Array(Box::new(t));
        }
        t
    };
    let scalars: Vec<(FieldType, &str)> = vec![
        (FieldType::Null, "Null"),
        (FieldType::Bool, "Bool"),
        (FieldType::Int8, "Int8"),
        (FieldType::Int16, "Int16"),
        (FieldType::Int32, "Int32"),
        (FieldType::Int64, "Int64"),
        (FieldType::UInt8, "UInt8"),
        (FieldType::UInt16, "UInt16"),
        (FieldType::UInt32, "UInt32"),
        (FieldType::UInt64, "UInt64"),
        (FieldType::Float32, "Float32"),
        (FieldType::Float64, "Float64"),
        (FieldType::String, "String"),
        (FieldType::Bytes, "Bytes"),
        (FieldType::Any, "Any"),
    ];
    for (leaf, name) in scalars {
        let rendered = format!("{:?}", deep_with(leaf));
        assert!(rendered.contains("Array("), "{name}: {rendered}");
        assert!(rendered.contains(name), "{rendered}");
    }

    // Two-entry maps and unions hit the separator line.
    let mut obj = IndexMap::new();
    obj.insert("a".to_string(), FieldType::Int32);
    obj.insert("b".to_string(), FieldType::Bool);
    let obj_deep = deep_with(FieldType::Object(obj));
    let rendered = format!("{obj_deep:?}");
    assert!(
        rendered.contains("{\"a\": Int32, \"b\": Bool}"),
        "{rendered}"
    );

    let mut leaf = IndexMap::new();
    leaf.insert("x".to_string(), FieldType::Int32);
    let mut leaf2 = IndexMap::new();
    leaf2.insert("y".to_string(), FieldType::Bool);
    let mut branches = IndexMap::new();
    branches.insert("A".to_string(), leaf);
    branches.insert("B".to_string(), leaf2);
    let union = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "$t".to_string(),
        branches,
    }));
    let rendered = format!("{:?}", deep_with(union));
    assert!(
        rendered.contains("\"A\": {\"x\": Int32}, \"B\": {\"y\": Bool}"),
        "{rendered}"
    );
}

/// Structural equality refuses an object whose key sets differ at the
/// same length — the missing-key arm of the iterative walk.
#[test]
fn field_type_eq_rejects_an_object_missing_a_key() {
    let mut left = IndexMap::new();
    left.insert("a".to_string(), FieldType::Int32);
    left.insert("b".to_string(), FieldType::Bool);
    let mut right = IndexMap::new();
    right.insert("a".to_string(), FieldType::Int32);
    right.insert("c".to_string(), FieldType::Bool);
    assert_ne!(
        FieldType::Object(left),
        FieldType::Object(right),
        "same length, different keys"
    );
}

/// A reference to a table's nonexistent field is reported (the
/// unknown-table arm already is).
#[test]
fn validate_reports_a_reference_to_an_unknown_field() {
    let field = |name: &str, t: FieldType, reference: Option<ReferenceSchema>| FieldSchema {
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
        reference,
        targets: Vec::new(),
        rules: Vec::new(),
        metadata: IndexMap::new(),
    };
    let table = |name: &str, pk: &str, fields: IndexMap<String, FieldSchema>| TableSchema {
        name: name.to_string(),
        description: None,
        primary_key: vec![pk.to_string()],
        fields,
        unique_constraints: Vec::new(),
        order_by: None,
        targets: Vec::new(),
        env_overrides: IndexMap::new(),
    };
    let mut owner = IndexMap::new();
    owner.insert("id".to_string(), field("id", FieldType::Int32, None));
    let mut ref_fields = IndexMap::new();
    ref_fields.insert(
        "owner".to_string(),
        field(
            "owner",
            FieldType::Int32,
            Some(ReferenceSchema {
                table: "Item".to_string(),
                field: "ghost".to_string(),
                predicate: None,
                cardinality: "one".to_string(),
                compatible_with: None,
            }),
        ),
    );
    let mut schema = Schema::new();
    schema
        .tables
        .insert("Item".to_string(), table("Item", "id", owner));
    schema
        .tables
        .insert("Owner".to_string(), table("Owner", "owner", ref_fields));
    let diags = schema.validate();
    assert!(
        diags
            .iter()
            .any(|d| d.code == "E1004"
                && d.message.contains("unknown field 'ghost' in table 'Item'")),
        "reference to a missing field reported: {diags:?}"
    );
}

/// The depth probe measures a union branch nested past its immediate
/// leaf — the branch walk contributes its real depth to the union's
/// measure without saturating.
#[test]
fn field_type_depth_measures_a_deep_union_branch() {
    let mut branch = IndexMap::new();
    branch.insert(
        "x".to_string(),
        FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Int32)))),
    );
    let mut branches = IndexMap::new();
    branches.insert("A".to_string(), branch);
    let union = FieldType::TaggedUnion(Box::new(TaggedUnion {
        tag: "$t".to_string(),
        branches,
    }));
    let m = field_type_depth(&union).expect("union is a container");
    assert_eq!(m.depth, 3);
    assert!(!m.saturated);
}
