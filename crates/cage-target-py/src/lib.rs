//! Python Target Generator - generates frozen dataclass bindings from a Cage
//! schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields without
//! defaults precede defaulted ones (dataclass syntax requires non-default
//! fields first; name order inside each group), enum values keep their schema
//! order, and no timestamps are written — the same schema always produces
//! byte-identical files.

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
// Stage: crate-prefixed type names (JsonTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churring at 0.1.0 — revisit #[must_use] before 1.0.
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
    schema::{EnumSchema, FieldSchema, FieldType, MapField, MapKeyType, Schema, TableSchema},
};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;

/// Python Target Generator
pub struct PyTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.py")
    pub file_template: String,
    /// File name of the shared enums module
    pub enums_file: String,
}

impl Default for PyTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/python"),
            file_template: "{table}.py".to_string(),
            enums_file: "cage_enums.py".to_string(),
        }
    }
}

/// Python keywords (hard + soft) that get a trailing `_` (PEP 8 style).
const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "match", "case", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "while", "with", "yield",
];

impl PyTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.py".to_string()),
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
            let content = self.render_table(schema, table, schema_hash);
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
            "# <auto-generated>\n\
             #   Generated by Cage — do not edit.\n\
             #   schema: {hash}\n\
             #   {what}\n\
             # </auto-generated>\n"
        )
    }

    fn render_table(
        &self,
        schema: &Schema,
        table: &TableSchema,
        schema_hash: Option<&str>,
    ) -> String {
        let mut out = String::new();
        out.push_str(&Self::header(
            &format!("table:  {}", table.name),
            schema_hash,
        ));
        let _ = writeln!(out);
        let _ = writeln!(out, "from __future__ import annotations");
        let _ = writeln!(out);

        let fields = Self::sorted_fields(table);

        // Import surface grows with what the module actually uses: `field`
        // only when a mutable (list/dict) default needs a default_factory,
        // `typing.Any` only when an Any-ish annotation appears.
        let needs_field = fields.iter().any(|(_, f)| {
            matches!(&f.field_type, FieldType::Array(_) | FieldType::Map(_))
                && f.default
                    .as_ref()
                    .is_some_and(|d| render_default(d, &f.field_type, schema).is_some())
        });
        let needs_any = fields
            .iter()
            .any(|(_, f)| annotation_uses_any(&f.field_type));
        let _ = writeln!(
            out,
            "from dataclasses import dataclass{}",
            if needs_field { ", field" } else { "" }
        );
        if needs_any {
            let _ = writeln!(out, "from typing import Any");
        }

        // Enum imports: emitted shared enums actually referenced by fields,
        // including through `list[...]`/`dict[...]` nesting.
        let used_enums: Vec<String> = {
            let mut referenced: HashSet<&str> = HashSet::new();
            for (_, f) in &fields {
                annotation_enum_names(&f.field_type, &mut referenced);
            }
            let mut names: Vec<String> = referenced
                .into_iter()
                .filter(|name| {
                    schema
                        .enums
                        .get(*name)
                        .is_some_and(|e| !e.values.is_empty())
                })
                .map(py_ident)
                .collect();
            names.sort();
            names.dedup();
            names
        };
        if !used_enums.is_empty() {
            let _ = writeln!(out);
            let module = self.enums_file.trim_end_matches(".py");
            let _ = writeln!(out, "from {module} import {}", used_enums.join(", "));
        }

        // Class banner: name, primary key, optional description.
        let _ = writeln!(out);
        let _ = writeln!(out);
        let head = if table.primary_key.is_empty() {
            table.name.clone()
        } else {
            format!(
                "{} — primary key: {}",
                table.name,
                table.primary_key.join(", ")
            )
        };
        let _ = writeln!(out, "# {head}");
        if let Some(desc) = &table.description {
            let _ = writeln!(out, "# {desc}");
        }
        let _ = writeln!(out, "@dataclass(frozen=True)");
        let _ = writeln!(out, "class {}:", py_ident(&table.name));

        if fields.is_empty() {
            let _ = writeln!(out, "    pass");
            return out;
        }

        // dataclass syntax: fields without defaults must precede defaulted
        // ones — required fields with no renderable default go first, then
        // everything carrying a default (name order inside each group).
        let mut plain: Vec<(String, String, Option<String>)> = Vec::new();
        let mut defaulted: Vec<(String, String, String, Option<String>)> = Vec::new();
        let mut used: HashSet<String> = HashSet::new();
        for (field_name, field) in &fields {
            let member = unique_ident(py_ident(field_name), &mut used);
            let doc = field_doc(schema, field);
            let default_expr = field
                .default
                .as_ref()
                .filter(|v| !v.is_null())
                .and_then(|d| render_default(d, &field.field_type, schema));
            match default_expr {
                None if field.required => {
                    plain.push((member, py_type(&field.field_type, schema), doc));
                }
                Some(expr) => {
                    // Mutable (list/dict) defaults must go through
                    // default_factory so instances never share one object;
                    // an empty dict spells that with the `dict` constructor.
                    let expr = match &field.field_type {
                        FieldType::Map(_) if expr == "{}" => {
                            "field(default_factory=dict)".to_string()
                        }
                        FieldType::Array(_) | FieldType::Map(_) => {
                            format!("field(default_factory=lambda: {expr})")
                        }
                        _ => expr,
                    };
                    defaulted.push((member, py_type(&field.field_type, schema), expr, doc));
                }
                None => {
                    let ty = format!("{} | None", py_type(&field.field_type, schema));
                    defaulted.push((member, ty, "None".to_string(), doc));
                }
            }
        }

        let mut first = true;
        for (member, ty, doc) in &plain {
            if !first {
                let _ = writeln!(out);
            }
            if let Some(d) = doc {
                let _ = writeln!(out, "    # {d}");
            }
            let _ = writeln!(out, "    {member}: {ty}");
            first = false;
        }
        for (member, ty, expr, doc) in &defaulted {
            if !first {
                let _ = writeln!(out);
            }
            if let Some(d) = doc {
                let _ = writeln!(out, "    # {d}");
            }
            let _ = writeln!(out, "    {member}: {ty} = {expr}");
            first = false;
        }

        out
    }

    fn render_enums(schema: &Schema, schema_hash: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&Self::header("enums:  shared definitions", schema_hash));
        let _ = writeln!(out);
        let _ = writeln!(out);
        let _ = writeln!(out, "from enum import IntEnum");

        // Member backing: an IntEnum is emitted only when EVERY member has
        // an explicit integral value (Python ints are unbounded, so there is
        // no backing-type concern). Otherwise the member names are the
        // values (Cage compares enums as strings) and we emit a plain class
        // of string constants.
        for e in Self::emitted_enums(schema) {
            let all_integral = !e.values.is_empty()
                && e.values.iter().all(|v| {
                    matches!(&v.value, Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64())
                });

            let _ = writeln!(out);
            let _ = writeln!(out);
            match &e.description {
                Some(desc) => {
                    let _ = writeln!(out, "# {} — {}", e.name, desc);
                }
                None => {
                    let _ = writeln!(out, "# {}", e.name);
                }
            }
            let ident = py_ident(&e.name);
            let mut used: HashSet<String> = HashSet::new();
            if all_integral {
                let _ = writeln!(out, "class {ident}(IntEnum):");
                for v in &e.values {
                    if let Some(desc) = &v.description {
                        let _ = writeln!(out, "    # {desc}");
                    }
                    let member = unique_ident(py_ident(&v.name), &mut used);
                    let value = match &v.value {
                        Some(serde_json::Value::Number(n)) => n.to_string(),
                        _ => v.name.clone(),
                    };
                    let _ = writeln!(out, "    {member} = {value}");
                }
            } else {
                let _ = writeln!(out, "class {ident}:");
                for v in &e.values {
                    if let Some(desc) = &v.description {
                        let _ = writeln!(out, "    # {desc}");
                    }
                    let member = unique_ident(py_ident(&v.name), &mut used);
                    let value = match &v.value {
                        Some(serde_json::Value::String(s)) => py_string_literal(s),
                        Some(serde_json::Value::Number(n)) => py_string_literal(&n.to_string()),
                        Some(serde_json::Value::Bool(b)) => {
                            py_string_literal(if *b { "true" } else { "false" })
                        }
                        _ => py_string_literal(&v.name),
                    };
                    let _ = writeln!(out, "    {member} = {value}");
                }
            }
        }

        out
    }
}

/// Python type annotation for a field type (no optionality suffix).
fn py_type(ft: &FieldType, schema: &Schema) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "Any".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => "int".to_string(),
        FieldType::Float32 | FieldType::Float64 => "float".to_string(),
        FieldType::String => "str".to_string(),
        FieldType::Bytes => "bytes".to_string(),
        FieldType::Array(inner) => format!("list[{}]", py_type(inner, schema)),
        FieldType::Object(_) => "dict[str, Any]".to_string(),
        FieldType::Map(map) => format!(
            "dict[{}, {}]",
            map_key_annotation(map.key_type),
            py_type(&map.value_type, schema)
        ),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => py_ident(name),
            // Unresolved (or empty) enum: fall back to plain str.
            None => "str".to_string(),
        },
    }
}

/// Render a schema default as a Python initializer expression; `None` when
/// the default does not map to a compile-safe literal (objects, bytes,
/// mismatched kinds) — the field then defaults to None instead.
fn render_default(value: &serde_json::Value, ft: &FieldType, schema: &Schema) -> Option<String> {
    match ft {
        FieldType::Bool => value.as_bool().map(|b| {
            if b {
                "True".to_string()
            } else {
                "False".to_string()
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
        FieldType::Float32 | FieldType::Float64 => value.as_f64().map(py_float_literal),
        FieldType::String => value.as_str().map(py_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .map(|entries| map_literal(entries, map, schema)),
        _ => None,
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Lua generators — keeps the three outputs aligned).
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
    Some(format!("[{}]", parts.join(", ")))
}

/// Python annotation for a map key type: string keys → `str`, integer
/// keys → `int` (they live as numeric strings in the data model, but the
/// annotation is the semantic type).
fn map_key_annotation(key_type: MapKeyType) -> &'static str {
    match key_type {
        MapKeyType::String => "str",
        MapKeyType::Int => "int",
    }
}

/// Map defaults render entry by entry through the existing rules; entries
/// whose key or value kind does not match are skipped (unlike arrays, where
/// one bad element sinks the whole default), so a partially renderable map
/// still yields its renderable members.
fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> String {
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        let Some(rendered_key) = map_key_literal(key, map.key_type) else {
            continue;
        };
        let Some(rendered_value) = render_default(value, &map.value_type, schema) else {
            continue;
        };
        parts.push(format!("{rendered_key}: {rendered_value}"));
    }
    format!("{{{}}}", parts.join(", "))
}

/// Python literal for one map key. Integer keys arrive as numeric strings
/// (`"42"`, `"-7"`) and become int literals; a non-numeric key does not
/// match an `int` key type and is skipped by the caller.
fn map_key_literal(key: &str, key_type: MapKeyType) -> Option<String> {
    match key_type {
        MapKeyType::String => Some(py_string_literal(key)),
        MapKeyType::Int => key.parse::<i64>().ok().map(|i| i.to_string()),
    }
}

/// Whether the Python annotation for this type mentions `Any` (Null/Any
/// kinds, untyped objects, or any Array/Map nesting thereof) — those
/// modules need `from typing import Any`.
fn annotation_uses_any(ft: &FieldType) -> bool {
    match ft {
        FieldType::Null | FieldType::Any | FieldType::Object(_) => true,
        FieldType::Array(inner) => annotation_uses_any(inner),
        FieldType::Map(map) => annotation_uses_any(&map.value_type),
        _ => false,
    }
}

/// Enum names a field's annotation spells out, collected through
/// `list[...]`/`dict[...]` nesting — the module must import exactly these,
/// or the generated annotation would name an undefined class.
fn annotation_enum_names<'a>(ft: &'a FieldType, out: &mut HashSet<&'a str>) {
    match ft {
        FieldType::Enum(name) => {
            out.insert(name.as_str());
        }
        FieldType::Array(inner) => annotation_enum_names(inner, out),
        FieldType::Map(map) => annotation_enum_names(&map.value_type, out),
        _ => {}
    }
}

fn py_float_literal(f: f64) -> String {
    if f.is_nan() {
        return "float(\"nan\")".to_string();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            "float(\"-inf\")".to_string()
        } else {
            "float(\"inf\")".to_string()
        };
    }
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn py_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Python literals: control characters become
/// `\n` / `\xHH`; printable Unicode passes through.
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
                let _ = write!(out, "\\x{code:02x}");
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

/// Identifier with a trailing `_` for Python keywords (PEP 8 convention).
fn py_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if PY_KEYWORDS.contains(&s.as_str()) {
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
/// there is nothing to say.
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
}
