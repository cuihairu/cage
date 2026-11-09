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
    register_filters(&mut tera, &schema, Some("CageEnums"));
    let mut ctx = tera::Context::new();
    ctx.insert("tables", &schema.tables);
    let out = tera
            .render_str(
                "{{ tables.Item.fields.price | java_type }}|{{ tables.Item.fields.price | java_default }}|{{ tables.Item.fields.kind | java_type }}",
                &ctx,
            )
            .unwrap();
    assert_eq!(out, "int|10|ItemKind");
}

#[test]
fn java_default_filter_yields_null_for_a_field_without_default() {
    use tera::Filter;

    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
",
    )
    .unwrap();
    let filter = JavaDefaultFilter {
        schema: schema.clone(),
    };
    let field = serde_json::to_value(&schema.tables["Item"].fields["kind"]).unwrap();
    let out = filter.filter(&field, &HashMap::new()).unwrap();
    assert_eq!(out, Value::Null);
}

use super::filters::{filter_enum_idents, JavaDefaultFilter};
use super::idents::sanitize_ident;
use super::literals::{int_literal, java_float_literal, java_string_literal};
use super::*;
use cage_core::schema::{EnumValue, MapField, MapKeyType};
use std::collections::HashMap;
use tera::Tera;

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

fn gen() -> JavaTargetGenerator {
    JavaTargetGenerator::default()
}

/// Edge-case schema shared by the dev-only sample writer and the real
/// edge-rendering test: every field kind (incl. Null/Any/Bytes/Object),
/// signed/unsigned widening, wrapped u64 enum payloads, keyword and
/// restricted-identifier idents, holder/table class collisions, string
/// escapes, constraint docs, and empty enums/tables.
fn edge_schema() -> Schema {
    serde_yaml::from_str(
            r#"
tables:
  CageEnums:
    name: CageEnums
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int64 }, required: true, default: 100 }
      big: { name: big, type: { kind: UInt64 }, default: 42 }
      overflow: { name: overflow, type: { kind: UInt64 }, default: 18446744073709551615 }
      u8: { name: u8, type: { kind: UInt8 }, required: true, default: 200 }
      u16: { name: u16, type: { kind: UInt16 }, default: 65535 }
      u32: { name: u32, type: { kind: UInt32 }, default: 4294967295 }
      i8: { name: i8, type: { kind: Int8 }, default: 127 }
      i16: { name: i16, type: { kind: Int16 }, default: -32768 }
      f32: { name: f32, type: { kind: Float32 }, default: 1.5 }
      f64: { name: f64, type: { kind: Float64 }, default: 100 }
      flag: { name: flag, type: { kind: Bool }, default: true }
      nums: { name: nums, type: { kind: Array, value: { kind: Int32 } }, default: [1, 2, 3] }
      f32s: { name: f32s, type: { kind: Array, value: { kind: Float32 } }, default: [1.5, 2] }
      flags: { name: flags, type: { kind: Array, value: { kind: Bool } }, default: [true, false] }
      empty: { name: empty, type: { kind: Array, value: { kind: String } }, default: [] }
      meta: { name: meta, type: { kind: Object, value: {} } }
      raw: { name: raw, type: { kind: Null } }
      blob: { name: blob, type: { kind: Bytes } }
      anyx: { name: anyx, type: { kind: Any } }
      qty: { name: qty, type: { kind: Int32 }, required: true, max: 99, description: Stock }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 10, pattern: "^[a-z]+$" }
      desc: { name: desc, type: { kind: String }, default: "a\"b\\c\r\nd\te日" }
      class: { name: class, type: { kind: String } }
      record: { name: record, type: { kind: String } }
      _: { name: _, type: { kind: String } }
      dead: { name: dead, type: { kind: Enum, value: EmptyEnum } }
  ItemKind:
    name: ItemKind
    primary_key: []
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
  Weird-Name:
    name: Weird-Name
    primary_key: []
    fields: {}
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
  BigKind:
    name: BigKind
    description: Values that outgrow a 32-bit payload.
    values:
      - { name: Big, value: 18446744073709551615 }
      - { name: Large, value: 3000000000 }
      - { name: Neg, value: -5 }
  MixKind:
    name: MixKind
    values:
      - { name: A, value: 1.5 }
      - { name: B, value: true }
      - { name: C, value: "x y" }
      - { name: D }
  EmptyEnum:
    name: EmptyEnum
    values: []
"#,
        )
        .expect("edge schema must parse")
}

/// Map-field schema: string/int keys, array / nested-map / enum values,
/// empty and non-empty defaults, and a map inside an array.
fn map_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Bag:
    name: Bag
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      counts: { name: counts, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, description: Per-tag counts }
      byId: { name: byId, type: { kind: Map, value: { key_type: int, value_type: { kind: Int64 } } } }
      nested: { name: nested, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int64 } } } } } }
      empty: { name: empty, type: { kind: Map, value: { key_type: string, value_type: { kind: String } } }, default: {} }
      tags: { name: tags, type: { kind: Map, value: { key_type: string, value_type: { kind: String } } }, default: { a: alpha, b: beta } }
      kinds: { name: kinds, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: ItemKind } } } }
      pairs: { name: pairs, type: { kind: Array, value: { kind: Map, value: { key_type: string, value_type: { kind: Int64 } } } } }
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

/// Write one schema's artifacts under `dir/cage/generated` — the default
/// package directory a manual `javac` run expects.
fn write_sample(dir: &Path, schema: &Schema) {
    let _ = std::fs::remove_dir_all(dir);
    let pkg_dir = dir.join("cage").join("generated");
    std::fs::create_dir_all(&pkg_dir).expect("sample dir");
    for (path, content) in gen().generate(schema, Some("abc123")) {
        let name = Path::new(&path)
            .file_name()
            .expect("artifact has a file name")
            .to_owned();
        std::fs::write(pkg_dir.join(name), content).expect("write sample");
    }
}

#[test]
fn test_generate_artifact_paths() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // Tables in name order (class-ident stems), shared enums unit last.
    assert_eq!(
        paths,
        vec![
            "build/java/Drop.java",
            "build/java/Item.java",
            "build/java/CageEnums.java",
        ]
    );
}

/// Dev-only: dump the test-schema output under `/tmp/cage-java-sample`
/// for a manual `javac` run (ignored — committed tests must not require
/// a JDK).
#[test]
#[ignore = "writes /tmp/cage-java-sample for manual javac runs"]
fn write_java_sample_for_javac() {
    write_sample(Path::new("/tmp/cage-java-sample"), &test_schema());
}

/// Dev-only: dump the edge-case schema under `/tmp/cage-java-sample/edge`
/// for a manual `javac` run.
#[test]
#[ignore = "writes /tmp/cage-java-sample/edge for manual javac runs"]
fn write_java_edge_sample_for_javac() {
    write_sample(Path::new("/tmp/cage-java-sample/edge"), &edge_schema());
}

/// Dev-only: dump the map-field schema under `/tmp/cage-java-sample/map`
/// for a manual `javac` run.
#[test]
#[ignore = "writes /tmp/cage-java-sample/map for manual javac runs"]
fn write_java_map_sample_for_javac() {
    write_sample(Path::new("/tmp/cage-java-sample/map"), &map_schema());
}

#[test]
fn test_write_sample_artifacts() {
    let dir = std::env::temp_dir().join(format!("cage-java-sample-{}", std::process::id()));
    write_sample(&dir, &test_schema());
    let pkg_dir = dir.join("cage").join("generated");
    for name in ["Drop.java", "Item.java", "CageEnums.java"] {
        assert!(pkg_dir.join(name).is_file(), "missing {name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_edge_schema_rendering() {
    let schema = edge_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // The holder claims `CageEnums` first: the like-named table becomes
    // `CageEnums_`; `Weird-Name` follows its class ident in the file stem.
    assert_eq!(
        paths,
        vec![
            "build/java/CageEnums_.java",
            "build/java/ItemKind.java",
            "build/java/Weird_Name.java",
            "build/java/CageEnums.java",
        ]
    );

    // Table 1: every field kind, widening rules and keyword idents.
    let holder_table = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(holder_table.contains("public final class CageEnums_ {"));
    // Imports: List/ArrayList (array defaults) + Map (object field).
    assert!(holder_table
        .contains("import java.util.ArrayList;\nimport java.util.List;\nimport java.util.Map;\n"));
    // Unsigned widening keeps the full range; long-typed literals get L.
    assert!(holder_table.contains("public long id = 100L;"));
    assert!(holder_table.contains("public long big = 42L;"));
    // A u64 above i64::MAX has no Java literal → wrapper type, no init.
    assert!(holder_table.contains("public Long overflow;"));
    assert!(holder_table.contains("public short u8 = 200;"));
    assert!(holder_table.contains("public int u16 = 65535;"));
    assert!(holder_table.contains("public long u32 = 4294967295L;"));
    assert!(holder_table.contains("public byte i8 = 127;"));
    assert!(holder_table.contains("public short i16 = -32768;"));
    assert!(holder_table.contains("public float f32 = 1.5f;"));
    assert!(holder_table.contains("public double f64 = 100.0;"));
    assert!(holder_table.contains("public boolean flag = true;"));
    // Reference types stay plain under the optionality rule.
    assert!(holder_table.contains("public Map<String, Object> meta;"));
    assert!(holder_table.contains("public Object raw;"));
    assert!(holder_table.contains("public Object anyx;"));
    assert!(holder_table.contains("public byte[] blob;"));
    assert!(holder_table.contains("public String class_;"));
    assert!(holder_table.contains("public String record_;"));
    assert!(holder_table.contains("public String __;"));
    assert!(holder_table.contains("public String dead;"));
    assert!(holder_table.contains("public int qty;"));
    // Array defaults box primitives and re-render each element literal.
    assert!(holder_table.contains("public List<Integer> nums = new ArrayList<>(List.of(1, 2, 3));"));
    assert!(
        holder_table.contains("public List<Float> f32s = new ArrayList<>(List.of(1.5f, 2.0f));")
    );
    assert!(holder_table
        .contains("public List<Boolean> flags = new ArrayList<>(List.of(true, false));"));
    assert!(holder_table.contains("public List<String> empty = new ArrayList<>(List.of());"));
    // String escaping: short escapes for \ " CR LF TAB, printable
    // Unicode passes through.
    assert!(holder_table.contains(r#"public String desc = "a\"b\\c\r\nd\te日";"#));
    // Constraint docs: max / min_length / max_length / pattern, plus the
    // Java-specific UInt64 caveat and the unresolved (empty) enum note.
    assert!(holder_table.contains("/** Stock, required, max: 99 */"));
    assert!(holder_table.contains("/** min_length: 1, max_length: 10, pattern: ^[a-z]+$ */"));
    assert!(holder_table.contains("/** values above Long.MAX_VALUE do not fit Java's long */"));
    assert!(holder_table.contains("/** unresolved enum: EmptyEnum */"));

    // Table 2: a table class sharing the enum's simple name uses the
    // qualified `Holder.Enum` form (JLS §7.5.1) and imports nothing; an
    // empty primary key prints the raw name as the banner head.
    let item_kind = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(item_kind.contains("/** ItemKind */"));
    assert!(item_kind.contains("public CageEnums.ItemKind kind;"));
    assert!(!item_kind.contains("import "));

    // Table 3: an empty field set closes the class right after the banner.
    let weird = String::from_utf8(artifacts[2].1.clone()).unwrap();
    assert!(weird.contains("public final class Weird_Name {\n}\n"));

    // Enums: long backing for values past i32, wrapped-u64 note, enum
    // description in the declaration Javadoc.
    let enums = String::from_utf8(artifacts[3].1.clone()).unwrap();
    assert!(enums.contains("/** BigKind — Values that outgrow a 32-bit payload. */"));
    assert!(enums.contains("public enum BigKind {"));
    assert!(enums.contains("public final long value;"));
    assert!(enums.contains("        Big(-1L), // wrapped from unsigned 18446744073709551615"));
    assert!(enums.contains("        Large(3000000000L),"));
    assert!(enums.contains("        Neg(-5L);"));
    // Int backing for the small enum; string bucket for the mixed one
    // (Number → decimal string, Bool → "true", String as-is, missing →
    // member name).
    assert!(enums.contains("public enum ItemKind {"));
    assert!(enums.contains("        Sword(1);"));
    assert!(enums.contains("public final int value;"));
    assert!(enums.contains("public enum MixKind {"));
    assert!(enums.contains("        A(\"1.5\"),"));
    assert!(enums.contains("        B(\"true\"),"));
    assert!(enums.contains("        C(\"x y\"),"));
    assert!(enums.contains("        D(\"D\");"));
    assert!(enums.contains("public final String value;"));
    assert!(!enums.contains("EmptyEnum"));
}

#[test]
fn test_schema_without_enums_emits_no_holder() {
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Solo:
    name: Solo
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
    )
    .expect("schema must parse");
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // No emitted enums → no shared unit at all; the table renders with
    // no enums class in scope (and nothing to import).
    assert_eq!(paths, vec!["build/java/Solo.java"]);
    let src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(src.contains("public final class Solo {"));
    assert!(!src.contains("import "));
}

#[test]
fn test_from_config_defaults_without_options() {
    let config: TargetConfig = serde_yaml::from_str(
        r"
format: java
output_dir: build/j
",
    )
    .expect("target config");
    let gen = JavaTargetGenerator::from_config(&config);
    // No `options` block → every generator knob keeps its default.
    assert_eq!(gen.output_dir, PathBuf::from("build/j"));
    assert_eq!(gen.file_template, "{table}.java");
    assert_eq!(gen.enums_file, "CageEnums.java");
    assert_eq!(gen.package, "cage.generated");
}

#[test]
fn test_enums_file_in_subdirectory() {
    let config: TargetConfig = serde_yaml::from_str(
        r"
format: java
output_dir: build/j
options:
  enums_file: shared/CageEnums.java
",
    )
    .expect("target config");
    let gen = JavaTargetGenerator::from_config(&config);
    let artifacts = gen.generate(&test_schema(), Some("abc123"));
    // The enums unit keeps the option's parent directory under the
    // output dir; the holder class name still comes from the stem.
    assert_eq!(artifacts[2].0, "build/j/shared/CageEnums.java");
    let holder = String::from_utf8(artifacts[2].1.clone()).unwrap();
    assert!(holder.contains("public final class CageEnums {"));
    // Table imports are built from package + holder class, unchanged.
    let item = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(item.contains("import cage.generated.CageEnums.ItemKind;"));
}

#[test]
fn test_enums_context_skips_unallocated_idents() {
    let schema = test_schema();
    let emitted = JavaTargetGenerator::emitted_enums(&schema);
    // An enum whose allocated ident is missing from the map is skipped
    // rather than rendered with a broken name.
    let no_idents: HashMap<String, String> = HashMap::new();
    let ctx = JavaTargetGenerator::java_enums_context(
        "cage.generated",
        &emitted,
        Some("CageEnums"),
        &no_idents,
    );
    assert_eq!(ctx["emitted_enums"].as_array().unwrap().len(), 0);
    assert_eq!(ctx["holder_ident"], "CageEnums");

    // With the idents generate() allocates, every emitted enum survives.
    let idents = filter_enum_idents(&schema, Some("CageEnums"));
    let ctx = JavaTargetGenerator::java_enums_context(
        "cage.generated",
        &emitted,
        Some("CageEnums"),
        &idents,
    );
    let enums = ctx["emitted_enums"].as_array().unwrap();
    assert_eq!(enums.len(), emitted.len());
    assert_eq!(enums[0]["ident"], "ItemKind");
    assert_eq!(enums[0]["payload"], "int");
    assert_eq!(enums[0]["banner"], "ItemKind");
    assert_eq!(enums[0]["members"].as_array().unwrap().len(), 2);
}

#[test]
fn test_enums_context_string_payload_bucket() {
    // An enum with no integral member values buckets to a String payload.
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums:
  Rarity:
    name: Rarity
    description: Item tiers.
    values:
      - { name: Common }
      - { name: Rare }
",
    )
    .unwrap();
    let emitted = JavaTargetGenerator::emitted_enums(&schema);
    let idents = filter_enum_idents(&schema, None);
    let ctx = JavaTargetGenerator::java_enums_context("cage.generated", &emitted, None, &idents);
    let e = &ctx["emitted_enums"].as_array().unwrap()[0];
    assert_eq!(e["payload"], "String");
    assert_eq!(e["banner"], "Rarity — Item tiers.");
    assert_eq!(e["members"].as_array().unwrap().len(), 2);
}

#[test]
fn test_table_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let src = String::from_utf8(artifacts[1].1.clone()).unwrap();

    assert!(src.starts_with("package cage.generated;\n"));
    assert!(src.contains("//   schema: abc123"));
    assert!(src.contains("//   table:  Item"));
    // One import block, sorted lexicographically by full name
    // (java.* and the holder's nested enums mixed together).
    assert!(src.contains(
            "import cage.generated.CageEnums.ItemKind;\nimport java.util.ArrayList;\nimport java.util.List;\n"
        ));
    assert!(src.contains(
        "/**\n * Equipment definitions.\n * Item — primary key: id\n */\npublic final class Item {"
    ));

    // Fields in name order (sort, not schema insertion order); the
    // optionality group rule keeps present types for required or
    // defaulted fields and widens the rest to wrappers.
    let id_pos = src.find("public int id;").unwrap();
    let kind_pos = src.find("public ItemKind kind;").unwrap();
    let name_pos = src.find("public String name;").unwrap();
    let note_pos = src.find("public String note;").unwrap();
    let price_pos = src.find("public Integer price;").unwrap();
    let rarity_pos = src.find("public String rarity;").unwrap();
    let tags_pos = src
        .find("public List<String> tags = new ArrayList<>(List.of(\"pvp\"));")
        .unwrap();
    let weight_pos = src.find("public double weight = 1.5;").unwrap();
    assert!(id_pos < kind_pos && kind_pos < name_pos && name_pos < note_pos);
    assert!(note_pos < price_pos && price_pos < rarity_pos);
    assert!(rarity_pos < tags_pos && tags_pos < weight_pos);

    // Per-field Javadoc with the shared field-doc summary.
    assert!(src.contains("/** Identifier, required */"));
    assert!(src.contains("/** Price in gold, min: 0 */"));
    assert!(src.contains("/** allowed: common | rare */"));
    assert!(src.contains("/** → Player.id */"));
}

#[test]
fn test_enums_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    assert!(enums_src.starts_with("package cage.generated;\n"));
    assert!(enums_src.contains("//   enums:  shared definitions"));
    assert!(enums_src.contains("public final class CageEnums {\n    private CageEnums() {}"));
    // Integral bucket: int payload when every value fits i32.
    assert!(enums_src.contains("public enum ItemKind {"));
    assert!(enums_src.contains("/** Sword weapon */\n        Sword(1),\n        Shield(2);"));
    assert!(enums_src.contains("public final int value;\n\n        ItemKind(int value) {"));
    // String bucket: valueless members carry their name as the payload.
    assert!(enums_src.contains("public enum Rarity {"));
    assert!(enums_src.contains("common(\"common\"),\n        rare(\"rare\");"));
    assert!(enums_src.contains("public final String value;"));
    assert!(enums_src.contains("Rarity(String value) {"));
    // Empty enums are not emitted.
    assert!(!enums_src.contains("EmptyEnum"));
}

#[test]
fn test_unresolved_enum_falls_back_to_string() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, None);
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(drop_src.contains("//   schema: (unavailable)"));
    assert!(drop_src.contains("public String item;"));
    assert!(drop_src.contains("/** unresolved enum: MissingEnum */"));
    // A unit that uses nothing imports nothing.
    assert!(!drop_src.contains("import "));
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
format: java
output_dir: build/j
file_template: "{table}_gen.java"
options:
  package: com.example.cfg
  enums_file: SharedEnums.java
"#,
    )
    .expect("target config");
    let gen = JavaTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/j"));
    assert_eq!(gen.file_template, "{table}_gen.java");
    assert_eq!(gen.enums_file, "SharedEnums.java");
    assert_eq!(gen.package, "com.example.cfg");

    // The options flow through: package clause, enum imports built from
    // the configured package + holder class, holder file named after the
    // enums_file option's stem.
    let schema = test_schema();
    let artifacts = gen.generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "build/j/Drop_gen.java",
            "build/j/Item_gen.java",
            "build/j/SharedEnums.java",
        ]
    );
    let item = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(item.contains("package com.example.cfg;"));
    assert!(item.contains("import com.example.cfg.SharedEnums.ItemKind;"));
    let holder = String::from_utf8(artifacts[2].1.clone()).unwrap();
    assert!(holder.contains("public final class SharedEnums {"));
}

#[test]
fn test_ident_sanitization() {
    assert_eq!(sanitize_ident("Item"), "Item");
    assert_eq!(sanitize_ident("drop-item"), "drop_item");
    assert_eq!(sanitize_ident("1st"), "_1st");
    assert_eq!(sanitize_ident("列"), "_");
    // Nothing left after sanitizing → a lone underscore.
    assert_eq!(sanitize_ident(""), "_");
    // Java keywords get a trailing underscore (no verbatim escape);
    // restricted identifiers and literals are covered by the same rule.
    assert_eq!(java_ident("class"), "class_");
    assert_eq!(java_ident("package"), "package_");
    assert_eq!(java_ident("var"), "var_");
    assert_eq!(java_ident("record"), "record_");
    assert_eq!(java_ident("yield"), "yield_");
    assert_eq!(java_ident("sealed"), "sealed_");
    assert_eq!(java_ident("permits"), "permits_");
    assert_eq!(java_ident("true"), "true_");
    assert_eq!(java_ident("null"), "null_");
    // A lone `_` is illegal since Java 9 — maps to `__`.
    assert_eq!(java_ident("_"), "__");
    assert_eq!(java_ident("name"), "name");
}

#[test]
fn test_field_collisions_get_unique_members() {
    let mut used = HashSet::new();
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b_");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b__");
}

#[test]
fn test_class_name_collision() {
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  CageEnums:
    name: CageEnums
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums:
  Kind:
    name: Kind
    values:
      - { name: A, value: 1 }
",
    )
    .expect("schema");
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // The holder claims its package-level name first: the like-named
    // table becomes CageEnums_ and its file stem follows the class
    // ident (the documented Java deviation).
    assert_eq!(
        paths,
        vec!["build/java/CageEnums_.java", "build/java/CageEnums.java"]
    );
    let table = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(table.contains("public final class CageEnums_ {"));
    let holder = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(holder.contains("public final class CageEnums {"));
}

#[test]
fn test_render_default_edge_cases() {
    let schema = Schema::new();
    // Float literals always keep a decimal point; Float32 adds the
    // mandatory `f` suffix (a bare `1.5` would not narrow to float).
    assert_eq!(
        render_default(&serde_json::json!(100.0), &FieldType::Float64, &schema).unwrap(),
        "100.0"
    );
    assert_eq!(
        render_default(&serde_json::json!(1.5), &FieldType::Float32, &schema).unwrap(),
        "1.5f"
    );
    // Non-finite floats map to the java.lang constants (serde_json
    // cannot hold NaN, so json!(NAN) would become Null — the literal
    // renderer is tested directly, like the C# generator).
    assert_eq!(java_float_literal(f64::NAN, false), "Double.NaN");
    assert_eq!(java_float_literal(f64::NAN, true), "Float.NaN");
    assert_eq!(
        java_float_literal(f64::INFINITY, true),
        "Float.POSITIVE_INFINITY"
    );
    assert_eq!(
        java_float_literal(f64::NEG_INFINITY, false),
        "Double.NEGATIVE_INFINITY"
    );
    // long-typed fields carry the L suffix; int-typed fields do not.
    assert_eq!(
        render_default(&serde_json::json!(100), &FieldType::Int64, &schema).unwrap(),
        "100L"
    );
    assert_eq!(
        render_default(&serde_json::json!(100), &FieldType::UInt32, &schema).unwrap(),
        "100L"
    );
    assert_eq!(
        render_default(&serde_json::json!(100), &FieldType::Int32, &schema).unwrap(),
        "100"
    );
    // Array defaults are fresh mutable lists with scalar elements.
    assert_eq!(
        render_default(
            &serde_json::json!(["pvp", "x"]),
            &FieldType::Array(Box::new(FieldType::String)),
            &schema
        )
        .unwrap(),
        "new ArrayList<>(List.of(\"pvp\", \"x\"))"
    );
    assert_eq!(
        render_default(
            &serde_json::json!([]),
            &FieldType::Array(Box::new(FieldType::String)),
            &schema
        )
        .unwrap(),
        "new ArrayList<>(List.of())"
    );
    // Array-of-array defaults are not scalar — skipped (shared rule).
    assert!(render_default(
        &serde_json::json!([[1]]),
        &FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Int32)))),
        &schema
    )
    .is_none());
    // UInt64 above i64::MAX has no Java literal — initializer skipped.
    assert!(render_default(
        &serde_json::json!(18_446_744_073_709_551_615u64),
        &FieldType::UInt64,
        &schema
    )
    .is_none());
    // A value that does not fit the field's Java type has no literal.
    assert!(render_default(
        &serde_json::json!(5_000_000_000i64),
        &FieldType::Int32,
        &schema
    )
    .is_none());
    // Kind mismatch → None (field falls back to its wrapper type).
    assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
    // Object defaults are not rendered (shared rule across targets).
    assert!(render_default(
        &serde_json::json!({"a": 1}),
        &FieldType::Object(indexmap::IndexMap::default()),
        &schema
    )
    .is_none());
    // Control characters use four-hex-digit \u00XX escapes — a raw
    // \u000a would inject a newline even inside a string literal, so
    // CR/LF/TAB keep their short escapes.
    assert_eq!(java_string_literal("a\u{1}b"), "\"a\\u0001b\"");
    assert_eq!(java_string_literal("a\u{7f}b"), "\"a\\u007fb\"");
    assert_eq!(java_string_literal("n\n"), "\"n\\n\"");
    assert_eq!(java_string_literal("q\"r\\s"), "\"q\\\"r\\\\s\"");
    // int_literal accepts only integral kinds — anything else falls in
    // the same "no safe literal" bucket as an out-of-range value.
    assert!(int_literal(1, &FieldType::Float32).is_none());
}

#[test]
fn test_map_field_rendering() {
    let schema = map_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        vec!["build/java/Bag.java", "build/java/CageEnums.java"]
    );
    let src = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // map<K, V> → HashMap<K, V>: string keys stay String, int keys
    // (i64 semantics) become Long, values recurse with primitives boxed
    // for the generic context — exactly the array element rule.
    assert!(src.contains("public HashMap<String, List<Integer>> counts;"));
    assert!(src.contains("public HashMap<Long, Long> byId;"));
    assert!(src.contains("public HashMap<String, HashMap<String, Long>> nested;"));
    // A map inside an array mentions HashMap through the element type.
    assert!(src.contains("public List<HashMap<String, Long>> pairs;"));
    // Enum values resolve through map value positions like array
    // elements, importing the holder's nested type.
    assert!(src.contains("public HashMap<String, ItemKind> kinds;"));
    // One sorted import block: HashMap for the map mentions, List for
    // the array mentions (nested through maps too), Map for the
    // Map.of(…) in the non-empty default.
    assert!(src.contains(
            "import cage.generated.CageEnums.ItemKind;\nimport java.util.HashMap;\nimport java.util.List;\nimport java.util.Map;\n"
        ));
    // Defaults: the empty map is the bare mutable base; scalar entries
    // wrap Map.of in a fresh HashMap per instance (never shared).
    assert!(src.contains("public HashMap<String, String> empty = new HashMap<>();"));
    assert!(src.contains(
            "public HashMap<String, String> tags = new HashMap<>(Map.of(\"a\", \"alpha\", \"b\", \"beta\"));"
        ));
    // Optionality: HashMap is a reference type — not-required fields
    // without defaults (byId, counts, …) keep the same spelling, no
    // wrapper widening. Field docs still render for map fields.
    assert!(src.contains("/** Per-tag counts */"));
}

#[test]
fn test_map_default_rendering() {
    let schema = Schema::new();
    let map = |k: MapKeyType, v: FieldType| {
        FieldType::Map(MapField {
            key_type: k,
            value_type: Box::new(v),
        })
    };
    // Empty default → the bare mutable base, never a shared instance.
    assert_eq!(
        render_default(
            &serde_json::json!({}),
            &map(MapKeyType::String, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "new HashMap<>()"
    );
    // Scalar entries: a fresh HashMap per instance wrapping Map.of,
    // values through the existing literal rules (long values carry L).
    assert_eq!(
        render_default(
            &serde_json::json!({ "a": 1, "b": 2 }),
            &map(MapKeyType::String, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "new HashMap<>(Map.of(\"a\", 1, \"b\", 2))"
    );
    assert_eq!(
        render_default(
            &serde_json::json!({ "hp": 100 }),
            &map(MapKeyType::String, FieldType::Int64),
            &schema
        )
        .unwrap(),
        "new HashMap<>(Map.of(\"hp\", 100L))"
    );
    // Int keys are numeric strings in the data model ("42", "-7") and
    // render as long literals in sorted key order — a bare int literal
    // would infer Map<Integer, …>, which does not copy into a
    // HashMap<Long, …>.
    assert_eq!(
        render_default(
            &serde_json::json!({ "-7": 6, "42": 5 }),
            &map(MapKeyType::Int, FieldType::Int64),
            &schema
        )
        .unwrap(),
        "new HashMap<>(Map.of(-7L, 6L, 42L, 5L))"
    );
    // A non-numeric key under an int key type → skipped initializer.
    assert!(render_default(
        &serde_json::json!({ "x": 1 }),
        &map(MapKeyType::Int, FieldType::Int32),
        &schema
    )
    .is_none());
    // Member kind mismatch (a string value for an Int32 value type) →
    // the whole initializer is skipped: the array-default
    // all-or-nothing rule.
    assert!(render_default(
        &serde_json::json!({ "a": "x" }),
        &map(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
    // Non-scalar value types skip their members (the shared
    // cross-target scalar rule — the default machinery covers scalars
    // and arrays-of-scalars, so map members with array/object/enum
    // values have no literal). The initializer is dropped entirely
    // rather than silently rendering an empty base that would lose the
    // declared data; for reference-typed fields the declaration then
    // spells exactly what the optionality rule would spell anyway.
    assert!(render_default(
        &serde_json::json!({ "a": [1] }),
        &map(
            MapKeyType::String,
            FieldType::Array(Box::new(FieldType::Int32))
        ),
        &schema
    )
    .is_none());
    assert!(render_default(
        &serde_json::json!({ "a": {} }),
        &map(
            MapKeyType::String,
            FieldType::Object(indexmap::IndexMap::default())
        ),
        &schema
    )
    .is_none());
    assert!(render_default(
        &serde_json::json!({ "a": "Sword" }),
        &map(MapKeyType::String, FieldType::Enum("ItemKind".to_string())),
        &schema
    )
    .is_none());
    // Map.of stops at ten pairs — beyond that there is no compile-safe
    // single-expression literal, so the default joins the skip bucket.
    let entries: serde_json::Map<String, serde_json::Value> = (0..11)
        .map(|i| (format!("k{i:02}"), serde_json::json!(i)))
        .collect();
    assert!(render_default(
        &serde_json::Value::Object(entries),
        &map(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
    let entries: serde_json::Map<String, serde_json::Value> = (0..10)
        .map(|i| (format!("k{i:02}"), serde_json::json!(i)))
        .collect();
    // Ten pairs still render — the full sorted-entry spelling.
    assert_eq!(
        render_default(
            &serde_json::Value::Object(entries),
            &map(MapKeyType::String, FieldType::Int32),
            &schema,
        )
        .unwrap(),
        "new HashMap<>(Map.of(\"k00\", 0, \"k01\", 1, \"k02\", 2, \"k03\", 3, \"k04\", 4, \
             \"k05\", 5, \"k06\", 6, \"k07\", 7, \"k08\", 8, \"k09\", 9))"
    );
    // Kind mismatch (an array where a map is declared) → skipped.
    assert!(render_default(
        &serde_json::json!([1]),
        &map(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
    // int_literal keeps rejecting non-integral kinds — maps included.
    assert!(int_literal(1, &map(MapKeyType::String, FieldType::Int32)).is_none());
}

#[test]
fn test_enum_payload_backing() {
    let value = |v: Option<serde_json::Value>, name: &str| EnumValue {
        name: name.to_string(),
        value: v,
        description: None,
    };
    let fits_i32 = EnumSchema {
        name: "A".to_string(),
        values: vec![
            value(Some(serde_json::json!(1)), "One"),
            value(Some(serde_json::json!(-5)), "Neg"),
        ],
        description: None,
    };
    let overflows = EnumSchema {
        name: "B".to_string(),
        values: vec![
            value(Some(serde_json::json!(3_000_000_000i64)), "Large"),
            value(
                Some(serde_json::json!(18_446_744_073_709_551_615u64)),
                "Huge",
            ),
        ],
        description: None,
    };
    // i32 range → int payload, no literal suffix.
    assert!(fits_i32.values.iter().all(value_fits_i32));
    let (lit, note) = integral_literal(&fits_i32.values[0], false);
    assert_eq!((lit.as_str(), note), ("1", None));
    // Beyond i32::MAX (u32-sized or a float payload) → long backing.
    assert!(overflows.values.iter().all(|v| !value_fits_i32(v)));
    let (lit, note) = integral_literal(&overflows.values[0], true);
    assert_eq!((lit.as_str(), note), ("3000000000L", None));
    // Above i64::MAX the u64 wraps to its two's-complement negative
    // literal with a note (Java has no unsigned 64-bit type).
    let (lit, note) = integral_literal(&overflows.values[1], true);
    assert_eq!(lit, "-1L");
    assert_eq!(
        note.as_deref(),
        Some("wrapped from unsigned 18446744073709551615")
    );
    // Without the long backing the wrapped value renders bare (int
    // payloads widen silently).
    let (lit, note) = integral_literal(&overflows.values[1], false);
    assert_eq!(lit, "-1");
    assert_eq!(
        note.as_deref(),
        Some("wrapped from unsigned 18446744073709551615")
    );
    // A value-less member has no integral payload: the name is the
    // fallback, and it never fits an int payload.
    let valueless = value(None, "NoVal");
    assert!(!value_fits_i32(&valueless));
    let (lit, note) = integral_literal(&valueless, false);
    assert_eq!((lit.as_str(), note), ("NoVal", None));
    // A float-valued member has no i64/u64 spelling either — same name
    // fallback.
    let floaty = value(Some(serde_json::json!(1.5)), "Floaty");
    let (lit, note) = integral_literal(&floaty, true);
    assert_eq!((lit.as_str(), note), ("Floaty", None));
}
