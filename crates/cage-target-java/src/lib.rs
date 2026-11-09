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
    schema::{EnumSchema, FieldSchema, FieldType, Schema, TableSchema},
};
use cage_target_template::TemplateTargetGenerator;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Official templates ship with the crate (design §22 G2): `include_str!`
/// compiles them into the binary so rendering never touches the filesystem.
const TABLE_JAVA_TEMPLATE: &str = include_str!("../templates/table.java.tera");
const ENUMS_JAVA_TEMPLATE: &str = include_str!("../templates/enums.java.tera");

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

        // Enums-unit context, precomputed once (the `extras` hook hands it
        // to the shared unit render).
        let enums_ctx = Self::java_enums_context(
            &self.package,
            &emitted,
            enums_class.as_deref(),
            &enum_idents,
        );

        // Official templates ship with this crate (design §22 G2): rendered
        // from memory, registered in the legacy emission order (tables, then
        // the shared enums unit when the schema has any).
        let mut templates: Vec<(&str, &str)> =
            vec![(self.file_template.as_str(), TABLE_JAVA_TEMPLATE)];
        if !emitted.is_empty() {
            templates.push((self.enums_file.as_str(), ENUMS_JAVA_TEMPLATE));
        }
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        let package = self.package.clone();
        let classes_by_name: HashMap<String, String> = tables
            .iter()
            .map(|(t, class)| (t.name.clone(), class.clone()))
            .collect();
        let holder_for_ctx = enums_class.clone();
        let mut artifacts = engine
            .generate_official(
                schema,
                schema_hash,
                &templates,
                |_tera, _schema| {},
                move |schema, table| {
                    Self::java_extras(
                        &package,
                        &classes_by_name,
                        holder_for_ctx.as_deref(),
                        &enum_idents,
                        &enums_ctx,
                        schema,
                        table,
                    )
                },
            )
            .expect("official Java templates are valid Tera");

        // Java paths follow the allocated class idents, never the raw schema
        // names (file stem = class ident); re-path the engine's artifacts in
        // emission order (tables in name order, then the shared unit).
        for ((path, _), (_, class)) in artifacts.iter_mut().zip(&tables) {
            *path = self.path_for(class);
        }
        if let Some(holder) = &enums_class {
            if let Some((path, _)) = artifacts.last_mut() {
                *path = self.enums_path(holder);
            }
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

    /// Enums-unit context for the shared enums template: per-enum shape
    /// decisions — nested `ident` allocation lookup, the single-line
    /// declaration Javadoc body, payload bucket, and the full constant decl
    /// lines. An enum whose allocated ident is missing from the map is
    /// skipped rather than rendered with a broken name.
    fn java_enums_context(
        package: &str,
        emitted: &[&EnumSchema],
        holder: Option<&str>,
        enum_idents: &HashMap<String, String>,
    ) -> Value {
        let enums: Vec<Value> = emitted
            .iter()
            .filter_map(|e| {
                let ident = enum_idents.get(e.name.as_str())?;

                // Bucket: every member carries an integral value → int/long
                // payload picked by value range; otherwise a String payload
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

                let mut used: HashSet<String> = HashSet::new();
                let n = e.values.len();
                let members: Vec<Value> = e
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let member = unique_ident(java_ident(&v.name), &mut used);
                        let sep = if i + 1 == n { ";" } else { "," };
                        let decl = if all_integral {
                            let (lit, note) = integral_literal(v, long_backing);
                            match note {
                                Some(note) => format!("{member}({lit}){sep} // {note}"),
                                None => format!("{member}({lit}){sep}"),
                            }
                        } else {
                            format!("{member}({}){sep}", string_bucket_value(v))
                        };
                        json!({ "decl": decl, "doc": v.description })
                    })
                    .collect();

                // Declaration Javadoc body: name, plus description after an
                // em dash.
                let banner = match &e.description {
                    Some(desc) => format!("{} — {}", e.name, desc),
                    None => e.name.clone(),
                };
                Some(json!({
                    "ident": ident,
                    "banner": banner,
                    "payload": payload,
                    "members": members,
                }))
            })
            .collect();
        json!({
            "header_what": "enums:  shared definitions",
            "package": package,
            "holder_ident": holder,
            "emitted_enums": enums,
        })
    }

    /// Language precomputation for the official Java templates (design §22
    /// G2): every decision the legacy renderer made — member idents and
    /// optionality, enum type text with the qualified-holder rule, the
    /// sorted import surface, defaults, banner — is computed here; the
    /// templates only express file shape.
    //
    // Contract: returns `Result` to match the `extras` hook signature even
    // though this precomputation is infallible.
    #[allow(clippy::unnecessary_wraps)]
    fn java_extras(
        package: &str,
        classes_by_name: &HashMap<String, String>,
        enums_class: Option<&str>,
        enum_idents: &HashMap<String, String>,
        enums_ctx: &Value,
        schema: &Schema,
        table: Option<&TableSchema>,
    ) -> Result<Value, String> {
        let Some(t) = table else {
            return Ok(enums_ctx.clone());
        };
        let class = &classes_by_name[&t.name];

        // Member idents, defaults and enum type text resolved once — the
        // import scan and the declarations both read this list. One `used`
        // set across the whole table: colliding field keys get suffixed
        // members.
        let mut used: HashSet<String> = HashSet::new();
        let rows: Vec<FieldRow> = Self::sorted_fields(t)
            .into_iter()
            .map(|(name, field)| {
                let member = unique_ident(java_ident(name), &mut used);
                let default = field
                    .default
                    .as_ref()
                    .filter(|v| !v.is_null())
                    .and_then(|d| render_default(d, &field.field_type, schema));
                // A table whose own class shares the enum's simple name
                // must not import it (JLS 7.5.1) — use the qualified
                // Holder.Enum form instead.
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
        // The collection scans walk the whole nested type — a map value may
        // itself be an array or map, so a map of arrays spells
        // HashMap<String, List<Integer>> and uses both imports.
        let mut imports: Vec<String> = Vec::new();
        if rows.iter().any(|r| uses_list(&r.field.field_type)) {
            imports.push("java.util.List".to_string());
        }
        if rows
            .iter()
            .any(|r| r.default.is_some() && matches!(r.field.field_type, FieldType::Array(_)))
        {
            imports.push("java.util.ArrayList".to_string());
        }
        if rows.iter().any(|r| uses_map_interface(&r.field.field_type))
            || rows
                .iter()
                .any(|r| r.default.as_deref().is_some_and(|d| d.contains("Map.of")))
        {
            imports.push("java.util.Map".to_string());
        }
        if rows.iter().any(|r| uses_hash_map(&r.field.field_type)) {
            imports.push("java.util.HashMap".to_string());
        }
        if let Some(holder) = enums_class {
            for row in &rows {
                // The qualified Holder.Enum form (class-name clash) needs
                // no import; idents never contain a dot.
                if let Some(text) = row.enum_text.as_ref().filter(|t| !t.contains('.')) {
                    let import = format!("{package}.{holder}.{text}");
                    if !imports.contains(&import) {
                        imports.push(import);
                    }
                }
            }
        }
        imports.sort();

        // Class banner: description first, then name / primary-key head
        // (raw schema name, same 口径 as the Python generator).
        let head = if t.primary_key.is_empty() {
            t.name.clone()
        } else {
            format!("{} — primary key: {}", t.name, t.primary_key.join(", "))
        };

        // Declarations: optionality group rule (required OR renderable
        // default keeps the present type; otherwise primitives widen to
        // wrappers, references are nullable as-is), default inlined.
        let members: Vec<Value> = rows
            .iter()
            .map(|row| {
                let optional = !row.field.required && row.default.is_none();
                let ty = java_type(&row.field.field_type, optional, row.enum_text.as_deref());
                let decl = match &row.default {
                    Some(d) => format!("{ty} {} = {d};", row.member),
                    None => format!("{ty} {};", row.member),
                };
                json!({ "decl": decl, "doc": field_doc(schema, row.field) })
            })
            .collect();

        Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "package": package,
            "imports": imports,
            "banner_head": head,
            "banner_desc": t.description,
            "class_ident": class,
            "members": members,
        }))
    }
}

mod filters;
mod idents;
mod literals;
#[cfg(test)]
mod tests;
mod types;

pub use filters::register_filters;
pub(crate) use idents::{java_ident, unique_ident};
pub(crate) use literals::{
    field_doc, integral_literal, render_default, resolve_enum, string_bucket_value, value_fits_i32,
};
pub(crate) use types::{java_type, uses_hash_map, uses_list, uses_map_interface};
