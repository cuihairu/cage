use super::*;

use super::idents::sanitize_ident;
use super::literals::py_float_literal;
use cage_core::schema::{FieldType, MapField, MapKeyType};
use tera::Tera;

#[test]
fn filter_library_renders_type_and_default() {
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      price: { name: price, type: { kind: Int32 }, default: 10 }
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
",
    )
    .unwrap();

    let mut tera = Tera::default();
    register_filters(&mut tera, &schema);
    let mut ctx = tera::Context::new();
    ctx.insert("tables", &schema.tables);
    let out = tera
            .render_str(
                "{{ tables.Item.fields.price | py_type }}|{{ tables.Item.fields.price | py_default }}|{{ tables.Item.fields.kind | py_type }}",
                &ctx,
            )
            .unwrap();
    assert_eq!(out, "int|10|ItemKind");
}

fn test_schema() -> Schema {
    serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    description: Equipment definitions.
    primary_key: [id]
    fields:
      name: { name: name, type: { kind: String }, required: true, description: Display name }
      id: { name: id, type: { kind: Int32 }, required: true, description: Identifier }
      price: { name: price, type: { kind: Int32 }, min: 0, description: Price in gold }
      weight: { name: weight, type: { kind: Float64 }, default: 1.5 }
      note: { name: note, type: { kind: String } }
      tags: { name: tags, type: { kind: Array, value: { kind: String } }, default: [pvp] }
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
      rarity: { name: rarity, type: { kind: String }, enum_values: [common, rare] }
      owner: { name: owner, type: { kind: String }, reference: { table: Player, field: id } }
  Drop:
    name: Drop
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int64 }, required: true }
      item: { name: item, type: { kind: Enum, value: MissingEnum } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1, description: Sword weapon }
      - { name: Shield, value: 2 }
  Rarity:
    name: Rarity
    values:
      - { name: common }
      - { name: rare }
  EmptyEnum:
    name: EmptyEnum
    values: []
",
    )
    .expect("test schema must parse")
}

/// Schema exercising paths the main fixture misses: an empty table,
/// Any/Null/Object/Bytes annotations, both Bool default branches,
/// length/pattern constraints, an enum description, and a non-integral
/// enum with string/number/bool members.
fn edge_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Ghost:
    name: Ghost
    primary_key: []
    fields: {}
  Edge:
    name: Edge
    primary_key: [key]
    fields:
      key: { name: key, type: { kind: String }, required: true }
      flag: { name: flag, type: { kind: Bool }, default: true }
      off: { name: off, type: { kind: Bool }, default: false }
      blob: { name: blob, type: { kind: Bytes } }
      meta: { name: meta, type: { kind: Object, value: {} } }
      wild: { name: wild, type: { kind: Any } }
      alist: { name: alist, type: { kind: Array, value: { kind: Null } } }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 8, pattern: '^[A-Z]+$' }
      count: { name: count, type: { kind: Int32 }, max: 100, description: Bounded count }
      kind: { name: kind, type: { kind: Enum, value: Described } }
      big: { name: big, type: { kind: UInt64 }, default: 18446744073709551615 }
      frac: { name: frac, type: { kind: Int64 }, default: 1.5 }
enums:
  Described:
    name: Described
    description: A described enum.
    values:
      - { name: First, value: 1, description: First member }
      - { name: Second, value: 2 }
  Mixed:
    name: Mixed
    values:
      - { name: one, value: first, description: String member }
      - { name: two, value: 2.5 }
      - { name: yes, value: true }
      - { name: no, value: false }
      - { name: bare }
  Huge:
    name: Huge
    values:
      - { name: Max, value: 18446744073709551615 }
",
        )
        .expect("edge schema must parse")
}

/// Schema with no emitted enums at all: `generate()` must skip the shared
/// enums module entirely.
fn no_enum_schema() -> Schema {
    serde_yaml::from_str(
        r"
tables:
  Plain:
    name: Plain
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
    )
    .expect("schema must parse")
}

/// Table with only optional fields: the required-first (plain) group is
/// empty, so the defaulted group opens the dataclass body.
fn optional_only_schema() -> Schema {
    serde_yaml::from_str(
        r"
tables:
  Solo:
    name: Solo
    primary_key: []
    fields:
      note: { name: note, type: { kind: String } }
      weight: { name: weight, type: { kind: Float64 }, default: 1.5 }
enums: {}
",
    )
    .expect("schema must parse")
}

/// Schema exercising map fields: string/int keys, nested value types,
/// empty and non-empty defaults, mismatched members, non-numeric keys,
/// an Any value type, and an enum value type.
fn map_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Lookup:
    name: Lookup
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      perks: { name: perks, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, required: true }
      counts: { name: counts, type: { kind: Map, value: { key_type: int, value_type: { kind: Int32 } } }, required: true }
      nested: { name: nested, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } }, required: true }
      empty: { name: empty, type: { kind: Map, value: { key_type: string, value_type: { kind: String } } }, default: {} }
      bonus: { name: bonus, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: { hp: 10, mp: -4 } }
      ranked: { name: ranked, type: { kind: Map, value: { key_type: string, value_type: { kind: String } } }, default: { a: alpha, b: 2, c: [x] } }
      ints: { name: ints, type: { kind: Map, value: { key_type: int, value_type: { kind: String } } }, default: { '1': one, '-7': neg, oops: x } }
      spare: { name: spare, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } }
      tags_by_kind: { name: tags_by_kind, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: ItemKind } } } }
      wild: { name: wild, type: { kind: Map, value: { key_type: string, value_type: { kind: Any } } } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
      - { name: Shield, value: 2 }
",
        )
        .expect("map schema must parse")
}

fn gen() -> PyTargetGenerator {
    PyTargetGenerator::default()
}

#[test]
fn test_generate_artifact_paths() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // Tables in name order, shared enums module last.
    assert_eq!(
        paths,
        vec![
            "build/python/Drop.py",
            "build/python/Item.py",
            "build/python/cage_enums.py",
        ]
    );
}

#[test]
fn test_table_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let item = String::from_utf8(artifacts[1].1.clone()).unwrap();

    assert!(item.contains("#   schema: abc123"));
    assert!(item.contains("#   table:  Item"));
    assert!(item.contains("from __future__ import annotations"));
    assert!(item.contains("@dataclass(frozen=True)"));
    assert!(item.contains("class Item:"));
    assert!(item.contains("# Item — primary key: id"));
    assert!(item.contains("# Equipment definitions."));
    // Enum import for the referenced shared enum.
    assert!(item.contains("from cage_enums import ItemKind"));

    // Required fields without defaults carry no default and come FIRST
    // (dataclass ordering); defaulted fields follow in name order.
    let id_pos = item.find("    id: int\n").unwrap();
    let name_pos = item.find("    name: str\n").unwrap();
    let kind_pos = item.find("    kind: ItemKind | None = None").unwrap();
    let note_pos = item.find("    note: str | None = None").unwrap();
    let price_pos = item.find("    price: int | None = None").unwrap();
    let rarity_pos = item.find("    rarity: str | None = None").unwrap();
    let tags_pos = item
        .find("    tags: list[str] = field(default_factory=lambda: [\"pvp\"])")
        .unwrap();
    let weight_pos = item.find("    weight: float = 1.5").unwrap();
    assert!(id_pos < name_pos);
    assert!(name_pos < kind_pos);
    assert!(kind_pos < note_pos);
    assert!(note_pos < price_pos);
    assert!(price_pos < rarity_pos);
    assert!(rarity_pos < tags_pos);
    assert!(tags_pos < weight_pos);

    // Mutable (list) defaults go through default_factory.
    assert!(item.contains("from dataclasses import dataclass, field"));
    // Constraint docs.
    assert!(item.contains("# Price in gold, min: 0"));
    assert!(item.contains("allowed: common | rare"));
    assert!(item.contains("→ Player.id"));
}

#[test]
fn test_enums_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    assert!(enums_src.contains("from enum import IntEnum"));
    // Integral enum → IntEnum (unbounded ints, no backing concern).
    assert!(enums_src.contains("class ItemKind(IntEnum):"));
    assert!(enums_src.contains("    Sword = 1"));
    assert!(enums_src.contains("    # Sword weapon"));
    // String enum → plain class of constants.
    assert!(enums_src.contains("class Rarity:"));
    assert!(enums_src.contains("    common = \"common\""));
    // Empty enums are not emitted.
    assert!(!enums_src.contains("EmptyEnum"));
}

#[test]
fn test_unresolved_enum_falls_back_to_str() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, None);
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(drop_src.contains("#   schema: (unavailable)"));
    assert!(drop_src.contains("    item: str | None = None\n"));
    assert!(drop_src.contains("unresolved enum: MissingEnum"));
    // No enum import when nothing resolves.
    assert!(!drop_src.contains("from cage_enums"));
}

#[test]
fn test_empty_table_renders_pass_stub() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    // Name order: Edge, Ghost, then the shared enums module.
    assert!(artifacts[0].0.ends_with("Edge.py"));
    let ghost = String::from_utf8(artifacts[1].1.clone()).unwrap();
    // Empty primary key: the banner is just the table name.
    assert!(ghost.contains("# Ghost\n"));
    // No fields → the class body is a bare `pass`.
    assert!(ghost.contains("class Ghost:\n    pass\n"));
    assert!(!ghost.contains("from typing import Any"));
}

#[test]
fn test_any_bytes_object_and_bool_defaults() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let edge = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // An Any-ish annotation anywhere pulls in the typing import.
    assert!(edge.contains("from typing import Any"));
    // Type mapping: Any / Bytes / Object / Bool / Array-of-Null.
    assert!(edge.contains("    wild: Any | None = None"));
    assert!(edge.contains("    blob: bytes | None = None"));
    assert!(edge.contains("    meta: dict[str, Any] | None = None"));
    assert!(edge.contains("    flag: bool = True"));
    assert!(edge.contains("    off: bool = False"));
    assert!(edge.contains("    alist: list[Any] | None = None"));
    // Constraint summaries in the member docs.
    assert!(edge.contains("    # Bounded count, max: 100"));
    assert!(edge.contains("    # min_length: 1, max_length: 8, pattern: ^[A-Z]+$"));
    // Enum import for the referenced shared enum.
    assert!(edge.contains("from cage_enums import Described"));
    // u64 default above i64::MAX renders via the or_else arm; a float
    // default on an int field renders nothing (member falls back to None).
    assert!(edge.contains("    big: int = 18446744073709551615"));
    assert!(edge.contains("    frac: int | None = None"));
    assert!(!edge.contains("    frac: int = "));
}

#[test]
fn test_enum_description_and_mixed_string_bucket() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    // Enum-level description comment (integral enum).
    assert!(enums_src.contains("# Described — A described enum."));
    assert!(enums_src.contains("class Described(IntEnum):"));
    assert!(enums_src.contains("    # First member"));
    assert!(enums_src.contains("    First = 1"));

    // Non-integral enum → plain class; String/Number/Bool member
    // values stringify through the bucket match.
    assert!(enums_src.contains("class Mixed:"));
    assert!(enums_src.contains("    # String member"));
    assert!(enums_src.contains("    one = \"first\""));
    assert!(enums_src.contains("    two = \"2.5\""));
    assert!(enums_src.contains("    yes = \"true\""));
    assert!(enums_src.contains("    no = \"false\""));
    assert!(enums_src.contains("    bare = \"bare\""));

    // u64 above i64::MAX stays in the integral (IntEnum) bucket.
    assert!(enums_src.contains("class Huge(IntEnum):"));
    assert!(enums_src.contains("    Max = 18446744073709551615"));
}

#[test]
fn test_from_config_without_enum_options() {
    // options: None — the option block never opens.
    let config: TargetConfig =
        serde_yaml::from_str("format: python\noutput_dir: build/py").expect("target config");
    let gen = PyTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/py"));
    assert_eq!(gen.file_template, "{table}.py");
    assert_eq!(gen.enums_file, "cage_enums.py");

    // Options present but no enums_file key — the inner arm is skipped.
    let config: TargetConfig =
        serde_yaml::from_str("format: python\noutput_dir: out\noptions:\n  other: 1")
            .expect("target config");
    let gen = PyTargetGenerator::from_config(&config);
    assert_eq!(gen.enums_file, "cage_enums.py");

    // Non-string enums_file value — as_str() fails, default kept.
    let config: TargetConfig =
        serde_yaml::from_str("format: python\noutput_dir: out\noptions:\n  enums_file: 42")
            .expect("target config");
    let gen = PyTargetGenerator::from_config(&config);
    assert_eq!(gen.enums_file, "cage_enums.py");
}

#[test]
fn test_schema_without_enums_emits_no_enums_file() {
    let artifacts = gen().generate(&no_enum_schema(), Some("abc123"));
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].0.ends_with("Plain.py"));
    let src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(!src.contains("from cage_enums"));
}

#[test]
fn test_optional_only_table_skips_plain_group() {
    let artifacts = gen().generate(&optional_only_schema(), Some("abc123"));
    let solo = String::from_utf8(artifacts[0].1.clone()).unwrap();
    // No required members: the class opens with the defaulted group
    // (blank line only between members, none before the first).
    assert!(solo.contains("class Solo:"));
    assert!(solo.contains("    note: str | None = None"));
    assert!(solo.contains("    weight: float = 1.5"));
    assert!(!solo.contains("    id:"));
}

#[test]
fn test_map_type_annotations() {
    let artifacts = gen().generate(&map_schema(), Some("abc123"));
    let lookup = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // Key/value spelling: string keys, int keys, nested value types.
    assert!(lookup.contains("    perks: dict[str, list[int]]\n"));
    assert!(lookup.contains("    counts: dict[int, int]\n"));
    assert!(lookup.contains("    nested: dict[str, dict[str, int]]\n"));
    // Optional map without default → nullable annotation (shared rule).
    assert!(lookup.contains("    spare: dict[str, int] | None = None"));
    // Enum-valued map: annotation names the shared enum and imports it.
    assert!(lookup.contains("    tags_by_kind: dict[str, ItemKind] | None = None"));
    assert!(lookup.contains("from cage_enums import ItemKind"));
    // Any-valued map pulls the typing import through the Map value.
    assert!(lookup.contains("    wild: dict[str, Any] | None = None"));
    assert!(lookup.contains("from typing import Any"));
    // Required maps without defaults sit in the plain (first) group.
    let counts_pos = lookup.find("    counts: dict[int, int]\n").unwrap();
    let bonus_pos = lookup.find("    bonus:").unwrap();
    assert!(counts_pos < bonus_pos);
}

#[test]
fn test_map_defaults() {
    let artifacts = gen().generate(&map_schema(), Some("abc123"));
    let lookup = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // Empty map default → the `dict` constructor (a fresh object per
    // instance); non-empty defaults render literal entries and also go
    // through default_factory so instances never share the dict.
    assert!(lookup.contains("    empty: dict[str, str] = field(default_factory=dict)"));
    assert!(lookup.contains(
        "    bonus: dict[str, int] = field(default_factory=lambda: {\"hp\": 10, \"mp\": -4})"
    ));
    // Mismatched member kinds are skipped, not fatal (unlike arrays).
    assert!(lookup.contains(
        "    ranked: dict[str, str] = field(default_factory=lambda: {\"a\": \"alpha\"})"
    ));
    // Integer keys render as int literals; non-numeric keys are skipped.
    assert!(lookup.contains(
        "    ints: dict[int, str] = field(default_factory=lambda: {-7: \"neg\", 1: \"one\"})"
    ));
    // A rendered map default pulls the `field` import.
    assert!(lookup.contains("from dataclasses import dataclass, field"));
}

#[test]
fn test_render_default_map_edge_cases() {
    let schema = Schema::new();
    let map = |key_type: MapKeyType, value_type: FieldType| {
        FieldType::Map(MapField {
            key_type,
            value_type: Box::new(value_type),
        })
    };
    // Non-object default on a map field → None (member falls back to
    // `| None = None`).
    assert!(render_default(
        &serde_json::json!([1]),
        &map(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
    // Every value unrenderable → empty literal (entries skipped).
    assert_eq!(
        render_default(
            &serde_json::json!({"a": {"x": 1}}),
            &map(MapKeyType::String, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "{}"
    );
    // A non-numeric key under an int key type is skipped entry-wise.
    assert_eq!(
        render_default(
            &serde_json::json!({"oops": 1}),
            &map(MapKeyType::Int, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "{}"
    );
    // Value recursion follows the existing rules: array-of-scalar
    // members render through the Array arm.
    assert_eq!(
        render_default(
            &serde_json::json!({"a": [1, 2]}),
            &map(
                MapKeyType::String,
                FieldType::Array(Box::new(FieldType::Int32))
            ),
            &schema
        )
        .unwrap(),
        "{\"a\": [1, 2]}"
    );
    // Enum-valued members render through the Enum arm (None: no
    // literal mapping), sinking just that entry.
    assert_eq!(
        render_default(
            &serde_json::json!({"a": 1}),
            &map(MapKeyType::String, FieldType::Enum("Missing".to_string())),
            &schema
        )
        .unwrap(),
        "{}"
    );
}

#[test]
fn test_deterministic_output() {
    let schema = test_schema();
    let a = gen().generate(&schema, Some("abc123"));
    let b = gen().generate(&schema, Some("abc123"));
    assert_eq!(a.len(), b.len());
    for ((pa, ca), (pb, cb)) in a.iter().zip(b.iter()) {
        assert_eq!(pa, pb);
        assert_eq!(ca, cb);
    }
}

#[test]
fn test_from_config_options() {
    let config: TargetConfig = serde_yaml::from_str(
        r#"
format: python
output_dir: build/py
file_template: "{table}_gen.py"
options:
  enums_file: shared_enums.py
"#,
    )
    .expect("target config");
    let gen = PyTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/py"));
    assert_eq!(gen.file_template, "{table}_gen.py");
    assert_eq!(gen.enums_file, "shared_enums.py");
}

#[test]
fn test_ident_sanitization() {
    assert_eq!(sanitize_ident("Item"), "Item");
    assert_eq!(sanitize_ident("drop-item"), "drop_item");
    assert_eq!(sanitize_ident("1st"), "_1st");
    assert_eq!(sanitize_ident("列"), "_");
    assert_eq!(sanitize_ident(""), "_");
    // PEP 8: keyword collision gets a trailing underscore.
    assert_eq!(py_ident("class"), "class_");
    assert_eq!(py_ident("import"), "import_");
    assert_eq!(py_ident("name"), "name");
}

#[test]
fn test_field_collisions_get_unique_members() {
    let mut used = HashSet::new();
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b_");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b__");
}

#[test]
fn test_render_default_edge_cases() {
    let schema = Schema::new();
    // Float literals always keep a decimal point.
    assert_eq!(
        render_default(&serde_json::json!(100.0), &FieldType::Float64, &schema).unwrap(),
        "100.0"
    );
    // Kind mismatch → None (field defaults to None instead).
    assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
    // Non-finite floats map to float() constructor calls.
    assert_eq!(py_float_literal(f64::NAN), "float(\"nan\")");
    assert_eq!(py_float_literal(f64::NEG_INFINITY), "float(\"-inf\")");
    // Object defaults are not rendered (shared rule across targets).
    assert!(render_default(
        &serde_json::json!({"a": 1}),
        &FieldType::Object(indexmap::IndexMap::default()),
        &schema
    )
    .is_none());
    // Bool defaults render both branches.
    assert_eq!(
        render_default(&serde_json::json!(true), &FieldType::Bool, &schema).unwrap(),
        "True"
    );
    assert_eq!(
        render_default(&serde_json::json!(false), &FieldType::Bool, &schema).unwrap(),
        "False"
    );
    // Positive infinity has its own constructor spelling.
    assert_eq!(py_float_literal(f64::INFINITY), "float(\"inf\")");
    // A non-scalar element type sinks the whole array default.
    assert!(render_default(
        &serde_json::json!([{"a": 1}]),
        &FieldType::Array(Box::new(FieldType::Object(indexmap::IndexMap::default()))),
        &schema
    )
    .is_none());
    // Scalar element types still render.
    assert_eq!(
        render_default(
            &serde_json::json!([1, 2]),
            &FieldType::Array(Box::new(FieldType::Int32)),
            &schema
        )
        .unwrap(),
        "[1, 2]"
    );
    // One unrenderable element sinks the whole array (`?` early exit).
    assert!(render_default(
        &serde_json::json!([1, "x"]),
        &FieldType::Array(Box::new(FieldType::Int32)),
        &schema
    )
    .is_none());
    // Escapes: control characters, newlines, quote and backslash.
    assert_eq!(py_string_literal("a\u{1}b"), "\"a\\x01b\"");
    assert_eq!(py_string_literal("a\nb\r\tc"), "\"a\\nb\\r\\tc\"");
    assert_eq!(py_string_literal("q\"\\q"), "\"q\\\"\\\\q\"");
}
