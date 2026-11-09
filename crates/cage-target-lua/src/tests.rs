//! Lua target generator tests (split out of lib.rs, LAY-022).

use super::docfmt::{MAP_INT_KEY_NOTE, MAP_NOTE};
use super::idents::sanitize_ident;
use super::*;
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
                "{{ tables.Item.fields.price | lua_type }}|{{ tables.Item.fields.price | lua_default }}|{{ tables.Item.fields.kind | lua_type }}",
                &ctx,
            )
            .unwrap();
    assert_eq!(out, "integer|10|ItemKind");
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
/// without a primary key, Any/Null/Object/Bytes annotations, both Bool
/// default branches, length/pattern constraints, an enum description,
/// and a non-integral enum with string/bool members.
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
      anything: { name: anything, type: { kind: Any } }
      nils: { name: nils, type: { kind: Null } }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 8, pattern: '^[A-Z]+$' }
      count: { name: count, type: { kind: Int32 }, max: 100, description: Bounded count }
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

/// Schema exercising every Map shape: string and int keys, array /
/// nested-map / enum members, an empty default, a default with one
/// mismatched member, and a wholly mismatched default kind.
fn map_schema() -> Schema {
    serde_yaml::from_str(
            r"
tables:
  Drop:
    name: Drop
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      pools: { name: pools, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }, default: { rare: [1, 2] }, description: Loot pools }
      weights: { name: weights, type: { kind: Map, value: { key_type: int, value_type: { kind: Float32 } } }, default: { '1': 0.5, '3': 1.25 } }
      nested: { name: nested, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } }, default: { outer: { inner: 7 } } }
      quota: { name: quota, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: {} }
      kinds: { name: kinds, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: ItemKind } } } }
      mixed: { name: mixed, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: { good: 1, bad: nope } }
      odd: { name: odd, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: [1, 2] }
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

fn gen() -> LuaTargetGenerator {
    LuaTargetGenerator::default()
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
            "build/lua/Drop.lua",
            "build/lua/Item.lua",
            "build/lua/cage_enums.lua",
        ]
    );
}

#[test]
fn test_table_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let item = String::from_utf8(artifacts[1].1.clone()).unwrap();

    assert!(item.contains("--   schema: abc123"));
    assert!(item.contains("--   table:  Item"));
    assert!(item.contains("local M = {}"));
    assert!(item.contains("M.name = \"Item\""));
    assert!(item.contains("M.description = \"Equipment definitions.\""));
    assert!(item.contains("M.primary_key = { \"id\" }"));

    // Field metadata: name order, original name + Lua key, doc summary.
    let name_meta = item
        .find("    { name = \"name\", key = \"name\", type = \"string\", required = true },")
        .unwrap();
    let id_meta = item
        .find("    { name = \"id\", key = \"id\", type = \"integer\", required = true },")
        .unwrap();
    let tags_meta = item
        .find(
            "    { name = \"tags\", key = \"tags\", type = \"array<string>\", required = false },",
        )
        .unwrap();
    let kind_meta = item
        .find("    { name = \"kind\", key = \"kind\", type = \"ItemKind\", required = false },")
        .unwrap();
    let owner_meta = item
        .find("    { name = \"owner\", key = \"owner\", type = \"string\", required = false },")
        .unwrap();
    // Pure name order: id < kind < name < owner < tags.
    assert!(id_meta < kind_meta);
    assert!(kind_meta < name_meta);
    assert!(name_meta < owner_meta);
    assert!(owner_meta < tags_meta);
    assert!(item.contains("    -- Price in gold, min: 0"));
    assert!(item.contains("allowed: common | rare"));
    assert!(item.contains("→ Player.id"));

    // Defaults: only fields carrying a renderable default get an entry.
    assert!(item.contains("    weight = 1.5,"));
    assert!(item.contains("    tags = { \"pvp\" },"));
    assert!(!item.contains("    note = "));
    assert!(!item.contains("    price = "));
    assert!(!item.contains("    id = "));
    assert!(!item.contains("    name = "));

    // Generic constructor with shared-state-safe default copies.
    assert!(item.contains("function M.new(t)"));
    assert!(item.contains("    for k, v in pairs(M.defaults) do"));
    assert!(item.contains("        if row[k] == nil then"));
    assert!(item.contains("return M\n"));
}

#[test]
fn test_enums_rendering() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    assert!(enums_src.contains("local M = {}"));
    // Integral enum keeps numeric values; inline descriptions.
    assert!(enums_src.contains("M.ItemKind = {"));
    assert!(enums_src.contains("    Sword = 1, -- Sword weapon"));
    assert!(enums_src.contains("    Shield = 2,"));
    // Value-less enum members fall back to their name strings.
    assert!(enums_src.contains("M.Rarity = {"));
    assert!(enums_src.contains("    common = \"common\","));
    assert!(enums_src.contains("    rare = \"rare\","));
    // Empty enums are not emitted.
    assert!(!enums_src.contains("EmptyEnum"));
    assert!(enums_src.contains("return M\n"));
}

#[test]
fn test_unresolved_enum_falls_back_to_string() {
    let schema = test_schema();
    let artifacts = gen().generate(&schema, None);
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(drop_src.contains("--   schema: (unavailable)"));
    assert!(drop_src.contains("type = \"string\""));
    assert!(drop_src.contains("unresolved enum: MissingEnum"));
}

#[test]
fn test_empty_table_without_primary_key() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    // Name order: Edge, Ghost, then the shared enums module.
    assert!(artifacts[0].0.ends_with("Edge.lua"));
    let ghost = String::from_utf8(artifacts[1].1.clone()).unwrap();
    // No primary key → the M.primary_key line is omitted entirely.
    assert!(!ghost.contains("M.primary_key"));
    assert!(ghost.contains("M.name = \"Ghost\""));
    // No fields → empty metadata and defaults tables.
    assert!(ghost.contains("M.fields = {\n}"));
    assert!(ghost.contains("M.defaults = {\n}"));
}

#[test]
fn test_type_labels_and_bool_defaults() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let edge = String::from_utf8(artifacts[0].1.clone()).unwrap();

    assert!(edge.contains("M.primary_key = { \"key\" }"));
    // Type labels: Any / Null / Bool / Bytes / Object.
    assert!(edge.contains(
        "{ name = \"anything\", key = \"anything\", type = \"any\", required = false },"
    ));
    assert!(edge.contains("{ name = \"nils\", key = \"nils\", type = \"any\", required = false },"));
    assert!(
        edge.contains("{ name = \"flag\", key = \"flag\", type = \"boolean\", required = false },")
    );
    assert!(
        edge.contains("{ name = \"blob\", key = \"blob\", type = \"bytes\", required = false },")
    );
    assert!(
        edge.contains("{ name = \"meta\", key = \"meta\", type = \"table\", required = false },")
    );
    // Both Bool default branches reach M.defaults.
    assert!(edge.contains("    flag = true,"));
    assert!(edge.contains("    off = false,"));
    // Constraint summaries in the metadata comments.
    assert!(edge.contains("    -- Bounded count, max: 100"));
    assert!(edge.contains("    -- min_length: 1, max_length: 8, pattern: ^[A-Z]+$"));
    // u64 default above i64::MAX renders via the or_else arm; a float
    // default on an int field renders nothing (no M.defaults entry).
    assert!(
        edge.contains("{ name = \"big\", key = \"big\", type = \"integer\", required = false },")
    );
    assert!(edge.contains("    big = 18446744073709551615,"));
    assert!(
        edge.contains("{ name = \"frac\", key = \"frac\", type = \"integer\", required = false },")
    );
    assert!(!edge.contains("    frac = "));
}

#[test]
fn test_map_type_labels_and_doc_notes() {
    let artifacts = gen().generate(&map_schema(), Some("map1"));
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // Type labels: `map<K, V>` — K is string/int, V recurses into the
    // existing label spellings (array<integer>, map<...>, enum names).
    assert!(drop_src.contains(
            "{ name = \"pools\", key = \"pools\", type = \"map<string, array<integer>>\", required = false },"
        ));
    assert!(drop_src.contains(
        "{ name = \"weights\", key = \"weights\", type = \"map<int, number>\", required = false },"
    ));
    assert!(drop_src.contains(
            "{ name = \"nested\", key = \"nested\", type = \"map<string, map<string, integer>>\", required = false },"
        ));
    assert!(drop_src.contains(
            "{ name = \"kinds\", key = \"kinds\", type = \"map<string, ItemKind>\", required = false },"
        ));

    // Every map field's metadata comment distinguishes the key-value
    // table (hash part) from an Array's sequential table; int-keyed
    // maps add the numeric-string/tonumber hint.
    assert!(drop_src.contains(&format!("    -- Loot pools, {MAP_NOTE}")));
    assert!(drop_src.contains(&format!("    -- {MAP_NOTE}；{MAP_INT_KEY_NOTE}")));
}

#[test]
fn test_map_defaults_render() {
    let artifacts = gen().generate(&map_schema(), Some("map1"));
    let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();

    // Empty map default → empty table; scalar members render recursively
    // (nested maps, and arrays of scalars inside maps).
    assert!(drop_src.contains("    quota = {  },"));
    assert!(drop_src.contains("    nested = { outer = { inner = 7 } },"));
    assert!(drop_src.contains("    pools = { rare = { 1, 2 } },"));
    // Int keys keep their data-model spelling — numeric strings, read
    // back with tonumber at the consumer.
    assert!(drop_src.contains("    weights = { [\"1\"] = 0.5, [\"3\"] = 1.25 },"));
    // A member with a mismatched kind is skipped entry-by-entry instead
    // of sinking the whole map default.
    assert!(drop_src.contains("    mixed = { good = 1 },"));
    assert!(!drop_src.contains("bad ="));
    // A wholly mismatched default kind (array on a map field) and map
    // fields without defaults get no M.defaults entry at all.
    assert!(!drop_src.contains("    odd = "));
    assert!(!drop_src.contains("    kinds = "));

    // The constructor deep-copies defaults via the recursive helper.
    assert!(drop_src.contains("local function copy_default(v)"));
    assert!(drop_src.contains("        copy[k] = copy_default(item)"));
    assert!(drop_src.contains("            row[k] = copy_default(v)"));
}

/// lua availability probe — the execution test below is a no-op (never a
/// failure) on machines without an interpreter.
fn lua_available() -> bool {
    std::process::Command::new("lua")
        .arg("-e")
        .arg("return 0")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run the real generated module under lua: rows built by M.new must not
/// share any table — not the top-level maps, nor nested maps, nor arrays
/// inside maps — and caller-supplied values must win over defaults.
#[test]
fn test_map_default_copy_runs_under_lua() {
    if !lua_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("cage-lua-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for (path, bytes) in gen().generate(&map_schema(), Some("map1")) {
        let name = std::path::Path::new(&path)
            .file_name()
            .expect("artifact file name")
            .to_string_lossy()
            .into_owned();
        std::fs::write(root.join(name), bytes).unwrap();
    }
    let module = root.join("Drop.lua").to_string_lossy().into_owned();
    let script = format!(
        r#"local M = dofile([[{module}]])
local a = M.new({{}})
local b = M.new({{}})
assert(a.id == nil and b.id == nil, "no id default exists")
assert(a.weights ~= b.weights, "rows share the top-level map")
assert(a.weights["1"] == 0.5, "int keys are numeric strings")
assert(tonumber("1") == 1, "consumers convert keys with tonumber")
a.weights["1"] = 42
assert(b.weights["1"] == 0.5, "rows share int-keyed entries")
assert(M.defaults.weights["1"] == 0.5, "M.defaults was mutated")
assert(a.nested.outer.inner == 7 and b.nested.outer.inner == 7)
a.nested.outer.inner = 99
assert(b.nested.outer.inner == 7, "rows share nested map tables")
assert(a.pools.rare[1] == 1 and a.pools.rare[2] == 2)
a.pools.rare[1] = 9
assert(b.pools.rare[1] == 1, "rows share arrays inside maps")
local c = M.new({{ id = 5 }})
assert(c.id == 5, "caller-supplied values win")
assert(next(c.quota) == nil, "empty map default stays empty")
"#
    );
    let out = std::process::Command::new("lua")
        .arg("-e")
        .arg(&script)
        .output()
        .expect("lua must run");
    assert!(
        out.status.success(),
        "lua failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_render_default_map_edges() {
    fn map_of(key: MapKeyType, value: FieldType) -> FieldType {
        FieldType::Map(MapField {
            key_type: key,
            value_type: Box::new(value),
        })
    }

    let schema = test_schema();

    // Wholly mismatched default kind (array on a map field) → no entry.
    assert!(render_default(
        &serde_json::json!([1, 2]),
        &map_of(MapKeyType::String, FieldType::Int32),
        &schema
    )
    .is_none());
    // Null default is filtered before render_default is even called, but
    // a JSON null inside a map is skipped like any other mismatch.
    assert_eq!(
        render_default(
            &serde_json::json!({"a": null, "b": 2}),
            &map_of(MapKeyType::String, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "{ b = 2 }"
    );
    // Enum members have no literal rule (same as bare enum fields) —
    // skipped entry-by-entry rather than sinking the map.
    assert_eq!(
        render_default(
            &serde_json::json!({"a": "Sword"}),
            &map_of(MapKeyType::String, FieldType::Enum("ItemKind".to_string())),
            &schema
        )
        .unwrap(),
        "{  }"
    );
    // Keys needing brackets: keywords, spaces, and the numeric-string
    // keys of int-keyed maps (kept verbatim — that is the data spelling).
    assert_eq!(
        render_default(
            &serde_json::json!({"a b": 1, "end": 2}),
            &map_of(MapKeyType::String, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "{ [\"a b\"] = 1, [\"end\"] = 2 }"
    );
    assert_eq!(
        render_default(
            &serde_json::json!({"42": 2, "-7": 1}),
            &map_of(MapKeyType::Int, FieldType::Int32),
            &schema
        )
        .unwrap(),
        "{ [\"-7\"] = 1, [\"42\"] = 2 }"
    );
    // One unrenderable array member sinks that entry only.
    assert_eq!(
        render_default(
            &serde_json::json!({"a": [1, "x"], "b": [3]}),
            &map_of(
                MapKeyType::String,
                FieldType::Array(Box::new(FieldType::Int32))
            ),
            &schema
        )
        .unwrap(),
        "{ b = { 3 } }"
    );
}

#[test]
fn test_schema_without_enums_emits_no_enums_file() {
    let artifacts = gen().generate(&no_enum_schema(), Some("abc123"));
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].0.ends_with("Plain.lua"));
    let src = String::from_utf8(artifacts[0].1.clone()).unwrap();
    assert!(!src.contains("cage_enums"));
}

#[test]
fn test_enum_description_and_mixed_value_arms() {
    let artifacts = gen().generate(&edge_schema(), Some("abc123"));
    let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

    // Enum-level description comment.
    assert!(enums_src.contains("-- Described — A described enum."));
    assert!(enums_src.contains("    First = 1, -- First member"));
    // String / Number / Bool / value-less member arms.
    assert!(enums_src.contains("    one = \"first\", -- String member"));
    assert!(enums_src.contains("    two = 2.5,"));
    assert!(enums_src.contains("    yes = \"true\","));
    assert!(enums_src.contains("    no = \"false\","));
    assert!(enums_src.contains("    bare = \"bare\","));
}

#[test]
fn test_from_config_without_enum_options() {
    // options: None — the option block never opens.
    let config: TargetConfig =
        serde_yaml::from_str("format: lua\noutput_dir: build/lua").expect("target config");
    let gen = LuaTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/lua"));
    assert_eq!(gen.file_template, "{table}.lua");
    assert_eq!(gen.enums_file, "cage_enums.lua");

    // Options present but no enums_file key — the inner arm is skipped.
    let config: TargetConfig =
        serde_yaml::from_str("format: lua\noutput_dir: out\noptions:\n  other: 1")
            .expect("target config");
    let gen = LuaTargetGenerator::from_config(&config);
    assert_eq!(gen.enums_file, "cage_enums.lua");

    // Non-string enums_file value — as_str() fails, default kept.
    let config: TargetConfig =
        serde_yaml::from_str("format: lua\noutput_dir: out\noptions:\n  enums_file: 42")
            .expect("target config");
    let gen = LuaTargetGenerator::from_config(&config);
    assert_eq!(gen.enums_file, "cage_enums.lua");
}

#[test]
fn test_field_collisions_get_unique_members() {
    let mut used = HashSet::new();
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b_");
    assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b__");
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
format: lua
output_dir: build/game
file_template: "{table}_gen.lua"
options:
  enums_file: shared_enums.lua
"#,
    )
    .expect("target config");
    let gen = LuaTargetGenerator::from_config(&config);
    assert_eq!(gen.output_dir, PathBuf::from("build/game"));
    assert_eq!(gen.file_template, "{table}_gen.lua");
    assert_eq!(gen.enums_file, "shared_enums.lua");
}

#[test]
fn test_ident_sanitization() {
    assert_eq!(sanitize_ident("Item"), "Item");
    assert_eq!(sanitize_ident("drop-item"), "drop_item");
    assert_eq!(sanitize_ident("1st"), "_1st");
    assert_eq!(sanitize_ident("列"), "_");
    assert_eq!(sanitize_ident(""), "_");
    // Keyword collision gets a trailing underscore (no escape syntax).
    assert_eq!(lua_ident("end"), "end_");
    assert_eq!(lua_ident("function"), "function_");
    assert_eq!(lua_ident("name"), "name");
}

#[test]
fn test_render_default_edge_cases() {
    let schema = Schema::new();
    // Float literals always keep a decimal point.
    assert_eq!(
        render_default(&serde_json::json!(100.0), &FieldType::Float64, &schema).unwrap(),
        "100.0"
    );
    // Non-finite floats have no Lua literal — skipped.
    assert!(render_default(&serde_json::json!(f64::NAN), &FieldType::Float64, &schema).is_none());
    assert!(render_default(
        &serde_json::json!(f64::INFINITY),
        &FieldType::Float64,
        &schema
    )
    .is_none());
    // Kind mismatch → no default entry.
    assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
    // Object defaults are not rendered (shared rule across targets).
    assert!(render_default(
        &serde_json::json!({"a": 1}),
        &FieldType::Object(indexmap::IndexMap::default()),
        &schema
    )
    .is_none());
    // Control characters use Lua's \ddd decimal escapes.
    assert_eq!(lua_string_literal("a\u{1}b"), "\"a\\001b\"");
    // Backslash / quote / newline / CR / tab escapes.
    assert_eq!(lua_string_literal("a\nb\r\tc"), "\"a\\nb\\r\\tc\"");
    assert_eq!(lua_string_literal("q\"\\q"), "\"q\\\"\\\\q\"");
    // Bool defaults render both branches.
    assert_eq!(
        render_default(&serde_json::json!(true), &FieldType::Bool, &schema).unwrap(),
        "true"
    );
    assert_eq!(
        render_default(&serde_json::json!(false), &FieldType::Bool, &schema).unwrap(),
        "false"
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
        "{ 1, 2 }"
    );
    // One unrenderable element sinks the whole array (`?` early exit).
    assert!(render_default(
        &serde_json::json!([1, "x"]),
        &FieldType::Array(Box::new(FieldType::Int32)),
        &schema
    )
    .is_none());
}
