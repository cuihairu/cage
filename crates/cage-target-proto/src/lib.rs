//! Protobuf Target Generator - generates .proto (proto3) definition files from the Cage schema
//!
//! The Cage design doc lists Protobuf under Data Targets (§23) but pins no
//! wire-encoding contract, so the landed form is the schema-driven shape
//! shared with the other code bindings: each table becomes a `proto3`
//! message in a per-table `.proto` file, shared integer-backed enums live in
//! one `cage_enums.proto` imported on demand. Data itself is not embedded —
//! consumers run `protoc` against these definitions with their own runtime.

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
// Stage: crate-prefixed type names (ProtoTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
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
use cage_core::schema::{EnumSchema, FieldType, MapKeyType, Schema};
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

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

/// Protobuf Target Generator
///
/// Determinism contract: tables and fields iterate in name order, field
/// numbers follow that order from 1, identifiers resolve through a shared
/// name-order allocator, and all decisions are pure functions of the schema
/// — the same schema therefore yields byte-identical `.proto` files.
pub struct ProtoTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.proto")
    pub file_template: String,
    /// File name of the shared enums file
    pub enums_file: String,
    /// proto package clause value (e.g., "cage.generated")
    pub package: String,
}

impl Default for ProtoTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/proto"),
            file_template: "{table}.proto".to_string(),
            enums_file: "cage_enums.proto".to_string(),
            package: "cage.generated".to_string(),
        }
    }
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
        self.output_dir
            .join(file_name)
            .to_string_lossy()
            .to_string()
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
        let _ = path;
        std::str::from_utf8(content).expect("proto output is UTF-8")
    }

    #[test]
    fn test_generate_artifact_paths() {
        let schema = test_schema();
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // Tables in name order, shared enums file last.
        assert_eq!(
            paths,
            vec![
                "build/proto/Drop.proto",
                "build/proto/Item.proto",
                "build/proto/cage_enums.proto",
            ]
        );
    }

    #[test]
    fn test_table_header_and_structure() {
        let schema = test_schema();
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("abc123"));
        let item = artifact(&artifacts, "Item.proto");
        assert!(item.starts_with("// Code generated by Cage — DO NOT EDIT.\n"));
        assert!(item.contains("// schema: abc123\n"));
        assert!(item.contains("// table: Item\n"));
        assert!(item.contains("syntax = \"proto3\";\n"));
        assert!(item.contains("package cage.generated;\n"));
        // Banner: primary key listing, then description.
        assert!(item.contains("// Item — primary key: id\n"));
        assert!(item.contains("// Equipment definitions.\n"));
        assert!(item.ends_with("}\n"));
    }

    #[test]
    fn test_field_mapping_name_order_and_numbers() {
        let schema = test_schema();
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("abc123"));
        let item = artifact(&artifacts, "Item.proto");
        // Fields in name order (id, kind, meta, name, prices, rarity, tags,
        // weight) with field numbers 1..n.
        let expect = |line: &str| assert!(item.contains(line), "missing: {line}\n{item}");
        // Required fields stay plain; non-required singular fields carry
        // proto3 explicit presence (`optional`); repeated/map never do.
        expect("  int32 id = 1;");
        expect("  optional ItemKind kind = 2;");
        expect("  optional google.protobuf.Struct meta = 3;");
        expect("  string name = 4;");
        expect("  map<string, int32> prices = 5;");
        // Non-integer enum → string fallback with a use-site note.
        expect("  // enum \"Rarity\" has non-integer members; field degrades to string");
        expect("  optional string rarity = 6;");
        expect("  repeated string tags = 7;");
        expect("  optional double weight = 8;");
    }

    #[test]
    fn test_wellknown_import_only_when_used() {
        let schema = test_schema();
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("abc123"));
        let item = artifact(&artifacts, "Item.proto");
        // Item uses Struct + enums: both imports, lexicographic order.
        assert!(item.contains("import \"cage_enums.proto\";\n"));
        assert!(item.contains("import \"google/protobuf/struct.proto\";\n"));
        assert!(
            item.find("import \"cage_enums.proto\";").unwrap()
                < item
                    .find("import \"google/protobuf/struct.proto\";")
                    .unwrap()
        );
        // Drop references no eligible enum and no free-form type: no imports.
        let drop = artifact(&artifacts, "Drop.proto");
        assert!(!drop.contains("import "), "drop file:\n{drop}");
    }

    #[test]
    fn test_shared_enums_file() {
        let schema = test_schema();
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("abc123"));
        let enums = artifact(&artifacts, "cage_enums.proto");
        assert!(enums.contains("// shared enums\n"));
        assert!(enums.contains("package cage.generated;\n"));
        // proto3 requires a zero first member: synthetic UNSPECIFIED.
        assert!(enums.contains(
            "enum ItemKind {\n  ItemKind_UNSPECIFIED = 0;\n  Sword = 1;\n  Shield = 2;\n}\n"
        ));
        // String-member enum stays out entirely (fields degrade to string).
        assert!(!enums.contains("enum Rarity"));
        assert!(!enums.contains("Rarity_UNSPECIFIED"));
    }

    #[test]
    fn test_enum_zero_present_and_aliases() {
        let schema = schema(
            r"
tables:
  Thing:
    name: Thing
    primary_key: [id]
    fields:
      state: { name: state, type: { kind: Enum, value: State } }
      mode: { name: mode, type: { kind: Enum, value: Mode } }
enums:
  State:
    name: State
    values:
      - { name: Off, value: 0 }
      - { name: On, value: 1 }
  Mode:
    name: Mode
    values:
      - { name: Fast, value: 3 }
      - { name: Quick, value: 3 }
      - { name: Slow, value: 4 }
",
        );
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("h"));
        let enums = artifact(&artifacts, "cage_enums.proto");
        // Zero already present → no synthetic member.
        assert!(enums.contains("enum State {\n  Off = 0;\n  On = 1;\n}\n"));
        // Duplicate values → allow_alias, still no synthetic (3 ≠ 0? no
        // zero here → synthetic zero comes first).
        assert!(enums.contains("option allow_alias = true;"));
        assert!(enums.contains("enum Mode {\n  option allow_alias = true;\n\n  Mode_UNSPECIFIED = 0;\n  Fast = 3;\n  Quick = 3;\n  Slow = 4;\n}\n"));
    }

    #[test]
    fn test_enum_out_of_int32_range_falls_back_to_string() {
        let schema = schema(
            r"
tables:
  Wide:
    name: Wide
    primary_key: [id]
    fields:
      big: { name: big, type: { kind: Enum, value: Big } }
enums:
  Big:
    name: Big
    values:
      - { name: Huge, value: 4294967296 }
",
        );
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("h"));
        assert_eq!(artifacts.len(), 1, "no enums file for out-of-range enum");
        let wide = artifact(&artifacts, "Wide.proto");
        assert!(wide.contains("  optional string big = 1;"));
    }

    #[test]
    fn test_nested_wrappers_for_arrays_and_maps() {
        let schema = schema(
            r"
tables:
  Grid:
    name: Grid
    primary_key: [id]
    fields:
      matrix: { name: matrix, type: { kind: Array, value: { kind: Array, value: { kind: Int32 } } } }
      lookup: { name: lookup, type: { kind: Map, value: { key_type: int, value_type: { kind: Map, value: { key_type: string, value_type: { kind: String } } } } } }
      deep: { name: deep, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Array, value: { kind: Int32 } } } } } }
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
        );
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("h"));
        let grid = artifact(&artifacts, "Grid.proto");
        // Fields first (name order), wrapper messages after, nested one
        // level inside the table message. Arrays/maps in element or map
        // value position get a `{field}Value` wrapper; depth stacks the
        // suffix.
        let expect = |line: &str| assert!(grid.contains(line), "missing: {line}\n{grid}");
        expect("  map<string, deepValue> deep = 1;");
        expect("  int32 id = 2;");
        expect("  map<int64, lookupValue> lookup = 3;");
        expect("  repeated matrixValue matrix = 4;");
        // matrix: Array<Array<Int32>> → one wrapper hosting the inner repeat.
        expect("message matrixValue {\n    repeated int32 items = 1;\n  }");
        // lookup: map<int, map<string, string>> → one wrapper hosting the
        // inner map.
        expect("message lookupValue {\n    map<string, string> value = 1;\n  }");
        // deep: map<string, Array<Array<Int32>>> → two stacked wrappers.
        expect("message deepValue {\n    repeated deepValueValue items = 1;\n  }");
        expect("message deepValueValue {\n    repeated int32 items = 1;\n  }");
    }

    #[test]
    fn test_ident_sanitization_and_dedupe() {
        let schema = schema(
            r#"
tables:
  my-table:
    name: my-table
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      message: { name: message, type: { kind: String } }
      "2nd": { name: "2nd", type: { kind: String } }
      string: { name: string, type: { kind: String } }
enums:
  my-table:
    name: my-table
    values:
      - { name: One, value: 1 }
      - { name: One, value: 1 }
"#,
        );
        let artifacts = ProtoTargetGenerator::default().generate(&schema, Some("h"));
        let table = artifact(&artifacts, "my-table.proto");
        // Illegal chars → `_`; keyword/scalar names get trailing `_`;
        // leading digit gets `_` prefix; numbers follow name order.
        assert!(table.contains("message my_table {"), "{table}");
        assert!(table.contains("  optional string _2nd = 1;"));
        assert!(table.contains("  int32 id = 2;"));
        assert!(table.contains("  optional string message_ = 3;"));
        assert!(table.contains("  optional string string_ = 4;"));
        let enums = artifact(&artifacts, "cage_enums.proto");
        // Enum loses the name race to the table message (tables allocate
        // first) and gains a trailing underscore; duplicate alias allowed.
        assert!(enums.contains("enum my_table_ {"), "{enums}");
        assert!(enums.contains("option allow_alias = true;"));
        // The table references no enum → no enums import in its file.
        assert!(!table.contains("import \"cage_enums.proto\";"), "{table}");
    }

    #[test]
    fn test_deterministic_output() {
        let schema = test_schema();
        let gen = ProtoTargetGenerator::default();
        let a = gen.generate(&schema, Some("abc123"));
        let b = gen.generate(&schema, Some("abc123"));
        assert_eq!(a, b);
    }

    #[test]
    fn test_schema_hash_optional() {
        let schema = test_schema();
        let artifacts = ProtoTargetGenerator::default().generate(&schema, None);
        let item = artifact(&artifacts, "Item.proto");
        assert!(!item.contains("schema:"), "{item}");
    }

    #[test]
    fn test_empty_table_message() {
        let schema = schema(
            r"
tables:
  Hollow:
    name: Hollow
    primary_key: []
    fields: {}
enums: {}
",
        );
        let artifacts = ProtoTargetGenerator::default().generate(&schema, None);
        let hollow = artifact(&artifacts, "Hollow.proto");
        assert!(hollow.contains("message Hollow {\n}\n"), "{hollow}");
    }

    #[test]
    fn test_from_config_options() {
        let yaml = r#"
format: "proto"
output_dir: "build/pb"
options:
  package: "game.config"
  enums_file: "defs.proto"
"#;
        let config: TargetConfig = serde_yaml::from_str(yaml).expect("config parses");
        let gen = ProtoTargetGenerator::from_config(&config);
        assert_eq!(gen.output_dir, PathBuf::from("build/pb"));
        assert_eq!(gen.file_template, "{table}.proto");
        assert_eq!(gen.enums_file, "defs.proto");
        assert_eq!(gen.package, "game.config");

        let gen = ProtoTargetGenerator::from_config(&TargetConfig {
            format: "proto".to_string(),
            output_dir: "build/x".to_string(),
            file_template: Some("{table}_pb.proto".to_string()),
            options: None,
        });
        assert_eq!(gen.file_template, "{table}_pb.proto");
        assert_eq!(gen.enums_file, "cage_enums.proto");
        assert_eq!(gen.package, "cage.generated");
    }

    /// The enum import path follows `enums_file` (incl. subdirectories).
    #[test]
    fn test_enums_file_option_drives_import() {
        let schema = test_schema();
        let gen = ProtoTargetGenerator {
            enums_file: "defs/enums.proto".to_string(),
            ..Default::default()
        };
        let artifacts = gen.generate(&schema, None);
        assert!(
            artifacts
                .iter()
                .any(|(p, _)| p.ends_with("defs/enums.proto")),
            "paths: {:?}",
            artifacts.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
        let item = artifact(&artifacts, "Item.proto");
        assert!(item.contains("import \"defs/enums.proto\";\n"), "{item}");
    }
}
