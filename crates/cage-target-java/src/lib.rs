//! Java Target Generator — generates class bindings from a Cage schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields keep name
//! order, enum values keep their schema order, and no timestamps are written
//! — the same schema always produces byte-identical files.
//!
//! One compilation unit per table (`public final class` with mutable public
//! fields and schema defaults as field initializers) plus one shared enums
//! unit holding nested `enum` types. Java requires the public class name to
//! equal the file stem, so — unlike every other target — paths derive from
//! the final *class ident*, not the raw schema name (a table `drop-item`
//! produces `drop_item.java`).
//!
//! Optionality: a field that is required **or** carries a renderable default
//! keeps its present type; a field that is neither is widened to its wrapper
//! type (`int` → `Integer`, …) so `null` can express "absent".

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
// Stage: crate-prefixed type names (JavaTargetGenerator, ...) are idiomatic
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
    schema::{EnumSchema, EnumValue, FieldSchema, FieldType, Schema, TableSchema},
};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Java Target Generator
pub struct JavaTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.java")
    pub file_template: String,
    /// File name of the shared enums compilation unit
    pub enums_file: String,
    /// Package clause value (e.g., "cage.generated")
    pub package: String,
}

impl Default for JavaTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/java"),
            file_template: "{table}.java".to_string(),
            enums_file: "CageEnums.java".to_string(),
            package: "cage.generated".to_string(),
        }
    }
}

/// Java hard keywords (JLS §3.9) that get a trailing `_` (Java has no
/// verbatim-identifier escape like C#'s `@`).
const JAVA_KEYWORDS_HARD: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
];

/// Restricted identifiers that cannot be used as type names (`var`, `record`,
/// `yield`, …), the three literals, the module-system contextual keywords,
/// and `_` — illegal as an identifier since Java 9 (mapped to `__`).
const JAVA_KEYWORDS_RESTRICTED: &[&str] = &[
    "_",
    "exports",
    "false",
    "module",
    "null",
    "open",
    "opens",
    "permits",
    "provides",
    "record",
    "requires",
    "sealed",
    "to",
    "transitive",
    "true",
    "uses",
    "var",
    "with",
    "yield",
];

/// One table field resolved for rendering: the schema, its Java member
/// ident, the renderable default initializer (when one exists) and the
/// resolved enum type text (when the field is enum-typed).
struct FieldRow<'a> {
    field: &'a FieldSchema,
    member: String,
    default: Option<String>,
    enum_text: Option<String>,
}

impl JavaTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.java".to_string()),
            ..Self::default()
        };
        if let Some(opts) = &config.options {
            if let Some(s) = opts.get("enums_file").and_then(|v| v.as_str()) {
                gen.enums_file = s.to_string();
            }
            if let Some(s) = opts.get("package").and_then(|v| v.as_str()) {
                gen.package = s.to_string();
            }
        }
        gen
    }

    /// Generate one compilation unit per table (name order) plus a shared
    /// enums unit.
    ///
    /// `schema_hash` is the same hash `manifest.json` records for the schema
    /// (Build Manifest 口径); it is stamped into every file header.
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        let mut artifacts = Vec::new();
        let emitted = Self::emitted_enums(schema);

        // One package-level namespace: the enums holder claims its class name
        // FIRST (file stem = class ident), then tables claim in name order —
        // so a table named like the holder deterministically becomes
        // `CageEnums_` with file `CageEnums_.java`.
        let mut used_classes: HashSet<String> = HashSet::new();
        let enums_class = if emitted.is_empty() {
            None
        } else {
            Some(unique_ident(
                java_ident(self.enums_stem()),
                &mut used_classes,
            ))
        };
        let tables: Vec<(&TableSchema, String)> = Self::sorted_tables(schema)
            .into_iter()
            .map(|t| {
                let class = unique_ident(java_ident(&t.name), &mut used_classes);
                (t, class)
            })
            .collect();

        // Nested enum names share one set inside the holder, seeded with the
        // holder's own name (a nested class may not repeat its enclosing
        // class name). Allocation runs in name order over the emitted enums.
        let mut used_enums: HashSet<String> = HashSet::new();
        if let Some(holder) = &enums_class {
            used_enums.insert(holder.clone());
        }
        let enum_idents: HashMap<String, String> = emitted
            .iter()
            .map(|e| {
                let ident = unique_ident(java_ident(&e.name), &mut used_enums);
                (e.name.clone(), ident)
            })
            .collect();

        for (table, class) in &tables {
            let content = self.render_table(
                schema,
                table,
                class,
                enums_class.as_deref(),
                &enum_idents,
                schema_hash,
            );
            artifacts.push((self.path_for(class), content.into_bytes()));
        }
        if let Some(holder) = &enums_class {
            let content = self.render_enums(&emitted, holder, &enum_idents, schema_hash);
            artifacts.push((self.enums_path(holder), content.into_bytes()));
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

    /// File stem of the shared enums unit; its sanitized form is the holder
    /// class name (default stem: `CageEnums`).
    fn enums_stem(&self) -> &str {
        Path::new(&self.enums_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("CageEnums")
    }

    /// Path of the shared enums unit — the stem is the holder's final class
    /// ident (file stem = class ident, the documented Java deviation).
    fn enums_path(&self, class_ident: &str) -> String {
        let p = Path::new(&self.enums_file);
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .map_or_else(String::new, |e| format!(".{e}"));
        let file_name = format!("{class_ident}{ext}");
        let base = match p.parent().filter(|d| !d.as_os_str().is_empty()) {
            Some(dir) => self.output_dir.join(dir),
            None => self.output_dir.clone(),
        };
        base.join(file_name).to_string_lossy().to_string()
    }

    /// Path of a table unit: the `{table}` placeholder is replaced with the
    /// class ident, never the raw schema name.
    fn path_for(&self, class_ident: &str) -> String {
        let file_name = self.file_template.replace("{table}", class_ident);
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
        class: &str,
        enums_class: Option<&str>,
        enum_idents: &HashMap<String, String>,
        schema_hash: Option<&str>,
    ) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "package {};", self.package);
        let _ = writeln!(out);
        out.push_str(&Self::header(
            &format!("table:  {}", table.name),
            schema_hash,
        ));
        let _ = writeln!(out);

        // Member idents, defaults and enum type text resolved once — the
        // import scan and the declarations both read this list.
        let mut used: HashSet<String> = HashSet::new();
        let rows: Vec<FieldRow> = Self::sorted_fields(table)
            .into_iter()
            .map(|(name, field)| {
                let member = unique_ident(java_ident(name), &mut used);
                let default = field
                    .default
                    .as_ref()
                    .filter(|v| !v.is_null())
                    .and_then(|d| render_default(d, &field.field_type, schema));
                // A table whose own class shares the enum's simple name
                // must not import it (JLS §7.5.1) — use the qualified
                // `Holder.Enum` form instead.
                let enum_text = match (
                    enums_class,
                    resolve_enum(schema, &field.field_type, enum_idents),
                ) {
                    (Some(holder), Some(ident)) => Some(if ident == class {
                        format!("{holder}.{ident}")
                    } else {
                        ident.to_string()
                    }),
                    _ => None,
                };
                FieldRow {
                    field,
                    member,
                    default,
                    enum_text,
                }
            })
            .collect();

        // Imports: only what the unit uses, one block sorted lexicographically
        // by full name (java.* and the holder's nested enums mixed together).
        let mut imports: Vec<String> = Vec::new();
        if rows
            .iter()
            .any(|r| matches!(r.field.field_type, FieldType::Array(_)))
        {
            imports.push("java.util.List".to_string());
        }
        if rows
            .iter()
            .any(|r| r.default.is_some() && matches!(r.field.field_type, FieldType::Array(_)))
        {
            imports.push("java.util.ArrayList".to_string());
        }
        if rows
            .iter()
            .any(|r| matches!(r.field.field_type, FieldType::Object(_)))
        {
            imports.push("java.util.Map".to_string());
        }
        if let Some(holder) = enums_class {
            for row in &rows {
                // The qualified `Holder.Enum` form (class-name clash) needs
                // no import; idents never contain a dot.
                if let Some(text) = row.enum_text.as_ref().filter(|t| !t.contains('.')) {
                    let import = format!("{}.{}.{}", self.package, holder, text);
                    if !imports.contains(&import) {
                        imports.push(import);
                    }
                }
            }
        }
        imports.sort();
        for import in &imports {
            let _ = writeln!(out, "import {import};");
        }
        if !imports.is_empty() {
            let _ = writeln!(out);
        }

        // Class banner: description first, then name / primary-key head
        // (raw schema name, same 口径 as the Python generator).
        let head = if table.primary_key.is_empty() {
            table.name.clone()
        } else {
            format!(
                "{} — primary key: {}",
                table.name,
                table.primary_key.join(", ")
            )
        };
        let mut banner: Vec<String> = Vec::new();
        if let Some(desc) = &table.description {
            banner.push(desc.clone());
        }
        banner.push(head);
        push_javadoc(&mut out, "", &banner);
        let _ = writeln!(out, "public final class {class} {{");
        if rows.is_empty() {
            out.push_str("}\n");
            return out;
        }

        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                let _ = writeln!(out);
            }
            if let Some(doc) = field_doc(schema, row.field) {
                push_javadoc(&mut out, "    ", &[doc]);
            }
            // Optionality group rule: required OR renderable default keeps
            // the present type; otherwise primitives widen to wrappers
            // (references are nullable as-is).
            let optional = !row.field.required && row.default.is_none();
            let ty = java_type(&row.field.field_type, optional, row.enum_text.as_deref());
            match &row.default {
                Some(d) => {
                    let _ = writeln!(out, "    public {ty} {} = {d};", row.member);
                }
                None => {
                    let _ = writeln!(out, "    public {ty} {};", row.member);
                }
            }
        }
        out.push_str("}\n");
        out
    }

    fn render_enums(
        &self,
        emitted: &[&EnumSchema],
        holder: &str,
        enum_idents: &HashMap<String, String>,
        schema_hash: Option<&str>,
    ) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "package {};", self.package);
        let _ = writeln!(out);
        out.push_str(&Self::header("enums:  shared definitions", schema_hash));
        let _ = writeln!(out);
        let _ = writeln!(out, "public final class {holder} {{");
        let _ = writeln!(out, "    private {holder}() {{}}");

        for e in emitted {
            let Some(ident) = enum_idents.get(e.name.as_str()) else {
                continue;
            };
            let _ = writeln!(out);

            // Declaration Javadoc: name, plus description after an em dash.
            match &e.description {
                Some(desc) => {
                    push_javadoc(&mut out, "    ", &[format!("{} — {}", e.name, desc)]);
                }
                None => push_javadoc(&mut out, "    ", std::slice::from_ref(&e.name)),
            }

            // Bucket: every member carries an integral value → `int`/`long`
            // payload picked by value range; otherwise a `String` payload
            // (Cage compares enums as strings — same rule as py/lua/cs).
            let all_integral = !e.values.is_empty()
                && e.values.iter().all(|v| {
                    matches!(&v.value, Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64())
                });
            let long_backing = all_integral && e.values.iter().any(|v| !value_fits_i32(v));
            let payload = if !all_integral {
                "String"
            } else if long_backing {
                "long"
            } else {
                "int"
            };

            let _ = writeln!(out, "    public enum {ident} {{");
            let mut used: HashSet<String> = HashSet::new();
            for (i, v) in e.values.iter().enumerate() {
                if let Some(desc) = &v.description {
                    push_javadoc(&mut out, "        ", std::slice::from_ref(desc));
                }
                let member = unique_ident(java_ident(&v.name), &mut used);
                let sep = if i + 1 == e.values.len() { ";" } else { "," };
                if all_integral {
                    let (lit, note) = integral_literal(v, long_backing);
                    match note {
                        Some(note) => {
                            let _ = writeln!(out, "        {member}({lit}){sep} // {note}");
                        }
                        None => {
                            let _ = writeln!(out, "        {member}({lit}){sep}");
                        }
                    }
                } else {
                    let _ = writeln!(out, "        {member}({}){sep}", string_bucket_value(v));
                }
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "        public final {payload} value;");
            let _ = writeln!(out);
            let _ = writeln!(out, "        {ident}({payload} value) {{");
            let _ = writeln!(out, "            this.value = value;");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "    }}");
        }
        out.push_str("}\n");
        out
    }
}

/// Java type for a field type. `optional` is the optionality group rule
/// (¬required ∧ no renderable default): primitives widen to their wrapper
/// type, while references (String, arrays, Map, byte[], enums, Object) are
/// implicitly nullable and stay as-is. `enum_text` is the already-resolved
/// type text for an enum-typed field (`None` falls back to `String`).
fn java_type(ft: &FieldType, optional: bool, enum_text: Option<&str>) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "Object".to_string(),
        FieldType::Bool => present_or(optional, "boolean", "Boolean"),
        // Unsigned integers widen one width (Java is signed-only) so the
        // full range of the schema kind stays representable:
        // UInt8→short, UInt16→int, UInt32→long, UInt64→long.
        FieldType::Int8 => present_or(optional, "byte", "Byte"),
        FieldType::UInt8 | FieldType::Int16 => present_or(optional, "short", "Short"),
        FieldType::UInt16 | FieldType::Int32 => present_or(optional, "int", "Integer"),
        FieldType::UInt32 | FieldType::Int64 | FieldType::UInt64 => {
            present_or(optional, "long", "Long")
        }
        FieldType::Float32 => present_or(optional, "float", "Float"),
        FieldType::Float64 => present_or(optional, "double", "Double"),
        FieldType::String => "String".to_string(),
        FieldType::Bytes => "byte[]".to_string(),
        FieldType::Array(inner) => {
            format!(
                "List<{}>",
                box_primitive(&java_type(inner, false, enum_text))
            )
        }
        FieldType::Object(_) => "Map<String, Object>".to_string(),
        FieldType::Enum(_) => enum_text.unwrap_or("String").to_string(),
    }
}

/// Present (group A) vs wrapper (group B) spelling of a primitive type.
fn present_or(optional: bool, primitive: &str, wrapper: &str) -> String {
    if optional {
        wrapper.to_string()
    } else {
        primitive.to_string()
    }
}

/// Box a primitive type name for generic contexts (`List<int>` is illegal
/// Java — array element types are always boxed).
fn box_primitive(ty: &str) -> String {
    match ty {
        "boolean" => "Boolean",
        "byte" => "Byte",
        "short" => "Short",
        "int" => "Integer",
        "long" => "Long",
        "float" => "Float",
        "double" => "Double",
        other => other,
    }
    .to_string()
}

/// The single enum a field type references (top level or through arrays);
/// `None` for every other kind. Object properties are erased to `Object`,
/// so only array element positions count as an enum use.
fn field_enum(ft: &FieldType) -> Option<&str> {
    match ft {
        FieldType::Enum(name) => Some(name),
        FieldType::Array(inner) => field_enum(inner),
        _ => None,
    }
}

/// Resolve an enum-typed field to the holder's allocated nested ident;
/// `None` when the enum is unknown or has no values (falls back to String).
fn resolve_enum<'e>(
    schema: &Schema,
    ft: &FieldType,
    enum_idents: &'e HashMap<String, String>,
) -> Option<&'e str> {
    let name = field_enum(ft)?;
    let e = schema.enums.get(name)?;
    if e.values.is_empty() {
        return None;
    }
    enum_idents.get(e.name.as_str()).map(String::as_str)
}

/// Javadoc block: one body line renders as `/** … */`, more as a block;
/// an empty body writes nothing.
fn push_javadoc(out: &mut String, indent: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    if lines.len() == 1 {
        let _ = writeln!(out, "{indent}/** {} */", lines[0]);
    } else {
        let _ = writeln!(out, "{indent}/**");
        for line in lines {
            let _ = writeln!(out, "{indent} * {line}");
        }
        let _ = writeln!(out, "{indent} */");
    }
}

/// Render a schema default as a Java field initializer; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, mismatched
/// or out-of-range kinds, Enum/Null/Any — the field then keeps its wrapper
/// or reference type instead).
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
        | FieldType::UInt64 => value.as_i64().and_then(|i| int_literal(i, ft)),
        FieldType::Float32 => value.as_f64().map(|f| java_float_literal(f, true)),
        FieldType::Float64 => value.as_f64().map(|f| java_float_literal(f, false)),
        FieldType::String => value.as_str().map(java_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        _ => None,
    }
}

/// Integer literal for a field's Java type: `long`-typed fields carry the
/// `L` suffix (`int`/`short`/`byte` widen silently), and a value that does
/// not fit the target type has no Java literal — it returns `None`, the
/// same "no safe literal" bucket as kind mismatches. A u64 above
/// `i64::MAX` never reaches here (`as_i64` fails) — Java is signed-only.
fn int_literal(i: i64, ft: &FieldType) -> Option<String> {
    let (fits, long) = match ft {
        FieldType::Int8 => (i8::try_from(i).is_ok(), false),
        FieldType::UInt8 => (u8::try_from(i).is_ok(), false),
        FieldType::Int16 => (i16::try_from(i).is_ok(), false),
        FieldType::UInt16 => (u16::try_from(i).is_ok(), false),
        FieldType::Int32 => (i32::try_from(i).is_ok(), false),
        FieldType::UInt32 => (u32::try_from(i).is_ok(), true),
        FieldType::Int64 => (true, true),
        FieldType::UInt64 => (i >= 0, true),
        _ => return None,
    };
    if !fits {
        return None;
    }
    Some(if long { format!("{i}L") } else { i.to_string() })
}

/// Array defaults render only for scalar element types (the shared rule
/// across targets) as a fresh mutable list per instance — Java arrays and
/// `List.of` are immutable, but field initializers re-run per instance, so
/// `new ArrayList<>(List.of(…))` never shares state between rows.
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
    Some(format!("new ArrayList<>(List.of({}))", parts.join(", ")))
}

/// Java float literal. Non-finite values have no literal — they use the
/// `java.lang` constants (`Float.NaN`, `Double.POSITIVE_INFINITY`, …, no
/// import needed); finite values keep a decimal point, and `float`-typed
/// literals carry the mandatory `f` suffix.
fn java_float_literal(f: f64, float32: bool) -> String {
    if f.is_nan() {
        return if float32 {
            "Float.NaN".to_string()
        } else {
            "Double.NaN".to_string()
        };
    }
    if f.is_infinite() {
        let class = if float32 { "Float" } else { "Double" };
        let sign = if f < 0.0 { "NEGATIVE" } else { "POSITIVE" };
        return format!("{class}.{sign}_INFINITY");
    }
    let mut s = f.to_string();
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        s.push_str(".0");
    }
    if float32 {
        s.push('f');
    }
    s
}

fn java_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Java literals: `\\` / `\"` / `\n` / `\r` /
/// `\t` keep their short escapes, other control characters (incl. DEL)
/// become `\u00XX` with exactly four hex digits — Java processes `\uXXXX`
/// even before parsing, so a raw `\u000a` would inject a newline; CR/LF/TAB
/// therefore must use the short forms. Printable Unicode passes through.
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

/// Identifier with a trailing `_` for Java keywords (Java has no verbatim
/// escape). Restricted identifiers (`var`, `record`, `yield`, …), the
/// literals, module keywords and the lone `_` (illegal since Java 9, which
/// maps to `__`) are all covered by the same rule.
fn java_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if JAVA_KEYWORDS_HARD.contains(&s.as_str()) || JAVA_KEYWORDS_RESTRICTED.contains(&s.as_str()) {
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

/// Field Javadoc body: description plus constraint summary (identical part
/// list/order/format as the Python/Lua generators), plus the Java-specific
/// caveat for `UInt64` fields; `None` when there is nothing to say.
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
    if matches!(field.field_type, FieldType::UInt64) {
        parts.push("values above Long.MAX_VALUE do not fit Java's long".to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Whether one enum member's integral value fits Java's `int` payload
/// (u64 above `i64::MAX` does not — it forces the `long` backing).
fn value_fits_i32(v: &EnumValue) -> bool {
    match &v.value {
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .is_some_and(|i| i >= i64::from(i32::MIN) && i <= i64::from(i32::MAX)),
        _ => false,
    }
}

/// Payload literal for one integral member, plus an optional trailing
/// note. Values above `i64::MAX` have no Java literal — they are wrapped
/// to their two's-complement `long` and flagged (Java has no unsigned
/// 64-bit type, so the wrapped negative is the faithful bit pattern).
fn integral_literal(v: &EnumValue, long_backing: bool) -> (String, Option<String>) {
    let Some(serde_json::Value::Number(n)) = &v.value else {
        return (v.name.clone(), None);
    };
    if let Some(i) = n.as_i64() {
        let lit = if long_backing {
            format!("{i}L")
        } else {
            i.to_string()
        };
        return (lit, None);
    }
    if let Some(u) = n.as_u64() {
        let wrapped = i64::from_ne_bytes(u.to_ne_bytes());
        let lit = if long_backing {
            format!("{wrapped}L")
        } else {
            wrapped.to_string()
        };
        return (lit, Some(format!("wrapped from unsigned {u}")));
    }
    (v.name.clone(), None)
}

/// String-bucket member value: String as-is / Number as a decimal string /
/// Bool as `"true"`/`"false"` / missing values fall back to the member
/// name (Cage compares enums as strings — same rule as py/lua).
fn string_bucket_value(v: &EnumValue) -> String {
    match &v.value {
        Some(serde_json::Value::String(s)) => java_string_literal(s),
        Some(serde_json::Value::Number(n)) => java_string_literal(&n.to_string()),
        Some(serde_json::Value::Bool(b)) => java_string_literal(if *b { "true" } else { "false" }),
        _ => java_string_literal(&v.name),
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

    fn gen() -> JavaTargetGenerator {
        JavaTargetGenerator::default()
    }

    /// Edge-case schema shared by the dev-only sample writer and the real
    /// edge-rendering test: every field kind (incl. Null/Any/Bytes/Object),
    /// signed/unsigned widening, wrapped u64 enum payloads, keyword and
    /// restricted-identifier idents, holder/table class collisions, string
    /// escapes, constraint docs, and empty enums/tables.
    fn edge_schema() -> Schema {
        serde_yaml::from_str(
            r#"
tables:
  CageEnums:
    name: CageEnums
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int64 }, required: true, default: 100 }
      big: { name: big, type: { kind: UInt64 }, default: 42 }
      overflow: { name: overflow, type: { kind: UInt64 }, default: 18446744073709551615 }
      u8: { name: u8, type: { kind: UInt8 }, required: true, default: 200 }
      u16: { name: u16, type: { kind: UInt16 }, default: 65535 }
      u32: { name: u32, type: { kind: UInt32 }, default: 4294967295 }
      i8: { name: i8, type: { kind: Int8 }, default: 127 }
      i16: { name: i16, type: { kind: Int16 }, default: -32768 }
      f32: { name: f32, type: { kind: Float32 }, default: 1.5 }
      f64: { name: f64, type: { kind: Float64 }, default: 100 }
      flag: { name: flag, type: { kind: Bool }, default: true }
      nums: { name: nums, type: { kind: Array, value: { kind: Int32 } }, default: [1, 2, 3] }
      f32s: { name: f32s, type: { kind: Array, value: { kind: Float32 } }, default: [1.5, 2] }
      flags: { name: flags, type: { kind: Array, value: { kind: Bool } }, default: [true, false] }
      empty: { name: empty, type: { kind: Array, value: { kind: String } }, default: [] }
      meta: { name: meta, type: { kind: Object, value: {} } }
      raw: { name: raw, type: { kind: Null } }
      blob: { name: blob, type: { kind: Bytes } }
      anyx: { name: anyx, type: { kind: Any } }
      qty: { name: qty, type: { kind: Int32 }, required: true, max: 99, description: Stock }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 10, pattern: "^[a-z]+$" }
      desc: { name: desc, type: { kind: String }, default: "a\"b\\c\r\nd\te日" }
      class: { name: class, type: { kind: String } }
      record: { name: record, type: { kind: String } }
      _: { name: _, type: { kind: String } }
      dead: { name: dead, type: { kind: Enum, value: EmptyEnum } }
  ItemKind:
    name: ItemKind
    primary_key: []
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
  Weird-Name:
    name: Weird-Name
    primary_key: []
    fields: {}
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
  BigKind:
    name: BigKind
    description: Values that outgrow a 32-bit payload.
    values:
      - { name: Big, value: 18446744073709551615 }
      - { name: Large, value: 3000000000 }
      - { name: Neg, value: -5 }
  MixKind:
    name: MixKind
    values:
      - { name: A, value: 1.5 }
      - { name: B, value: true }
      - { name: C, value: "x y" }
      - { name: D }
  EmptyEnum:
    name: EmptyEnum
    values: []
"#,
        )
        .expect("edge schema must parse")
    }

    /// Write one schema's artifacts under `dir/cage/generated` — the default
    /// package directory a manual `javac` run expects.
    fn write_sample(dir: &Path, schema: &Schema) {
        let _ = std::fs::remove_dir_all(dir);
        let pkg_dir = dir.join("cage").join("generated");
        std::fs::create_dir_all(&pkg_dir).expect("sample dir");
        for (path, content) in gen().generate(schema, Some("abc123")) {
            let name = Path::new(&path)
                .file_name()
                .expect("artifact has a file name")
                .to_owned();
            std::fs::write(pkg_dir.join(name), content).expect("write sample");
        }
    }

    #[test]
    fn test_generate_artifact_paths() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // Tables in name order (class-ident stems), shared enums unit last.
        assert_eq!(
            paths,
            vec![
                "build/java/Drop.java",
                "build/java/Item.java",
                "build/java/CageEnums.java",
            ]
        );
    }

    /// Dev-only: dump the test-schema output under `/tmp/cage-java-sample`
    /// for a manual `javac` run (ignored — committed tests must not require
    /// a JDK).
    #[test]
    #[ignore = "writes /tmp/cage-java-sample for manual javac runs"]
    fn write_java_sample_for_javac() {
        write_sample(Path::new("/tmp/cage-java-sample"), &test_schema());
    }

    /// Dev-only: dump the edge-case schema under `/tmp/cage-java-sample/edge`
    /// for a manual `javac` run.
    #[test]
    #[ignore = "writes /tmp/cage-java-sample/edge for manual javac runs"]
    fn write_java_edge_sample_for_javac() {
        write_sample(Path::new("/tmp/cage-java-sample/edge"), &edge_schema());
    }

    #[test]
    fn test_write_sample_artifacts() {
        let dir = std::env::temp_dir().join(format!("cage-java-sample-{}", std::process::id()));
        write_sample(&dir, &test_schema());
        let pkg_dir = dir.join("cage").join("generated");
        for name in ["Drop.java", "Item.java", "CageEnums.java"] {
            assert!(pkg_dir.join(name).is_file(), "missing {name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edge_schema_rendering() {
        let schema = edge_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // The holder claims `CageEnums` first: the like-named table becomes
        // `CageEnums_`; `Weird-Name` follows its class ident in the file stem.
        assert_eq!(
            paths,
            vec![
                "build/java/CageEnums_.java",
                "build/java/ItemKind.java",
                "build/java/Weird_Name.java",
                "build/java/CageEnums.java",
            ]
        );

        // Table 1: every field kind, widening rules and keyword idents.
        let holder_table = String::from_utf8(artifacts[0].1.clone()).unwrap();
        assert!(holder_table.contains("public final class CageEnums_ {"));
        // Imports: List/ArrayList (array defaults) + Map (object field).
        assert!(holder_table.contains(
            "import java.util.ArrayList;\nimport java.util.List;\nimport java.util.Map;\n"
        ));
        // Unsigned widening keeps the full range; long-typed literals get L.
        assert!(holder_table.contains("public long id = 100L;"));
        assert!(holder_table.contains("public long big = 42L;"));
        // A u64 above i64::MAX has no Java literal → wrapper type, no init.
        assert!(holder_table.contains("public Long overflow;"));
        assert!(holder_table.contains("public short u8 = 200;"));
        assert!(holder_table.contains("public int u16 = 65535;"));
        assert!(holder_table.contains("public long u32 = 4294967295L;"));
        assert!(holder_table.contains("public byte i8 = 127;"));
        assert!(holder_table.contains("public short i16 = -32768;"));
        assert!(holder_table.contains("public float f32 = 1.5f;"));
        assert!(holder_table.contains("public double f64 = 100.0;"));
        assert!(holder_table.contains("public boolean flag = true;"));
        // Reference types stay plain under the optionality rule.
        assert!(holder_table.contains("public Map<String, Object> meta;"));
        assert!(holder_table.contains("public Object raw;"));
        assert!(holder_table.contains("public Object anyx;"));
        assert!(holder_table.contains("public byte[] blob;"));
        assert!(holder_table.contains("public String class_;"));
        assert!(holder_table.contains("public String record_;"));
        assert!(holder_table.contains("public String __;"));
        assert!(holder_table.contains("public String dead;"));
        assert!(holder_table.contains("public int qty;"));
        // Array defaults box primitives and re-render each element literal.
        assert!(
            holder_table.contains("public List<Integer> nums = new ArrayList<>(List.of(1, 2, 3));")
        );
        assert!(holder_table
            .contains("public List<Float> f32s = new ArrayList<>(List.of(1.5f, 2.0f));"));
        assert!(holder_table
            .contains("public List<Boolean> flags = new ArrayList<>(List.of(true, false));"));
        assert!(holder_table.contains("public List<String> empty = new ArrayList<>(List.of());"));
        // String escaping: short escapes for \ " CR LF TAB, printable
        // Unicode passes through.
        assert!(holder_table.contains(r#"public String desc = "a\"b\\c\r\nd\te日";"#));
        // Constraint docs: max / min_length / max_length / pattern, plus the
        // Java-specific UInt64 caveat and the unresolved (empty) enum note.
        assert!(holder_table.contains("/** Stock, required, max: 99 */"));
        assert!(holder_table.contains("/** min_length: 1, max_length: 10, pattern: ^[a-z]+$ */"));
        assert!(holder_table.contains("/** values above Long.MAX_VALUE do not fit Java's long */"));
        assert!(holder_table.contains("/** unresolved enum: EmptyEnum */"));

        // Table 2: a table class sharing the enum's simple name uses the
        // qualified `Holder.Enum` form (JLS §7.5.1) and imports nothing; an
        // empty primary key prints the raw name as the banner head.
        let item_kind = String::from_utf8(artifacts[1].1.clone()).unwrap();
        assert!(item_kind.contains("/** ItemKind */"));
        assert!(item_kind.contains("public CageEnums.ItemKind kind;"));
        assert!(!item_kind.contains("import "));

        // Table 3: an empty field set closes the class right after the banner.
        let weird = String::from_utf8(artifacts[2].1.clone()).unwrap();
        assert!(weird.contains("public final class Weird_Name {\n}\n"));

        // Enums: long backing for values past i32, wrapped-u64 note, enum
        // description in the declaration Javadoc.
        let enums = String::from_utf8(artifacts[3].1.clone()).unwrap();
        assert!(enums.contains("/** BigKind — Values that outgrow a 32-bit payload. */"));
        assert!(enums.contains("public enum BigKind {"));
        assert!(enums.contains("public final long value;"));
        assert!(enums.contains("        Big(-1L), // wrapped from unsigned 18446744073709551615"));
        assert!(enums.contains("        Large(3000000000L),"));
        assert!(enums.contains("        Neg(-5L);"));
        // Int backing for the small enum; string bucket for the mixed one
        // (Number → decimal string, Bool → "true", String as-is, missing →
        // member name).
        assert!(enums.contains("public enum ItemKind {"));
        assert!(enums.contains("        Sword(1);"));
        assert!(enums.contains("public final int value;"));
        assert!(enums.contains("public enum MixKind {"));
        assert!(enums.contains("        A(\"1.5\"),"));
        assert!(enums.contains("        B(\"true\"),"));
        assert!(enums.contains("        C(\"x y\"),"));
        assert!(enums.contains("        D(\"D\");"));
        assert!(enums.contains("public final String value;"));
        assert!(!enums.contains("EmptyEnum"));
    }

    #[test]
    fn test_schema_without_enums_emits_no_holder() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  Solo:
    name: Solo
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums: {}
",
        )
        .expect("schema must parse");
        let artifacts = gen().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // No emitted enums → no shared unit at all; the table renders with
        // no enums class in scope (and nothing to import).
        assert_eq!(paths, vec!["build/java/Solo.java"]);
        let src = String::from_utf8(artifacts[0].1.clone()).unwrap();
        assert!(src.contains("public final class Solo {"));
        assert!(!src.contains("import "));
    }

    #[test]
    fn test_from_config_defaults_without_options() {
        let config: TargetConfig = serde_yaml::from_str(
            r"
format: java
output_dir: build/j
",
        )
        .expect("target config");
        let gen = JavaTargetGenerator::from_config(&config);
        // No `options` block → every generator knob keeps its default.
        assert_eq!(gen.output_dir, PathBuf::from("build/j"));
        assert_eq!(gen.file_template, "{table}.java");
        assert_eq!(gen.enums_file, "CageEnums.java");
        assert_eq!(gen.package, "cage.generated");
    }

    #[test]
    fn test_enums_file_in_subdirectory() {
        let config: TargetConfig = serde_yaml::from_str(
            r"
format: java
output_dir: build/j
options:
  enums_file: shared/CageEnums.java
",
        )
        .expect("target config");
        let gen = JavaTargetGenerator::from_config(&config);
        let artifacts = gen.generate(&test_schema(), Some("abc123"));
        // The enums unit keeps the option's parent directory under the
        // output dir; the holder class name still comes from the stem.
        assert_eq!(artifacts[2].0, "build/j/shared/CageEnums.java");
        let holder = String::from_utf8(artifacts[2].1.clone()).unwrap();
        assert!(holder.contains("public final class CageEnums {"));
        // Table imports are built from package + holder class, unchanged.
        let item = String::from_utf8(artifacts[1].1.clone()).unwrap();
        assert!(item.contains("import cage.generated.CageEnums.ItemKind;"));
    }

    #[test]
    fn test_render_enums_skips_unallocated_idents() {
        let schema = test_schema();
        let emitted = JavaTargetGenerator::emitted_enums(&schema);
        let no_idents: HashMap<String, String> = HashMap::new();
        // An enum whose allocated ident is missing from the map is skipped
        // rather than rendered with a broken name.
        let src = gen().render_enums(&emitted, "CageEnums", &no_idents, Some("abc123"));
        assert!(src.contains("public final class CageEnums {"));
        assert!(!src.contains("public enum"));
    }

    #[test]
    fn test_push_javadoc_empty_body_writes_nothing() {
        let mut out = String::new();
        push_javadoc(&mut out, "    ", &[]);
        assert_eq!(out, "");
    }

    #[test]
    fn test_table_rendering() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let src = String::from_utf8(artifacts[1].1.clone()).unwrap();

        assert!(src.starts_with("package cage.generated;\n"));
        assert!(src.contains("//   schema: abc123"));
        assert!(src.contains("//   table:  Item"));
        // One import block, sorted lexicographically by full name
        // (java.* and the holder's nested enums mixed together).
        assert!(src.contains(
            "import cage.generated.CageEnums.ItemKind;\nimport java.util.ArrayList;\nimport java.util.List;\n"
        ));
        assert!(src.contains(
            "/**\n * Equipment definitions.\n * Item — primary key: id\n */\npublic final class Item {"
        ));

        // Fields in name order (sort, not schema insertion order); the
        // optionality group rule keeps present types for required or
        // defaulted fields and widens the rest to wrappers.
        let id_pos = src.find("public int id;").unwrap();
        let kind_pos = src.find("public ItemKind kind;").unwrap();
        let name_pos = src.find("public String name;").unwrap();
        let note_pos = src.find("public String note;").unwrap();
        let price_pos = src.find("public Integer price;").unwrap();
        let rarity_pos = src.find("public String rarity;").unwrap();
        let tags_pos = src
            .find("public List<String> tags = new ArrayList<>(List.of(\"pvp\"));")
            .unwrap();
        let weight_pos = src.find("public double weight = 1.5;").unwrap();
        assert!(id_pos < kind_pos && kind_pos < name_pos && name_pos < note_pos);
        assert!(note_pos < price_pos && price_pos < rarity_pos);
        assert!(rarity_pos < tags_pos && tags_pos < weight_pos);

        // Per-field Javadoc with the shared field-doc summary.
        assert!(src.contains("/** Identifier, required */"));
        assert!(src.contains("/** Price in gold, min: 0 */"));
        assert!(src.contains("/** allowed: common | rare */"));
        assert!(src.contains("/** → Player.id */"));
    }

    #[test]
    fn test_enums_rendering() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

        assert!(enums_src.starts_with("package cage.generated;\n"));
        assert!(enums_src.contains("//   enums:  shared definitions"));
        assert!(enums_src.contains("public final class CageEnums {\n    private CageEnums() {}"));
        // Integral bucket: int payload when every value fits i32.
        assert!(enums_src.contains("public enum ItemKind {"));
        assert!(enums_src.contains("/** Sword weapon */\n        Sword(1),\n        Shield(2);"));
        assert!(enums_src.contains("public final int value;\n\n        ItemKind(int value) {"));
        // String bucket: valueless members carry their name as the payload.
        assert!(enums_src.contains("public enum Rarity {"));
        assert!(enums_src.contains("common(\"common\"),\n        rare(\"rare\");"));
        assert!(enums_src.contains("public final String value;"));
        assert!(enums_src.contains("Rarity(String value) {"));
        // Empty enums are not emitted.
        assert!(!enums_src.contains("EmptyEnum"));
    }

    #[test]
    fn test_unresolved_enum_falls_back_to_string() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, None);
        let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();
        assert!(drop_src.contains("//   schema: (unavailable)"));
        assert!(drop_src.contains("public String item;"));
        assert!(drop_src.contains("/** unresolved enum: MissingEnum */"));
        // A unit that uses nothing imports nothing.
        assert!(!drop_src.contains("import "));
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
format: java
output_dir: build/j
file_template: "{table}_gen.java"
options:
  package: com.example.cfg
  enums_file: SharedEnums.java
"#,
        )
        .expect("target config");
        let gen = JavaTargetGenerator::from_config(&config);
        assert_eq!(gen.output_dir, PathBuf::from("build/j"));
        assert_eq!(gen.file_template, "{table}_gen.java");
        assert_eq!(gen.enums_file, "SharedEnums.java");
        assert_eq!(gen.package, "com.example.cfg");

        // The options flow through: package clause, enum imports built from
        // the configured package + holder class, holder file named after the
        // enums_file option's stem.
        let schema = test_schema();
        let artifacts = gen.generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "build/j/Drop_gen.java",
                "build/j/Item_gen.java",
                "build/j/SharedEnums.java",
            ]
        );
        let item = String::from_utf8(artifacts[1].1.clone()).unwrap();
        assert!(item.contains("package com.example.cfg;"));
        assert!(item.contains("import com.example.cfg.SharedEnums.ItemKind;"));
        let holder = String::from_utf8(artifacts[2].1.clone()).unwrap();
        assert!(holder.contains("public final class SharedEnums {"));
    }

    #[test]
    fn test_ident_sanitization() {
        assert_eq!(sanitize_ident("Item"), "Item");
        assert_eq!(sanitize_ident("drop-item"), "drop_item");
        assert_eq!(sanitize_ident("1st"), "_1st");
        assert_eq!(sanitize_ident("列"), "_");
        // Nothing left after sanitizing → a lone underscore.
        assert_eq!(sanitize_ident(""), "_");
        // Java keywords get a trailing underscore (no verbatim escape);
        // restricted identifiers and literals are covered by the same rule.
        assert_eq!(java_ident("class"), "class_");
        assert_eq!(java_ident("package"), "package_");
        assert_eq!(java_ident("var"), "var_");
        assert_eq!(java_ident("record"), "record_");
        assert_eq!(java_ident("yield"), "yield_");
        assert_eq!(java_ident("sealed"), "sealed_");
        assert_eq!(java_ident("permits"), "permits_");
        assert_eq!(java_ident("true"), "true_");
        assert_eq!(java_ident("null"), "null_");
        // A lone `_` is illegal since Java 9 — maps to `__`.
        assert_eq!(java_ident("_"), "__");
        assert_eq!(java_ident("name"), "name");
    }

    #[test]
    fn test_field_collisions_get_unique_members() {
        let mut used = HashSet::new();
        assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b");
        assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b_");
        assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b__");
    }

    #[test]
    fn test_class_name_collision() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  CageEnums:
    name: CageEnums
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
enums:
  Kind:
    name: Kind
    values:
      - { name: A, value: 1 }
",
        )
        .expect("schema");
        let artifacts = gen().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // The holder claims its package-level name first: the like-named
        // table becomes CageEnums_ and its file stem follows the class
        // ident (the documented Java deviation).
        assert_eq!(
            paths,
            vec!["build/java/CageEnums_.java", "build/java/CageEnums.java"]
        );
        let table = String::from_utf8(artifacts[0].1.clone()).unwrap();
        assert!(table.contains("public final class CageEnums_ {"));
        let holder = String::from_utf8(artifacts[1].1.clone()).unwrap();
        assert!(holder.contains("public final class CageEnums {"));
    }

    #[test]
    fn test_render_default_edge_cases() {
        let schema = Schema::new();
        // Float literals always keep a decimal point; Float32 adds the
        // mandatory `f` suffix (a bare `1.5` would not narrow to float).
        assert_eq!(
            render_default(&serde_json::json!(100.0), &FieldType::Float64, &schema).unwrap(),
            "100.0"
        );
        assert_eq!(
            render_default(&serde_json::json!(1.5), &FieldType::Float32, &schema).unwrap(),
            "1.5f"
        );
        // Non-finite floats map to the java.lang constants (serde_json
        // cannot hold NaN, so json!(NAN) would become Null — the literal
        // renderer is tested directly, like the C# generator).
        assert_eq!(java_float_literal(f64::NAN, false), "Double.NaN");
        assert_eq!(java_float_literal(f64::NAN, true), "Float.NaN");
        assert_eq!(
            java_float_literal(f64::INFINITY, true),
            "Float.POSITIVE_INFINITY"
        );
        assert_eq!(
            java_float_literal(f64::NEG_INFINITY, false),
            "Double.NEGATIVE_INFINITY"
        );
        // long-typed fields carry the L suffix; int-typed fields do not.
        assert_eq!(
            render_default(&serde_json::json!(100), &FieldType::Int64, &schema).unwrap(),
            "100L"
        );
        assert_eq!(
            render_default(&serde_json::json!(100), &FieldType::UInt32, &schema).unwrap(),
            "100L"
        );
        assert_eq!(
            render_default(&serde_json::json!(100), &FieldType::Int32, &schema).unwrap(),
            "100"
        );
        // Array defaults are fresh mutable lists with scalar elements.
        assert_eq!(
            render_default(
                &serde_json::json!(["pvp", "x"]),
                &FieldType::Array(Box::new(FieldType::String)),
                &schema
            )
            .unwrap(),
            "new ArrayList<>(List.of(\"pvp\", \"x\"))"
        );
        assert_eq!(
            render_default(
                &serde_json::json!([]),
                &FieldType::Array(Box::new(FieldType::String)),
                &schema
            )
            .unwrap(),
            "new ArrayList<>(List.of())"
        );
        // Array-of-array defaults are not scalar — skipped (shared rule).
        assert!(render_default(
            &serde_json::json!([[1]]),
            &FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Int32)))),
            &schema
        )
        .is_none());
        // UInt64 above i64::MAX has no Java literal — initializer skipped.
        assert!(render_default(
            &serde_json::json!(18_446_744_073_709_551_615u64),
            &FieldType::UInt64,
            &schema
        )
        .is_none());
        // A value that does not fit the field's Java type has no literal.
        assert!(render_default(
            &serde_json::json!(5_000_000_000i64),
            &FieldType::Int32,
            &schema
        )
        .is_none());
        // Kind mismatch → None (field falls back to its wrapper type).
        assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &schema).is_none());
        // Object defaults are not rendered (shared rule across targets).
        assert!(render_default(
            &serde_json::json!({"a": 1}),
            &FieldType::Object(indexmap::IndexMap::default()),
            &schema
        )
        .is_none());
        // Control characters use four-hex-digit \u00XX escapes — a raw
        // \u000a would inject a newline even inside a string literal, so
        // CR/LF/TAB keep their short escapes.
        assert_eq!(java_string_literal("a\u{1}b"), "\"a\\u0001b\"");
        assert_eq!(java_string_literal("a\u{7f}b"), "\"a\\u007fb\"");
        assert_eq!(java_string_literal("n\n"), "\"n\\n\"");
        assert_eq!(java_string_literal("q\"r\\s"), "\"q\\\"r\\\\s\"");
        // int_literal accepts only integral kinds — anything else falls in
        // the same "no safe literal" bucket as an out-of-range value.
        assert!(int_literal(1, &FieldType::Float32).is_none());
    }

    #[test]
    fn test_enum_payload_backing() {
        let value = |v: Option<serde_json::Value>, name: &str| EnumValue {
            name: name.to_string(),
            value: v,
            description: None,
        };
        let fits_i32 = EnumSchema {
            name: "A".to_string(),
            values: vec![
                value(Some(serde_json::json!(1)), "One"),
                value(Some(serde_json::json!(-5)), "Neg"),
            ],
            description: None,
        };
        let overflows = EnumSchema {
            name: "B".to_string(),
            values: vec![
                value(Some(serde_json::json!(3_000_000_000i64)), "Large"),
                value(
                    Some(serde_json::json!(18_446_744_073_709_551_615u64)),
                    "Huge",
                ),
            ],
            description: None,
        };
        // i32 range → int payload, no literal suffix.
        assert!(fits_i32.values.iter().all(value_fits_i32));
        let (lit, note) = integral_literal(&fits_i32.values[0], false);
        assert_eq!((lit.as_str(), note), ("1", None));
        // Beyond i32::MAX (u32-sized or a float payload) → long backing.
        assert!(overflows.values.iter().all(|v| !value_fits_i32(v)));
        let (lit, note) = integral_literal(&overflows.values[0], true);
        assert_eq!((lit.as_str(), note), ("3000000000L", None));
        // Above i64::MAX the u64 wraps to its two's-complement negative
        // literal with a note (Java has no unsigned 64-bit type).
        let (lit, note) = integral_literal(&overflows.values[1], true);
        assert_eq!(lit, "-1L");
        assert_eq!(
            note.as_deref(),
            Some("wrapped from unsigned 18446744073709551615")
        );
        // Without the long backing the wrapped value renders bare (int
        // payloads widen silently).
        let (lit, note) = integral_literal(&overflows.values[1], false);
        assert_eq!(lit, "-1");
        assert_eq!(
            note.as_deref(),
            Some("wrapped from unsigned 18446744073709551615")
        );
        // A value-less member has no integral payload: the name is the
        // fallback, and it never fits an int payload.
        let valueless = value(None, "NoVal");
        assert!(!value_fits_i32(&valueless));
        let (lit, note) = integral_literal(&valueless, false);
        assert_eq!((lit.as_str(), note), ("NoVal", None));
        // A float-valued member has no i64/u64 spelling either — same name
        // fallback.
        let floaty = value(Some(serde_json::json!(1.5)), "Floaty");
        let (lit, note) = integral_literal(&floaty, true);
        assert_eq!((lit.as_str(), note), ("Floaty", None));
    }
}
