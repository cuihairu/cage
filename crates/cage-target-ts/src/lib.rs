//! TypeScript/JavaScript Target Generator — generates typed module bindings
//! from a Cage schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields keep name
//! order, enum values keep their schema order, and no timestamps are written
//! — the same schema always produces byte-identical files.
//!
//! Two modes: TypeScript (`format = typescript|ts`, `{Table}.ts` +
//! `cage_enums.ts`) emits `export interface` + `*Defaults` const + `new*`
//! factory per table with a type-only import of the shared enums module;
//! JavaScript (`format = javascript|js`, `{Table}.js` + `cage_enums.js`)
//! emits `JSDoc` `@typedef`-typed plain modules, optionally paired with a
//! `.d.ts` declaration per file (`emit_dts`, default true).

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
// Stage: crate-prefixed type names (TsTargetGenerator, ...) are idiomatic
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
    schema::{
        EnumSchema, EnumValue, FieldSchema, FieldType, MapField, MapKeyType, Schema, TableSchema,
    },
};
use cage_target_template::TemplateTargetGenerator;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use tera::Tera;

/// Emitted language flavor of the ts crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsMode {
    /// `.ts` modules (format `typescript` / `ts`).
    TypeScript,
    /// `.js` modules with `JSDoc` types, optionally paired with `.d.ts`
    /// (format `javascript` / `js`).
    JavaScript,
}

/// Official templates ship with the crate (design §22 G2): `include_str!`
/// compiles them into the binary so no filesystem is needed.
const TABLE_TS_TEMPLATE: &str = include_str!("../templates/table.ts.tera");
const TABLE_JS_TEMPLATE: &str = include_str!("../templates/table.js.tera");
const TABLE_DTS_TEMPLATE: &str = include_str!("../templates/table.d.ts.tera");
const ENUMS_TS_TEMPLATE: &str = include_str!("../templates/enums.ts.tera");
const ENUMS_JS_TEMPLATE: &str = include_str!("../templates/enums.js.tera");
const ENUMS_DTS_TEMPLATE: &str = include_str!("../templates/enums.d.ts.tera");

/// TypeScript/JavaScript Target Generator
pub struct TsTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.ts")
    pub file_template: String,
    /// File name of the shared enums module
    pub enums_file: String,
    /// Which flavor to emit
    pub mode: TsMode,
    /// JavaScript mode: also emit a paired `.d.ts` (default true)
    pub emit_dts: bool,
}

impl Default for TsTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/typescript"),
            file_template: "{table}.ts".to_string(),
            enums_file: "cage_enums.ts".to_string(),
            mode: TsMode::TypeScript,
            emit_dts: true,
        }
    }
}

impl TsTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.ts".to_string()),
            ..Self::default()
        };
        if matches!(config.format.as_str(), "javascript" | "js") {
            gen.mode = TsMode::JavaScript;
            gen.file_template = config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.js".to_string());
            gen.enums_file = "cage_enums.js".to_string();
        }
        if let Some(opts) = &config.options {
            if let Some(s) = opts.get("enums_file").and_then(|v| v.as_str()) {
                gen.enums_file = s.to_string();
            }
            if let Some(b) = opts.get("emit_dts").and_then(serde_json::Value::as_bool) {
                gen.emit_dts = b;
            }
        }
        gen
    }

    /// Generate one module per table (name order) plus a shared enums module.
    ///
    /// `schema_hash` is the same hash `manifest.json` records for the schema
    /// (Build Manifest 口径); it is stamped into every file header.
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        // Official templates ship with this crate (design §22 G2): rendered
        // from memory, registered in the legacy emission order.
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        match self.mode {
            TsMode::TypeScript => {
                let mut templates: Vec<(&str, &str)> =
                    vec![(self.file_template.as_str(), TABLE_TS_TEMPLATE)];
                if !emitted_enums(schema).is_empty() {
                    templates.push((self.enums_file.as_str(), ENUMS_TS_TEMPLATE));
                }
                engine
                    .generate_official(
                        schema,
                        schema_hash,
                        &templates,
                        |_tera, _schema| {},
                        |schema, table| ts_extras(&self.enums_file, schema, table),
                    )
                    .expect("official TypeScript templates are valid Tera")
            }
            TsMode::JavaScript => {
                // Legacy emission order interleaves each `.js` module with
                // its `.d.ts` declaration (then the enums pair), so the two
                // flavors render in separate passes and zip back together —
                // both passes walk the same table/enums set, so the zip is
                // 1:1.
                let mut mains: Vec<(&str, &str)> =
                    vec![(self.file_template.as_str(), TABLE_JS_TEMPLATE)];
                let dts_table_name = dts_path_for(&self.file_template);
                let dts_enums_name = dts_path_for(&self.enums_file);
                let mut decls: Vec<(&str, &str)> =
                    vec![(dts_table_name.as_str(), TABLE_DTS_TEMPLATE)];
                if !emitted_enums(schema).is_empty() {
                    mains.push((self.enums_file.as_str(), ENUMS_JS_TEMPLATE));
                    decls.push((dts_enums_name.as_str(), ENUMS_DTS_TEMPLATE));
                }
                let extras = |schema: &Schema, table: Option<&TableSchema>| {
                    ts_extras(&self.enums_file, schema, table)
                };
                let main_artifacts = engine
                    .generate_official(schema, schema_hash, &mains, |_tera, _schema| {}, extras)
                    .expect("official JavaScript templates are valid Tera");
                if !self.emit_dts {
                    return main_artifacts;
                }
                let decl_artifacts = engine
                    .generate_official(schema, schema_hash, &decls, |_tera, _schema| {}, extras)
                    .expect("official JavaScript declaration templates are valid Tera");
                debug_assert_eq!(main_artifacts.len(), decl_artifacts.len());
                let mut artifacts = Vec::with_capacity(main_artifacts.len() + decl_artifacts.len());
                for pair in main_artifacts.into_iter().zip(decl_artifacts) {
                    artifacts.push(pair.0);
                    artifacts.push(pair.1);
                }
                artifacts
            }
        }
    }
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

/// Fields in name order (deterministic member order).
fn sorted_fields(table: &TableSchema) -> Vec<(&str, &FieldSchema)> {
    let mut fields: Vec<(&str, &FieldSchema)> =
        table.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
    fields.sort_by(|a, b| a.0.cmp(b.0));
    fields
}

/// Exported binding of each emitted enum, in name order: (schema name,
/// identifier as it appears in the shared enums module).
fn enum_exports(schema: &Schema) -> Vec<(&str, String)> {
    let mut used: HashSet<String> = HashSet::new();
    emitted_enums(schema)
        .into_iter()
        .map(|e| {
            let export = unique_ident(ts_ident(&e.name), &mut used);
            (e.name.as_str(), export)
        })
        .collect()
}

/// Module specifier for a type-only import of the shared enums module:
/// trailing `.ts`/`.js` stripped, `./` prefixed unless already relative.
fn module_specifier(enums_file: &str) -> String {
    let stripped = enums_file
        .strip_suffix(".ts")
        .or_else(|| enums_file.strip_suffix(".js"))
        .unwrap_or(enums_file);
    if stripped.starts_with("./") || stripped.starts_with("../") {
        stripped.to_string()
    } else {
        format!("./{stripped}")
    }
}

/// `JSDoc` inline-import path for the enums module (JS mode): extension is
/// kept (`import("./cage_enums.js").X`), `./` prefixed unless relative.
fn jsdoc_import_path(enums_file: &str) -> String {
    if enums_file.starts_with("./") || enums_file.starts_with("../") {
        enums_file.to_string()
    } else {
        format!("./{enums_file}")
    }
}

/// Banner line under the file docs: table name plus primary key(s), same
/// wording as the Python generator's class banner.
fn banner_line(table: &TableSchema) -> String {
    if table.primary_key.is_empty() {
        table.name.clone()
    } else {
        format!(
            "{} — primary key: {}",
            table.name,
            table.primary_key.join(", ")
        )
    }
}

/// Collect the resolved enums referenced by a field type (recursing into
/// arrays), so a table only imports what it actually uses.
fn collect_enum_refs(
    ft: &FieldType,
    schema: &Schema,
    out: &mut std::collections::BTreeSet<String>,
) {
    match ft {
        FieldType::Enum(name) if schema.enums.get(name).is_some_and(|e| !e.values.is_empty()) => {
            out.insert(name.clone());
        }
        FieldType::Array(inner) => collect_enum_refs(inner, schema, out),
        FieldType::Map(map) => collect_enum_refs(&map.value_type, schema, out),
        _ => {}
    }
}

/// Paired declaration path for a `.js` artifact: trailing `.js` replaced
/// with `.d.ts` (appended when the configured name lacks the extension).
fn dts_path_for(js_path: &str) -> String {
    match js_path.strip_suffix(".js") {
        Some(stem) => format!("{stem}.d.ts"),
        None => format!("{js_path}.d.ts"),
    }
}

/// JS reserved + strict-mode reserved + TS-specific words that can appear
/// as bindings. Escape = trailing `_`, applied ONLY to binding identifiers
/// (interface/const/function/type-alias/import-alias names), never to
/// property keys — `default: string` is legal TS.
const TS_KEYWORDS: &[&str] = &[
    // JS reserved words
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    // strict-mode reserved
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
    "await",
    // TS reserved/contextual usable as bindings
    "declare",
    "namespace",
    "module",
    "type",
    "abstract",
    "readonly",
    "satisfies",
    "keyof",
    "infer",
    "is",
    "asserts",
    "unique",
    "override",
    "any",
    "boolean",
    "number",
    "string",
    "symbol",
    "unknown",
    "never",
    "object",
    "undefined",
    "bigint",
];

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

/// Identifier with a trailing `_` for JS/TS keywords (no escape syntax).
fn ts_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if TS_KEYWORDS.contains(&s.as_str()) {
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
/// there is nothing to say (same format as the C#/Python/Lua generators).
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

/// TypeScript type annotation for a field type (no optionality suffix).
///
/// `enum_ty` maps a resolved enum's schema name to how it is spelled in
/// this file: the (possibly aliased) local binding in TS mode / `.d.ts`,
/// or an inline `import("…")` type in JS-mode `JSDoc`.
fn ts_type(
    ft: &FieldType,
    schema: &Schema,
    enum_ty: &std::collections::HashMap<String, String>,
) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "unknown".to_string(),
        FieldType::Bool => "boolean".to_string(),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64
        | FieldType::Float32
        | FieldType::Float64 => "number".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "Uint8Array".to_string(),
        FieldType::Array(inner) => format!("{}[]", ts_type(inner, schema, enum_ty)),
        FieldType::Map(map) => format!(
            "Map<{}, {}>",
            ts_map_key(map.key_type),
            ts_type(&map.value_type, schema, enum_ty)
        ),
        FieldType::Object(_) => "Record<string, unknown>".to_string(),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => enum_ty.get(name).cloned().unwrap_or_else(|| ts_ident(name)),
            // Unresolved (or empty) enum: fall back to plain string.
            None => "string".to_string(),
        },
    }
}

/// Map key annotation: `string` keys stay strings; int keys are typed as
/// `number` (the data model stores them as numeric strings, but the typed
/// surface spells them as numbers).
fn ts_map_key(key: MapKeyType) -> &'static str {
    match key {
        MapKeyType::String => "string",
        MapKeyType::Int => "number",
    }
}

/// Render a schema default as a TS/JS initializer expression; `None` when
/// the default does not map to a literal (objects, bytes, mismatched kinds).
/// Non-finite floats ARE rendered — `NaN` / `Infinity` are real TS values.
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
        FieldType::Float32 | FieldType::Float64 => value.as_f64().map(ts_float_literal),
        FieldType::String => value.as_str().map(ts_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| map_literal(entries, map, schema)),
        _ => None,
    }
}

/// Value types a collection default can render (`array_literal` and
/// `map_literal` share this gate — same rule as the C#/Python/Lua
/// generators, keeps the outputs aligned).
fn is_defaultable_scalar(ft: &FieldType) -> bool {
    matches!(
        ft,
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
    )
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Python/Lua generators — keeps the outputs aligned).
fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    schema: &Schema,
) -> Option<String> {
    if !is_defaultable_scalar(inner) {
        return None;
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(render_default(item, inner, schema)?);
    }
    Some(format!("[{}]", parts.join(", ")))
}

/// Map defaults render as `new Map(...)` entries (never an object literal —
/// the typed surface is `Map<K, V>`): an empty object gives `new Map()`,
/// other values follow the array rule (scalar value types only, and any
/// unrenderable entry sinks the whole default). Keys are strings on the
/// wire; an int-keyed map's numeric-string keys are promoted to number
/// literals (a non-numeric key is a mismatched kind → dropped).
fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> Option<String> {
    if !is_defaultable_scalar(&map.value_type) {
        return None;
    }
    if entries.is_empty() {
        return Some("new Map()".to_string());
    }
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        let key_lit = match map.key_type {
            MapKeyType::String => ts_string_literal(key),
            MapKeyType::Int => key.parse::<i64>().ok()?.to_string(),
        };
        let value_lit = render_default(value, &map.value_type, schema)?;
        parts.push(format!("[{key_lit}, {value_lit}]"));
    }
    Some(format!("new Map([{}])", parts.join(", ")))
}

/// TS/JS float literal: non-finite values render as the real `NaN` /
/// `Infinity` / `-Infinity` identifiers; finite values keep a `.`/`e`.
fn ts_float_literal(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }
    f.to_string()
}

fn ts_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for TS/JS literals: control characters become
/// `\n` / `\t` / `\xHH` (valid in both); printable Unicode passes through.
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

/// How a resolved enum type is spelled inside one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnumStyle {
    /// Type-only import binding (TS mode and `.d.ts` files), possibly
    /// aliased when it collides with a local declaration.
    Local,
    /// `JSDoc` inline `import("…").X` type (JS mode table files).
    InlineImport,
}

/// One interface/`JSDoc` property, in schema name order.
struct Member<'a> {
    /// Property key: sanitized + uniqued, never keyword-escaped.
    prop: String,
    /// Field type (rendered per file style).
    ft: &'a FieldType,
    /// Constraint summary comment, when there is anything to say.
    doc: Option<String>,
    /// Whether the field must be present.
    required: bool,
    /// Rendered default literal, when the default maps to one.
    default: Option<String>,
}

impl Member<'_> {
    /// Optional members carry no renderable default (see the optionality
    /// group rule); in `JSDoc` they get `[name]` brackets.
    fn is_optional(&self) -> bool {
        !self.required && self.default.is_none()
    }

    /// `JSDoc` property name: optional members get `[name]` brackets.
    fn jsdoc_name(&self) -> String {
        if self.is_optional() {
            format!("[{}]", self.prop)
        } else {
            self.prop.clone()
        }
    }
}

/// Whether a `@property` description may follow `[name] ` on the same
/// line. The `JSDoc` scanner re-lexes what comes after the closing bracket,
/// so the description must start with a cleanly-scannable token: quotes,
/// backslashes and backticks open unterminated literals, non-ASCII
/// punctuation is an “Invalid character” (TS1127), and a number glued to
/// an identifier (`1st`) is TS1351. Anything else moves the doc to its
/// own continuation line — same text, still a valid comment.
fn jsdoc_desc_safe(doc: &str) -> bool {
    let Some(first) = doc.chars().next() else {
        return true;
    };
    match first {
        '"' | '\\' | '`' => false,
        c if c.is_ascii_digit() => digit_leading_word_ok(doc),
        c if c.is_ascii() => c.is_ascii_graphic(),
        c => c.is_alphabetic(),
    }
}

/// For a digit-leading description, the leading numeric literal must not
/// be glued to an identifier (`1st` → TS1351 after `[name] `).
fn digit_leading_word_ok(doc: &str) -> bool {
    let word = doc.split_whitespace().next().unwrap_or("");
    let mut rest = word;
    if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        if hex.is_empty() {
            return false;
        }
        let end = hex
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(hex.len());
        rest = &hex[end..];
    } else {
        let end = word
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(word.len());
        rest = &word[end..];
        if let Some(exp) = rest.strip_prefix(['e', 'E']) {
            let digits = exp.strip_prefix(['+', '-']).unwrap_or(exp);
            if digits.starts_with(|c: char| c.is_ascii_digit()) {
                let end = digits
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(digits.len());
                rest = &digits[end..];
            }
        }
    }
    // An identifier character right after the number is glued to it.
    !rest.starts_with(|c: char| c == '_' || c == '$' || c.is_alphabetic())
}

/// Everything a table file needs, computed once per file so interface,
/// defaults and factory can never disagree.
struct TablePlan<'a> {
    /// Keyword-escaped binding for the interface / typedef / factory.
    table_ident: String,
    /// (export ident, local binding) pairs for the type-only import.
    import_rows: Vec<(String, String)>,
    /// Enum schema name → type expression in this file.
    enum_ty: std::collections::HashMap<String, String>,
    /// Properties in field name order.
    members: Vec<Member<'a>>,
}

/// Compute the per-file plan: binding namespace, enum imports/aliases and
/// members in field name order.
fn plan_table<'a>(
    schema: &'a Schema,
    table: &'a TableSchema,
    style: EnumStyle,
    enums_file: &str,
) -> TablePlan<'a> {
    let table_ident = ts_ident(&table.name);
    let mut used_bindings: HashSet<String> = HashSet::new();
    let mut used_props: HashSet<String> = HashSet::new();

    let exports = enum_exports(schema);
    let mut used_enums = std::collections::BTreeSet::new();
    for (_, field) in sorted_fields(table) {
        collect_enum_refs(&field.field_type, schema, &mut used_enums);
    }

    let mut import_rows = Vec::new();
    let mut enum_ty = std::collections::HashMap::new();
    match style {
        EnumStyle::Local => {
            used_bindings.insert(table_ident.clone());
            used_bindings.insert(format!("{table_ident}Defaults"));
            used_bindings.insert(format!("new{table_ident}"));
            for (name, export) in &exports {
                if !used_enums.contains(*name) {
                    continue;
                }
                let local = unique_ident(export.clone(), &mut used_bindings);
                import_rows.push((export.clone(), local.clone()));
                enum_ty.insert((*name).to_string(), local);
            }
        }
        EnumStyle::InlineImport => {
            let path = jsdoc_import_path(enums_file);
            for (name, export) in &exports {
                if used_enums.contains(*name) {
                    enum_ty.insert((*name).to_string(), format!("import(\"{path}\").{export}"));
                }
            }
        }
    }

    let mut members = Vec::with_capacity(table.fields.len());
    for (field_name, field) in sorted_fields(table) {
        let default = field
            .default
            .as_ref()
            .filter(|v| !v.is_null())
            .and_then(|d| render_default(d, &field.field_type, schema));
        members.push(Member {
            prop: unique_ident(sanitize_ident(field_name), &mut used_props),
            ft: &field.field_type,
            doc: field_doc(schema, field),
            required: field.required,
            default,
        });
    }

    TablePlan {
        table_ident,
        import_rows,
        enum_ty,
        members,
    }
}

/// String-bucket value for one enum member: `String` as-is, `Number` as
/// its decimal string, `Bool` as `"true"`/`"false"`, anything else (or
/// missing) as the member name (Cage compares enums as strings).
fn enum_string_value(v: &EnumValue) -> String {
    match &v.value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        _ => v.name.clone(),
    }
}

/// Language precomputation for the official TypeScript/JavaScript
/// templates (design §22 G2): every decision the legacy renderer made —
/// import surface, both enum spellings (Local binding vs inline
/// `import("…")`), `JSDoc` scanner safety, defaults and factory inlining —
/// is computed here; the templates only express text shape. The context
/// serves all registered flavors of one mode: `.ts` uses the Local `ty`,
/// `.js` the braced `ty_jsdoc`, `.d.ts` the Local `ty` again.
//
// Contract: returns `Result` to match the `extras` hook signature even
// though this precomputation is infallible.
#[allow(clippy::unnecessary_wraps)]
fn ts_extras(
    enums_file: &str,
    schema: &Schema,
    table: Option<&TableSchema>,
) -> Result<Value, String> {
    if let Some(t) = table {
        // Both enum spellings, so every registered flavor can pick its
        // own: `.ts`/`.d.ts` render Local types, `.js` inline imports.
        let plan_local = plan_table(schema, t, EnumStyle::Local, enums_file);
        let plan_inline = plan_table(schema, t, EnumStyle::InlineImport, enums_file);

        // Type-only import of the enums actually used (TS table files and
        // `.d.ts`; JSDoc spells enums through inline `import("…")` types).
        let import_line: Option<String> = if plan_local.import_rows.is_empty() {
            None
        } else {
            let names = plan_local
                .import_rows
                .iter()
                .map(|(export, local)| {
                    if export == local {
                        export.clone()
                    } else {
                        format!("{export} as {local}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "import type {{ {names} }} from \"{}\";",
                module_specifier(enums_file)
            ))
        };

        let members: Vec<Value> = plan_local
            .members
            .iter()
            .zip(plan_inline.members.iter())
            .map(|(m, mj)| {
                let doc = m.doc.clone();
                // Bracketed names re-lex the description's first
                // character; unsafe starters move to a continuation line.
                let doc_on_line = match &doc {
                    Some(d) => !m.is_optional() || jsdoc_desc_safe(d),
                    None => true,
                };
                json!({
                    "prop": m.prop,
                    "opt": if m.is_optional() { "?" } else { "" },
                    "ty": ts_type(m.ft, schema, &plan_local.enum_ty),
                    "ty_jsdoc": format!("{{{}}}", ts_type(mj.ft, schema, &plan_inline.enum_ty)),
                    "jsdoc_name": m.jsdoc_name(),
                    "doc": doc,
                    "doc_on_line": doc_on_line,
                })
            })
            .collect();

        // Defaults const and factory are rendered from the same collection
        // so they can never drift; the factory inlines fresh literals so
        // rows never share arrays.
        let default_rows: Vec<Value> = plan_local
            .members
            .iter()
            .filter_map(|m| {
                m.default
                    .as_ref()
                    .map(|d| json!({ "prop": m.prop, "expr": d }))
            })
            .collect();
        let factory_inline = plan_local
            .members
            .iter()
            .filter_map(|m| m.default.as_ref().map(|d| format!("{}: {d}", m.prop)))
            .collect::<Vec<_>>()
            .join(", ");

        Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "description": t.description,
            "banner": banner_line(t),
            "ident": plan_local.table_ident,
            "import_line": import_line,
            "interface_empty": plan_local.members.is_empty(),
            "defaults_empty": default_rows.is_empty(),
            "factory_inline": factory_inline,
            "default_rows": default_rows,
            "members": members,
        }))
    } else {
        let enums: Vec<Value> = enum_exports(schema)
            .into_iter()
            .map(|(name, export)| {
                // `emitted_enums` already dropped empty enums.
                let e = &schema.enums[name];
                // Numeric bucket only when every member carries an
                // integral value; otherwise members are stringified (name
                // as fallback). The bucket literal is shared by the
                // runtime rows and the `.d.ts` readonly pairs (the only
                // textual divergence, a non-number value in the integral
                // bucket, is unreachable by construction).
                let all_integral = e.values.iter().all(|v| {
                    matches!(
                        &v.value,
                        Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64()
                    )
                });
                let mut used: HashSet<String> = HashSet::new();
                let members: Vec<Value> = e
                    .values
                    .iter()
                    .map(|v| {
                        let key = unique_ident(sanitize_ident(&v.name), &mut used);
                        let value = if all_integral {
                            match &v.value {
                                Some(serde_json::Value::Number(n)) => n.to_string(),
                                _ => v.name.clone(),
                            }
                        } else {
                            ts_string_literal(&enum_string_value(v))
                        };
                        json!({ "key": key, "value": value, "desc": v.description })
                    })
                    .collect();
                let base = if all_integral { "number" } else { "string" };
                let typedef_line = format!("/** @typedef {{{base}}} {export} */");
                json!({
                    "name": e.name,
                    "export": export,
                    "description": e.description,
                    "typedef_line": typedef_line,
                    "members": members,
                })
            })
            .collect();
        Ok(json!({
            "header_what": "enums:  shared definitions",
            "emitted_enums": enums,
        }))
    }
}

// ————— G4 filter library (design §22): `ts_type` / `ts_default` —————

fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// `{{ field | ts_type }}` — the TypeScript type text (Local enum
/// spellings, as on the `.ts` / `.d.ts` surface); enum references resolve
/// through the shared export names.
struct TsTypeFilter {
    schema: Schema,
    enum_ty: HashMap<String, String>,
}

impl tera::Filter for TsTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, _field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        Ok(Value::String(ts_type(&ft, &self.schema, &self.enum_ty)))
    }
}

/// `{{ field | ts_default }}` — the TypeScript literal for the field's
/// default, or null when the field has no renderable default.
struct TsDefaultFilter {
    schema: Schema,
}

impl tera::Filter for TsDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the TypeScript filter library for user templates
/// (`options.lang_filters = "ts"`).
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    let enum_ty: HashMap<String, String> = enum_exports(schema)
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    tera.register_filter(
        "ts_type",
        TsTypeFilter {
            schema: schema.clone(),
            enum_ty,
        },
    );
    tera.register_filter(
        "ts_default",
        TsDefaultFilter {
            schema: schema.clone(),
        },
    );
}

#[cfg(test)]
mod tests {
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
        assert!(
            enums_src.contains("export type ItemKind = (typeof ItemKind)[keyof typeof ItemKind];")
        );
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
        assert!(
            ghost_dts.contains("export declare function newGhost(init?: Partial<Ghost>): Ghost;")
        );
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
        assert!(enums_dts
            .contains("export declare const Huge: { readonly Max: 18446744073709551615 };"));
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
        assert!(
            enums_dts.contains("export type ItemKind = (typeof ItemKind)[keyof typeof ItemKind];")
        );
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
}
