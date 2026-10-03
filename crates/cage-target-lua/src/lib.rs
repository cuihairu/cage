//! Lua Target Generator — generates table-binding modules from a Cage schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields keep name
//! order, enum values keep their schema order, and no timestamps are written
//! — the same schema always produces byte-identical files.
//!
//! Each table module exposes name / `primary_key` / field metadata / schema
//! defaults plus an `M.new(t)` constructor that fills missing fields from the
//! defaults (array defaults are copied so rows never share state). Shared
//! enums live in one flat module (`M.<Enum> = { member = value, ... }`).

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: the Value/normalize layer is a numeric coercion engine —
// float<->int casts are its job, not a defect.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::approx_constant,
    clippy::checked_conversions
)]
// Stage: crate-prefixed type names (LuaTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: Diagnostics is the deliberate first-class error type,
// returned by value from every pipeline stage.
#![allow(clippy::result_large_err)]
// Restriction lints kept off for a 0.1.0 codebase.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names
)]
use cage_core::{
    manifest::TargetConfig,
    schema::{EnumSchema, FieldSchema, FieldType, Schema, TableSchema},
};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;

/// Lua Target Generator
pub struct LuaTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.lua")
    pub file_template: String,
    /// File name of the shared enums module
    pub enums_file: String,
}

impl Default for LuaTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/lua"),
            file_template: "{table}.lua".to_string(),
            enums_file: "cage_enums.lua".to_string(),
        }
    }
}

/// Lua keywords that get a trailing `_` (Lua has no `@`-style escape).
const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

impl LuaTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.lua".to_string()),
            ..Self::default()
        };
        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("enums_file") {
                if let Some(s) = v.as_str() {
                    gen.enums_file = s.to_string();
                }
            }
        }
        gen
    }

    /// Generate one module per table (name order) plus a shared enums module.
    ///
    /// `schema_hash` is the same hash `manifest.json` records for the schema
    /// (Build Manifest 口径); it is stamped into every file header.
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        let mut artifacts = Vec::new();
        for table in Self::sorted_tables(schema) {
            let content = Self::render_table(schema, table, schema_hash);
            artifacts.push((self.path_for(&table.name), content.into_bytes()));
        }
        if !Self::emitted_enums(schema).is_empty() {
            let content = Self::render_enums(schema, schema_hash);
            let path = self
                .output_dir
                .join(&self.enums_file)
                .to_string_lossy()
                .to_string();
            artifacts.push((path, content.into_bytes()));
        }
        artifacts
    }

    /// Tables in name order (deterministic emission order).
    fn sorted_tables(schema: &Schema) -> Vec<&TableSchema> {
        let mut tables: Vec<&TableSchema> = schema.tables.values().collect();
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        tables
    }

    /// Fields in name order (deterministic member order).
    fn sorted_fields(table: &TableSchema) -> Vec<(&str, &FieldSchema)> {
        let mut fields: Vec<(&str, &FieldSchema)> =
            table.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
        fields.sort_by(|a, b| a.0.cmp(b.0));
        fields
    }

    /// Emitted shared enums in name order; empty enums are skipped everywhere.
    fn emitted_enums(schema: &Schema) -> Vec<&EnumSchema> {
        let mut enums: Vec<&EnumSchema> = schema
            .enums
            .values()
            .filter(|e| !e.values.is_empty())
            .collect();
        enums.sort_by(|a, b| a.name.cmp(&b.name));
        enums
    }

    fn path_for(&self, table_name: &str) -> String {
        let file_name = self.file_template.replace("{table}", table_name);
        self.output_dir
            .join(file_name)
            .to_string_lossy()
            .to_string()
    }

    fn header(what: &str, schema_hash: Option<&str>) -> String {
        let hash = schema_hash.unwrap_or("(unavailable)");
        format!(
            "-- <auto-generated>\n\
             --   Generated by Cage — do not edit.\n\
             --   schema: {hash}\n\
             --   {what}\n\
             -- </auto-generated>\n"
        )
    }

    fn render_table(schema: &Schema, table: &TableSchema, schema_hash: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&Self::header(
            &format!("table:  {}", table.name),
            schema_hash,
        ));
        let _ = writeln!(out);
        let _ = writeln!(out, "local M = {{}}");
        let _ = writeln!(out);
        let _ = writeln!(out, "M.name = {}", lua_string_literal(&table.name));
        if let Some(desc) = &table.description {
            let _ = writeln!(out, "M.description = {}", lua_string_literal(desc));
        }
        if !table.primary_key.is_empty() {
            let keys = table
                .primary_key
                .iter()
                .map(|k| lua_string_literal(k))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "M.primary_key = {{ {keys} }}");
        }

        let fields = Self::sorted_fields(table);

        // Resolve each field's Lua key once, so M.fields metadata and
        // M.defaults entries can never drift apart.
        let mut used: HashSet<String> = HashSet::new();
        let members: Vec<(&str, &FieldSchema, String)> = fields
            .iter()
            .map(|(name, field)| (*name, *field, unique_ident(lua_ident(name), &mut used)))
            .collect();

        // Field metadata (name order): one table per field, constraints kept
        // as comments (same summary format as the C#/Python generators).
        let _ = writeln!(out);
        let _ = writeln!(out, "-- Field metadata (name order).");
        let _ = writeln!(out, "M.fields = {{");
        for (field_name, field, member) in &members {
            if let Some(doc) = field_doc(schema, field) {
                let _ = writeln!(out, "    -- {doc}");
            }
            let ty = lua_type_label(&field.field_type, schema);
            let required = if field.required { "true" } else { "false" };
            let _ = writeln!(
                out,
                "    {{ name = {}, key = {}, type = \"{ty}\", required = {required} }},",
                lua_string_literal(field_name),
                lua_string_literal(member)
            );
        }
        let _ = writeln!(out, "}}");

        // Schema defaults by field key; fields whose default has no safe
        // literal (objects, mismatched kinds, non-finite floats) simply have
        // no entry.
        let _ = writeln!(out);
        let _ = writeln!(out, "-- Schema defaults by field key.");
        let _ = writeln!(out, "M.defaults = {{");
        for (_, field, member) in &members {
            let Some(value) = field
                .default
                .as_ref()
                .filter(|v| !v.is_null())
                .and_then(|d| render_default(d, &field.field_type, schema))
            else {
                continue;
            };
            let _ = writeln!(out, "    {member} = {value},");
        }
        let _ = writeln!(out, "}}");

        // Generic constructor: caller values win, missing keys fall back to
        // M.defaults; array defaults are copied so rows never share state.
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "--- Build one row: caller-supplied values win, missing"
        );
        let _ = writeln!(
            out,
            "--- fields fall back to M.defaults (copied, not shared)."
        );
        let _ = writeln!(out, "function M.new(t)");
        let _ = writeln!(out, "    local row = {{}}");
        let _ = writeln!(out, "    for k, v in pairs(t or {{}}) do");
        let _ = writeln!(out, "        row[k] = v");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    for k, v in pairs(M.defaults) do");
        let _ = writeln!(out, "        if row[k] == nil then");
        let _ = writeln!(out, "            if type(v) == \"table\" then");
        let _ = writeln!(out, "                local copy = {{}}");
        let _ = writeln!(out, "                for i, item in ipairs(v) do");
        let _ = writeln!(out, "                    copy[i] = item");
        let _ = writeln!(out, "                end");
        let _ = writeln!(out, "                row[k] = copy");
        let _ = writeln!(out, "            else");
        let _ = writeln!(out, "                row[k] = v");
        let _ = writeln!(out, "            end");
        let _ = writeln!(out, "        end");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    return row");
        let _ = writeln!(out, "end");
        let _ = writeln!(out);
        let _ = writeln!(out, "return M");

        out
    }

    fn render_enums(schema: &Schema, schema_hash: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&Self::header("enums:  shared definitions", schema_hash));
        let _ = writeln!(out);
        let _ = writeln!(out, "local M = {{}}");

        // One flat table per enum. Unlike C#/Python there is no backing-type
        // concern: numeric values render as numbers, everything else (string,
        // bool, value-less) as its string form — Cage compares enums as
        // strings, so the member name is the value of last resort.
        let mut used_tables: HashSet<String> = HashSet::new();
        for e in Self::emitted_enums(schema) {
            let enum_ident = unique_ident(lua_ident(&e.name), &mut used_tables);
            let _ = writeln!(out);
            match &e.description {
                Some(desc) => {
                    let _ = writeln!(out, "-- {} — {}", e.name, desc);
                }
                None => {
                    let _ = writeln!(out, "-- {}", e.name);
                }
            }
            let _ = writeln!(out, "M.{enum_ident} = {{");
            let mut used_members: HashSet<String> = HashSet::new();
            for v in &e.values {
                let member = unique_ident(lua_ident(&v.name), &mut used_members);
                let value = match &v.value {
                    Some(serde_json::Value::Number(n)) => n.to_string(),
                    Some(serde_json::Value::String(s)) => lua_string_literal(s),
                    Some(serde_json::Value::Bool(b)) => {
                        lua_string_literal(if *b { "true" } else { "false" })
                    }
                    _ => lua_string_literal(&v.name),
                };
                match &v.description {
                    Some(desc) => {
                        let _ = writeln!(out, "    {member} = {value}, -- {desc}");
                    }
                    None => {
                        let _ = writeln!(out, "    {member} = {value},");
                    }
                }
            }
            let _ = writeln!(out, "}}");
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "return M");
        out
    }
}

/// Lua type label for a field type (metadata string, not a real type system).
fn lua_type_label(ft: &FieldType, schema: &Schema) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "any".to_string(),
        FieldType::Bool => "boolean".to_string(),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => "integer".to_string(),
        FieldType::Float32 | FieldType::Float64 => "number".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "bytes".to_string(),
        FieldType::Array(inner) => format!("array<{}>", lua_type_label(inner, schema)),
        FieldType::Object(_) => "table".to_string(),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => name.clone(),
            // Unresolved (or empty) enum: fall back to plain string.
            None => "string".to_string(),
        },
    }
}

/// Render a schema default as a Lua initializer expression; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, mismatched
/// kinds, non-finite floats — Lua has no literal for nan/inf).
fn render_default(value: &serde_json::Value, ft: &FieldType, schema: &Schema) -> Option<String> {
    match ft {
        FieldType::Bool => value.as_bool().map(|b| {
            if b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => value
            .as_i64()
            .map(|i| i.to_string())
            .or_else(|| value.as_u64().map(|u| u.to_string())),
        FieldType::Float32 | FieldType::Float64 => value.as_f64().and_then(|f| {
            if f.is_finite() {
                Some(lua_float_literal(f))
            } else {
                None
            }
        }),
        FieldType::String => value.as_str().map(lua_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        _ => None,
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Python generators — keeps the three outputs aligned).
fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    schema: &Schema,
) -> Option<String> {
    if !matches!(
        inner,
        FieldType::Bool
            | FieldType::Int8
            | FieldType::Int16
            | FieldType::Int32
            | FieldType::Int64
            | FieldType::UInt8
            | FieldType::UInt16
            | FieldType::UInt32
            | FieldType::UInt64
            | FieldType::Float32
            | FieldType::Float64
            | FieldType::String
    ) {
        return None;
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(render_default(item, inner, schema)?);
    }
    Some(format!("{{ {} }}", parts.join(", ")))
}

fn lua_float_literal(f: f64) -> String {
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn lua_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Lua literals: control characters become `\n` /
/// `\ddd` (Lua has no `\xHH`); printable Unicode passes through.
fn escape_string_content(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                let code = u32::from(c);
                let _ = write!(out, "\\{code:03}");
            }
            c => out.push(c),
        }
    }
    out
}

/// Sanitize a schema name into a valid identifier (ASCII alnum + `_`,
/// never starting with a digit).
fn sanitize_ident(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Identifier with a trailing `_` for Lua keywords (no escape syntax exists).
fn lua_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if LUA_KEYWORDS.contains(&s.as_str()) {
        format!("{s}_")
    } else {
        s
    }
}

/// First-unique wins: colliding identifiers get `_` appended until free
/// (deterministic because callers iterate in name order).
fn unique_ident(base: String, used: &mut HashSet<String>) -> String {
    let mut candidate = base;
    while !used.insert(candidate.clone()) {
        candidate.push('_');
    }
    candidate
}

/// Field comment body: description plus constraint summary; `None` when
/// there is nothing to say (same format as the C#/Python generators).
fn field_doc(schema: &Schema, field: &FieldSchema) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = &field.description {
        parts.push(d.clone());
    }
    if field.required {
        parts.push("required".to_string());
    }
    if let Some(min) = field.min {
        parts.push(format!("min: {min}"));
    }
    if let Some(max) = field.max {
        parts.push(format!("max: {max}"));
    }
    if let Some(v) = field.min_length {
        parts.push(format!("min_length: {v}"));
    }
    if let Some(v) = field.max_length {
        parts.push(format!("max_length: {v}"));
    }
    if let Some(p) = &field.pattern {
        parts.push(format!("pattern: {p}"));
    }
    if let Some(vals) = &field.enum_values {
        parts.push(format!("allowed: {}", vals.join(" | ")));
    }
    if let Some(r) = &field.reference {
        parts.push(format!("→ {}.{}", r.table, r.field));
    }
    if let FieldType::Enum(name) = &field.field_type {
        if schema.enums.get(name).is_none_or(|e| e.values.is_empty()) {
            parts.push(format!("unresolved enum: {name}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
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
        assert!(
            edge.contains("{ name = \"nils\", key = \"nils\", type = \"any\", required = false },")
        );
        assert!(edge.contains(
            "{ name = \"flag\", key = \"flag\", type = \"boolean\", required = false },"
        ));
        assert!(edge
            .contains("{ name = \"blob\", key = \"blob\", type = \"bytes\", required = false },"));
        assert!(edge
            .contains("{ name = \"meta\", key = \"meta\", type = \"table\", required = false },"));
        // Both Bool default branches reach M.defaults.
        assert!(edge.contains("    flag = true,"));
        assert!(edge.contains("    off = false,"));
        // Constraint summaries in the metadata comments.
        assert!(edge.contains("    -- Bounded count, max: 100"));
        assert!(edge.contains("    -- min_length: 1, max_length: 8, pattern: ^[A-Z]+$"));
        // u64 default above i64::MAX renders via the or_else arm; a float
        // default on an int field renders nothing (no M.defaults entry).
        assert!(edge
            .contains("{ name = \"big\", key = \"big\", type = \"integer\", required = false },"));
        assert!(edge.contains("    big = 18446744073709551615,"));
        assert!(edge.contains(
            "{ name = \"frac\", key = \"frac\", type = \"integer\", required = false },"
        ));
        assert!(!edge.contains("    frac = "));
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
        assert!(
            render_default(&serde_json::json!(f64::NAN), &FieldType::Float64, &schema).is_none()
        );
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
}
