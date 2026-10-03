//! C# Target Generator - generates C# class bindings from a Cage schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables and fields are emitted in name order, enum
//! values keep their schema order, and no timestamps are written — the same
//! schema always produces byte-identical files.

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

/// C# Target Generator
pub struct CsTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.cs")
    pub file_template: String,
    /// C# namespace for generated types
    pub namespace: String,
    /// File name of the shared enum compilation unit
    pub enums_file: String,
}

impl Default for CsTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/csharp"),
            file_template: "{table}.cs".to_string(),
            namespace: "Cage.Generated".to_string(),
            enums_file: "CageEnums.cs".to_string(),
        }
    }
}

/// C# hard keywords that need a `@` verbatim prefix as identifiers.
const CS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

impl CsTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.cs".to_string()),
            ..Self::default()
        };
        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("namespace") {
                if let Some(s) = v.as_str() {
                    gen.namespace = s.to_string();
                }
            }
            if let Some(v) = opts.get("enums_file") {
                if let Some(s) = v.as_str() {
                    gen.enums_file = s.to_string();
                }
            }
        }
        gen
    }

    /// Generate one artifact per table (name order) plus a shared enums file.
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
            let content = self.render_enums(schema, schema_hash);
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
            "// <auto-generated>\n\
             //   Generated by Cage — do not edit.\n\
             //   schema: {hash}\n\
             //   {what}\n\
             // </auto-generated>\n"
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
        out.push_str("\n#nullable enable\n\n");
        out.push_str("using System;\nusing System.Collections.Generic;\n\n");
        let _ = writeln!(out, "namespace {}\n{{", self.namespace);

        // Class summary: name, primary key, optional description.
        let head = if table.primary_key.is_empty() {
            xml_escape(&table.name)
        } else {
            format!(
                "{} — primary key: {}",
                xml_escape(&table.name),
                xml_escape(&table.primary_key.join(", "))
            )
        };
        match table.description.as_deref() {
            Some(desc) => {
                let desc_esc = xml_escape(desc);
                let _ = writeln!(
                    out,
                    "    /// <summary>\n    /// {head}\n    /// <para>{desc_esc}</para>\n    /// </summary>"
                );
            }
            None => {
                let _ = writeln!(out, "    /// <summary>{head}</summary>");
            }
        }
        let class = cs_ident(&table.name);
        let _ = writeln!(out, "    public sealed class {class}\n    {{");

        let mut used: HashSet<String> = HashSet::new();
        for (i, (field_name, field)) in Self::sorted_fields(table).into_iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if let Some(doc) = field_doc(schema, field) {
                let _ = writeln!(out, "        /// <summary>{doc}</summary>");
            }

            let default = field
                .default
                .as_ref()
                .filter(|v| !v.is_null())
                .and_then(|d| render_default(d, &field.field_type, schema));
            let optional = !field.required && default.is_none();
            let mut ty = cs_type_inner(&field.field_type, schema);
            if optional && !ty.ends_with('?') {
                ty.push('?');
            }
            let init = if let Some(d) = default {
                format!(" = {d};")
            } else if !optional && needs_ref_init(&ty) {
                format!(" = {};", ref_empty_init(&ty))
            } else {
                String::new()
            };

            let member = unique_ident(cs_ident(field_name), &mut used);
            let _ = writeln!(out, "        public {ty} {member} {{ get; init; }}{init}");
        }

        out.push_str("    }\n}\n");
        out
    }

    fn render_enums(&self, schema: &Schema, schema_hash: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&Self::header("enums:  shared definitions", schema_hash));
        out.push_str("\n#nullable enable\n\n");
        out.push_str("using System;\nusing System.Collections.Generic;\n\n");
        let _ = writeln!(out, "namespace {}\n{{", self.namespace);

        for (i, e) in Self::emitted_enums(schema).iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }

            // Member backing: a C# enum is emitted only when EVERY member has
            // an explicit integral value. Otherwise the member names are the
            // values (Cage compares enums as strings) and we emit a static
            // class of string constants — C# enums cannot hold strings.
            let all_integral = !e.values.is_empty()
                && e.values.iter().all(|v| {
                    matches!(&v.value, Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64())
                });
            let mut max_u64: Option<u64> = None;
            let mut has_negative = false;
            if all_integral {
                for v in &e.values {
                    if let Some(serde_json::Value::Number(n)) = &v.value {
                        if let Some(u) = n.as_u64() {
                            max_u64 = Some(max_u64.map_or(u, |m| m.max(u)));
                        } else {
                            has_negative = true;
                        }
                    }
                }
            }

            let summary = match &e.description {
                Some(desc) => format!("{} — {}", xml_escape(&e.name), xml_escape(desc)),
                None => xml_escape(&e.name),
            };
            let ident = cs_ident(&e.name);
            let mut used: HashSet<String> = HashSet::new();

            if all_integral {
                let backing = if has_negative {
                    " : long".to_string()
                } else if max_u64.is_some_and(|m| m > i64::MAX as u64) {
                    " : ulong".to_string()
                } else if max_u64.is_some_and(|m| m > i32::MAX as u64) {
                    " : long".to_string()
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "    /// <summary>{summary}</summary>\n    public enum {ident}{backing}\n    {{"
                );
                for (j, v) in e.values.iter().enumerate() {
                    if j > 0 {
                        out.push('\n');
                    }
                    if let Some(desc) = &v.description {
                        let desc_esc = xml_escape(desc);
                        let _ = writeln!(out, "        /// <summary>{desc_esc}</summary>");
                    }
                    let member = unique_ident(cs_ident(&v.name), &mut used);
                    match &v.value {
                        Some(serde_json::Value::Number(n)) => {
                            let _ = writeln!(out, "        {member} = {n},");
                        }
                        _ => {
                            let _ = writeln!(out, "        {member},");
                        }
                    }
                }
                out.push_str("    }\n");
            } else {
                let _ = writeln!(
                    out,
                    "    /// <summary>{summary}</summary>\n    public static class {ident}\n    {{"
                );
                for (j, v) in e.values.iter().enumerate() {
                    if j > 0 {
                        out.push('\n');
                    }
                    if let Some(desc) = &v.description {
                        let desc_esc = xml_escape(desc);
                        let _ = writeln!(out, "        /// <summary>{desc_esc}</summary>");
                    }
                    let member = unique_ident(cs_ident(&v.name), &mut used);
                    let value = match &v.value {
                        Some(serde_json::Value::String(s)) => cs_string_literal(s),
                        Some(serde_json::Value::Number(n)) => cs_string_literal(&n.to_string()),
                        Some(serde_json::Value::Bool(b)) => cs_string_literal(&b.to_string()),
                        _ => cs_string_literal(&v.name),
                    };
                    let _ = writeln!(out, "        public const string {member} = {value};");
                }
                out.push_str("    }\n");
            }
        }

        out.push_str("}\n");
        out
    }
}

/// C# type for a field type (no optionality suffix).
fn cs_type_inner(ft: &FieldType, schema: &Schema) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "object?".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Int8 => "sbyte".to_string(),
        FieldType::Int16 => "short".to_string(),
        FieldType::Int32 => "int".to_string(),
        FieldType::Int64 => "long".to_string(),
        FieldType::UInt8 => "byte".to_string(),
        FieldType::UInt16 => "ushort".to_string(),
        FieldType::UInt32 => "uint".to_string(),
        FieldType::UInt64 => "ulong".to_string(),
        FieldType::Float32 => "float".to_string(),
        FieldType::Float64 => "double".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "byte[]".to_string(),
        FieldType::Array(inner) => format!("IReadOnlyList<{}>", cs_type_inner(inner, schema)),
        FieldType::Object(_) => "IReadOnlyDictionary<string, object?>".to_string(),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => cs_ident(name),
            // Unresolved (or empty) enum: fall back to plain string.
            None => "string".to_string(),
        },
    }
}

/// A non-nullable reference type with no schema default needs an empty
/// initializer so the file compiles warning-free under `#nullable enable`.
fn needs_ref_init(ty: &str) -> bool {
    !ty.ends_with('?')
        && (ty == "string"
            || ty.starts_with("byte[")
            || ty.starts_with("IReadOnlyList<")
            || ty.starts_with("IReadOnlyDictionary<"))
}

fn ref_empty_init(ty: &str) -> String {
    if ty == "string" {
        "string.Empty".to_string()
    } else if ty.starts_with("byte[") {
        "Array.Empty<byte>()".to_string()
    } else if let Some(inner) = ty
        .strip_prefix("IReadOnlyList<")
        .and_then(|s| s.strip_suffix('>'))
    {
        format!("Array.Empty<{inner}>()")
    } else {
        "new Dictionary<string, object?>()".to_string()
    }
}

/// Render a schema default as a C# initializer literal; `None` when the
/// default does not map to a compile-safe literal (objects, mismatched
/// kinds) — the field then stays optional/nullable.
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
        FieldType::Float32 => value.as_f64().map(|f| cs_float_literal(f, true)),
        FieldType::Float64 => value.as_f64().map(|f| cs_float_literal(f, false)),
        FieldType::String => value.as_str().map(cs_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| render_array_default(items, inner, schema)),
        _ => None,
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the Python/Lua generators — keeps the three outputs aligned).
fn render_array_default(
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
    let elem = cs_type_inner(inner, schema);
    Some(format!("new {elem}[] {{ {} }}", parts.join(", ")))
}

fn cs_float_literal(f: f64, float32: bool) -> String {
    if f.is_nan() {
        return if float32 {
            "float.NaN".to_string()
        } else {
            "double.NaN".to_string()
        };
    }
    if f.is_infinite() {
        let ty = if float32 { "float" } else { "double" };
        let sign = if f < 0.0 { "Negative" } else { "Positive" };
        return format!("{ty}.{sign}Infinity");
    }
    let s = f.to_string();
    if float32 {
        format!("{s}f")
    } else {
        s
    }
}

fn cs_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for C# (and Lua-compatible) literals: control
/// characters become `\n` / `\uXXXX`; printable Unicode passes through.
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
                let _ = write!(out, "\\u{code:04x}");
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

/// Identifier with `@` verbatim prefix for C# hard keywords.
fn cs_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if CS_KEYWORDS.contains(&s.as_str()) {
        format!("@{s}")
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Field doc comment body: description plus constraint summary (already
/// XML-escaped); `None` when there is nothing to say.
fn field_doc(schema: &Schema, field: &FieldSchema) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = &field.description {
        parts.push(xml_escape(d));
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
        parts.push(format!("pattern: {}", xml_escape(p)));
    }
    if let Some(vals) = &field.enum_values {
        let allowed: Vec<String> = vals.iter().map(|v| xml_escape(v)).collect();
        parts.push(format!("allowed: {}", allowed.join(" | ")));
    }
    if let Some(r) = &field.reference {
        parts.push(format!(
            "→ {}.{}",
            xml_escape(&r.table),
            xml_escape(&r.field)
        ));
    }
    if let FieldType::Enum(name) = &field.field_type {
        if schema.enums.get(name).is_none_or(|e| e.values.is_empty()) {
            parts.push(format!("unresolved enum: {}", xml_escape(name)));
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
        assert!(src.contains(
            "public IReadOnlyList<string> tags { get; init; } = new string[] { \"pvp\" };"
        ));
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
    }
}
