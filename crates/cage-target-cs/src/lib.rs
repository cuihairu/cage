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
    schema::{EnumSchema, FieldSchema, Schema, TableSchema},
};
use cage_target_template::TemplateTargetGenerator;
use serde_json::{json, Value};
use std::collections::HashSet;
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
        // Official templates ship with this crate (design §22 G2): rendered
        // from memory, registered in the legacy emission order (tables,
        // then the shared enums file when the schema has any).
        let mut templates: Vec<(&str, &str)> = vec![(
            self.file_template.as_str(),
            include_str!("../templates/table.cs.tera"),
        )];
        if !Self::emitted_enums(schema).is_empty() {
            templates.push((
                self.enums_file.as_str(),
                include_str!("../templates/enums.cs.tera"),
            ));
        }
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        let namespace = self.namespace.clone();
        engine
            .generate_official(
                schema,
                schema_hash,
                &templates,
                |_tera, _schema| {},
                |schema, table| cs_extras(&namespace, schema, table),
            )
            .expect("official C# templates are valid Tera")
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
}

/// Language decisions merged into every template context (design §22 G2):
/// namespace, summaries, class idents, resolved member types/inits, and
/// the enum-vs-static-class split — templates express text shape only.
// The Result matches the engine's extras contract even though nothing here
// can fail today; the contract keeps future fallible precomputation open.
#[allow(clippy::unnecessary_wraps)]
fn cs_extras(
    namespace: &str,
    schema: &Schema,
    table: Option<&TableSchema>,
) -> Result<Value, String> {
    if let Some(t) = table {
        let head = if t.primary_key.is_empty() {
            xml_escape(&t.name)
        } else {
            format!(
                "{} — primary key: {}",
                xml_escape(&t.name),
                xml_escape(&t.primary_key.join(", "))
            )
        };
        let mut used: HashSet<String> = HashSet::new();
        let members: Vec<Value> = CsTargetGenerator::sorted_fields(t)
            .into_iter()
            .map(|(field_name, field)| {
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
                json!({
                    "member": unique_ident(cs_ident(field_name), &mut used),
                    "ty": ty,
                    "doc": field_doc(schema, field),
                    "init": init,
                })
            })
            .collect();
        Ok(json!({
            "namespace_x": namespace,
            "header_what": format!("table:  {}", t.name),
            "summary_head": head,
            "summary_desc": t.description.as_deref().map(xml_escape),
            "class_ident": cs_ident(&t.name),
            "members": members,
        }))
    } else {
        let enums: Vec<Value> = CsTargetGenerator::emitted_enums(schema)
            .into_iter()
            .map(|e| {
                // Member backing: a C# enum is emitted only when EVERY
                // member has an explicit integral value. Otherwise the
                // member names are the values (Cage compares enums as
                // strings) and we emit a static class of string
                // constants — C# enums cannot hold strings.
                let all_integral = !e.values.is_empty()
                    && e.values.iter().all(
                        |v| matches!(&v.value, Some(Value::Number(n)) if n.is_i64() || n.is_u64()),
                    );
                let mut max_u64: Option<u64> = None;
                let mut has_negative = false;
                if all_integral {
                    for v in &e.values {
                        if let Some(Value::Number(n)) = &v.value {
                            if let Some(u) = n.as_u64() {
                                max_u64 = Some(max_u64.map_or(u, |m| m.max(u)));
                            } else {
                                has_negative = true;
                            }
                        }
                    }
                }
                let backing = if !all_integral {
                    String::new()
                } else if has_negative {
                    " : long".to_string()
                } else if max_u64.is_some_and(|m| m > i64::MAX as u64) {
                    " : ulong".to_string()
                } else if max_u64.is_some_and(|m| m > i32::MAX as u64) {
                    " : long".to_string()
                } else {
                    String::new()
                };
                let summary = match &e.description {
                    Some(desc) => {
                        format!("{} — {}", xml_escape(&e.name), xml_escape(desc))
                    }
                    None => xml_escape(&e.name),
                };
                let mut used: HashSet<String> = HashSet::new();
                let members: Vec<Value> = e
                    .values
                    .iter()
                    .map(|v| {
                        let member = unique_ident(cs_ident(&v.name), &mut used);
                        let decl = if all_integral {
                            match &v.value {
                                Some(Value::Number(n)) => format!("{member} = {n},"),
                                _ => format!("{member},"),
                            }
                        } else {
                            let value = match &v.value {
                                Some(Value::String(s)) => cs_string_literal(s),
                                Some(Value::Number(n)) => cs_string_literal(&n.to_string()),
                                Some(Value::Bool(b)) => cs_string_literal(&b.to_string()),
                                _ => cs_string_literal(&v.name),
                            };
                            format!("public const string {member} = {value};")
                        };
                        json!({ "member": member, "decl": decl, "doc": v.description })
                    })
                    .collect();
                json!({
                    "ident": cs_ident(&e.name),
                    "summary": summary,
                    "is_enum": all_integral,
                    "backing": backing,
                    "members": members,
                })
            })
            .collect();
        Ok(json!({ "namespace_x": namespace, "emitted_enums": enums }))
    }
}

mod docfmt;
mod filters;
mod idents;
mod literals;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use docfmt::{field_doc, xml_escape};
pub use filters::register_filters;
pub(crate) use idents::{cs_ident, unique_ident};
pub(crate) use literals::{cs_string_literal, needs_ref_init, ref_empty_init, render_default};
pub(crate) use types::cs_type_inner;
