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
    schema::{EnumSchema, FieldSchema, FieldType, Schema, TableSchema},
};
use cage_target_template::TemplateTargetGenerator;
use std::collections::HashSet;
use std::path::PathBuf;

use idents::{ts_ident, unique_ident};

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

use plan::ts_extras;

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
pub(crate) fn emitted_enums(schema: &Schema) -> Vec<&EnumSchema> {
    let mut enums: Vec<&EnumSchema> = schema
        .enums
        .values()
        .filter(|e| !e.values.is_empty())
        .collect();
    enums.sort_by(|a, b| a.name.cmp(&b.name));
    enums
}

/// Fields in name order (deterministic member order).
pub(crate) fn sorted_fields(table: &TableSchema) -> Vec<(&str, &FieldSchema)> {
    let mut fields: Vec<(&str, &FieldSchema)> =
        table.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
    fields.sort_by(|a, b| a.0.cmp(b.0));
    fields
}

/// Exported binding of each emitted enum, in name order: (schema name,
/// identifier as it appears in the shared enums module).
pub(crate) fn enum_exports(schema: &Schema) -> Vec<(&str, String)> {
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
pub(crate) fn module_specifier(enums_file: &str) -> String {
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
pub(crate) fn jsdoc_import_path(enums_file: &str) -> String {
    if enums_file.starts_with("./") || enums_file.starts_with("../") {
        enums_file.to_string()
    } else {
        format!("./{enums_file}")
    }
}

/// Banner line under the file docs: table name plus primary key(s), same
/// wording as the Python generator's class banner.
pub(crate) fn banner_line(table: &TableSchema) -> String {
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
pub(crate) fn collect_enum_refs(
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
pub(crate) fn dts_path_for(js_path: &str) -> String {
    match js_path.strip_suffix(".js") {
        Some(stem) => format!("{stem}.d.ts"),
        None => format!("{js_path}.d.ts"),
    }
}

mod docfmt;
mod filters;
mod idents;
mod literals;
mod plan;
#[cfg(test)]
mod tests;
mod types;

pub use filters::register_filters;
