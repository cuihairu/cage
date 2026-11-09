//! Lua Target Generator — generates table-binding modules from a Cage schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields keep name
//! order, enum values keep their schema order, and no timestamps are written
//! — the same schema always produces byte-identical files.
//!
//! Each table module exposes name / `primary_key` / field metadata / schema
//! defaults plus an `M.new(t)` constructor that fills missing fields from the
//! defaults (table defaults — arrays and maps alike — are copied recursively
//! so rows never share state). Shared enums live in one flat module
//! (`M.<Enum> = { member = value, ... }`).

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
// Stage: crate-prefixed type names (LuaTargetGenerator, ...) are idiomatic
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

/// Lua Target Generator
pub struct LuaTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.lua")
    pub file_template: String,
    /// File name of the shared enums module
    pub enums_file: String,
}

impl Default for LuaTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/lua"),
            file_template: "{table}.lua".to_string(),
            enums_file: "cage_enums.lua".to_string(),
        }
    }
}

impl LuaTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.lua".to_string()),
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
            include_str!("../templates/table.lua.tera"),
        )];
        if !Self::emitted_enums(schema).is_empty() {
            templates.push((
                self.enums_file.as_str(),
                include_str!("../templates/enums.lua.tera"),
            ));
        }
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        engine
            .generate_official(
                schema,
                schema_hash,
                &templates,
                |tera, _schema| {
                    tera.register_filter("lua_string", LuaStringFilter);
                },
                lua_extras,
            )
            .expect("official Lua templates are valid Tera")
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
/// schema mappings, collected literals, and collision-free member names —
/// templates express text shape only.
// The Result matches the engine's extras contract even though nothing here
// can fail today; the contract keeps future fallible precomputation open.
#[allow(clippy::unnecessary_wraps)]
fn lua_extras(schema: &Schema, table: Option<&TableSchema>) -> Result<Value, String> {
    match table {
        Some(t) => Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "primary_key_lit": primary_key_lit(t),
            "member_fields": member_fields(schema, t),
        })),
        None => Ok(json!({ "emitted_enums": emitted_enum_objects(schema) })),
    }
}

/// `M.primary_key = { <literal list> }` contents; empty when the table
/// declares no primary key (the template then omits the line).
fn primary_key_lit(table: &TableSchema) -> String {
    table
        .primary_key
        .iter()
        .map(|k| lua_string_literal(k))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Per-field decisions in name order (the emission order): unique Lua
/// member, type label, doc summary, required spelling, safe default
/// literal (or null when the default has no safe rendering).
fn member_fields(schema: &Schema, table: &TableSchema) -> Vec<Value> {
    let mut used: HashSet<String> = HashSet::new();
    LuaTargetGenerator::sorted_fields(table)
        .into_iter()
        .map(|(name, field)| {
            json!({
                "name": name,
                "member": unique_ident(lua_ident(name), &mut used),
                "type_label": lua_type_label(&field.field_type, schema),
                "doc": field_doc(schema, field),
                "required_str": if field.required { "true" } else { "false" },
                "default_expr": field
                    .default
                    .as_ref()
                    .filter(|v| !v.is_null())
                    .and_then(|d| render_default(d, &field.field_type, schema)),
            })
        })
        .collect()
}

/// Shared enums in name order (empty enums filtered out), with
/// collision-free idents/members and Lua value literals — the emission
/// order the legacy render loop used.
fn emitted_enum_objects(schema: &Schema) -> Vec<Value> {
    let mut used_tables: HashSet<String> = HashSet::new();
    LuaTargetGenerator::emitted_enums(schema)
        .into_iter()
        .map(|e| {
            let mut used_members: HashSet<String> = HashSet::new();
            let members: Vec<Value> = e
                .values
                .iter()
                .map(|v| {
                    let member = unique_ident(lua_ident(&v.name), &mut used_members);
                    let value_lit = match &v.value {
                        Some(Value::Number(n)) => n.to_string(),
                        Some(Value::String(s)) => lua_string_literal(s),
                        Some(Value::Bool(b)) => {
                            lua_string_literal(if *b { "true" } else { "false" })
                        }
                        _ => lua_string_literal(&v.name),
                    };
                    json!({
                        "member": member,
                        "value_lit": value_lit,
                        "description": v.description,
                    })
                })
                .collect();
            json!({
                "ident": unique_ident(lua_ident(&e.name), &mut used_tables),
                "name": e.name,
                "description": e.description,
                "members": members,
            })
        })
        .collect()
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
pub(crate) use filters::LuaStringFilter;
pub(crate) use idents::{lua_ident, unique_ident};
pub(crate) use literals::{lua_string_literal, render_default};
pub(crate) use types::lua_type_label;
