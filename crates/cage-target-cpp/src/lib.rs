//! C++ Target Generator — generates header-only struct bindings from a Cage
//! schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields keep name
//! order, enum values keep their schema order, and no timestamps are written
//! — the same schema always produces byte-identical files.
//!
//! Each table becomes one `{table}.h` with a `struct` whose members are
//! value-initialized (`{}`) or carry their schema default as a member
//! initializer; absent-able fields wrap in `std::optional`. Shared enums live
//! in one `cage_enums.h`: all-integral enums become `enum class` with a
//! range-selected backing type, string/valueless enums become a `namespace`
//! of `inline constexpr std::string_view` constants. Every file header is
//! stamped with the manifest schema hash.

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
// Stage: crate-prefixed type names (CppTargetGenerator, ...) are idiomatic
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
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use idents::{cpp_ident, unique_ident};
use literals::{
    enum_backing, field_doc, integral_value_literal, is_integral_enum, render_default,
    string_bucket_value,
};
use types::cpp_type;

/// Official templates ship with the crate (design §22 G2): `include_str!`
/// compiles them into the binary so no filesystem is needed.
const TABLE_H_TEMPLATE: &str = include_str!("../templates/table.h.tera");
const ENUMS_H_TEMPLATE: &str = include_str!("../templates/enums.h.tera");

/// C++ Target Generator
pub struct CppTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.h")
    pub file_template: String,
    /// File name of the shared enums header
    pub enums_file: String,
    /// Enclosing namespace (e.g., `cage::generated`)
    pub namespace: String,
}

impl Default for CppTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/cpp"),
            file_template: "{table}.h".to_string(),
            enums_file: "cage_enums.h".to_string(),
            namespace: "cage::generated".to_string(),
        }
    }
}

/// ISO C++ keyword set (core keywords + alternative tokens such as `and`,
/// `or`, `not`, …). C++ has no identifier escape syntax, so keywords get a
/// trailing `_`.
/// Allocated name + bucket of one emitted enum.
#[derive(Clone)]
struct EnumAlloc {
    /// Identifier used at the declaration site (keyword-escaped, unique).
    ident: String,
    /// `true` when every member value is an i64/u64 number (`enum class`).
    integral: bool,
}

/// Include set accumulated while rendering one file: sorted angle-bracket
/// system includes plus a flag for the quoted enums header.
#[derive(Default)]
struct Needs {
    /// System include names without angle brackets (already name-sorted).
    angle: BTreeSet<&'static str>,
    /// Whether the file references at least one resolved enum.
    enums_header: bool,
}

/// Allocation result shared by `generate` and the G4 filter library:
/// struct idents in table order, enum allocations by schema key, and the
/// same allocations paired with their schemas for the header render.
type CppAllocations<'a> = (
    Vec<String>,
    HashMap<&'a str, EnumAlloc>,
    Vec<(&'a EnumSchema, EnumAlloc)>,
);

impl CppTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.h".to_string()),
            ..Self::default()
        };
        if let Some(opts) = &config.options {
            if let Some(s) = opts.get("enums_file").and_then(|v| v.as_str()) {
                gen.enums_file = s.to_string();
            }
            if let Some(s) = opts.get("namespace").and_then(|v| v.as_str()) {
                gen.namespace = s.to_string();
            }
        }
        gen
    }

    /// Generate one header per table (name order) plus a shared enums header.
    ///
    /// `schema_hash` is the same hash `manifest.json` records for the schema
    /// (Build Manifest 口径); it is stamped into every file header.
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        let tables = Self::sorted_tables(schema);

        // ONE shared set for every type name (see `allocate_names`):
        // struct names first, then enum names — file content and paths
        // can never drift.
        let (struct_names, enum_by_key, enum_render) = Self::allocate_names(schema);

        // Shared-enums context, precomputed once (the `extras` hook hands
        // it to the enums header render).
        let enums_ctx = Self::cpp_enums_context(&self.namespace, &enum_render);

        // Official templates ship with this crate (design §22 G2): rendered
        // from memory, registered in the legacy emission order (tables,
        // then the shared enums header when the schema has any).
        let mut templates: Vec<(&str, &str)> =
            vec![(self.file_template.as_str(), TABLE_H_TEMPLATE)];
        if !enum_render.is_empty() {
            templates.push((self.enums_file.as_str(), ENUMS_H_TEMPLATE));
        }
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        let ns = self.namespace.clone();
        let enums_file = self.enums_file.clone();
        let struct_by_name: HashMap<String, String> = tables
            .iter()
            .zip(&struct_names)
            .map(|(t, n)| (t.name.clone(), n.clone()))
            .collect();
        let enum_by_key: HashMap<String, EnumAlloc> = enum_by_key
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        engine
            .generate_official(
                schema,
                schema_hash,
                &templates,
                |_tera, _schema| {},
                move |schema, table| {
                    Self::cpp_extras(
                        &ns,
                        &enums_file,
                        &struct_by_name,
                        &enum_by_key,
                        &enums_ctx,
                        schema,
                        table,
                    )
                },
            )
            .expect("official C++ templates are valid Tera")
    }

    /// Package-level name allocation, shared by `generate` and the G4
    /// filter library (`register_filters`): ONE shared set for every type
    /// name in the generated namespace — struct names first (tables in
    /// name order), then enum class / namespace names (enums in name
    /// order). Allocated before any rendering so file content and paths
    /// can never drift.
    fn allocate_names(schema: &Schema) -> CppAllocations<'_> {
        let tables = Self::sorted_tables(schema);
        let emitted = Self::emitted_enums(schema);
        let mut used: HashSet<String> = HashSet::new();
        let struct_names: Vec<String> = tables
            .iter()
            .map(|t| unique_ident(cpp_ident(&t.name), &mut used))
            .collect();

        // Field types resolve enums through the schema key (same lookup the
        // other code targets use); the header renders them in name order.
        let mut enum_by_key: HashMap<&str, EnumAlloc> = HashMap::new();
        let mut enum_render: Vec<(&EnumSchema, EnumAlloc)> = Vec::new();
        for &(key, e) in &emitted {
            let alloc = EnumAlloc {
                ident: unique_ident(cpp_ident(&e.name), &mut used),
                integral: is_integral_enum(e),
            };
            enum_by_key.insert(key, alloc.clone());
            enum_render.push((e, alloc));
        }
        (struct_names, enum_by_key, enum_render)
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

    /// Emitted shared enums in name order, paired with their schema keys;
    /// empty enums are skipped everywhere.
    fn emitted_enums(schema: &Schema) -> Vec<(&str, &EnumSchema)> {
        let mut enums: Vec<(&str, &EnumSchema)> = schema
            .enums
            .iter()
            .filter(|(_, e)| !e.values.is_empty())
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        enums.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        enums
    }

    /// Shared-enums context for the enums header template: sorted system
    /// includes, then per-enum shape decisions (`enum class` vs string
    /// constants namespace, backing type, member decl lines).
    fn cpp_enums_context(namespace: &str, enum_render: &[(&EnumSchema, EnumAlloc)]) -> Value {
        let mut angle: BTreeSet<&'static str> = BTreeSet::new();
        for (_, alloc) in enum_render {
            if alloc.integral {
                angle.insert("cstdint");
            } else {
                angle.insert("string_view");
            }
        }
        let enums: Vec<Value> = enum_render
            .iter()
            .map(|(e, alloc)| {
                let mut used_members: HashSet<String> = HashSet::new();
                let members: Vec<Value> = e
                    .values
                    .iter()
                    .map(|v| {
                        let member = unique_ident(cpp_ident(&v.name), &mut used_members);
                        let decl = if alloc.integral {
                            format!("{member} = {},", integral_value_literal(v))
                        } else {
                            format!(
                                "inline constexpr std::string_view {member}{{{}}};",
                                string_bucket_value(v)
                            )
                        };
                        json!({ "decl": decl, "desc": v.description })
                    })
                    .collect();
                let backing = if alloc.integral {
                    enum_backing(e).to_string()
                } else {
                    String::new()
                };
                json!({
                    "ident": alloc.ident,
                    "description": e.description,
                    "integral": alloc.integral,
                    "backing": backing,
                    "members": members,
                })
            })
            .collect();
        json!({
            "header_what": "enums:  shared definitions",
            "namespace": namespace,
            "angle": angle.into_iter().collect::<Vec<_>>(),
            "emitted_enums": enums,
        })
    }

    /// Language precomputation for the official C++ templates (design §22
    /// G2): every decision the legacy renderer made — include surface,
    /// optionality, `{}`-initialization, shared name allocation — is
    /// computed here; the templates only express text shape.
    //
    // Contract: returns `Result` to match the `extras` hook signature even
    // though this precomputation is infallible.
    #[allow(clippy::unnecessary_wraps)]
    fn cpp_extras(
        namespace: &str,
        enums_file: &str,
        struct_by_name: &HashMap<String, String>,
        enum_by_key: &HashMap<String, EnumAlloc>,
        enums_ctx: &Value,
        schema: &Schema,
        table: Option<&TableSchema>,
    ) -> Result<Value, String> {
        let Some(t) = table else {
            return Ok(enums_ctx.clone());
        };
        let struct_name = &struct_by_name[&t.name];
        let enum_map: HashMap<&str, EnumAlloc> = enum_by_key
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let fields = CppTargetGenerator::sorted_fields(t);
        let mut used_members: HashSet<String> = HashSet::new();
        let mut needs = Needs::default();
        let mut members: Vec<Value> = Vec::new();

        // Members first (name order), so the include set reflects exactly
        // what this file ends up using.
        for (field_name, field) in fields {
            let member = unique_ident(cpp_ident(field_name), &mut used_members);
            let ty = cpp_type(&field.field_type, &enum_map, &mut needs);
            let default = field
                .default
                .as_ref()
                .filter(|v| !v.is_null())
                .and_then(|d| render_default(d, &field.field_type, schema));
            // Present members are always `{…}`-initialized: with the rendered
            // default, or value-initialized (`{}`, deterministic zero) when
            // required without one. Array defaults already render as a full
            // brace-init list, so only scalars get wrapped.
            let init = match &default {
                Some(d) => {
                    if d.contains("numeric_limits") {
                        needs.angle.insert("limits");
                    }
                    if d.starts_with('{') {
                        d.clone()
                    } else {
                        format!("{{{d}}}")
                    }
                }
                None => "{}".to_string(),
            };
            let decl = if field.required || default.is_some() {
                format!("{ty} {member}{init};")
            } else {
                needs.angle.insert("optional");
                format!("std::optional<{ty}> {member};")
            };
            members.push(json!({ "decl": decl, "doc": field_doc(schema, field) }));
        }

        let banner_head = if t.primary_key.is_empty() {
            struct_name.clone()
        } else {
            format!("{struct_name} — primary key: {}", t.primary_key.join(", "))
        };

        Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "includes": needs.angle.iter().copied().collect::<Vec<_>>(),
            "enums_file": enums_file,
            "enums_header": needs.enums_header,
            "namespace": namespace,
            "banner_head": banner_head,
            "banner_desc": t.description,
            "struct_name": struct_name,
            "members": members,
        }))
    }
}

/// C++ type for a schema field type, recording which system includes the
/// file needs along the way.
mod filters;
mod idents;
mod literals;
#[cfg(test)]
mod tests;
mod types;

pub use filters::register_filters;
