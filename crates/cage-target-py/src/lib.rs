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
    schema::{EnumSchema, FieldSchema, FieldType, Schema, TableSchema},
};
use cage_target_template::TemplateTargetGenerator;
use serde_json::{json, Value};
use std::collections::HashSet;
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
        // Official templates ship with this crate (design §22 G2): rendered
        // from memory, registered in the legacy emission order (tables,
        // then the shared enums module when the schema has any).
        let mut templates: Vec<(&str, &str)> = vec![(
            self.file_template.as_str(),
            include_str!("../templates/table.py.tera"),
        )];
        if !Self::emitted_enums(schema).is_empty() {
            templates.push((
                self.enums_file.as_str(),
                include_str!("../templates/enums.py.tera"),
            ));
        }
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        let enums_file = self.enums_file.clone();
        engine
            .generate_official(
                schema,
                schema_hash,
                &templates,
                |_tera, _schema| {},
                |schema, table| py_extras(&enums_file, schema, table),
            )
            .expect("official Python templates are valid Tera")
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

/// Language precomputation for the official Python templates (design §22
/// G2): every decision the legacy renderer made — import surface,
/// dataclass field grouping, `IntEnum` vs plain-class bucketing, literals
/// and identifiers — is computed here; the template only expresses text
/// shape.
//
// Contract: returns `Result` to match the `extras` hook signature even
// though this precomputation is infallible.
#[allow(clippy::unnecessary_wraps)]
fn py_extras(
    enums_file: &str,
    schema: &Schema,
    table: Option<&TableSchema>,
) -> Result<Value, String> {
    if let Some(t) = table {
        let fields = PyTargetGenerator::sorted_fields(t);

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

        // Enum imports: emitted shared enums actually referenced by fields,
        // including through `list[...]`/`dict[...]` nesting.
        let enum_import: Option<String> = {
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
            (!names.is_empty()).then(|| {
                let module = enums_file.trim_end_matches(".py");
                format!("from {module} import {}", names.join(", "))
            })
        };

        // Class banner: name, primary key, optional description.
        let banner_head = if t.primary_key.is_empty() {
            t.name.clone()
        } else {
            format!("{} — primary key: {}", t.name, t.primary_key.join(", "))
        };

        // dataclass syntax: fields without defaults must precede defaulted
        // ones — required fields with no renderable default go first, then
        // everything carrying a default (name order inside each group).
        let mut plain: Vec<Value> = Vec::new();
        let mut defaulted: Vec<Value> = Vec::new();
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
                    plain.push(json!({
                        "doc": doc,
                        "line": format!("{member}: {}", py_type(&field.field_type, schema)),
                    }));
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
                    defaulted.push(json!({
                        "doc": doc,
                        "line": format!("{member}: {} = {expr}", py_type(&field.field_type, schema)),
                    }));
                }
                None => {
                    let line = format!(
                        "{member}: {} | None = None",
                        py_type(&field.field_type, schema)
                    );
                    defaulted.push(json!({ "doc": doc, "line": line }));
                }
            }
        }
        let mut members = plain;
        members.append(&mut defaulted);

        Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "field_import": if needs_field { ", field" } else { "" },
            "needs_any": needs_any,
            "enum_import": enum_import,
            "banner_head": banner_head,
            "banner_desc": t.description,
            "class_ident": py_ident(&t.name),
            "pass_only": fields.is_empty(),
            "members": members,
        }))
    } else {
        // Member backing: an IntEnum is emitted only when EVERY member has
        // an explicit integral value (Python ints are unbounded, so there is
        // no backing-type concern). Otherwise the member names are the
        // values (Cage compares enums as strings) and we emit a plain class
        // of string constants.
        let enums: Vec<Value> = PyTargetGenerator::emitted_enums(schema)
            .into_iter()
            .map(|e| {
                let all_integral = !e.values.is_empty()
                    && e.values.iter().all(
                        |v| matches!(&v.value, Some(Value::Number(n)) if n.is_i64() || n.is_u64()),
                    );
                let ident = py_ident(&e.name);
                let class_head = if all_integral {
                    format!("class {ident}(IntEnum):")
                } else {
                    format!("class {ident}:")
                };
                let mut used: HashSet<String> = HashSet::new();
                let members: Vec<Value> = e
                    .values
                    .iter()
                    .map(|v| {
                        let member = unique_ident(py_ident(&v.name), &mut used);
                        let decl = if all_integral {
                            match &v.value {
                                Some(Value::Number(n)) => format!("{member} = {n}"),
                                _ => format!("{member} = {}", v.name),
                            }
                        } else {
                            let value = match &v.value {
                                Some(Value::String(s)) => py_string_literal(s),
                                Some(Value::Number(n)) => py_string_literal(&n.to_string()),
                                Some(Value::Bool(b)) => {
                                    py_string_literal(if *b { "true" } else { "false" })
                                }
                                _ => py_string_literal(&v.name),
                            };
                            format!("{member} = {value}")
                        };
                        json!({ "doc": v.description, "decl": decl })
                    })
                    .collect();
                json!({
                    "name": e.name,
                    "banner_desc": e.description,
                    "class_head": class_head,
                    "members": members,
                })
            })
            .collect();
        Ok(json!({ "emitted_enums": enums }))
    }
}

mod docfmt;
mod filters;
mod idents;
mod literals;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use docfmt::field_doc;
pub use filters::register_filters;
pub(crate) use idents::{py_ident, unique_ident};
pub(crate) use literals::{py_string_literal, render_default};
pub(crate) use types::{annotation_enum_names, annotation_uses_any, py_type};
