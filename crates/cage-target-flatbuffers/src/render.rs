//! `FlatBuffers` rendering: the keyword table, render-state types,
//! the `FlatbuffersTargetGenerator` implementation, and the free
//! helper functions (split out of lib.rs, LAY-030).

use cage_core::manifest::TargetConfig;
use cage_core::schema::{FieldType, MapKeyType, Schema};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use super::FlatbuffersTargetGenerator;

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
        // string or an integer scalar — flatc's eligibility rule. A PK of
        // any other type (float, bool, vector, object, …) drops the `key`
        // attribute rather than emitting a `.fbs` flatc rejects.
        let key_field = table
            .primary_key
            .first()
            .filter(|_| table.primary_key.len() == 1)
            .filter(|pk| {
                table
                    .fields
                    .get(*pk)
                    .is_some_and(|f| key_eligible(&f.field_type))
            })
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
            FieldType::Object(_) | FieldType::TaggedUnion(_) | FieldType::Null | FieldType::Any => {
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
            FieldType::Null
            | FieldType::Any
            | FieldType::Object(_)
            | FieldType::TaggedUnion(_)
            | FieldType::String => "string".to_string(),
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

/// flatc's `(key)` eligibility: the field must be a string or an integer
/// scalar (the key type needs an ordered binary search). Enums are
/// integer-backed when eligible and degrade to `string` otherwise — both
/// mappings are key-valid. Everything else (float, bool, bytes/vector,
/// array, map, object, union, null, any) is refused so a non-conforming
/// single-field primary key never emits an attribute flatc rejects.
fn key_eligible(ft: &FieldType) -> bool {
    matches!(
        ft,
        FieldType::String
            | FieldType::Int8
            | FieldType::Int16
            | FieldType::Int32
            | FieldType::Int64
            | FieldType::UInt8
            | FieldType::UInt16
            | FieldType::UInt32
            | FieldType::UInt64
            | FieldType::Enum(_)
    )
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
