//! `Protobuf` rendering: the well-known-type import, keyword table,
//! render-state types, the `ProtoTargetGenerator` implementation, and
//! the free helper functions (split out of lib.rs, LAY-031).

use cage_core::manifest::TargetConfig;
use cage_core::schema::{EnumSchema, FieldType, MapKeyType, Schema};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use super::ProtoTargetGenerator;

/// Well-known-type import for free-form payloads (`Object` / `Any` / `Null`
/// map to `google.protobuf.Struct` / `google.protobuf.Value`).
const WKT_IMPORT: &str = "google/protobuf/struct.proto";

/// proto keywords plus the scalar type names — all escaped with a trailing
/// `_` when they show up as identifiers. Conservative on purpose: protoc
/// accepts some of these contextually, but a fixed deny list keeps generated
/// files valid across protoc versions.
const PROTO_KEYWORDS: &[&str] = &[
    "syntax",
    "package",
    "import",
    "option",
    "message",
    "enum",
    "service",
    "rpc",
    "returns",
    "oneof",
    "map",
    "repeated",
    "optional",
    "required",
    "reserved",
    "to",
    "max",
    "extensions",
    "extend",
    "group",
    "public",
    "weak",
    "stream",
    "inf",
    "nan",
    "bool",
    "bytes",
    "double",
    "fixed32",
    "fixed64",
    "float",
    "int32",
    "int64",
    "sfixed32",
    "sfixed64",
    "sint32",
    "sint64",
    "string",
    "uint32",
    "uint64",
];

/// One emitted proto enum: allocated ident plus resolved members.
struct ProtoEnum {
    /// Enum schema name (key in `Schema::enums`)
    name: String,
    /// Allocated proto identifier
    ident: String,
    /// `(member name, value)` in schema order
    members: Vec<(String, i64)>,
    /// Duplicate values present → emit `option allow_alias = true;`
    aliases: bool,
    /// No member carries 0 → prepend a synthetic `_UNSPECIFIED = 0`
    synthetic_zero: bool,
}

/// Per-table render state gathered during the field pass.
#[derive(Default)]
struct TableCtx {
    /// Table references at least one shared enum (→ import enums file)
    enum_refs: BTreeSet<String>,
    /// Table uses a well-known free-form type (→ import struct.proto)
    wellknown: bool,
    /// Nested wrapper messages, first-encounter order, already indented
    wrappers: Vec<String>,
    /// Identifiers taken inside the table message (fields + wrappers)
    used: HashSet<String>,
}

impl ProtoTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.proto".to_string()),
            enums_file: "cage_enums.proto".to_string(),
            package: "cage.generated".to_string(),
        };

        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("package") {
                if let Some(s) = v.as_str() {
                    gen.package = s.to_string();
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

    /// Generate `.proto` artifacts for all tables in the schema.
    ///
    /// Artifact order: tables in name order, shared enums file last
    /// (registered only when at least one integer-backed enum exists —
    /// same rule as the Go/Java generators' enums unit).
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        // Shared name allocation, tables first then enums (family rule:
        // struct names win, enum names avoid), both in name order.
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

        // Integer-backed enums become proto enums; everything else (string
        // members, missing literals, values outside the proto int32 enum
        // range) falls back to `string` fields and stays out of the file.
        let proto_enums: Vec<ProtoEnum> = enum_idents
            .iter()
            .filter_map(|(name, ident)| Self::resolve(schema, name, ident))
            .collect();
        let eligible: BTreeSet<String> = proto_enums.iter().map(|e| e.name.clone()).collect();

        let mut artifacts = Vec::new();
        for (table_name, table_ident) in &table_idents {
            let table = &schema.tables[table_name];
            let content =
                self.render_table(table, table_ident, &eligible, &proto_enums, schema_hash);
            let file_name = self.file_template.replace("{table}", table_name);
            artifacts.push((self.path(&file_name), content.into_bytes()));
        }
        if !proto_enums.is_empty() {
            let content = self.render_enums(&enum_idents, &proto_enums, schema_hash);
            artifacts.push((self.path(&self.enums_file), content.into_bytes()));
        }
        artifacts
    }

    fn path(&self, file_name: &str) -> String {
        // Artifact keys are contract strings — always `/`.
        self.output_dir
            .join(file_name)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }

    /// Resolve one schema enum into a proto enum, or `None` when any member
    /// disqualifies the whole enum (non-integer literal, value outside the
    /// int32 range proto enums are defined over).
    fn resolve(schema: &Schema, name: &str, ident: &str) -> Option<ProtoEnum> {
        let def: &EnumSchema = schema.enums.get(name)?;
        let mut members = Vec::with_capacity(def.values.len());
        let mut seen: HashSet<i64> = HashSet::new();
        let mut aliases = false;
        let mut has_zero = false;
        for value in &def.values {
            let literal = integer_literal(value.value.as_ref())?;
            if !(-2_147_483_648i64..=2_147_483_647).contains(&literal) {
                return None;
            }
            if !seen.insert(literal) {
                aliases = true;
            }
            if literal == 0 {
                has_zero = true;
            }
            members.push((value.name.clone(), literal));
        }
        if members.is_empty() {
            return None;
        }
        Some(ProtoEnum {
            name: name.to_string(),
            ident: ident.to_string(),
            members,
            aliases,
            synthetic_zero: !has_zero,
        })
    }

    /// Render one table file: header, syntax, package, imports, message.
    fn render_table(
        &self,
        table: &cage_core::schema::TableSchema,
        table_ident: &str,
        eligible: &BTreeSet<String>,
        proto_enums: &[ProtoEnum],
        schema_hash: Option<&str>,
    ) -> String {
        let mut ctx = TableCtx::default();
        let mut fields = Vec::new();

        // Fields in name order; field numbers follow that order from 1.
        let mut names: Vec<&String> = table.fields.keys().collect();
        names.sort();
        for (idx, field_name) in names.iter().enumerate() {
            let field = &table.fields[*field_name];
            let field_ident = unique_ident(escape_ident(field_name), &mut ctx.used);
            let number = idx + 1;
            self.render_field(
                field,
                &field.name,
                &field_ident,
                number,
                eligible,
                proto_enums,
                &mut ctx,
                &mut fields,
            );
        }

        let mut out = String::new();
        Self::header(&mut out, schema_hash, &format!("table: {table_ident}"));
        out.push('\n');
        let _ = writeln!(out, "syntax = \"proto3\";");
        out.push('\n');
        let _ = writeln!(out, "package {};", self.package);

        let mut imports: Vec<String> = Vec::new();
        if !ctx.enum_refs.is_empty() {
            imports.push(self.enums_file.clone());
        }
        if ctx.wellknown {
            imports.push(WKT_IMPORT.to_string());
        }
        imports.sort();
        if !imports.is_empty() {
            out.push('\n');
            for import in imports {
                let _ = writeln!(out, "import \"{import}\";");
            }
        }

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
        let _ = writeln!(out, "message {table_ident} {{");
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
        out
    }

    /// Render one field line (plus any wrapper messages it needs) into
    /// `fields` / `ctx.wrappers`.
    #[allow(clippy::too_many_arguments)]
    fn render_field(
        &self,
        field: &cage_core::schema::FieldSchema,
        display_name: &str,
        field_ident: &str,
        number: usize,
        eligible: &BTreeSet<String>,
        proto_enums: &[ProtoEnum],
        ctx: &mut TableCtx,
        fields: &mut Vec<String>,
    ) {
        let type_text = self.field_type(
            &field.field_type,
            display_name,
            0,
            eligible,
            proto_enums,
            ctx,
        );

        if let Some(desc) = &field.description {
            fields.push(format!("// {}", comment_text(desc)));
        }
        // Degrading an enum reference to `string` is a data-shape decision
        // worth flagging at the use site.
        if let FieldType::Enum(name) = &field.field_type {
            if !eligible.contains(name) {
                fields.push(format!(
                    "// enum \"{name}\" has non-integer members; field degrades to string"
                ));
            }
        }
        // proto3 explicit presence: `optional` only on singular scalars and
        // message fields — repeated elements and map fields never take it.
        let optional =
            !field.required && !matches!(field.field_type, FieldType::Array(_) | FieldType::Map(_));
        let prefix = if optional { "optional " } else { "" };
        fields.push(format!("{prefix}{type_text} {field_ident} = {number};"));
    }

    /// proto type text for one field type. `base` names the field (wrapper
    /// message naming), `depth` counts wrapper nesting for deterministic
    /// name suffixes.
    fn field_type(
        &self,
        ft: &FieldType,
        base: &str,
        depth: usize,
        eligible: &BTreeSet<String>,
        proto_enums: &[ProtoEnum],
        ctx: &mut TableCtx,
    ) -> String {
        match ft {
            FieldType::Array(elem) => {
                match self.composite(elem, base, depth + 1, eligible, proto_enums, ctx) {
                    Composite::Inline(text) => format!("repeated {text}"),
                    Composite::Wrapper(name, lines) => {
                        ctx.wrappers.push(render_message(&name, &lines));
                        format!("repeated {name}")
                    }
                }
            }
            FieldType::Map(map) => {
                let key = match map.key_type {
                    MapKeyType::String => "string".to_string(),
                    // Family rule (go/cpp/java): integer map keys widen to
                    // int64; the data model stores them as numeric strings.
                    MapKeyType::Int => "int64".to_string(),
                };
                match self.composite(&map.value_type, base, depth + 1, eligible, proto_enums, ctx) {
                    Composite::Inline(text) => format!("map<{key}, {text}>"),
                    Composite::Wrapper(name, lines) => {
                        ctx.wrappers.push(render_message(&name, &lines));
                        format!("map<{key}, {name}>")
                    }
                }
            }
            FieldType::Null | FieldType::Any => {
                ctx.wellknown = true;
                "google.protobuf.Value".to_string()
            }
            FieldType::Object(_) => {
                // Family rule (C++ std::map<string, any>): objects are
                // free-form; proto's canonical free-form payload is Struct.
                ctx.wellknown = true;
                "google.protobuf.Struct".to_string()
            }
            // Tagged union renders as its runtime shape (a flat object with
            // the discriminant key); proto's canonical free-form payload is
            // Struct. A `oneof` hierarchy is a follow-up.
            FieldType::TaggedUnion(_) => {
                ctx.wellknown = true;
                "google.protobuf.Struct".to_string()
            }
            FieldType::Enum(name) => {
                if eligible.contains(name) {
                    let ident = &proto_enums
                        .iter()
                        .find(|e| e.name == *name)
                        .map(|e| e.ident.clone())
                        .unwrap_or_default();
                    ctx.enum_refs.insert(ident.clone());
                    ident.clone()
                } else {
                    // String members / out-of-range values: proto enums are
                    // int32-backed, so the field degrades to `string` (the
                    // same fallback other generators use for unresolved
                    // enums).
                    "string".to_string()
                }
            }
            FieldType::Bool => "bool".to_string(),
            FieldType::Bytes => "bytes".to_string(),
            FieldType::String => "string".to_string(),
            FieldType::Int8 | FieldType::Int16 | FieldType::Int32 => "int32".to_string(),
            FieldType::Int64 => "int64".to_string(),
            FieldType::UInt8 | FieldType::UInt16 | FieldType::UInt32 => "uint32".to_string(),
            FieldType::UInt64 => "uint64".to_string(),
            FieldType::Float32 => "float".to_string(),
            FieldType::Float64 => "double".to_string(),
        }
    }

    /// Type text for a position that cannot carry `repeated`/`map` directly
    /// (repeated elements, map values): arrays and maps there are wrapped in
    /// a nested message named `{Field}Value` (`{Field}ValueValue`, ...).
    fn composite(
        &self,
        ft: &FieldType,
        base: &str,
        depth: usize,
        eligible: &BTreeSet<String>,
        proto_enums: &[ProtoEnum],
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
                            match self.composite(elem, base, depth + 1, eligible, proto_enums, ctx)
                            {
                                Composite::Inline(text) => text,
                                Composite::Wrapper(wname, wlines) => {
                                    ctx.wrappers.push(render_message(&wname, &wlines));
                                    wname
                                }
                            };
                        format!("repeated {text} items = 1;")
                    }
                    FieldType::Map(map) => {
                        let key = match map.key_type {
                            MapKeyType::String => "string".to_string(),
                            MapKeyType::Int => "int64".to_string(),
                        };
                        let text = match self.composite(
                            &map.value_type,
                            base,
                            depth + 1,
                            eligible,
                            proto_enums,
                            ctx,
                        ) {
                            Composite::Inline(text) => text,
                            Composite::Wrapper(wname, wlines) => {
                                ctx.wrappers.push(render_message(&wname, &wlines));
                                wname
                            }
                        };
                        format!("map<{key}, {text}> value = 1;")
                    }
                    _ => unreachable!("composite wrapper only for Array/Map"),
                };
                Composite::Wrapper(name, vec![inner])
            }
            _ => Composite::Inline(self.field_type(ft, base, depth, eligible, proto_enums, ctx)),
        }
    }

    /// Render the shared enums file (header, syntax, package, enums).
    fn render_enums(
        &self,
        enum_idents: &[(String, String)],
        proto_enums: &[ProtoEnum],
        schema_hash: Option<&str>,
    ) -> String {
        let mut out = String::new();
        Self::header(&mut out, schema_hash, "shared enums");
        out.push('\n');
        let _ = writeln!(out, "syntax = \"proto3\";");
        out.push('\n');
        let _ = writeln!(out, "package {};", self.package);
        for (name, ident) in enum_idents {
            let Some(proto_enum) = proto_enums.iter().find(|e| e.name == *name) else {
                continue;
            };
            out.push('\n');
            let _ = writeln!(out, "// {ident}");
            let _ = writeln!(out, "enum {} {{", proto_enum.ident);
            if proto_enum.aliases {
                let _ = writeln!(out, "  option allow_alias = true;");
                out.push('\n');
            }
            if proto_enum.synthetic_zero {
                let _ = writeln!(out, "  {}_UNSPECIFIED = 0;", proto_enum.ident);
            }
            for (member, value) in &proto_enum.members {
                let _ = writeln!(out, "  {} = {value};", escape_ident(member));
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

/// A type that fits its position directly, or the wrapper message needed to
/// host an array/map there.
enum Composite {
    Inline(String),
    Wrapper(String, Vec<String>),
}

/// Render a (nested) message block with its one field, wrapper-indented.
fn render_message(name: &str, lines: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "message {name} {{");
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

/// Sanitize a schema name into a valid proto identifier (ASCII alnum + `_`,
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
    if PROTO_KEYWORDS.contains(&out.as_str()) {
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
