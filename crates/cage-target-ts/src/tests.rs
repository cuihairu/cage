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
                "{{ tables.Item.fields.price | ts_type }}|{{ tables.Item.fields.price | ts_default }}|{{ tables.Item.fields.kind | ts_type }}",
                &ctx,
            )
            .unwrap();
    assert_eq!(out, "number|10|ItemKind");
}

use super::docfmt::jsdoc_desc_safe;
use super::idents::{sanitize_ident, ts_ident, unique_ident};
use super::literals::{render_default, ts_float_literal, ts_string_literal};
use super::types::ts_type;
use cage_core::schema::{FieldType, MapField, MapKeyType};
use tera::Tera;

use super::*;

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
/// length/pattern constraints, an enum with a description, and an
/// enum whose members are not all integral (string bucket).
fn edge_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Ghost:
    name: Ghost
    primary_key: []
    fields: {}
  Limits:
    name: Limits
    primary_key: [key]
    fields:
      key: { name: key, type: { kind: String }, required: true }
      count: { name: count, type: { kind: Int32 }, max: 100, description: Bounded count }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 8, pattern: '^[A-Z]+$' }
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
      - { name: one, value: first }
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

/// Schema exercising the Map field type: string and int keys, array /
/// nested-map / enum value types, a renderable empty default and a
/// non-empty scalar-member default.
fn map_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Loot:
    name: Loot
    description: Loot table.
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      bonuses: { name: bonuses, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, required: true, description: Bonus tables }
      cooldowns: { name: cooldowns, type: { kind: Map, value: { key_type: int, value_type: { kind: Int64 } } }, description: Per-level cooldown }
      grid: { name: grid, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } }, required: true }
      labels: { name: labels, type: { kind: Map, value: { key_type: string, value_type: { kind: String } } }, default: {} }
      scores: { name: scores, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: { easy: 1, hard: 7 } }
      tier: { name: tier, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: Rarity } } }, required: true }
enums:
  Rarity:
    name: Rarity
    values:
      - { name: common }
      - { name: rare }
",
        )
        .expect("map schema must parse")
}

/// Map defaults that must NOT render: a non-object default, a non-numeric
/// key on an int-keyed map, one unrenderable member, and a non-scalar
/// value type.
fn bad_map_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  BadMap:
    name: BadMap
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      scalar: { name: scalar, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: oops }
      bad_key: { name: bad_key, type: { kind: Map, value: { key_type: int, value_type: { kind: Int32 } } }, default: { x: 1 } }
      bad_value: { name: bad_value, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: { a: 1, b: nope } }
      nonscalar: { name: nonscalar, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, default: { a: [1] } }
enums: {}
",
        )
        .expect("bad map schema must parse")
}

fn gen() -> TsTargetGenerator {
    TsTargetGenerator::default()
}

fn js_gen(output_dir: &str) -> TsTargetGenerator {
    let config: TargetConfig = serde_yaml::from_str(&format!(
        r"
format: javascript
output_dir: {output_dir}
"
    ))
    .expect("target config");
    TsTargetGenerator::from_config(&config)
}

fn src(artifacts: &[(String, Vec<u8>)], idx: usize) -> String {
    String::from_utf8(artifacts[idx].1.clone()).expect("utf-8")
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
            "build/typescript/Drop.ts",
            "build/typescript/Item.ts",
            "build/typescript/cage_enums.ts",
        ]
    );
}

#[test]
fn test_javascript_artifact_order() {
    let schema = test_schema();
    // Default emit_dts = true: each table pairs .js with .d.ts.
    let artifacts = js_gen("build/js").generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "build/js/Drop.js",
            "build/js/Drop.d.ts",
            "build/js/Item.js",
            "build/js/Item.d.ts",
            "build/js/cage_enums.js",
            "build/js/cage_enums.d.ts",
        ]
    );

    // emit_dts: false — the declaration entries vanish.
    let config: TargetConfig = serde_yaml::from_str(
        r"
format: javascript
output_dir: build/js
options:
  emit_dts: false
",
    )
    .expect("target config");
    let artifacts = TsTargetGenerator::from_config(&config).generate(&schema, Some("abc123"));
    let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "build/js/Drop.js",
            "build/js/Item.js",
            "build/js/cage_enums.js",
        ]
    );
}

#[test]
fn test_table_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let item = src(&artifacts, 1);

    assert!(item.contains("//   schema: abc123"));
    assert!(item.contains("//   table:  Item"));
    assert!(item.contains("//   Generated by Cage — do not edit."));
    // Type-only import of the enum this table uses.
    assert!(item.contains("import type { ItemKind } from \"./cage_enums\";"));
    // Header docs: description line + primary-key banner.
    assert!(item.contains(" * Equipment definitions."));
    assert!(item.contains(" * Item — primary key: id"));

    // Interface: name order, optionality per required-or-default rule.
    let id_pos = item.find("  id: number;\n").unwrap();
    let kind_pos = item.find("  kind?: ItemKind;\n").unwrap();
    let name_pos = item.find("  name: string;\n").unwrap();
    let note_pos = item.find("  note?: string;\n").unwrap();
    let owner_pos = item.find("  owner?: string;\n").unwrap();
    let price_pos = item.find("  price?: number;\n").unwrap();
    let rarity_pos = item.find("  rarity?: string;\n").unwrap();
    let tags_pos = item.find("  tags: string[];\n").unwrap();
    let weight_pos = item.find("  weight: number;\n").unwrap();
    assert!(id_pos < kind_pos);
    assert!(kind_pos < name_pos);
    assert!(name_pos < note_pos);
    assert!(note_pos < owner_pos);
    assert!(owner_pos < price_pos);
    assert!(price_pos < rarity_pos);
    assert!(rarity_pos < tags_pos);
    assert!(tags_pos < weight_pos);
    assert!(item.contains("export interface Item {"));

    // Member docs (shared field_doc format).
    assert!(item.contains("/** Identifier, required */"));
    assert!(item.contains("/** Display name, required */"));
    assert!(item.contains("/** Price in gold, min: 0 */"));
    assert!(item.contains("allowed: common | rare"));
    assert!(item.contains("→ Player.id"));

    // Defaults const + factory (same literals, name order).
    assert!(item.contains("export const ItemDefaults: Partial<Item> = {"));
    assert!(item.contains("  tags: [\"pvp\"],"));
    assert!(item.contains("  weight: 1.5,"));
    assert!(item.contains("export function newItem(init: Partial<Item> = {}): Item {"));
    assert!(item.contains("  return { tags: [\"pvp\"], weight: 1.5, ...init } as Item;"));
    // Fields without renderable defaults stay out of the const.
    assert!(!item.contains("  note: "));
    assert!(!item.contains("  price: "));
}

#[test]
fn test_empty_defaults_still_emit_members() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let drop = src(&artifacts, 0);
    // Even with no defaults: `= {}` and `{ ...init }`.
    assert!(drop.contains("export const DropDefaults: Partial<Drop> = {};"));
    assert!(drop.contains("  return { ...init } as Drop;"));
    // No import when nothing resolves.
    assert!(!drop.contains("import type"));
}

#[test]
fn test_map_types_ts() {
    let schema = map_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let loot = src(&artifacts, 0);

    assert!(loot.contains("export interface Loot {"));
    // K/V spellings: string and int keys, array / nested-map / enum values.
    assert!(loot.contains("  bonuses: Map<string, number[]>;"));
    assert!(loot.contains("  cooldowns?: Map<number, number>;"));
    assert!(loot.contains("  grid: Map<string, Map<string, number>>;"));
    assert!(loot.contains("  labels: Map<string, string>;"));
    assert!(loot.contains("  scores: Map<string, number>;"));
    assert!(loot.contains("  tier: Map<string, Rarity>;"));
    // The enum reached through the map value is imported like any other.
    assert!(loot.contains("import type { Rarity } from \"./cage_enums\";"));
    // Only the int-keyed map (no default) is optional; the empty-object
    // default counts as a renderable default, so `labels` stays required.
    assert!(loot.contains("  labels: Map<string, string>;\n") && !loot.contains("  labels?:"));
    assert_eq!(loot.matches("?:").count(), 1);
}

#[test]
fn test_map_defaults_new_map_ts() {
    let schema = map_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let loot = src(&artifacts, 0);

    // Map defaults are `new Map(...)` entries, never object literals.
    assert!(loot.contains("export const LootDefaults: Partial<Loot> = {"));
    assert!(loot.contains("  labels: new Map(),"));
    assert!(loot.contains("  scores: new Map([[\"easy\", 1], [\"hard\", 7]]),"));
    assert!(!loot.contains("labels: {},"));
    // The factory inlines fresh maps so rows never share one instance.
    assert!(loot.contains("export function newLoot(init: Partial<Loot> = {}): Loot {"));
    assert!(loot.contains(
            "  return { labels: new Map(), scores: new Map([[\"easy\", 1], [\"hard\", 7]]), ...init } as Loot;"
        ));
}

#[test]
fn test_map_jsdoc_typedef() {
    let schema = map_schema();
    let artifacts = js_gen("build/js").generate(&schema, Some("abc123"));
    let js = src(&artifacts, 0);

    // JSDoc types carry the same Map spellings; enums inline-import.
    assert!(js.contains(" * @typedef {Object} Loot"));
    assert!(js.contains(" * @property {Map<string, number[]>} bonuses Bonus tables"));
    assert!(js.contains(" * @property {Map<number, number>} [cooldowns] Per-level cooldown"));
    assert!(js.contains(" * @property {Map<string, Map<string, number>>} grid"));
    assert!(js.contains(" * @property {Map<string, string>} labels"));
    assert!(js.contains(" * @property {Map<string, number>} scores"));
    assert!(js.contains(" * @property {Map<string, import(\"./cage_enums.js\").Rarity>} tier"));
    // Defaults/factory use the same `new Map(...)` expressions.
    assert!(js.contains("export const LootDefaults = {"));
    assert!(js.contains("  labels: new Map(),"));
    assert!(js.contains("  scores: new Map([[\"easy\", 1], [\"hard\", 7]]),"));
    assert!(js.contains(
        "  return { labels: new Map(), scores: new Map([[\"easy\", 1], [\"hard\", 7]]), ...init };"
    ));
}

#[test]
fn test_map_dts_declarations() {
    let schema = map_schema();
    let artifacts = js_gen("build/js").generate(&schema, Some("abc123"));
    let dts = src(&artifacts, 1);

    // The .d.ts pair restates the TS-mode surface with real Map types.
    assert!(dts.contains("import type { Rarity } from \"./cage_enums\";"));
    assert!(dts.contains("export interface Loot {"));
    assert!(dts.contains("  bonuses: Map<string, number[]>;"));
    assert!(dts.contains("  cooldowns?: Map<number, number>;"));
    assert!(dts.contains("  grid: Map<string, Map<string, number>>;"));
    assert!(dts.contains("  tier: Map<string, Rarity>;"));
    assert!(dts.contains("export declare const LootDefaults: Partial<Loot>;"));
    assert!(dts.contains("export declare function newLoot(init?: Partial<Loot>): Loot;"));
}

#[test]
fn test_map_default_mismatch_skipped() {
    let schema = bad_map_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let out = src(&artifacts, 0);

    // No mismatched default survives: empty Defaults const + factory.
    assert!(out.contains("export const BadMapDefaults: Partial<BadMap> = {};"));
    assert!(out.contains("  return { ...init } as BadMap;"));
    assert!(!out.contains("new Map"));
    // Every dropped default leaves an optional member instead.
    assert!(out.contains("  bad_key?: Map<number, number>;"));
    assert!(out.contains("  bad_value?: Map<string, number>;"));
    assert!(out.contains("  nonscalar?: Map<string, number[]>;"));
    assert!(out.contains("  scalar?: Map<string, number>;"));
}

#[test]
fn test_enums_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let enums_src = src(&artifacts, 2);

    assert!(enums_src.contains("//   schema: abc123"));
    assert!(enums_src.contains("//   enums:  shared definitions"));
    assert!(enums_src.contains("// ItemKind"));
    // Numeric bucket: `as const` object + derived type alias.
    assert!(enums_src.contains("export const ItemKind = {"));
    assert!(enums_src.contains("  Sword: 1, // Sword weapon"));
    assert!(enums_src.contains("  Shield: 2,"));
    assert!(enums_src.contains("} as const;"));
    assert!(enums_src.contains("export type ItemKind = (typeof ItemKind)[keyof typeof ItemKind];"));
    // String bucket: value-less members fall back to their name.
    assert!(enums_src.contains("// Rarity"));
    assert!(enums_src.contains("export const Rarity = {"));
    assert!(enums_src.contains("  common: \"common\","));
    assert!(enums_src.contains("  rare: \"rare\","));
    assert!(enums_src.contains("export type Rarity = (typeof Rarity)[keyof typeof Rarity];"));
    // Empty enums are not emitted.
    assert!(!enums_src.contains("EmptyEnum"));
}

#[test]
fn test_unresolved_enum_falls_back() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, None);
    let drop = src(&artifacts, 0);
    assert!(drop.contains("//   schema: (unavailable)"));
    assert!(drop.contains("item?: string;"));
    assert!(drop.contains("unresolved enum: MissingEnum"));
}

#[test]
fn test_empty_table_renders_bare_interface() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    // Tables in name order: Ghost, Limits, then the enums module.
    let ghost = src(&artifacts, 0);
    assert!(ghost.contains("export interface Ghost {}"));
    assert!(ghost.contains("export const GhostDefaults: Partial<Ghost> = {};"));
    assert!(ghost.contains("  return { ...init } as Ghost;"));
    // No JSDoc member docs on the typedef either.
    assert!(!ghost.contains("@property"));

    // JS mode: typedef with no @property rows + paired empty interface.
    let artifacts = js_gen("build/js").generate(&edge_schema(), Some("abc123"));
    let ghost_js = src(&artifacts, 0);
    assert!(ghost_js.contains(" * @typedef {Object} Ghost"));
    assert!(!ghost_js.contains("@property"));
    let ghost_dts = src(&artifacts, 1);
    assert!(ghost_dts.contains("export interface Ghost {}"));
    assert!(ghost_dts.contains("export declare function newGhost(init?: Partial<Ghost>): Ghost;"));
}

#[test]
fn test_enum_description_and_mixed_value_buckets() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let enums_src = src(&artifacts, 2);
    // Enum-level description comment.
    assert!(enums_src.contains("// Described — A described enum."));
    // Non-integral members stringify through enum_string_value.
    assert!(enums_src.contains("  one: \"first\","));
    assert!(enums_src.contains("  two: \"2.5\","));
    assert!(enums_src.contains("  yes: \"true\","));
    assert!(enums_src.contains("  no: \"false\","));
    assert!(enums_src.contains("  bare: \"bare\","));
    // u64 above i64::MAX stays in the integral bucket.
    assert!(enums_src.contains("  Max: 18446744073709551615,"));

    // JS mode: same description in the paired enums declaration.
    let artifacts = js_gen("build/js").generate(&edge_schema(), Some("abc123"));
    let enums_dts = src(&artifacts, 5);
    assert!(enums_dts.contains("// Described — A described enum."));
    assert!(enums_dts.contains(
        "export declare const Mixed: { readonly one: \"first\"; readonly two: \"2.5\"; \
             readonly yes: \"true\"; readonly no: \"false\"; readonly bare: \"bare\" };"
    ));
    assert!(
        enums_dts.contains("export declare const Huge: { readonly Max: 18446744073709551615 };")
    );
}

#[test]
fn test_length_pattern_constraints_in_member_docs() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let limits = src(&artifacts, 1);
    assert!(limits.contains("/** Bounded count, max: 100 */"));
    assert!(limits.contains("/** min_length: 1, max_length: 8, pattern: ^[A-Z]+$ */"));
    // u64 default above i64::MAX renders via the or_else arm; a float
    // default on an int field renders nothing (member stays optional).
    assert!(limits.contains("  big: number;"));
    assert!(limits.contains("  big: 18446744073709551615,"));
    assert!(limits.contains("  frac?: number;"));
    assert!(!limits.contains("  frac: "));
    assert!(limits.contains("  count?: number;"));
    assert!(limits.contains("  code?: string;"));
}

#[test]
fn test_deterministic_output() {
    let schema = test_schema();
    for gen in [gen(), js_gen("build/js")] {
        let a = gen.generate(&schema, Some("abc123"));
        let b = gen.generate(&schema, Some("abc123"));
        assert_eq!(a.len(), b.len());
        for ((pa, ca), (pb, cb)) in a.iter().zip(b.iter()) {
            assert_eq!(pa, pb);
            assert_eq!(ca, cb);
        }
    }
}

#[test]
fn test_javascript_table_rendering() {
    let schema = test_schema();
    let artifacts = js_gen("build/js").generate(&schema, Some("abc123"));
    let item = src(&artifacts, 2);

    assert!(item.contains("//   table:  Item"));
    // JSDoc typedef block: header docs + @typedef + @property rows.
    assert!(item.contains(" * Equipment definitions."));
    assert!(item.contains(" * Item — primary key: id"));
    assert!(item.contains(" * @typedef {Object} Item"));
    assert!(item.contains(" * @property {number} id Identifier, required"));
    assert!(item.contains(" * @property {string} name Display name, required"));
    // Optional members get [brackets].
    assert!(item.contains(" * @property {number} [price] Price in gold, min: 0"));
    assert!(item.contains(" * @property {string} [note]"));
    // A doc starting with `→` (non-ASCII) after `[name]` trips the
    // JSDoc scanner (TS1127), so it gets a continuation line instead.
    assert!(item.contains(" * @property {string} [owner]\n * → Player.id"));
    assert!(item.contains(" * @property {string} [rarity] allowed: common | rare"));
    assert!(item.contains(" * @property {string[]} tags"));
    assert!(item.contains(" * @property {number} weight"));
    // Enum types go through inline JSDoc import() — no runtime imports.
    assert!(item.contains(" * @property {import(\"./cage_enums.js\").ItemKind} [kind]"));
    assert!(!item.contains("import type"));
    assert!(!item.contains("export interface"));

    // Plain-object defaults + untyped factory.
    assert!(item.contains("export const ItemDefaults = {"));
    assert!(item.contains("  tags: [\"pvp\"],"));
    assert!(item.contains("  weight: 1.5,"));
    assert!(item.contains("export function newItem(init = {}) {"));
    assert!(item.contains("  return { tags: [\"pvp\"], weight: 1.5, ...init };"));

    let drop = src(&artifacts, 0);
    assert!(drop.contains("export const DropDefaults = {};"));
    assert!(drop.contains("export function newDrop(init = {}) {"));
    assert!(drop.contains("  return { ...init };"));
    assert!(drop.contains(" * @property {string} [item] unresolved enum: MissingEnum"));
}

#[test]
fn test_javascript_enums_and_dts() {
    let schema = test_schema();
    let artifacts = js_gen("build/js").generate(&schema, Some("abc123"));

    let enums_src = src(&artifacts, 4);
    assert!(enums_src.contains("export const ItemKind = Object.freeze({"));
    assert!(enums_src.contains("  Sword: 1, // Sword weapon"));
    assert!(enums_src.contains("  Shield: 2,"));
    assert!(enums_src.contains("/** @typedef {number} ItemKind */"));
    assert!(enums_src.contains("export const Rarity = Object.freeze({"));
    assert!(enums_src.contains("  common: \"common\","));
    assert!(enums_src.contains("/** @typedef {string} Rarity */"));
    assert!(!enums_src.contains("EmptyEnum"));

    // Paired table declaration: real interface + declare members.
    let dts = src(&artifacts, 3);
    assert!(dts.contains("//   table:  Item"));
    assert!(dts.contains("import type { ItemKind } from \"./cage_enums\";"));
    assert!(dts.contains("export interface Item {"));
    assert!(dts.contains("  kind?: ItemKind;"));
    assert!(dts.contains("  price?: number;"));
    assert!(dts.contains("export declare const ItemDefaults: Partial<Item>;"));
    assert!(dts.contains("export declare function newItem(init?: Partial<Item>): Item;"));
    assert!(!dts.contains("Object.freeze"));

    // Paired enums declaration: literal object type per bucket.
    let enums_dts = src(&artifacts, 5);
    assert!(enums_dts
        .contains("export declare const ItemKind: { readonly Sword: 1; readonly Shield: 2 };"));
    assert!(enums_dts.contains("export type ItemKind = (typeof ItemKind)[keyof typeof ItemKind];"));
    assert!(enums_dts.contains(
        "export declare const Rarity: { readonly common: \"common\"; readonly rare: \"rare\" };"
    ));
    assert!(!enums_dts.contains("EmptyEnum"));
}

#[test]
fn test_from_config_options() {
    // TypeScript mode defaults.
    let config: TargetConfig = serde_yaml::from_str(
        r"
format: typescript
output_dir: build/game
",
    )
    .expect("target config");
    let gen = TsTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/game"));
    assert_eq!(gen.file_template, "{table}.ts");
    assert_eq!(gen.enums_file, "cage_enums.ts");
    assert_eq!(gen.mode, TsMode::TypeScript);
    assert!(gen.emit_dts);

    // JavaScript mode: flipped extensions, options honoured.
    let config: TargetConfig = serde_yaml::from_str(
        r#"
format: javascript
output_dir: build/game
file_template: "{table}_gen.js"
options:
  enums_file: shared_enums.js
  emit_dts: false
"#,
    )
    .expect("target config");
    let gen = TsTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/game"));
    assert_eq!(gen.file_template, "{table}_gen.js");
    assert_eq!(gen.enums_file, "shared_enums.js");
    assert_eq!(gen.mode, TsMode::JavaScript);
    assert!(!gen.emit_dts);

    // Options present but no emit_dts key — the inner arm is skipped.
    let config: TargetConfig = serde_yaml::from_str(
        r"
format: javascript
output_dir: build/game
options:
  enums_file: shared_enums.js
",
    )
    .expect("target config");
    let gen = TsTargetGenerator::from_config(&config);
    assert_eq!(gen.enums_file, "shared_enums.js");
    assert!(gen.emit_dts);

    // Short alias "js" behaves like "javascript".
    let config: TargetConfig =
        serde_yaml::from_str("format: js\noutput_dir: out").expect("target config");
    let gen = TsTargetGenerator::from_config(&config);
    assert_eq!(gen.mode, TsMode::JavaScript);
    assert_eq!(gen.file_template, "{table}.js");
    assert_eq!(gen.enums_file, "cage_enums.js");
}

#[test]
fn test_ident_sanitization() {
    assert_eq!(sanitize_ident("Item"), "Item");
    assert_eq!(sanitize_ident("drop-item"), "drop_item");
    assert_eq!(sanitize_ident("1st"), "_1st");
    assert_eq!(sanitize_ident("列"), "_");
    assert_eq!(sanitize_ident(""), "_");
    // Keywords get a trailing underscore — binding positions only.
    assert_eq!(ts_ident("class"), "class_");
    assert_eq!(ts_ident("default"), "default_");
    assert_eq!(ts_ident("type"), "type_");
    assert_eq!(ts_ident("implements"), "implements_");
    assert_eq!(ts_ident("name"), "name");
}

#[test]
fn test_property_keys_stay_unescaped() {
    // Property positions are legal TS: schema names keep their shape.
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  default:
    name: default
    primary_key: []
    fields:
      type: { name: type, type: { kind: String } }
enums: {}
",
    )
    .expect("test schema must parse");
    let artifacts = gen().generate(&schema, None);
    let src = String::from_utf8(artifacts[0].1.clone()).expect("utf-8");
    // Interface / const / function are bindings → escaped.
    assert!(src.contains("export interface default_ {"));
    assert!(src.contains("export const default_Defaults"));
    assert!(src.contains("export function newdefault_"));
    // The property key itself is NOT escaped.
    assert!(src.contains("  type?: string;"));
}

#[test]
fn test_field_collisions_get_unique_members() {
    let mut used = HashSet::new();
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b_");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b__");
}

#[test]
fn test_import_alias_on_local_collision() {
    // The interface and the enum share a name → the import is aliased.
    let schema: Schema = serde_yaml::from_str(
        r"
tables:
  ItemKind:
    name: ItemKind
    primary_key: []
    fields:
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
",
    )
    .expect("test schema must parse");
    let artifacts = gen().generate(&schema, None);
    let src = String::from_utf8(artifacts[0].1.clone()).expect("utf-8");
    assert!(src.contains("import type { ItemKind as ItemKind_ } from \"./cage_enums\";"));
    assert!(src.contains("kind?: ItemKind_;"));
    assert!(src.contains("export interface ItemKind {"));
    // JS mode needs no alias: inline imports are always qualified.
    let artifacts = js_gen("build/js").generate(&schema, None);
    let js = String::from_utf8(artifacts[0].1.clone()).expect("utf-8");
    assert!(js.contains(" * @property {import(\"./cage_enums.js\").ItemKind} [kind]"));
    assert!(!js.contains("ItemKind_"));
}

#[test]
fn test_ts_type_mapping() {
    let schema = test_schema();
    let empty = std::collections::HashMap::new();
    assert_eq!(ts_type(&FieldType::Null, &schema, &empty), "unknown");
    assert_eq!(ts_type(&FieldType::Any, &schema, &empty), "unknown");
    assert_eq!(ts_type(&FieldType::Bool, &schema, &empty), "boolean");
    assert_eq!(ts_type(&FieldType::Int8, &schema, &empty), "number");
    assert_eq!(ts_type(&FieldType::Int64, &schema, &empty), "number");
    assert_eq!(ts_type(&FieldType::UInt64, &schema, &empty), "number");
    assert_eq!(ts_type(&FieldType::Float32, &schema, &empty), "number");
    assert_eq!(ts_type(&FieldType::Float64, &schema, &empty), "number");
    assert_eq!(ts_type(&FieldType::String, &schema, &empty), "string");
    assert_eq!(ts_type(&FieldType::Bytes, &schema, &empty), "Uint8Array");
    assert_eq!(
        ts_type(
            &FieldType::Array(Box::new(FieldType::Float64)),
            &schema,
            &empty
        ),
        "number[]"
    );
    assert_eq!(
        ts_type(
            &FieldType::Object(indexmap::IndexMap::default()),
            &schema,
            &empty
        ),
        "Record<string, unknown>"
    );
    // Resolved enum → its ident (honouring the file's alias map);
    // unresolved/empty enum → plain string.
    assert_eq!(
        ts_type(&FieldType::Enum("ItemKind".into()), &schema, &empty),
        "ItemKind"
    );
    assert_eq!(
        ts_type(&FieldType::Enum("MissingEnum".into()), &schema, &empty),
        "string"
    );
    assert_eq!(
        ts_type(&FieldType::Enum("EmptyEnum".into()), &schema, &empty),
        "string"
    );
    let mut aliases = std::collections::HashMap::new();
    aliases.insert("ItemKind".to_string(), "ItemKind_".to_string());
    assert_eq!(
        ts_type(&FieldType::Enum("ItemKind".into()), &schema, &aliases),
        "ItemKind_"
    );
}

#[test]
fn test_render_default_edge_cases() {
    let schema = Schema::new();
    // Finite floats render literally (no int/float split in TS).
    assert_eq!(
        render_default(&serde_json::json!(100.0), &FieldType::Float64, &schema).unwrap(),
        "100"
    );
    assert_eq!(
        render_default(&serde_json::json!(1.5), &FieldType::Float64, &schema).unwrap(),
        "1.5"
    );
    // Non-finite floats are real TS/JS values — rendered, not skipped.
    assert_eq!(ts_float_literal(f64::NAN), "NaN");
    assert_eq!(ts_float_literal(f64::INFINITY), "Infinity");
    assert_eq!(ts_float_literal(f64::NEG_INFINITY), "-Infinity");
    // Kind mismatch → None.
    assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
    // Object defaults are not rendered (shared rule across targets).
    assert!(render_default(
        &serde_json::json!({"a": 1}),
        &FieldType::Object(indexmap::IndexMap::default()),
        &schema
    )
    .is_none());
    // Scalar arrays render; non-scalar element types do not.
    assert_eq!(
        render_default(
            &serde_json::json!(["a", "b"]),
            &FieldType::Array(Box::new(FieldType::String)),
            &schema
        )
        .unwrap(),
        "[\"a\", \"b\"]"
    );
    assert_eq!(
        render_default(
            &serde_json::json!([true, false]),
            &FieldType::Array(Box::new(FieldType::Bool)),
            &schema
        )
        .unwrap(),
        "[true, false]"
    );
    // One unrenderable element sinks the whole array.
    assert!(render_default(
        &serde_json::json!([1, "x"]),
        &FieldType::Array(Box::new(FieldType::Int32)),
        &schema
    )
    .is_none());
    assert!(render_default(
        &serde_json::json!([{"a": 1}]),
        &FieldType::Array(Box::new(FieldType::Object(indexmap::IndexMap::default()))),
        &schema
    )
    .is_none());
    // Control characters use \xHH escapes; printable Unicode passes.
    assert_eq!(ts_string_literal("a\u{1}b"), "\"a\\x01b\"");
    assert_eq!(ts_string_literal("a\nb\r\tc"), "\"a\\nb\\r\\tc\"");
    assert_eq!(ts_string_literal("naïve — 列"), "\"naïve — 列\"");
    assert_eq!(ts_string_literal("q\"\\q"), "\"q\\\"\\\\q\"");
}

#[test]
fn test_map_type_and_default_units() {
    let schema = test_schema();
    let empty = std::collections::HashMap::new();
    let map = |key: MapKeyType, value: FieldType| {
        FieldType::Map(MapField {
            key_type: key,
            value_type: Box::new(value),
        })
    };

    // Type spellings: keys string/number, values recurse.
    assert_eq!(
        ts_type(&map(MapKeyType::String, FieldType::String), &schema, &empty),
        "Map<string, string>"
    );
    assert_eq!(
        ts_type(&map(MapKeyType::Int, FieldType::Int64), &schema, &empty),
        "Map<number, number>"
    );
    assert_eq!(
        ts_type(
            &map(
                MapKeyType::String,
                FieldType::Array(Box::new(FieldType::Int32))
            ),
            &schema,
            &empty
        ),
        "Map<string, number[]>"
    );
    assert_eq!(
        ts_type(
            &map(MapKeyType::String, map(MapKeyType::Int, FieldType::Float64)),
            &schema,
            &empty
        ),
        "Map<string, Map<number, number>>"
    );
    // Enum values resolve through the alias map like any other position.
    let mut aliases = std::collections::HashMap::new();
    aliases.insert("ItemKind".to_string(), "ItemKind_".to_string());
    assert_eq!(
        ts_type(
            &map(MapKeyType::String, FieldType::Enum("ItemKind".into())),
            &schema,
            &aliases
        ),
        "Map<string, ItemKind_>"
    );

    // Empty object default → `new Map()` (never `{}`).
    let string_ints = map(MapKeyType::String, FieldType::Int32);
    assert_eq!(
        render_default(&serde_json::json!({}), &string_ints, &schema).unwrap(),
        "new Map()"
    );
    // Scalar members recurse through the shared rules (keys sorted:
    // serde_json's map is ordered here).
    assert_eq!(
        render_default(&serde_json::json!({"a": 1, "b": -2}), &string_ints, &schema).unwrap(),
        "new Map([[\"a\", 1], [\"b\", -2]])"
    );
    // Int keys: numeric strings promote to number literals.
    let int_strings = map(MapKeyType::Int, FieldType::String);
    assert_eq!(
        render_default(
            &serde_json::json!({"42": "x", "-7": "y"}),
            &int_strings,
            &schema
        )
        .unwrap(),
        "new Map([[-7, \"y\"], [42, \"x\"]])"
    );
    // Mismatched shapes are all skipped: non-object default,
    // non-scalar value type, non-numeric int key, bad member.
    assert!(render_default(&serde_json::json!("x"), &string_ints, &schema).is_none());
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
        &serde_json::json!({"x": 1}),
        &map(MapKeyType::Int, FieldType::Int32),
        &schema
    )
    .is_none());
    assert!(render_default(&serde_json::json!({"a": "nope"}), &string_ints, &schema).is_none());
}

#[test]
fn test_dts_path() {
    assert_eq!(dts_path_for("build/Item.js"), "build/Item.d.ts");
    assert_eq!(dts_path_for("build/x"), "build/x.d.ts");
}

#[test]
fn test_jsdoc_desc_safety() {
    // Letter/punctuation starters scan cleanly after `[name] `.
    assert!(jsdoc_desc_safe("Price in gold, min: 0"));
    assert!(jsdoc_desc_safe("allowed: common | rare"));
    assert!(jsdoc_desc_safe("-1 means none"));
    assert!(jsdoc_desc_safe("(optional)"));
    assert!(jsdoc_desc_safe("0-based index"));
    assert!(jsdoc_desc_safe("9.5 seconds"));
    assert!(jsdoc_desc_safe("50% chance"));
    assert!(jsdoc_desc_safe("列 description"));
    // Unterminated literals / glued identifiers / non-ASCII punctuation
    // are scanner errors after a bracketed name → continuation line.
    assert!(!jsdoc_desc_safe("→ Player.id"));
    assert!(!jsdoc_desc_safe("— em dash"));
    assert!(!jsdoc_desc_safe("\"quoted"));
    assert!(!jsdoc_desc_safe("\\back"));
    assert!(!jsdoc_desc_safe("`tmpl"));
    assert!(!jsdoc_desc_safe("1st place"));
    assert!(!jsdoc_desc_safe("0desc"));
    assert!(!jsdoc_desc_safe("0x"));
    // Hex-prefixed numbers parse as hex: safe when the identifier
    // stops at the hex digits, unsafe when glued to them.
    assert!(jsdoc_desc_safe("0xFF bytes"));
    assert!(jsdoc_desc_safe("0X1f bytes"));
    assert!(!jsdoc_desc_safe("0xFFxyz"));
    // Exponent-leading numbers: the exponent must not be glued to
    // an identifier either.
    assert!(jsdoc_desc_safe("1e5 seconds"));
    assert!(jsdoc_desc_safe("1e+5 seconds"));
    assert!(!jsdoc_desc_safe("1ex seconds"));
    assert!(jsdoc_desc_safe(""));
}

#[test]
fn test_module_specifier() {
    assert_eq!(module_specifier("cage_enums.ts"), "./cage_enums");
    assert_eq!(module_specifier("cage_enums.js"), "./cage_enums");
    assert_eq!(module_specifier("./shared/enums.ts"), "./shared/enums");
    assert_eq!(module_specifier("../up/enums.js"), "../up/enums");
    assert_eq!(module_specifier("enums"), "./enums");
    // JSDoc inline imports keep the extension.
    assert_eq!(jsdoc_import_path("cage_enums.js"), "./cage_enums.js");
    assert_eq!(jsdoc_import_path("../up/enums.js"), "../up/enums.js");
}

/// Writes the test-schema output under `/tmp/cage-ts-sample/` for
/// manual `tsc` / `node --check` verification (ignored in normal runs —
/// the committed suite never shells out to external toolchains).
#[test]
#[ignore = "writes /tmp/cage-ts-sample for tsc/node sanity checks"]
fn write_sample_output() {
    let schema = test_schema();
    let root = std::path::Path::new("/tmp/cage-ts-sample");
    let _ = std::fs::remove_dir_all(root);

    let ts_config: TargetConfig = serde_yaml::from_str(&format!(
        "format: typescript\noutput_dir: {}/ts",
        root.display()
    ))
    .expect("target config");
    let js_config: TargetConfig = serde_yaml::from_str(&format!(
        "format: javascript\noutput_dir: {}/js",
        root.display()
    ))
    .expect("target config");
    for config in [&ts_config, &js_config] {
        let generator = TsTargetGenerator::from_config(config);
        for (path, bytes) in generator.generate(&schema, Some("abc123")) {
            let path = std::path::Path::new(&path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(path, bytes).expect("write sample");
        }
    }
}
