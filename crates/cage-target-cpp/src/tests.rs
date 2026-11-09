use super::filters::CppDefaultFilter;
use super::idents::{cpp_ident, sanitize_ident};
use super::literals::{
    cpp_float_literal, cpp_string_literal, cpp_uint_literal, enum_backing, integral_value_literal,
    render_default,
};
use cage_core::schema::{EnumValue, FieldType, MapField, MapKeyType};
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
                "{{ tables.Item.fields.price | cpp_type }}|{{ tables.Item.fields.price | cpp_default }}|{{ tables.Item.fields.kind | cpp_type }}",
                &ctx,
            )
            .unwrap();
    assert_eq!(out, "std::int32_t|10|ItemKind");
}

use super::*;
use std::path::Path;

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

fn gen() -> CppTargetGenerator {
    CppTargetGenerator::default()
}

/// Second schema: extreme integer literals, every remaining field kind
/// (incl. Any/Null/Object/Bytes), the string escapes, constraint docs,
/// enum descriptions, a mixed string bucket, empty-primary-key and
/// include-free tables, and a string default whose text trips the
/// <limits> include guard.
const EXTREMES_SCHEMA: &str = r#"
tables:
  Edge:
    name: Edge
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt64 }, required: true, default: 18446744073709551615 }
      big: { name: big, type: { kind: Int64 }, required: true, default: -9223372036854775808 }
      mode: { name: mode, type: { kind: Enum, value: Mode } }
      meta: { name: meta, type: { kind: Object, value: {} } }
      raw: { name: raw, type: { kind: Null } }
      tag: { name: tag, type: { kind: String }, default: "x\x01y" }
      anyx: { name: anyx, type: { kind: Any }, default: 7 }
      i8: { name: i8, type: { kind: Int8 }, default: -128 }
      i16: { name: i16, type: { kind: Int16 }, default: -32768 }
      u8x: { name: u8x, type: { kind: UInt8 }, default: 200 }
      u16x: { name: u16x, type: { kind: UInt16 }, default: 65535 }
      u32x: { name: u32x, type: { kind: UInt32 }, default: 4294967295 }
      u64mid: { name: u64mid, type: { kind: UInt64 }, default: 42 }
      f32: { name: f32, type: { kind: Float32 }, default: 1.5 }
      flag: { name: flag, type: { kind: Bool }, default: true }
      blob: { name: blob, type: { kind: Bytes } }
      qty: { name: qty, type: { kind: Int32 }, required: true, max: 99, description: Stock }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 10, pattern: "^[a-z]+$" }
      esc: { name: esc, type: { kind: String }, default: "a\nb\rc" }
      lim: { name: lim, type: { kind: String }, default: "std::numeric_limits<int>::max()" }
  NoKey:
    name: NoKey
    primary_key: []
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
  Plain:
    name: Plain
    primary_key: [id]
    fields:
      flag: { name: flag, type: { kind: Bool }, required: true }
      ratio: { name: ratio, type: { kind: Float64 }, default: 1.5 }
      small: { name: small, type: { kind: Float32 }, default: 1.5 }
enums:
  EdgeEnum:
    name: EdgeEnum
    values:
      - { name: Max, value: 9223372036854775807 }
      - { name: Min, value: -9223372036854775808 }
  Huge:
    name: Huge
    values:
      - { name: Top, value: 18446744073709551615 }
  Labeled:
    name: Labeled
    description: Values past the 32-bit range.
    values:
      - { name: Large, value: 3000000000 }
  MixKind:
    name: MixKind
    values:
      - { name: S, value: "x y", description: spaced }
      - { name: N, value: 42 }
      - { name: T, value: true }
  Mode:
    name: Mode
    values:
      - { name: "on" }
      - { name: "off" }
"#;

const BASIC_MAIN_CPP: &str = r#"#include "cage_enums.h"
#include "Drop.h"
#include "Item.h"

int main() {
    cage::generated::Item item{};
    item.name = "Sword";
    item.id = 1;
    item.price = 10;
    item.weight = 1.5;
    item.note = "n";
    item.owner = "o";
    item.rarity = "rare";
    item.tags = {"pvp"};
    item.kind = cage::generated::ItemKind::Sword;

    cage::generated::Drop drop{};
    drop.id = 42;
    drop.item = "x";

    cage::generated::ItemKind k = cage::generated::ItemKind::Shield;
    auto r = cage::generated::Rarity::rare;
    auto c = cage::generated::Rarity::common;
    (void)k;
    (void)r;
    (void)c;
    return 0;
}
"#;

const EXTREMES_MAIN_CPP: &str = r#"#include "Edge.h"
#include "NoKey.h"
#include "Plain.h"
#include "cage_enums.h"

int main() {
    cage::generated::Edge e{};
    e.big = -1;
    e.id = 7;
    e.mode = "on";
    e.anyx = 7;
    e.i8 = -1;
    e.i16 = 2;
    e.u8x = 3;
    e.u16x = 4;
    e.u32x = 5;
    e.u64mid = 6;
    e.f32 = 0.5f;
    e.flag = false;
    e.qty = 1;
    e.code = "a";
    e.esc = "x";
    e.lim = "y";
    (void)e.meta;
    (void)e.raw;
    (void)e.tag;
    (void)e.blob;
    auto mx = cage::generated::EdgeEnum::Max;
    auto mn = cage::generated::EdgeEnum::Min;
    auto top = cage::generated::Huge::Top;
    auto lg = cage::generated::Labeled::Large;
    auto s = cage::generated::MixKind::S;
    auto n2 = cage::generated::MixKind::N;
    auto t = cage::generated::MixKind::T;
    auto on = cage::generated::Mode::on;
    auto off = cage::generated::Mode::off;
    (void)mx;
    (void)mn;
    (void)top;
    (void)lg;
    (void)s;
    (void)n2;
    (void)t;
    (void)on;
    (void)off;
    cage::generated::NoKey nk{};
    nk.id = 1;
    (void)nk;
    cage::generated::Plain p{};
    p.flag = true;
    p.ratio = 1.0;
    p.small = 0.5f;
    (void)p;
    return 0;
}
"#;

#[test]
fn test_generate_artifact_paths() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    // Tables in name order, shared enums header last.
    assert_eq!(
        paths,
        vec![
            "build/cpp/Drop.h",
            "build/cpp/Item.h",
            "build/cpp/cage_enums.h",
        ]
    );
}

#[test]
fn test_table_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let item = String::from_utf8(artifacts[1].1.clone()).unwrap();

    assert!(item.contains("//   schema: abc123"));
    assert!(item.contains("//   table:  Item"));
    assert!(item.contains("#pragma once"));
    assert!(item.contains("namespace cage::generated {"));
    assert!(item.contains("}  // namespace cage::generated"));

    // Include set: angle includes sorted alphabetically, enums header
    // quoted in its own last group; unused headers stay out.
    assert!(item.contains("#include <cstdint>"));
    assert!(item.contains("#include <optional>"));
    assert!(item.contains("#include <string>"));
    assert!(item.contains("#include <vector>"));
    assert!(item.contains("#include \"cage_enums.h\""));
    assert!(!item.contains("#include <map>"));
    assert!(!item.contains("#include <any>"));
    assert!(!item.contains("#include <limits>"));
    assert!(!item.contains("#include <string_view>"));
    let cstdint = item.find("#include <cstdint>").unwrap();
    let optional = item.find("#include <optional>").unwrap();
    let string = item.find("#include <string>").unwrap();
    let vector = item.find("#include <vector>").unwrap();
    let enums_header = item.find("#include \"cage_enums.h\"").unwrap();
    assert!(cstdint < optional);
    assert!(optional < string);
    assert!(string < vector);
    assert!(vector < enums_header);

    // Struct banner + description above the declaration.
    let banner = item.find("// Item — primary key: id").unwrap();
    let description = item.find("// Equipment definitions.").unwrap();
    let struct_open = item.find("struct Item {").unwrap();
    assert!(banner < struct_open);
    assert!(description < struct_open);

    // Members in pure name order: id, kind, name, note, owner, price,
    // rarity, tags, weight.
    let id = item.find("std::int32_t id{};").unwrap();
    let kind = item.find("std::optional<ItemKind> kind;").unwrap();
    let name = item.find("std::string name{};").unwrap();
    let note = item.find("std::optional<std::string> note;").unwrap();
    let owner = item.find("std::optional<std::string> owner;").unwrap();
    let price = item.find("std::optional<std::int32_t> price;").unwrap();
    let rarity = item.find("std::optional<std::string> rarity;").unwrap();
    let tags = item
        .find("std::vector<std::string> tags{\"pvp\"};")
        .unwrap();
    let weight = item.find("double weight{1.5};").unwrap();
    assert!(id < kind, "id before kind");
    assert!(kind < name, "kind before name");
    assert!(name < note, "name before note");
    assert!(note < owner, "note before owner");
    assert!(owner < price, "owner before price");
    assert!(price < rarity, "price before rarity");
    assert!(rarity < tags, "rarity before tags");
    assert!(tags < weight, "tags before weight");

    // Optionality group rule: required or defaulted → present type
    // (value init `{}` without a default); otherwise std::optional.
    assert!(item.contains("std::int32_t id{};"));
    assert!(item.contains("std::string name{};"));
    assert!(item.contains("std::optional<std::int32_t> price;"));
    assert!(item.contains("std::optional<std::string> note;"));
    assert!(!item.contains("std::optional<double>"));
    assert!(!item.contains("std::optional<std::vector"));

    // Field docs: one `//` line above the member, four-space indent.
    assert!(item.contains("    // Display name, required"));
    assert!(item.contains("    // Identifier, required"));
    assert!(item.contains("    // Price in gold, min: 0"));
    assert!(item.contains("    // allowed: common | rare"));
    assert!(item.contains("    // → Player.id"));
}

#[test]
fn test_enums_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let enums = String::from_utf8(artifacts[2].1.clone()).unwrap();

    assert!(enums.contains("//   schema: abc123"));
    assert!(enums.contains("//   enums:  shared definitions"));
    assert!(enums.contains("#pragma once"));
    assert!(enums.contains("#include <cstdint>"));
    assert!(enums.contains("#include <string_view>"));
    let cstdint = enums.find("#include <cstdint>").unwrap();
    let string_view = enums.find("#include <string_view>").unwrap();
    assert!(cstdint < string_view);

    // Integral enum: enum class with range-selected backing, decimal
    // values, member descriptions as trailing comments.
    assert!(enums.contains("// ItemKind\n"));
    assert!(enums.contains("enum class ItemKind : std::int32_t {"));
    assert!(enums.contains("    Sword = 1,  // Sword weapon"));
    assert!(enums.contains("    Shield = 2,"));

    // String-bucket enum: namespace of string_view constants.
    assert!(enums.contains("// Rarity\n"));
    assert!(enums.contains("namespace Rarity {"));
    assert!(enums.contains("inline constexpr std::string_view common{\"common\"};"));
    assert!(enums.contains("inline constexpr std::string_view rare{\"rare\"};"));

    // Empty enums are never emitted; no structs leak into this file.
    assert!(!enums.contains("EmptyEnum"));
    assert!(!enums.contains("struct "));
    assert!(enums.contains("}  // namespace cage::generated"));
}

#[test]
fn test_unresolved_enum_falls_back() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, None);
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();

    assert!(drop_src.contains("//   schema: (unavailable)"));
    assert!(drop_src.contains("//   table:  Drop"));
    assert!(drop_src.contains("// Drop — primary key: id"));
    assert!(drop_src.contains("std::int64_t id{};"));
    assert!(drop_src.contains("    // required"));
    // Unresolved enum → std::optional<std::string> + doc note, and no
    // enums header include.
    assert!(drop_src.contains("std::optional<std::string> item;"));
    assert!(drop_src.contains("unresolved enum: MissingEnum"));
    assert!(!drop_src.contains("cage_enums.h"));
    assert!(!drop_src.contains("#include <string_view>"));
    assert!(!drop_src.contains("#include <vector>"));
    assert!(!drop_src.contains("#include <map>"));
    assert!(!drop_src.contains("#include <any>"));
    assert!(!drop_src.contains("#include <limits>"));
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
format: cpp
output_dir: build/game
file_template: "{table}_gen.h"
options:
  enums_file: shared_enums.h
  namespace: game::cfg
"#,
    )
    .expect("target config");
    let gen = CppTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/game"));
    assert_eq!(gen.file_template, "{table}_gen.h");
    assert_eq!(gen.enums_file, "shared_enums.h");
    assert_eq!(gen.namespace, "game::cfg");

    // The options reach the rendered output verbatim.
    let artifacts = gen.generate(&test_schema(), None);
    assert_eq!(artifacts[0].0, "build/game/Drop_gen.h");
    assert_eq!(artifacts[2].0, "build/game/shared_enums.h");
    let item = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(item.contains("namespace game::cfg {"));
    assert!(item.contains("}  // namespace game::cfg"));
    assert!(item.contains("#include \"shared_enums.h\""));
    let enums = String::from_utf8(artifacts[2].1.clone()).unwrap();
    assert!(enums.contains("namespace game::cfg {"));
}

#[test]
fn test_ident_sanitization() {
    assert_eq!(sanitize_ident("Item"), "Item");
    assert_eq!(sanitize_ident("drop-item"), "drop_item");
    assert_eq!(sanitize_ident("1st"), "_1st");
    assert_eq!(sanitize_ident("列"), "_");
    // Nothing left after sanitizing → a lone underscore.
    assert_eq!(sanitize_ident(""), "_");
    // Keyword collision gets a trailing underscore (no escape syntax).
    assert_eq!(cpp_ident("class"), "class_");
    assert_eq!(cpp_ident("and"), "and_");
    assert_eq!(cpp_ident("template"), "template_");
    assert_eq!(cpp_ident("name"), "name");
}

#[test]
fn test_field_collisions_get_unique_members() {
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  Rarity:
    name: Rarity
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      a-b: { name: a-b, type: { kind: Int32 }, required: true }
      a_b: { name: a_b, type: { kind: Int32 }, required: true }
      class: { name: class, type: { kind: Bool }, required: true }
enums:
  Rarity:
    name: Rarity
    values:
      - { name: common }
",
    )
    .expect("collision schema");
    let artifacts = gen().generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, vec!["build/cpp/Rarity.h", "build/cpp/cage_enums.h"]);

    // Table wins the shared namespace; the same-named enum is suffixed.
    let header = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(header.contains("struct Rarity {"));
    // Sanitized collision + keyword escape, first-unique-wins in order.
    let a_b = header.find("std::int32_t a_b{};").unwrap();
    let a_b_collided = header.find("std::int32_t a_b_{};").unwrap();
    let class = header.find("bool class_{};").unwrap();
    let id = header.find("std::int32_t id{};").unwrap();
    assert!(a_b < a_b_collided, "a-b before a_b");
    assert!(a_b_collided < class, "a_b before class");
    assert!(class < id, "class before id");

    let enums = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(enums.contains("// Rarity_"));
    assert!(enums.contains("namespace Rarity_ {"));
    assert!(enums.contains("inline constexpr std::string_view common{\"common\"};"));
}

#[test]
fn test_string_enum_field_maps_to_string_view() {
    // A resolved string-bucket enum has no type of its own (it is a
    // namespace of constants), so fields hold std::string_view views of
    // those constants and pull in <string_view> + the enums header.
    let schema: Schema = serde_yaml::from_str(
        r#"
tables:
  Toggle:
    name: Toggle
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      mode: { name: mode, type: { kind: Enum, value: Mode } }
enums:
  Mode:
    name: Mode
    values:
      - { name: "on" }
      - { name: "off" }
"#,
    )
    .expect("string enum schema");
    let artifacts = gen().generate(&schema, Some("abc123"));
    let toggle = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(toggle.contains("std::optional<std::string_view> mode;"));
    assert!(toggle.contains("#include <string_view>"));
    assert!(toggle.contains("#include \"cage_enums.h\""));
    let enums = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(enums.contains("inline constexpr std::string_view on{\"on\"};"));
    assert!(enums.contains("inline constexpr std::string_view off{\"off\"};"));
}

/// Map-typed schema: string and int keys, scalar / array / nested-map /
/// enum value types, an Object member to prove `<map>` and
/// `<unordered_map>` coexist, plus defaults on both the good and bad
/// paths (empty map, non-empty entries, non-scalar values).
const MAP_SCHEMA: &str = r#"
tables:
  Board:
    name: Board
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      counts: { name: counts, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: { gold: 10, silver: 2 } }
      byRank: { name: byRank, type: { kind: Map, value: { key_type: int, value_type: { kind: String } } }, default: { "1": one, "2": two } }
      slots: { name: slots, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } } }
      layers: { name: layers, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: int, value_type: { kind: Bool } } } } } }
      rates: { name: rates, type: { kind: Map, value: { key_type: string, value_type: { kind: Float64 } } }, default: {} }
      kinds: { name: kinds, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: ItemKind } } } }
      labels: { name: labels, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: Rarity } } } }
      meta: { name: meta, type: { kind: Object, value: {} } }
      badSlots: { name: badSlots, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, default: { a: [1] } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
  Rarity:
    name: Rarity
    values:
      - { name: common }
      - { name: rare }
"#;

const MAP_MAIN_CPP: &str = r#"#include "Board.h"
#include "cage_enums.h"

int main() {
    cage::generated::Board b{};
    b.id = 1;
    b.counts["gold"] = 3;
    b.byRank[1] = "one";
    b.rates["x"] = 0.5;
    b.slots.value()["row"] = {1, 2};
    b.layers.value()["grid"][7] = true;
    b.kinds.value()["k"] = cage::generated::ItemKind::Sword;
    b.labels.value()["l"] = cage::generated::Rarity::common;
    (void)b.meta;
    (void)b.badSlots;
    return 0;
}
"#;

#[test]
fn test_map_field_type_mapping() {
    let schema: Schema = serde_yaml::from_str(MAP_SCHEMA).expect("map schema");
    let artifacts = gen().generate(&schema, Some("map77"));
    assert_eq!(artifacts.len(), 2);
    let board = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // `<unordered_map>` is injected on demand and keeps its place in the
    // sorted angle-include block — next to, but distinct from, the `<map>`
    // an Object member pulls in.
    assert!(board.contains(
            "#include <any>\n#include <cstdint>\n#include <map>\n#include <optional>\n#include <string>\n#include <string_view>\n#include <unordered_map>\n#include <vector>\n"
        ));
    assert!(board.contains("#include \"cage_enums.h\""));

    // Key mapping (String → std::string, Int → std::int64_t) and value
    // recursion through the ordinary rules: arrays, nested maps, and enum
    // values keep the spelling they would have as plain field types.
    for expected in [
            "std::unordered_map<std::string, std::int32_t> counts{{\"gold\", 10}, {\"silver\", 2}};",
            "std::unordered_map<std::int64_t, std::string> byRank{{1, \"one\"}, {2, \"two\"}};",
            "std::optional<std::unordered_map<std::string, std::vector<std::int32_t>>> slots;",
            "std::optional<std::unordered_map<std::string, std::unordered_map<std::int64_t, bool>>> layers;",
            "std::unordered_map<std::string, double> rates{};",
            "std::optional<std::unordered_map<std::string, ItemKind>> kinds;",
            "std::optional<std::unordered_map<std::string, std::string_view>> labels;",
            "std::optional<std::map<std::string, std::any>> meta;",
            // Non-scalar value type: the default is skipped and the member
            // goes optional without an initializer (nullability unchanged).
            "std::optional<std::unordered_map<std::string, std::vector<std::int32_t>>> badSlots;",
            "std::int32_t id{};",
        ] {
            assert!(board.contains(expected), "missing: {expected}");
        }
}

#[test]
fn test_map_default_rendering_rules() {
    let schema = Schema::new();
    let map = |key: MapKeyType, value: FieldType| {
        FieldType::Map(MapField {
            key_type: key,
            value_type: Box::new(value),
        })
    };
    // Empty object default → the empty brace-init list.
    assert_eq!(
        render_default(
            &serde_json::json!({}),
            &map(MapKeyType::String, FieldType::Float64),
            &schema
        )
        .unwrap(),
        "{}"
    );
    // Int keys arrive as numeric strings and literalize as integers.
    assert_eq!(
        render_default(
            &serde_json::json!({"-7": "x"}),
            &map(MapKeyType::Int, FieldType::String),
            &schema
        )
        .unwrap(),
        "{{-7, \"x\"}}"
    );
    // Non-object payload → kind mismatch, no default.
    assert!(render_default(
        &serde_json::json!(5),
        &map(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
    // Non-scalar value types (arrays, enums) are skipped, like objects.
    assert!(render_default(
        &serde_json::json!({"a": [1]}),
        &map(
            MapKeyType::String,
            FieldType::Array(Box::new(FieldType::Int32))
        ),
        &schema
    )
    .is_none());
    assert!(render_default(
        &serde_json::json!({"a": "Sword"}),
        &map(MapKeyType::String, FieldType::Enum("ItemKind".to_string())),
        &schema
    )
    .is_none());
    // A non-numeric int key (L2 would reject it) skips the default.
    assert!(render_default(
        &serde_json::json!({"x": 1}),
        &map(MapKeyType::Int, FieldType::Int32),
        &schema
    )
    .is_none());
    // A value-kind mismatch inside an otherwise valid entry skips it too.
    assert!(render_default(
        &serde_json::json!({"a": "s"}),
        &map(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
}

#[test]
fn test_render_default_edge_cases() {
    let schema = Schema::new();
    // Float literals always keep a decimal point.
    assert_eq!(
        render_default(&serde_json::json!(100.0), &FieldType::Float64, &schema).unwrap(),
        "100.0"
    );
    assert_eq!(
        render_default(&serde_json::json!(1.5), &FieldType::Float64, &schema).unwrap(),
        "1.5"
    );
    assert_eq!(
        render_default(&serde_json::json!(true), &FieldType::Bool, &schema).unwrap(),
        "true"
    );
    // Extreme integers stay exact (compile-safe spellings).
    assert_eq!(
        render_default(&serde_json::json!(i64::MIN), &FieldType::Int64, &schema).unwrap(),
        "-9223372036854775807 - 1"
    );
    assert_eq!(
        render_default(&serde_json::json!(u64::MAX), &FieldType::UInt64, &schema).unwrap(),
        "18446744073709551615ULL"
    );
    // Non-finite floats render through <numeric_limits> (reachable only
    // in the literal helper — serde_json cannot carry non-finite numbers
    // in a schema default, so those schema lookups yield None).
    assert_eq!(
        cpp_float_literal(f64::NAN, false),
        "std::numeric_limits<double>::quiet_NaN()"
    );
    assert_eq!(
        cpp_float_literal(f64::INFINITY, false),
        "std::numeric_limits<double>::infinity()"
    );
    assert_eq!(
        cpp_float_literal(-f64::INFINITY, false),
        "-std::numeric_limits<double>::infinity()"
    );
    assert_eq!(
        cpp_float_literal(f64::NAN, true),
        "std::numeric_limits<float>::quiet_NaN()"
    );
    assert!(render_default(&serde_json::json!(f64::NAN), &FieldType::Float64, &schema).is_none());
    assert!(render_default(
        &serde_json::json!(f64::INFINITY),
        &FieldType::Float64,
        &schema
    )
    .is_none());
    // Kind mismatch → no default.
    assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
    // Object defaults are never rendered (shared rule across targets).
    assert!(render_default(
        &serde_json::json!({"a": 1}),
        &FieldType::Object(indexmap::IndexMap::default()),
        &schema
    )
    .is_none());
    // Bytes / enum kinds are never rendered either.
    assert!(render_default(&serde_json::json!("x"), &FieldType::Bytes, &schema).is_none());
    assert!(render_default(
        &serde_json::json!("Sword"),
        &FieldType::Enum("ItemKind".to_string()),
        &schema
    )
    .is_none());
    // Array defaults: initializer list of scalar elements only.
    assert_eq!(
        render_default(
            &serde_json::json!(["pvp", "duo"]),
            &FieldType::Array(Box::new(FieldType::String)),
            &schema
        )
        .unwrap(),
        "{\"pvp\", \"duo\"}"
    );
    assert!(render_default(
        &serde_json::json!([{"a": 1}]),
        &FieldType::Array(Box::new(FieldType::Object(indexmap::IndexMap::default()))),
        &schema
    )
    .is_none());
    // Control characters use 3-digit octal escapes (C++ \x is greedy).
    let ctrl = format!("a{}b", char::from(1u8));
    assert_eq!(cpp_string_literal(&ctrl), "\"a\\001b\"");
    assert_eq!(
        cpp_string_literal("tab\t\"q\"\\z"),
        "\"tab\\t\\\"q\\\"\\\\z\""
    );
    assert_eq!(cpp_string_literal("n\nr\r"), "\"n\\nr\\r\"");
}

/// g++ availability probe — the compile test below is a no-op (never a
/// failure) on machines without a toolchain.
fn gpp_available() -> bool {
    std::process::Command::new("g++")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Compiler sanity: generate every fixture schema and syntax-check the
/// headers with g++ (`-Wall -Wextra -Werror`). Skipped, not failed, when
/// no g++ is installed.
#[test]
fn test_generated_headers_compile() {
    if !gpp_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("cage-cpp-sample-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let basics = root.join("basic");
    let extremes = root.join("extremes");
    std::fs::create_dir_all(&basics).unwrap();
    std::fs::create_dir_all(&extremes).unwrap();

    write_headers(&basics, &gen().generate(&test_schema(), Some("abc123")));
    std::fs::write(basics.join("main.cpp"), BASIC_MAIN_CPP).unwrap();
    run_gpp(&basics, "main.cpp");

    let schema: Schema = serde_yaml::from_str(EXTREMES_SCHEMA).expect("extremes schema");
    write_headers(&extremes, &gen().generate(&schema, Some("def456")));
    std::fs::write(extremes.join("main.cpp"), EXTREMES_MAIN_CPP).unwrap();
    run_gpp(&extremes, "main.cpp");

    let maps = root.join("maps");
    std::fs::create_dir_all(&maps).unwrap();
    let schema: Schema = serde_yaml::from_str(MAP_SCHEMA).expect("map schema");
    write_headers(&maps, &gen().generate(&schema, Some("map77")));
    std::fs::write(maps.join("main.cpp"), MAP_MAIN_CPP).unwrap();
    run_gpp(&maps, "main.cpp");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_extremes_schema_rendering() {
    let schema: Schema = serde_yaml::from_str(EXTREMES_SCHEMA).expect("extremes schema");
    let artifacts = gen().generate(&schema, Some("def456"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "build/cpp/Edge.h",
            "build/cpp/NoKey.h",
            "build/cpp/Plain.h",
            "build/cpp/cage_enums.h",
        ]
    );

    // Edge: every include group at once — <limits> pulled in by a string
    // default whose text trips the numeric_limits guard — and the full
    // member set with extreme literals.
    let edge = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(edge.contains(
            "#include <any>\n#include <cstdint>\n#include <limits>\n#include <map>\n#include <optional>\n#include <string>\n#include <string_view>\n#include <vector>\n"
        ));
    assert!(edge.contains("#include \"cage_enums.h\""));
    for expected in [
        "std::uint64_t id{18446744073709551615ULL};",
        "std::int64_t big{-9223372036854775807 - 1};",
        "std::optional<std::string_view> mode;",
        "std::optional<std::map<std::string, std::any>> meta;",
        "std::optional<std::any> raw;",
        "std::optional<std::any> anyx;",
        "std::int8_t i8{-128};",
        "std::int16_t i16{-32768};",
        "std::uint8_t u8x{200};",
        "std::uint16_t u16x{65535};",
        "std::uint32_t u32x{4294967295};",
        "std::uint64_t u64mid{42};",
        "float f32{1.5};",
        "bool flag{true};",
        "std::optional<std::vector<std::uint8_t>> blob;",
        "std::int32_t qty{};",
        "std::optional<std::string> code;",
        "std::string esc{\"a\\nb\\rc\"};",
        "std::string lim{\"std::numeric_limits<int>::max()\"};",
    ] {
        assert!(edge.contains(expected), "missing: {expected}");
    }
    assert!(edge.contains("    // Stock, required, max: 99"));
    assert!(edge.contains("    // min_length: 1, max_length: 10, pattern: ^[a-z]+$"));

    // Empty primary key: the banner is the bare struct name.
    let nokey = String::from_utf8(artifacts[1].1.clone()).unwrap();
    assert!(nokey.contains("// NoKey\n"));
    assert!(!nokey.contains("primary key"));
    assert!(nokey.contains("std::int32_t id{};"));

    // Only bool/float members: no system include is needed at all.
    let plain = String::from_utf8(artifacts[2].1.clone()).unwrap();
    assert!(!plain.contains("#include"));
    assert!(plain.contains("// Plain — primary key: id"));

    // Enums: description banners, range-selected backings (int64/uint64),
    // i64::MIN spelling, ULL suffix, and the mixed string bucket with a
    // trailing comment on the described member.
    let enums = String::from_utf8(artifacts[3].1.clone()).unwrap();
    assert!(enums.contains("#include <cstdint>\n#include <string_view>\n"));
    assert!(enums.contains("// Labeled — Values past the 32-bit range."));
    assert!(enums.contains("enum class Labeled : std::int64_t {"));
    assert!(enums.contains("    Large = 3000000000,"));
    assert!(enums.contains("enum class EdgeEnum : std::int64_t {"));
    assert!(enums.contains("    Max = 9223372036854775807,"));
    assert!(enums.contains("    Min = -9223372036854775807 - 1,"));
    assert!(enums.contains("enum class Huge : std::uint64_t {"));
    assert!(enums.contains("    Top = 18446744073709551615ULL,"));
    assert!(enums.contains("namespace MixKind {"));
    assert!(enums.contains("inline constexpr std::string_view S{\"x y\"};  // spaced"));
    assert!(enums.contains("inline constexpr std::string_view N{\"42\"};"));
    assert!(enums.contains("inline constexpr std::string_view T{\"true\"};"));
}

#[test]
fn test_write_headers_writes_relative_paths() {
    let root = std::env::temp_dir().join(format!("cage-cpp-headers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write_headers(&root, &gen().generate(&test_schema(), Some("abc123")));
    // The `build/cpp` prefix is stripped; file names land at the root.
    for name in ["Drop.h", "Item.h", "cage_enums.h"] {
        assert!(root.join(name).is_file(), "missing {name}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_from_config_defaults_without_options() {
    let config: TargetConfig = serde_yaml::from_str(
        r"
format: cpp
output_dir: build/game
",
    )
    .expect("target config");
    let gen = CppTargetGenerator::from_config(&config);
    // No `options` block → every generator knob keeps its default.
    assert_eq!(gen.output_dir, PathBuf::from("build/game"));
    assert_eq!(gen.file_template, "{table}.h");
    assert_eq!(gen.enums_file, "cage_enums.h");
    assert_eq!(gen.namespace, "cage::generated");
}

#[test]
fn test_enums_context_empty_slice() {
    // Defensive boundary: an empty enum set yields no includes and no
    // enum entries (generate never calls it this way).
    let ctx = super::CppTargetGenerator::cpp_enums_context("cage.generated", &[]);
    assert!(ctx["angle"].as_array().is_some_and(Vec::is_empty));
    assert!(ctx["emitted_enums"].as_array().is_some_and(Vec::is_empty));
}

#[test]
fn test_cpp_uint_literal_small_values_stay_plain() {
    // u64 values that fit a signed type keep the plain decimal spelling;
    // only values above i64::MAX need the ULL suffix (schema defaults
    // reach the u64 branch only above i64::MAX, so the small side is
    // pinned directly).
    assert_eq!(cpp_uint_literal(42), "42");
    assert_eq!(cpp_uint_literal(i64::MAX as u64), "9223372036854775807");
    assert_eq!(cpp_uint_literal(u64::MAX), "18446744073709551615ULL");
}

#[test]
fn test_integral_value_literal_fallbacks() {
    let member = |v: Option<serde_json::Value>| EnumValue {
        name: "Only".to_string(),
        value: v,
        description: None,
    };
    // A float payload keeps its decimal spelling; a value-less member
    // falls back to a string literal (both are kept out of the numeric
    // bucket by is_integral_enum, so the arms are pinned directly).
    assert_eq!(
        integral_value_literal(&member(Some(serde_json::json!(1.5)))),
        "1.5"
    );
    assert_eq!(integral_value_literal(&member(None)), "\"Only\"");
    assert_eq!(
        integral_value_literal(&member(Some(serde_json::json!(7)))),
        "7"
    );
}

#[test]
fn test_enum_backing_buckets() {
    let member = |v: Option<serde_json::Value>| EnumValue {
        name: "Only".to_string(),
        value: v,
        description: None,
    };
    let e = |values: Vec<EnumValue>| EnumSchema {
        name: "Kind".to_string(),
        values,
        description: None,
    };
    // Values pick the smallest signed type that fits both ends.
    assert_eq!(
        enum_backing(&e(vec![member(Some(1.into())), member(Some(2.into()))])),
        "std::int32_t"
    );
    assert_eq!(
        enum_backing(&e(vec![member(Some(9_000_000_000i64.into()))])),
        "std::int64_t"
    );
    assert_eq!(
        enum_backing(&e(vec![member(Some(u64::MAX.into()))])),
        "std::uint64_t"
    );
    // Non-numeric members are skipped when sizing: floats keep the
    // decimal spelling, value-less members fall back to string literals
    // (both stay out of the numeric bucket via is_integral_enum, so the
    // arms are pinned directly).
    assert_eq!(
        enum_backing(&e(vec![member(Some(1.5.into())), member(Some(3.into()))])),
        "std::int32_t"
    );
    assert_eq!(
        enum_backing(&e(vec![member(None), member(Some(3.into()))])),
        "std::int32_t"
    );
}

#[test]
fn test_default_filter_null_without_default() {
    use tera::Filter;
    let filter = CppDefaultFilter {
        schema: test_schema(),
    };
    // `note` carries no default: the filter yields null and the template
    // omits the initializer.
    let field = serde_json::to_value(&test_schema().tables["Item"].fields["note"]).unwrap();
    let out = filter.filter(&field, &HashMap::new()).unwrap();
    assert_eq!(out, serde_json::Value::Null);
}

fn write_headers(root: &Path, artifacts: &[(String, Vec<u8>)]) {
    for (path, bytes) in artifacts {
        // Path::strip_prefix (component-aware), not str::strip_prefix:
        // the str version returns the raw remainder "/Drop.h", which
        // PathBuf::join would then treat as absolute.
        let rel = Path::new(path.as_str())
            .strip_prefix("build/cpp")
            .expect("default output dir");
        let dst = root.join(rel);
        std::fs::write(&dst, bytes).unwrap_or_else(|e| panic!("write {}: {e}", dst.display()));
    }
}

fn run_gpp(dir: &Path, source: &str) {
    let out = std::process::Command::new("g++")
        .args([
            "-std=c++17",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
            source,
            "-I.",
        ])
        .current_dir(dir)
        .output()
        .expect("g++ must be on PATH");
    assert!(
        out.status.success(),
        "g++ {source} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
