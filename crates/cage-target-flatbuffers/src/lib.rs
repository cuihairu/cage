//! `FlatBuffers` Target Generator - generates `.fbs` definition files from the Cage schema
//!
//! `FlatBuffers` consumers must have a compiled schema before they can read
//! data, so the landed form is the schema-driven definition generation
//! shared with the `Protobuf` target: each table becomes a `FlatBuffers`
//! `table` in a per-table `.fbs` file, shared integer-backed enums live in
//! one `cage_enums.fbs` included on demand. Data itself is not embedded —
//! consumers run `flatc` against these definitions with their own runtime.
//! Wire encoding is explicitly out of scope (design §49: no flatc in this
//! repo to act as a byte-level oracle; deterministic binary needs are
//! covered by the `MessagePack` target).

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: enum values are numeric literals — casts are the mapping's job.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
// Stage: crate-prefixed type names (FlatbuffersTargetGenerator, ...) are
// idiomatic across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: the generator renders into an owned String buffer.
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
use cage_core::manifest::TargetConfig;
use cage_core::schema::{FieldType, MapKeyType, Schema};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

/// .fbs keywords plus the scalar type spellings — all escaped with a
/// trailing `_` when they show up as identifiers. Conservative on purpose:
/// flatc accepts some of these contextually, but a fixed deny list keeps
/// generated files valid across flatc versions.
const FBS_KEYWORDS: &[&str] = &[
    "attribute",
    "bit_flags",
    "bool",
    "byte",
    "cpp_type",
    "cpp_ptr_type",
    "cpp_ptr_type_get",
    "deprecated",
    "double",
    "enum",
    "false",
    "file_extension",
    "file_identifier",
    "float",
    "float32",
    "float64",
    "force_align",
    // "id" is not a grammar keyword — flatc's own reflection schema.fbs
    // declares `id: ushort` as a plain field name — so primary-key fields
    // called `id` keep their name.
    "include",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "key",
    "length",
    "long",
    "namespace",
    "native_inline",
    "native_type",
    "native_default",
    "nested_flatbuffer",
    "null",
    "object",
    "required",
    "root_type",
    "rpc_service",
    "shared",
    "short",
    "stream",
    "string",
    "struct",
    "table",
    "true",
    "ubyte",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "union",
    "ulong",
    "ushort",
];

/// One emitted .fbs enum: allocated ident plus resolved members.
struct FbEnum {
    /// Enum schema name (key in `Schema::enums`)
    name: String,
    /// Allocated .fbs identifier
    ident: String,
    /// `(member name, value)` in schema order
    members: Vec<(String, i64)>,
    /// Smallest legal `FlatBuffers` enum base type that fits all members
    base: &'static str,
}

/// Per-table render state gathered during the field pass.
#[derive(Default)]
struct TableCtx {
    /// Table references at least one shared enum (→ include enums file)
    enum_refs: BTreeSet<String>,
    /// Wrapper tables, first-encounter order, already rendered blocks
    wrappers: Vec<String>,
    /// Identifiers taken inside the table file (fields + wrappers)
    used: HashSet<String>,
}

/// A type that fits its position directly, or the wrapper table needed to
/// host an array/map there.
enum Composite {
    Inline(String),
    Wrapper(String, Vec<String>),
}

/// `FlatBuffers` Target Generator
///
/// Determinism contract: tables and fields iterate in name order, wrapper
/// tables resolve through a shared name-order allocator, and all decisions
/// are pure functions of the schema — the same schema therefore yields
/// byte-identical `.fbs` files.
pub struct FlatbuffersTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.fbs")
    pub file_template: String,
    /// File name of the shared enums file
    pub enums_file: String,
    /// namespace clause value (e.g., "cage.generated")
    pub namespace: String,
}

impl Default for FlatbuffersTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/fbs"),
            file_template: "{table}.fbs".to_string(),
            enums_file: "cage_enums.fbs".to_string(),
            namespace: "cage.generated".to_string(),
        }
    }
}

impl FlatbuffersTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.fbs".to_string()),
            enums_file: "cage_enums.fbs".to_string(),
            namespace: "cage.generated".to_string(),
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

    /// Generate `.fbs` artifacts for all tables in the schema.
    ///
    /// Artifact order: tables in name order, shared enums file last
    /// (registered only when at least one legal integer-backed enum
    /// exists — same rule as the proto generator's enums file).
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        // Shared name allocation, tables first then enums (family rule:
        // table names win, enum names avoid), both in name order.
        let mut used: HashSet<String> = HashSet::new();
        let mut table_names: Vec<&String> = schema.tables.keys().collect();
        table_names.sort();
        let table_idents: Vec<(String, String)> = table_names
            .into_iter()
            .map(|name| (name.clone(), unique_ident(escape_ident(name), &mut used)))
            .collect();
        let mut enum_names: Vec<&String> = schema.enums.keys().collect();
        enum_names.sort();
        let enum_idents: Vec<(String, String)> = enum_names
            .into_iter()
            .map(|name| (name.clone(), unique_ident(escape_ident(name), &mut used)))
            .collect();

        // Integer-backed, duplicate-free enums become .fbs enums; everything
        // else (string members, missing literals, out-of-range values,
        // duplicate values — flatc has no allow_alias) falls back to
        // `string` fields and stays out of the files.
        let fb_enums: Vec<FbEnum> = enum_idents
            .iter()
            .filter_map(|(name, ident)| Self::resolve(schema, name, ident))
            .collect();
        let eligible: BTreeSet<String> = fb_enums.iter().map(|e| e.name.clone()).collect();

        let mut artifacts = Vec::new();
        for (table_name, table_ident) in &table_idents {
            let table = &schema.tables[table_name];
            let content = self.render_table(table, table_ident, &eligible, &fb_enums, schema_hash);
            let file_name = self.file_template.replace("{table}", table_name);
            artifacts.push((self.path(&file_name), content.into_bytes()));
        }
        if !fb_enums.is_empty() {
            let content = self.render_enums(&enum_idents, &fb_enums, schema_hash);
            artifacts.push((self.path(&self.enums_file), content.into_bytes()));
        }
        artifacts
    }

    fn path(&self, file_name: &str) -> String {
        self.output_dir
            .join(file_name)
            .to_string_lossy()
            .to_string()
    }

    /// Resolve one schema enum into a .fbs enum, or `None` when any member
    /// disqualifies the whole enum (non-integer literal, value outside the
    /// i64 domain, duplicate value — flatc rejects duplicates outright).
    fn resolve(schema: &Schema, name: &str, ident: &str) -> Option<FbEnum> {
        let def = schema.enums.get(name)?;
        let mut members = Vec::with_capacity(def.values.len());
        let mut seen: HashSet<i64> = HashSet::new();
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        for value in &def.values {
            let literal = integer_literal(value.value.as_ref())?;
            if !seen.insert(literal) {
                return None;
            }
            min = min.min(literal);
            max = max.max(literal);
            members.push((value.name.clone(), literal));
        }
        if members.is_empty() {
            return None;
        }
        Some(FbEnum {
            name: name.to_string(),
            ident: ident.to_string(),
            members,
            base: enum_base(min, max),
        })
    }

    /// Render one table file: header, include, namespace, table, `root_type`.
    fn render_table(
        &self,
        table: &cage_core::schema::TableSchema,
        table_ident: &str,
        eligible: &BTreeSet<String>,
        fb_enums: &[FbEnum],
        schema_hash: Option<&str>,
    ) -> String {
        let mut ctx = TableCtx::default();
        let mut fields = Vec::new();

        // Fields in name order (the .fbs field order is wire-irrelevant, but
        // determinism of the *bytes* requires a fixed order anyway).
        let mut names: Vec<&String> = table.fields.keys().collect();
        names.sort();
        // (key) attribute: a single-field primary key whose mapped type is
        // string or an integer scalar — flatc's eligibility rule.
        let key_field = table
            .primary_key
            .first()
            .filter(|_| table.primary_key.len() == 1)
            .map(String::as_str);
        for field_name in &names {
            let field = &table.fields[*field_name];
            let field_ident = unique_ident(escape_ident(field_name), &mut ctx.used);
            let is_key = Some(field_name.as_str()) == key_field;
            self.render_field(
                field,
                &field.name,
                &field_ident,
                is_key,
                eligible,
                fb_enums,
                &mut ctx,
                &mut fields,
            );
        }

        let mut out = String::new();
        Self::header(&mut out, schema_hash, &format!("table: {table_ident}"));
        out.push('\n');
        if !ctx.enum_refs.is_empty() {
            let _ = writeln!(out, "include \"{}\";", self.enums_file);
            out.push('\n');
        }
        let _ = writeln!(out, "namespace {};", self.namespace);

        out.push('\n');
        let _ = writeln!(
            out,
            "// {} — primary key: {}",
            table.name,
            table.primary_key.join(", ")
        );
        if let Some(desc) = &table.description {
            let _ = writeln!(out, "// {}", comment_text(desc));
        }
        let _ = writeln!(out, "table {table_ident} {{");
        for line in &fields {
            let _ = writeln!(out, "  {line}");
        }
        for wrapper in &ctx.wrappers {
            out.push('\n');
            for line in wrapper.lines() {
                let _ = writeln!(out, "  {line}");
            }
        }
        let _ = writeln!(out, "}}");
        out.push('\n');
        let _ = writeln!(out, "root_type {table_ident};");
        out
    }

    /// Render one field line (plus any wrapper tables it needs) into
    /// `fields` / `ctx.wrappers`.
    #[allow(clippy::too_many_arguments)]
    fn render_field(
        &self,
        field: &cage_core::schema::FieldSchema,
        display_name: &str,
        field_ident: &str,
        is_key: bool,
        eligible: &BTreeSet<String>,
        fb_enums: &[FbEnum],
        ctx: &mut TableCtx,
        fields: &mut Vec<String>,
    ) {
        let type_text =
            self.field_type(&field.field_type, display_name, 0, eligible, fb_enums, ctx);

        if let Some(desc) = &field.description {
            fields.push(format!("// {}", comment_text(desc)));
        }
        // Degrading an enum reference or a free-form payload to `string` is
        // a data-shape decision worth flagging at the use site.
        match &field.field_type {
            FieldType::Enum(name) if !eligible.contains(name) => {
                fields.push(format!(
                    "// enum \"{name}\" has non-integer or duplicate members; field degrades to string"
                ));
            }
            FieldType::Object(_) | FieldType::Null | FieldType::Any => {
                fields.push(
                    "// free-form payload; .fbs has no open type, carried as JSON text".to_string(),
                );
            }
            _ => {}
        }
        // Attributes ride one comma-separated group, alphabetical (a single
        // `(...)` list is valid .fbs regardless of flatc's repeated-group
        // support). `required` is only legal on strings, vectors, and
        // tables — scalar fields keep FlatBuffers' default-value semantics
        // for optionality; `key` only when flatc's eligibility holds.
        let mut attributes = Vec::new();
        if is_key {
            attributes.push("key");
        }
        if field.required && required_eligible(&type_text) {
            attributes.push("required");
        }
        attributes.sort_unstable();
        match attributes.as_slice() {
            [] => fields.push(format!("{field_ident}: {type_text};")),
            [one] => fields.push(format!("{field_ident}: {type_text} ({one});")),
            many => fields.push(format!("{field_ident}: {type_text} ({});", many.join(", "))),
        }
    }

    /// .fbs type text for one field type. `base` names the field (wrapper
    /// table naming), `depth` counts wrapper nesting for deterministic
    /// name suffixes.
    fn field_type(
        &self,
        ft: &FieldType,
        base: &str,
        depth: usize,
        eligible: &BTreeSet<String>,
        fb_enums: &[FbEnum],
        ctx: &mut TableCtx,
    ) -> String {
        match ft {
            FieldType::Array(elem) => {
                match self.composite(elem, base, depth + 1, eligible, fb_enums, ctx) {
                    Composite::Inline(text) => format!("[{text}]"),
                    Composite::Wrapper(name, lines) => {
                        ctx.wrappers.push(render_wrapper_table(&name, &lines));
                        format!("[{name}]")
                    }
                }
            }
            FieldType::Map(map) => {
                // FlatBuffers has no native map: the vector-of-entries
                // idiom. Integer keys widen to long (family rule — the data
                // model stores numeric keys as strings, proto int64 alike).
                let key = match map.key_type {
                    MapKeyType::String => "string".to_string(),
                    MapKeyType::Int => "long".to_string(),
                };
                // Wrapper naming shares the array convention: the suffix
                // depth counts the nesting level *inside* this map, so a
                // top-level map (depth 0) names its wrapper `{Field}Value`.
                let wrapper_depth = depth + 1;
                match self.composite(
                    &map.value_type,
                    base,
                    wrapper_depth,
                    eligible,
                    fb_enums,
                    ctx,
                ) {
                    Composite::Inline(text) => {
                        let name = unique_ident(
                            format!("{}{}", escape_ident(base), "Value".repeat(wrapper_depth)),
                            &mut ctx.used,
                        );
                        ctx.wrappers.push(render_wrapper_table(
                            &name,
                            &[format!("key: {key};"), format!("value: {text};")],
                        ));
                        format!("[{name}]")
                    }
                    Composite::Wrapper(name, lines) => {
                        // The map-entry wrapper hosts the composite value:
                        // emit the inner wrapper first, then the entry.
                        ctx.wrappers.push(render_wrapper_table(&name, &lines));
                        let entry = unique_ident(
                            format!("{}{}", escape_ident(base), "Entry".repeat(wrapper_depth)),
                            &mut ctx.used,
                        );
                        ctx.wrappers.push(render_wrapper_table(
                            &entry,
                            &[format!("key: {key};"), format!("value: {name};")],
                        ));
                        format!("[{entry}]")
                    }
                }
            }
            // No open type in .fbs: free-form payloads ride as JSON text.
            FieldType::Null | FieldType::Any | FieldType::Object(_) | FieldType::String => {
                "string".to_string()
            }
            FieldType::Enum(name) => {
                if eligible.contains(name) {
                    let ident = &fb_enums
                        .iter()
                        .find(|e| e.name == *name)
                        .map(|e| e.ident.clone())
                        .unwrap_or_default();
                    ctx.enum_refs.insert(ident.clone());
                    ident.clone()
                } else {
                    "string".to_string()
                }
            }
            FieldType::Bool => "bool".to_string(),
            FieldType::Bytes => "[ubyte]".to_string(),
            FieldType::Int8 => "byte".to_string(),
            FieldType::Int16 => "short".to_string(),
            FieldType::Int32 => "int".to_string(),
            FieldType::Int64 => "long".to_string(),
            FieldType::UInt8 => "ubyte".to_string(),
            FieldType::UInt16 => "ushort".to_string(),
            FieldType::UInt32 => "uint".to_string(),
            FieldType::UInt64 => "ulong".to_string(),
            FieldType::Float32 => "float".to_string(),
            FieldType::Float64 => "double".to_string(),
        }
    }

    /// Type text for a position that cannot carry `[…]` directly (element
    /// of a vector, value of a map entry): arrays and maps there are hosted
    /// in a wrapper table named `{Field}Value` (`{Field}ValueValue`, ...).
    fn composite(
        &self,
        ft: &FieldType,
        base: &str,
        depth: usize,
        eligible: &BTreeSet<String>,
        fb_enums: &[FbEnum],
        ctx: &mut TableCtx,
    ) -> Composite {
        match ft {
            FieldType::Array(_) | FieldType::Map(_) => {
                let name = unique_ident(
                    format!("{}{}", escape_ident(base), "Value".repeat(depth)),
                    &mut ctx.used,
                );
                let inner = match ft {
                    FieldType::Array(elem) => {
                        let text =
                            match self.composite(elem, base, depth + 1, eligible, fb_enums, ctx) {
                                Composite::Inline(text) => text,
                                Composite::Wrapper(wname, wlines) => {
                                    ctx.wrappers.push(render_wrapper_table(&wname, &wlines));
                                    wname
                                }
                            };
                        format!("items: [{text}];")
                    }
                    FieldType::Map(map) => {
                        let key = match map.key_type {
                            MapKeyType::String => "string".to_string(),
                            MapKeyType::Int => "long".to_string(),
                        };
                        let text = match self.composite(
                            &map.value_type,
                            base,
                            depth + 1,
                            eligible,
                            fb_enums,
                            ctx,
                        ) {
                            Composite::Inline(text) => format!("value: {text};"),
                            Composite::Wrapper(wname, wlines) => {
                                ctx.wrappers.push(render_wrapper_table(&wname, &wlines));
                                format!("value: {wname};")
                            }
                        };
                        format!("key: {key}; {text}")
                    }
                    _ => unreachable!("composite called on scalar"),
                };
                Composite::Wrapper(name, vec![inner])
            }
            other => {
                Composite::Inline(self.field_type(other, base, depth, eligible, fb_enums, ctx))
            }
        }
    }

    /// Render the shared enums file (header, namespace, enums).
    fn render_enums(
        &self,
        enum_idents: &[(String, String)],
        fb_enums: &[FbEnum],
        schema_hash: Option<&str>,
    ) -> String {
        let mut out = String::new();
        Self::header(&mut out, schema_hash, "shared enums");
        out.push('\n');
        let _ = writeln!(out, "namespace {};", self.namespace);
        for (name, ident) in enum_idents {
            let Some(fb_enum) = fb_enums.iter().find(|e| e.name == *name) else {
                continue;
            };
            out.push('\n');
            let _ = writeln!(out, "// {ident}");
            let _ = writeln!(out, "enum {} : {} {{", fb_enum.ident, fb_enum.base);
            for (member, value) in &fb_enum.members {
                // Trailing comma on the last member: flatc accepts it, and
                // it keeps member lines uniform.
                let _ = writeln!(out, "  {} = {value},", escape_ident(member));
            }
            let _ = writeln!(out, "}}");
        }
        out
    }

    /// Standard Cage header block (tooling marker + schema hash + subject).
    fn header(out: &mut String, schema_hash: Option<&str>, subject: &str) {
        let _ = writeln!(out, "// Code generated by Cage — DO NOT EDIT.");
        out.push_str("//\n");
        let _ = writeln!(out, "// Generated by Cage — do not edit.");
        if let Some(hash) = schema_hash {
            let _ = writeln!(out, "// schema: {hash}");
        }
        let _ = writeln!(out, "// {subject}");
    }
}

/// Smallest legal `FlatBuffers` enum base type covering [min, max] — the
/// byte→ubyte→short→ushort→int→uint→long→ulong ladder.
fn enum_base(min: i64, max: i64) -> &'static str {
    if min >= 0 {
        if max <= u8::MAX as i64 {
            "ubyte"
        } else if max <= u16::MAX as i64 {
            "ushort"
        } else if max <= u32::MAX as i64 {
            "uint"
        } else {
            "ulong"
        }
    } else if min >= i8::MIN as i64 && max <= i8::MAX as i64 {
        "byte"
    } else if min >= i16::MIN as i64 && max <= i16::MAX as i64 {
        "short"
    } else if min >= i32::MIN as i64 && max <= i32::MAX as i64 {
        "int"
    } else {
        "long"
    }
}

/// `required` is legal on strings, vectors, and tables only.
fn required_eligible(type_text: &str) -> bool {
    type_text == "string" || type_text.starts_with('[')
}

/// Render a (nested) wrapper table block with its field lines.
fn render_wrapper_table(name: &str, lines: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "table {name} {{");
    for line in lines {
        let _ = writeln!(out, "  {line}");
    }
    let _ = writeln!(out, "}}");
    // Trim the trailing newline — callers re-indent line by line.
    out.trim_end_matches('\n').to_string()
}

/// Integer literal behind an enum member, if any.
fn integer_literal(value: Option<&serde_json::Value>) -> Option<i64> {
    match value {
        Some(serde_json::Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                Some(i)
            } else {
                n.as_u64().map(|u| u as i64)
            }
        }
        _ => None,
    }
}

/// Sanitize a schema name into a valid .fbs identifier (ASCII alnum + `_`,
/// never starting with a digit, never a keyword).
fn escape_ident(name: &str) -> String {
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
    // flatc's keyword set is all-lowercase, but identifier comparison here is
    // case-insensitive: `Table` is a legal type name yet escapes too, so no
    // casing of a grammar keyword can ever collide.
    if FBS_KEYWORDS.iter().any(|kw| out.eq_ignore_ascii_case(kw)) {
        out.push('_');
    }
    out
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

/// Comment body: newlines collapse so one description stays one `//` line.
fn comment_text(text: &str) -> String {
    text.split(['\r', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(yaml: &str) -> Schema {
        serde_yaml::from_str(yaml).expect("test schema must parse")
    }

    fn test_schema() -> Schema {
        schema(
            r"
tables:
  Item:
    name: Item
    description: Equipment definitions.
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true, description: Display name }
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
      rarity: { name: rarity, type: { kind: Enum, value: Rarity } }
      tags: { name: tags, type: { kind: Array, value: { kind: String } } }
      prices: { name: prices, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } }
      meta: { name: meta, type: { kind: Object, value: {} } }
      weight: { name: weight, type: { kind: Float64 } }
  Drop:
    name: Drop
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int64 }, required: true }
      item: { name: item, type: { kind: Enum, value: MissingEnum } }
enums:
  ItemKind:
    name: ItemKind
    description: Weapon category
    values:
      - { name: Sword, value: 1, description: Sword weapon }
      - { name: Shield, value: 2 }
  Rarity:
    name: Rarity
    values:
      - { name: common }
      - { name: rare }
  Duplicate:
    name: Duplicate
    values:
      - { name: A, value: 1 }
      - { name: B, value: 1 }
  MissingEnum:
    name: MissingEnum
    values: []
",
        )
    }

    fn artifact<'a>(artifacts: &'a [(String, Vec<u8>)], suffix: &str) -> &'a str {
        let (path, content) = artifacts
            .iter()
            .find(|(p, _)| p.ends_with(suffix))
            .unwrap_or_else(|| panic!("artifact {suffix} not found"));
        assert!(
            path.ends_with(suffix),
            "expected suffix {suffix}, got {path}"
        );
        std::str::from_utf8(content).expect("fbs is utf-8")
    }

    #[test]
    fn test_generate_artifact_paths() {
        let gen = FlatbuffersTargetGenerator::default();
        let artifacts = gen.generate(&test_schema(), Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "build/fbs/Drop.fbs",
                "build/fbs/Item.fbs",
                "build/fbs/cage_enums.fbs",
            ]
        );
        // No enums file when nothing qualifies: a schema whose only enums
        // are string-backed produces table files alone.
        let bare = schema(
            r"
tables:
  T:
    name: T
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      r: { name: r, type: { kind: Enum, value: R } }
enums:
  R:
    name: R
    values:
      - { name: only }
",
        );
        let gen = FlatbuffersTargetGenerator::default();
        assert_eq!(gen.generate(&bare, None).len(), 1);
    }

    #[test]
    fn test_table_header_and_structure() {
        let gen = FlatbuffersTargetGenerator::default();
        let artifacts = gen.generate(&test_schema(), Some("abc123"));
        let item = artifact(&artifacts, "Item.fbs");
        assert!(item.starts_with("// Code generated by Cage — DO NOT EDIT."));
        assert!(item.contains("// schema: abc123"));
        assert!(item.contains("// table: Item"));
        assert!(item.contains("namespace cage.generated;"));
        assert!(item.contains("// Item — primary key: id"));
        assert!(item.contains("table Item {"));
        assert!(item.ends_with("root_type Item;\n"));
        // Item.kind resolves to a shared legal enum, so the enums file is
        // included.
        assert!(item.contains("include \"cage_enums.fbs\";"), "\n{item}");
    }

    #[test]
    fn test_field_mapping_name_order_and_attributes() {
        let gen = FlatbuffersTargetGenerator::default();
        let artifacts = gen.generate(&test_schema(), None);
        let item = artifact(&artifacts, "Item.fbs");
        // Name order with numbers-free .fbs fields; scalar types per F0.
        let id = "  id: int (key);";
        assert!(
            item.contains(id),
            "key attribute on the integer pk:\n{item}"
        );
        assert!(item.contains("  name: string (required);"));
        assert!(item.contains("  tags: [string];"));
        assert!(item.contains("  weight: double;"));
        // The map rides the vector-of-entries idiom with a wrapper table.
        assert!(item.contains("  prices: [pricesValue];"));
        assert!(item.contains("table pricesValue {"));
        assert!(item.contains("key: string;"));
        assert!(item.contains("value: int;"));
        // Free-form object degrades to JSON text with a comment.
        assert!(item.contains("  meta: string;"));
        assert!(item.contains("free-form payload"));
        // The string enum degrades with a comment; the integer one refers.
        assert!(item.contains("  rarity: string;"));
        assert!(item.contains("field degrades to string"));
        assert!(item.contains("  kind: ItemKind;"));
        // An integer pk keeps the key attribute; `required` never lands on
        // a scalar (flatc restricts it to strings and vectors).
        let drop = artifact(&artifacts, "Drop.fbs");
        assert!(drop.contains("  id: long (key);"));
    }

    #[test]
    fn test_enum_base_ladder_and_shared_file() {
        let gen = FlatbuffersTargetGenerator::default();
        let artifacts = gen.generate(&test_schema(), Some("h"));
        let enums = artifact(&artifacts, "cage_enums.fbs");
        assert!(enums.contains("namespace cage.generated;"));
        // 1..=2 fits ubyte — smallest legal base wins.
        assert!(enums.contains("enum ItemKind : ubyte {"), "\n{enums}");
        assert!(enums.contains("  Sword = 1,"), "\n{enums}");
        assert!(enums.contains("  Shield = 2,"));
        // Duplicate values disqualify the whole enum (flatc has no
        // allow_alias): `Duplicate` stays out of the shared file.
        assert!(!enums.contains("enum Duplicate"), "\n{enums}");
        // The referencing table includes the enums file.
        let item = artifact(&artifacts, "Item.fbs");
        assert!(item.contains("include \"cage_enums.fbs\";"));

        // Base ladder: negative values shift to signed, wide values widen.
        let wide = schema(
            r"
tables:
  T:
    name: T
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      s: { name: s, type: { kind: Enum, value: Signed } }
      u: { name: u, type: { kind: Enum, value: Wide } }
enums:
  Signed:
    name: Signed
    values:
      - { name: Neg, value: -5 }
      - { name: Pos, value: 5 }
  Wide:
    name: Wide
    values:
      - { name: Big, value: 4294967296 }
",
        );
        let artifacts = gen.generate(&wide, None);
        let enums = artifact(&artifacts, "cage_enums.fbs");
        assert!(enums.contains("enum Signed : byte {"), "\n{enums}");
        assert!(enums.contains("enum Wide : ulong {"), "\n{enums}");
    }

    #[test]
    fn test_nested_array_and_map_wrappers() {
        let nested = schema(
            r"
tables:
  T:
    name: T
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      grid: { name: grid, type: { kind: Array, value: { kind: Array, value: { kind: Int32 } } } }
      index: { name: index, type: { kind: Map, value: { key_type: int, value_type: { kind: Array, value: { kind: String } } } } }
enums: {}
",
        );
        let gen = FlatbuffersTargetGenerator::default();
        let artifacts = gen.generate(&nested, None);
        let t = artifact(&artifacts, "T.fbs");
        // Array of arrays: outer vector over a wrapper table.
        assert!(t.contains("  grid: [gridValue];"), "\n{t}");
        assert!(t.contains("table gridValue {"));
        assert!(t.contains("items: [int];"));
        // Map of array: entry table over a wrapper table.
        assert!(t.contains("  index: [indexEntry];"), "\n{t}");
        assert!(t.contains("table indexEntry {"));
        assert!(t.contains("key: long;"));
        assert!(t.contains("value: indexValue;"));
        assert!(t.contains("table indexValue {"));
    }

    #[test]
    fn test_keyword_escaping_and_determinism() {
        let kw = schema(
            r"
tables:
  Table:
    name: Table
    primary_key: [key]
    fields:
      key: { name: key, type: { kind: String }, required: true }
      double: { name: double, type: { kind: Float64 } }
enums: {}
",
        );
        let gen = FlatbuffersTargetGenerator::default();
        let artifacts = gen.generate(&kw, None);
        let t = artifact(&artifacts, "Table.fbs");
        // Keyword idents escape with a trailing underscore; the table name
        // itself collides with `table` and escapes too.
        assert!(t.contains("table Table_ {"), "\n{t}");
        assert!(t.contains("  key_: string (key, required);"), "\n{t}");
        assert!(t.contains("  double_: double;"), "\n{t}");

        // Same schema, same bytes — twice (schema hash included in both).
        let again = gen.generate(&kw, Some("h"));
        let first = gen.generate(&kw, Some("h"));
        assert_eq!(
            first.iter().map(|(p, c)| (p, c)).collect::<Vec<_>>(),
            again.iter().map(|(p, c)| (p, c)).collect::<Vec<_>>()
        );
    }
}
