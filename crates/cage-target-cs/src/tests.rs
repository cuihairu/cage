use super::*;

use super::idents::sanitize_ident;
use super::literals::cs_float_literal;
use cage_core::schema::{FieldType, MapField, MapKeyType};
use std::collections::HashSet;
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
                "{{ tables.Item.fields.price | cs_type }}|{{ tables.Item.fields.price | cs_default }}|{{ tables.Item.fields.kind | cs_type }}",
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

/// Schema exercising paths the main fixture misses: an empty table
/// without a primary key, every rarely-used scalar type, required
/// byte[]/list/dict reference initializers, both Bool default branches,
/// length/pattern constraints, integral enums with negative / >i32 / >i64
/// backings, an enum description, and a non-integral string-bucket enum.
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
      anything: { name: anything, type: { kind: Any } }
      i8: { name: i8, type: { kind: Int8 } }
      i16: { name: i16, type: { kind: Int16 } }
      u8: { name: u8, type: { kind: UInt8 } }
      u16: { name: u16, type: { kind: UInt16 } }
      u32: { name: u32, type: { kind: UInt32 } }
      u64: { name: u64, type: { kind: UInt64 } }
      f32: { name: f32, type: { kind: Float32 } }
      blob: { name: blob, type: { kind: Bytes }, required: true }
      meta: { name: meta, type: { kind: Object, value: {} }, required: true }
      tags: { name: tags, type: { kind: Array, value: { kind: String } }, required: true }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 8, pattern: '^[A-Z]+$' }
      count: { name: count, type: { kind: Int32 }, max: 100, description: Bounded count }
      big: { name: big, type: { kind: UInt64 }, default: 18446744073709551615 }
      frac: { name: frac, type: { kind: Int64 }, default: 1.5 }
      mix: { name: mix, type: { kind: Array, value: { kind: Int32 } }, default: [1, 'x'] }
enums:
  Signed:
    name: Signed
    values:
      - { name: Low, value: -5 }
      - { name: High, value: 3 }
  Wide:
    name: Wide
    description: Wide-backed enum.
    values:
      - { name: Big, value: 4294967296 }
  Huge:
    name: Huge
    values:
      - { name: Max, value: 18446744073709551615 }
  Mixed:
    name: Mixed
    values:
      - { name: one, value: first, description: String member }
      - { name: two, value: 2.5 }
      - { name: yes, value: true }
      - { name: no, value: false }
      - { name: bare }
",
        )
        .expect("edge schema must parse")
}

/// Schema exercising `FieldType::Map`: string/int keys, nested and
/// enum values, empty / scalar / dropped defaults, nullability.
fn map_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Mapped:
    name: Mapped
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      scores: { name: scores, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, required: true }
      names: { name: names, type: { kind: Map, value: { key_type: int, value_type: { kind: String } } } }
      nested: { name: nested, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } } }
      empty: { name: empty, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: {} }
      counts: { name: counts, type: { kind: Map, value: { key_type: string, value_type: { kind: Int64 } } }, default: { a: 1, b: -2 } }
      quotas: { name: quotas, type: { kind: Map, value: { key_type: int, value_type: { kind: Int32 } } }, default: { '1': 10, '-2': 20 } }
      kinds: { name: kinds, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: ItemKind } } } }
      lists: { name: lists, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, default: { a: [1] } }
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

fn gen() -> CsTargetGenerator {
    CsTargetGenerator::default()
}

#[test]
fn test_generate_artifact_paths() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // Tables in name order, shared enums file last.
    assert_eq!(
        paths,
        vec![
            "build/csharp/Drop.cs",
            "build/csharp/Item.cs",
            "build/csharp/CageEnums.cs",
        ]
    );
}

#[test]
fn test_table_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let item = &artifacts[1].1;
    let src = String::from_utf8(item.clone()).unwrap();

    assert!(src.contains("//   schema: abc123"));
    assert!(src.contains("//   table:  Item"));
    assert!(src.contains("#nullable enable"));
    assert!(src.contains("namespace Cage.Generated"));
    assert!(src.contains(
            "/// <summary>\n    /// Item — primary key: id\n    /// <para>Equipment definitions.</para>"
        ));

    // Fields in name order (sort, not schema insertion order).
    let id_pos = src.find("public int id").unwrap();
    let kind_pos = src.find("public ItemKind? kind").unwrap();
    let name_pos = src.find("public string name").unwrap();
    let note_pos = src.find("public string? note").unwrap();
    let price_pos = src.find("public int? price").unwrap();
    let rarity_pos = src.find("public string? rarity").unwrap();
    let tags_pos = src.find("public IReadOnlyList<string> tags").unwrap();
    let weight_pos = src.find("public double weight").unwrap();
    assert!(id_pos < kind_pos && kind_pos < name_pos && name_pos < note_pos);
    assert!(note_pos < price_pos && price_pos < rarity_pos);
    assert!(rarity_pos < tags_pos && tags_pos < weight_pos);

    // Required reference gets an empty initializer; optional stays nullable.
    assert!(src.contains("public string name { get; init; } = string.Empty;"));
    assert!(src.contains("public string? note { get; init; }"));
    assert!(src.contains("public int id { get; init; }"));
    // Rendered defaults become initializers.
    assert!(src.contains("public double weight { get; init; } = 1.5;"));
    assert!(src
        .contains("public IReadOnlyList<string> tags { get; init; } = new string[] { \"pvp\" };"));
    // Constraint docs.
    assert!(src.contains("Price in gold, min: 0"));
    assert!(src.contains("allowed: common | rare"));
    assert!(src.contains("→ Player.id"));
    // Required value type stays non-nullable without initializer.
    assert!(src.contains("public int id { get; init; }\n"));
}

#[test]
fn test_enums_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    // Numeric-backed enum keeps default (int) backing: values fit in int32.
    assert!(enums_src.contains("public enum ItemKind"));
    assert!(!enums_src.contains("public enum ItemKind :"));
    assert!(enums_src.contains("Sword = 1,"));
    assert!(enums_src.contains("/// <summary>Sword weapon</summary>\n        Sword = 1,"));
    // String-backed enum → static class of constants.
    assert!(enums_src.contains("public static class Rarity"));
    assert!(enums_src.contains("public const string common = \"common\";"));
    // Empty enums are not emitted (and referenced fields fall back).
    assert!(!enums_src.contains("EmptyEnum"));
}

#[test]
fn test_unresolved_enum_falls_back_to_string() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, None);
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(drop_src.contains("//   schema: (unavailable)"));
    assert!(drop_src.contains("public string? item { get; init; }"));
    assert!(drop_src.contains("unresolved enum: MissingEnum"));
}

#[test]
fn test_empty_primary_key_summary() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    // Name order: Edge, Ghost, then the shared enums file.
    assert!(artifacts[0].0.ends_with("Edge.cs"));
    let ghost = String::from_utf8(artifacts[1].1.clone()).unwrap();
    // Empty primary key → the summary is just the (escaped) name.
    assert!(ghost.contains("/// <summary>Ghost</summary>"));
    assert!(!ghost.contains("primary key"));
}

#[test]
fn test_integral_enum_backings() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    // Enum-level description in the summary.
    assert!(enums_src.contains("/// <summary>Wide — Wide-backed enum.</summary>"));
    // Backing type selection per value domain.
    assert!(enums_src.contains("public enum Signed : long")); // negative
    assert!(enums_src.contains("public enum Wide : long")); // > int.MaxValue
    assert!(enums_src.contains("public enum Huge : ulong")); // > long.MaxValue
    assert!(enums_src.contains("        Low = -5,"));
    assert!(enums_src.contains("        Big = 4294967296,"));
    assert!(enums_src.contains("        Max = 18446744073709551615,"));
}

#[test]
fn test_static_class_string_bucket() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    // Non-integral members → static class of string constants.
    assert!(enums_src.contains("public static class Mixed"));
    // Member description lands in its own summary line.
    assert!(enums_src.contains("/// <summary>String member</summary>"));
    // String / Number / Bool / value-less arms.
    assert!(enums_src.contains("public const string one = \"first\";"));
    assert!(enums_src.contains("public const string two = \"2.5\";"));
    assert!(enums_src.contains("public const string yes = \"true\";"));
    assert!(enums_src.contains("public const string no = \"false\";"));
    assert!(enums_src.contains("public const string bare = \"bare\";"));
}

#[test]
fn test_edge_table_types_and_ref_inits() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let edge = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // Type mapping for the rarely-used scalar kinds.
    assert!(edge.contains("public object? anything { get; init; }"));
    assert!(edge.contains("public sbyte? i8 { get; init; }"));
    assert!(edge.contains("public short? i16 { get; init; }"));
    assert!(edge.contains("public byte? u8 { get; init; }"));
    assert!(edge.contains("public ushort? u16 { get; init; }"));
    assert!(edge.contains("public uint? u32 { get; init; }"));
    assert!(edge.contains("public ulong? u64 { get; init; }"));
    assert!(edge.contains("public float? f32 { get; init; }"));
    // Both Bool default branches reach the initializer.
    assert!(edge.contains("public bool flag { get; init; } = true;"));
    assert!(edge.contains("public bool off { get; init; } = false;"));
    // Required reference types with no default get empty initializers.
    assert!(edge.contains("public byte[] blob { get; init; } = Array.Empty<byte>();"));
    assert!(edge.contains("public IReadOnlyDictionary<string, object?> meta { get; init; }"));
    assert!(
        edge.contains("public IReadOnlyList<string> tags { get; init; } = Array.Empty<string>();")
    );
    // Numeric-literal edges: a u64 default above i64::MAX renders via
    // the or_else arm; a float default on an int field renders nothing
    // (field stays nullable); an array default with one unrenderable
    // item is dropped entirely.
    assert!(edge.contains("public ulong big { get; init; } = 18446744073709551615;"));
    assert!(edge.contains("public long? frac { get; init; }"));
    assert!(edge.contains("public IReadOnlyList<int>? mix { get; init; }"));
    // Constraint summaries in the field docs.
    assert!(edge.contains("/// <summary>Bounded count, max: 100</summary>"));
    assert!(edge.contains("/// <summary>min_length: 1, max_length: 8, pattern: ^[A-Z]+$</summary>"));
}

#[test]
fn test_map_type_mapping() {
    let artifacts = gen().generate(&map_schema(), Some("abc123"));
    let src = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // String keys stay `string`; values recurse with the crate's usual
    // spellings (Array<Int32> keeps IReadOnlyList).
    assert!(src.contains("public Dictionary<string, IReadOnlyList<int>> scores"));
    // A required map without a default gets the target-typed empty init.
    assert!(src
        .contains("public Dictionary<string, IReadOnlyList<int>> scores { get; init; } = new();"));
    // Int keys widen to `long`.
    assert!(src.contains("public Dictionary<long, string>? names"));
    // Nested maps recurse on the value side.
    assert!(src.contains("public Dictionary<string, Dictionary<string, int>>? nested"));
    // Enum values keep the enum member name.
    assert!(src.contains("public Dictionary<string, ItemKind>? kinds"));
}

#[test]
fn test_map_default_rendering() {
    let artifacts = gen().generate(&map_schema(), Some("abc123"));
    let src = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // Empty map default → target-typed `new()`.
    assert!(src.contains("public Dictionary<string, int> empty { get; init; } = new();"));
    // Non-empty scalar-valued default renders as a dictionary initializer.
    assert!(src.contains(
            "public Dictionary<string, long> counts { get; init; } = new Dictionary<string, long> { { \"a\", 1 }, { \"b\", -2 } };"
        ));
    // Int keys promote numeric-string keys to long literals (entry order
    // follows the data model's object key order).
    assert!(src.contains("new Dictionary<long, int> {"));
    assert!(src.contains("{ 1, 10 }"));
    assert!(src.contains("{ -2, 20 }"));
    // Non-scalar value types sink the default; the field stays nullable.
    assert!(src.contains("public Dictionary<string, IReadOnlyList<int>>? lists"));
}

#[test]
fn test_render_default_map_edges() {
    let schema = Schema::new();
    let string_int = FieldType::Map(MapField {
        key_type: MapKeyType::String,
        value_type: Box::new(FieldType::Int32),
    });
    // Empty object default → target-typed `new()`.
    assert_eq!(
        render_default(&serde_json::json!({}), &string_int, &schema).unwrap(),
        "new()"
    );
    // Mismatched default kind (array on a map field) → dropped.
    assert!(render_default(&serde_json::json!([1]), &string_int, &schema).is_none());
    // Unrenderable value entry sinks the whole default.
    assert!(render_default(&serde_json::json!({ "a": "s" }), &string_int, &schema).is_none());
    // Non-scalar value types never render, even for `{}`.
    let string_list = FieldType::Map(MapField {
        key_type: MapKeyType::String,
        value_type: Box::new(FieldType::Array(Box::new(FieldType::Int32))),
    });
    assert!(render_default(&serde_json::json!({}), &string_list, &schema).is_none());
    // Non-numeric key on an int-keyed map → dropped.
    let int_int = FieldType::Map(MapField {
        key_type: MapKeyType::Int,
        value_type: Box::new(FieldType::Int32),
    });
    assert!(render_default(&serde_json::json!({ "x": 1 }), &int_int, &schema).is_none());
    // Numeric-string keys render as long literals.
    assert_eq!(
        render_default(&serde_json::json!({ "42": 1, "-7": 2 }), &int_int, &schema).unwrap(),
        "new Dictionary<long, int> { { -7, 2 }, { 42, 1 } }"
    );
}

#[test]
fn test_from_config_without_enum_options() {
    // options: None — the option block never opens.
    let config: TargetConfig =
        serde_yaml::from_str("format: csharp\noutput_dir: build/cs").expect("target config");
    let gen = CsTargetGenerator::from_config(&config);
    let defaults = CsTargetGenerator::default();
    assert_eq!(gen.output_dir, PathBuf::from("build/cs"));
    assert_eq!(gen.file_template, "{table}.cs");
    assert_eq!(gen.namespace, defaults.namespace);
    assert_eq!(gen.enums_file, defaults.enums_file);

    // Options present but neither key — both inner arms are skipped.
    let config: TargetConfig =
        serde_yaml::from_str("format: csharp\noutput_dir: out\noptions:\n  other: 1")
            .expect("target config");
    let gen = CsTargetGenerator::from_config(&config);
    assert_eq!(gen.namespace, defaults.namespace);
    assert_eq!(gen.enums_file, defaults.enums_file);

    // Non-string option values are ignored (the inner as_str arm fails).
    let config: TargetConfig = serde_yaml::from_str(
        "format: csharp\noutput_dir: out\noptions:\n  namespace: 42\n  enums_file: 42",
    )
    .expect("target config");
    let gen = CsTargetGenerator::from_config(&config);
    assert_eq!(gen.namespace, defaults.namespace);
    assert_eq!(gen.enums_file, defaults.enums_file);
}

#[test]
fn test_schema_without_enums_emits_no_enums_file() {
    // No (non-empty) enums → the shared enums artifact is skipped.
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Only:
    name: Only
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
    )
    .expect("enumless schema must parse");
    let artifacts = gen().generate(&schema, None);
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].0.ends_with("Only.cs"));
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
format: csharp
output_dir: build/cs
file_template: "{table}.g.cs"
options:
  namespace: Game.Config
  enums_file: SharedEnums.cs
"#,
    )
    .expect("target config");
    let gen = CsTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/cs"));
    assert_eq!(gen.file_template, "{table}.g.cs");
    assert_eq!(gen.namespace, "Game.Config");
    assert_eq!(gen.enums_file, "SharedEnums.cs");
}

#[test]
fn test_ident_sanitization() {
    assert_eq!(sanitize_ident("Item"), "Item");
    assert_eq!(sanitize_ident("drop-item"), "drop_item");
    assert_eq!(sanitize_ident("1st"), "_1st");
    assert_eq!(sanitize_ident("列"), "_");
    assert_eq!(sanitize_ident(""), "_");
    assert_eq!(cs_ident("class"), "@class");
    assert_eq!(cs_ident("int"), "@int");
    assert_eq!(cs_ident("name"), "name");
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
    // Float32 literals carry the `f` suffix.
    let lit = render_default(&serde_json::json!(1.5), &FieldType::Float32, &schema).unwrap();
    assert_eq!(lit, "1.5f");
    // Kind mismatch → None (field becomes optional instead).
    assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
    // Non-finite floats map to C# constants (via the float literal
    // renderer — serde_json::Value cannot hold NaN, so json!(NAN)
    // would become Null and never reach the float branch).
    assert_eq!(cs_float_literal(f64::NAN, false), "double.NaN");
    assert_eq!(
        cs_float_literal(f64::INFINITY, true),
        "float.PositiveInfinity"
    );
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
        "true"
    );
    assert_eq!(
        render_default(&serde_json::json!(false), &FieldType::Bool, &schema).unwrap(),
        "false"
    );
    // float32 NaN has its own constant spelling.
    assert_eq!(cs_float_literal(f64::NAN, true), "float.NaN");
    // Both widths of infinity, both signs.
    assert_eq!(
        cs_float_literal(f64::INFINITY, false),
        "double.PositiveInfinity"
    );
    assert_eq!(
        cs_float_literal(f64::NEG_INFINITY, true),
        "float.NegativeInfinity"
    );
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
        "new int[] { 1, 2 }"
    );
    // Escapes: control characters, newlines, quote and backslash.
    assert_eq!(cs_string_literal("a\u{1}b"), "\"a\\u0001b\"");
    assert_eq!(cs_string_literal("a\nb\r\tc"), "\"a\\nb\\r\\tc\"");
    assert_eq!(cs_string_literal("q\"\\q"), "\"q\\\"\\\\q\"");
}
