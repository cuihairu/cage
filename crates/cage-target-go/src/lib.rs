//! Go Target Generator — generates struct bindings from a Cage schema.
//!
//! Schema-driven code generation (design §22/§23 Code Targets): types and
//! metadata come from the Schema, not from row data. Output is deterministic
//! (T4 Canonical 口径): tables are emitted in name order, fields keep name
//! order, enum values keep their schema order, and no timestamps are written
//! — the same schema always produces byte-identical files.
//!
//! Each table becomes one exported struct (its `json` tags carry the original
//! field names) plus a `New{Table}` constructor that applies the schema
//! defaults; shared enums live in one package-level file (`type X int32` /
//! `type X string` plus prefixed constants). Optional value-typed fields are
//! pointers so `nil` can express absence; slices, maps and `any` stay plain
//! because they are nil-able already.
//!
//! The emitter reproduces gofmt's column alignment — text/tabwriter's Elastic
//! Tabstops fed with gofmt's exact parameters and cell layout — so generated
//! files are byte-identical to `gofmt` output for any field set. Field names
//! containing a backtick cannot be expressed in a Go raw-string struct tag
//! and are unsupported.

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
// Stage: crate-prefixed type names (GoTargetGenerator, ...) are idiomatic
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
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Official templates ship with the crate (design §22 G2): `include_str!`
/// compiles them into the binary so rendering never touches the filesystem.
const TABLE_GO_TEMPLATE: &str = include_str!("../templates/table.go.tera");
const ENUMS_GO_TEMPLATE: &str = include_str!("../templates/enums.go.tera");

/// Go Target Generator
pub struct GoTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.go")
    pub file_template: String,
    /// File name of the shared enums file
    pub enums_file: String,
    /// Package clause value (e.g., "config")
    pub package: String,
}

impl Default for GoTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/go"),
            file_template: "{table}.go".to_string(),
            enums_file: "cage_enums.go".to_string(),
            package: "config".to_string(),
        }
    }
}

/// One resolved table: its allocated struct type name and `New` func name.
struct TableNames<'a> {
    schema: &'a TableSchema,
    type_ident: String,
    new_func: String,
}

/// One emitted enum: allocated type name, backing type and member constants.
struct EnumInfo<'a> {
    schema: &'a EnumSchema,
    type_ident: String,
    /// `int32` / `int64` / `uint64` for integral enums, `string` otherwise.
    backing: &'static str,
    consts: Vec<EnumConst>,
}

/// One enum constant: package-level name plus its literal value.
struct EnumConst {
    name: String,
    value: String,
    description: Option<String>,
}

/// One struct member: original field key, exported Go name, type, doc and
/// default literal (when the schema default is renderable).
struct Member {
    field_name: String,
    ident: String,
    ty: String,
    doc: Option<String>,
    default_expr: Option<String>,
}

impl GoTargetGenerator {
    /// Package-level name allocation, shared by `generate` and the G4
    /// filter library (`register_filters`): ONE shared set, in emission
    /// order (tables by name: struct type, then New func; then enums by
    /// name: type, then member consts in schema order).
    fn allocate_names(schema: &Schema) -> (Vec<TableNames<'_>>, Vec<EnumInfo<'_>>) {
        let mut used: HashSet<String> = HashSet::new();
        let tables: Vec<TableNames> = Self::sorted_tables(schema)
            .into_iter()
            .map(|t| {
                let type_ident = unique_ident(go_ident(&t.name), &mut used);
                let new_func = unique_ident(format!("New{type_ident}"), &mut used);
                TableNames {
                    schema: t,
                    type_ident,
                    new_func,
                }
            })
            .collect();
        let enums: Vec<EnumInfo> = Self::emitted_enums(schema)
            .into_iter()
            .map(|e| {
                let type_ident = unique_ident(go_ident(&e.name), &mut used);
                let backing = enum_backing(e);
                let consts = e
                    .values
                    .iter()
                    .map(|v| {
                        let name =
                            unique_ident(format!("{type_ident}{}", go_ident(&v.name)), &mut used);
                        EnumConst {
                            name,
                            value: enum_value_literal(v, backing),
                            description: v.description.clone(),
                        }
                    })
                    .collect();
                EnumInfo {
                    schema: e,
                    type_ident,
                    backing,
                    consts,
                }
            })
            .collect();
        (tables, enums)
    }

    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.go".to_string()),
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

    /// Generate one file per table (name order) plus a shared enums file.
    ///
    /// `schema_hash` is the same hash `manifest.json` records for the schema
    /// (Build Manifest 口径); it is stamped into every file header.
    pub fn generate(&self, schema: &Schema, schema_hash: Option<&str>) -> Vec<(String, Vec<u8>)> {
        // Package-level names are allocated from ONE shared set, in emission
        // Package-level names are allocated from ONE shared set, in emission
        // order (see `allocate_names`): so a `type Item` can never collide
        // with a `func NewItem` or an enum type.
        let (tables, enums) = Self::allocate_names(schema);
        // Field types reference enums by their ALLOCATED (possibly suffixed)
        // type ident; unresolved or empty enums fall back to `string`.
        let enum_types: HashMap<String, String> = enums
            .iter()
            .map(|e| (e.schema.name.clone(), e.type_ident.clone()))
            .collect();
        let tables_by_name: HashMap<String, (String, String)> = tables
            .iter()
            .map(|t| {
                (
                    t.schema.name.clone(),
                    (t.type_ident.clone(), t.new_func.clone()),
                )
            })
            .collect();

        // Shared-enums context, precomputed once (the `extras` hook hands it
        // to the enums file render).
        let enums_ctx = Self::go_enums_context(&self.package, &enums);

        // Official templates ship with this crate (design §22 G2): rendered
        // from memory, registered in the legacy emission order (tables, then
        // the shared enums file when the schema has any).
        let mut templates: Vec<(&str, &str)> =
            vec![(self.file_template.as_str(), TABLE_GO_TEMPLATE)];
        if !enums.is_empty() {
            templates.push((self.enums_file.as_str(), ENUMS_GO_TEMPLATE));
        }
        let engine = TemplateTargetGenerator {
            output_dir: self.output_dir.clone(),
            // Official mode renders from memory; the directory is unused.
            template_dir: PathBuf::new(),
        };
        let package = self.package.clone();
        engine
            .generate_official(
                schema,
                schema_hash,
                &templates,
                |_tera, _schema| {},
                move |schema, table| {
                    Self::go_extras(
                        &package,
                        &tables_by_name,
                        &enum_types,
                        &enums_ctx,
                        schema,
                        table,
                    )
                },
            )
            .expect("official Go templates are valid Tera")
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

    /// Language precomputation for the official Go templates (design §22
    /// G2): every decision the legacy renderer made — member idents, types,
    /// default literals, the `math` import face, the gofmt-aligned block
    /// lines — is computed here; the templates only express file shape.
    //
    // Contract: returns `Result` to match the `extras` hook signature even
    // though this precomputation is infallible.
    #[allow(clippy::unnecessary_wraps)]
    fn go_extras(
        package: &str,
        tables_by_name: &HashMap<String, (String, String)>,
        enum_types: &HashMap<String, String>,
        enums_ctx: &Value,
        schema: &Schema,
        table: Option<&TableSchema>,
    ) -> Result<Value, String> {
        let Some(t) = table else {
            return Ok(enums_ctx.clone());
        };
        let (type_ident, new_func) = &tables_by_name[&t.name];
        let enum_types: HashMap<&str, String> = enum_types
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();

        // Resolve members once: the struct fields and the New{X} literal must
        // never drift.
        let fields = Self::sorted_fields(t);
        let mut used: HashSet<String> = HashSet::new();
        let mut members: Vec<Member> = Vec::new();
        let mut needs_math = false;
        for (field_name, field) in &fields {
            let ident = unique_ident(go_ident(field_name), &mut used);
            let doc_line = field_doc(schema, field);
            let default_expr = field
                .default
                .as_ref()
                .filter(|v| !v.is_null())
                .and_then(|d| render_default(d, &field.field_type, &enum_types));
            if default_expr
                .as_ref()
                .is_some_and(|e| e.starts_with("math."))
            {
                needs_math = true;
            }
            let ty = field_go_type(
                &field.field_type,
                &enum_types,
                field.required,
                default_expr.is_some(),
            );
            members.push(Member {
                field_name: (*field_name).to_string(),
                ident,
                ty,
                doc: doc_line,
                default_expr,
            });
        }

        // The gofmt alignment (tabwriter port) stays in Rust: each block is
        // rendered to its final lines; the template places them.
        let struct_lines = Self::struct_block_lines(type_ident, &members);
        let (new_doc, new_lines) = Self::new_func_lines(new_func, type_ident, &members);

        let banner_head = if t.primary_key.is_empty() {
            t.name.clone()
        } else {
            format!("{} — primary key: {}", t.name, t.primary_key.join(", "))
        };

        Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "package_line": format!("package {}", package_ident(package)),
            "needs_math": needs_math,
            "banner_head": banner_head,
            "banner_desc": t.description,
            "struct_lines": struct_lines,
            "new_doc": new_doc,
            "new_lines": new_lines,
        }))
    }

    /// The gofmt-aligned struct block, as final lines: the `type X struct{}`
    /// one-liner for an empty field set, otherwise `type X struct {` … `}`.
    fn struct_block_lines(type_ident: &str, members: &[Member]) -> Vec<String> {
        let mut doc = Doc::new();
        if members.is_empty() {
            // gofmt prints an empty struct as a one-liner without inner space.
            doc.raw(format!("type {type_ident} struct{{}}"));
        } else {
            doc.line(vec![Cell::new(
                format!("type {type_ident} struct {{"),
                Term::Ff,
            )]);
            let n = members.len();
            for (i, m) in members.iter().enumerate() {
                let last = i + 1 == n;
                if let Some(d) = &m.doc {
                    doc.line(vec![Cell::indent(), Cell::new(format!("// {d}"), Term::Nl)]);
                }
                let tag = json_tag(&m.field_name);
                if n > 1 {
                    // go/printer separates a named field's name / type / tag
                    // with vtabs (plus an empty vtab cell before the tag),
                    // which is exactly what gofmt aligns on.
                    doc.line(vec![
                        Cell::indent(),
                        Cell::new(m.ident.clone(), Term::Vtab),
                        Cell::new(m.ty.clone(), Term::Vtab),
                        Cell::new(String::new(), Term::Vtab),
                        Cell::new(tag, if last { Term::Ff } else { Term::Nl }),
                    ]);
                } else {
                    // A single-field struct uses blank separators — no column
                    // alignment at all.
                    doc.line(vec![
                        Cell::indent(),
                        Cell::new(
                            format!("{} {} {}", m.ident, m.ty, tag),
                            if last { Term::Ff } else { Term::Nl },
                        ),
                    ]);
                }
            }
            doc.raw("}");
        }
        let rendered = doc.finish();
        rendered.lines().map(str::to_string).collect()
    }

    /// The gofmt-aligned constructor block, as final lines (`func New…() T {`
    /// … `}`), plus its doc comment body — `{new} returns an {ty} with the
    /// schema defaults applied.` — without the `//` prefix.
    fn new_func_lines(
        new_func: &str,
        type_ident: &str,
        members: &[Member],
    ) -> (String, Vec<String>) {
        let mut doc = Doc::new();
        doc.line(vec![Cell::new(
            format!("func {new_func}() {type_ident} {{"),
            Term::Ff,
        )]);

        let defaults: Vec<&Member> = members
            .iter()
            .filter(|m| m.default_expr.is_some())
            .collect();
        if defaults.is_empty() {
            doc.line(vec![
                Cell::indent(),
                Cell::new(format!("return {type_ident}{{}}"), Term::Ff),
            ]);
        } else {
            doc.line(vec![
                Cell::indent(),
                Cell::new(format!("return {type_ident}{{"), Term::Ff),
            ]);
            let terms = literal_break_terms(&defaults);
            let n = defaults.len();
            for (i, m) in defaults.iter().enumerate() {
                let expr = m.default_expr.as_deref().unwrap_or_default();
                if n > 1 {
                    doc.line(vec![
                        Cell::indent(),
                        Cell::indent(),
                        Cell::new(format!("{}:", m.ident), Term::Vtab),
                        Cell::new(format!("{expr},"), terms[i]),
                    ]);
                } else {
                    doc.line(vec![
                        Cell::indent(),
                        Cell::indent(),
                        Cell::new(format!("{}: {expr},", m.ident), terms[i]),
                    ]);
                }
            }
            doc.line(vec![Cell::indent(), Cell::new("}", Term::Ff)]);
        }
        doc.raw("}");
        let rendered = doc.finish();
        (
            format!("{new_func} returns an {type_ident} with the schema defaults applied."),
            rendered.lines().map(str::to_string).collect(),
        )
    }

    /// Shared-enums context for the enums file template: per-enum shape
    /// decisions — the schema `name` for the banner, the package-level
    /// allocated `ident`, the backing type literal, and the gofmt-aligned
    /// const spec lines.
    fn go_enums_context(package: &str, enums: &[EnumInfo]) -> Value {
        let emitted: Vec<Value> = enums
            .iter()
            .map(|e| {
                json!({
                    "name": e.schema.name,
                    "ident": e.type_ident,
                    "backing": e.backing,
                    "description": e.schema.description,
                    "const_lines": Self::enum_const_lines(e),
                })
            })
            .collect();
        json!({
            "header_what": "enums:  shared definitions",
            "package_line": format!("package {}", package_ident(package)),
            "emitted_enums": emitted,
        })
    }

    /// The gofmt-aligned `const` spec lines for one enum (no `const (` /
    /// `)` fence — the template spells those).
    fn enum_const_lines(e: &EnumInfo) -> Vec<String> {
        let mut doc = Doc::new();
        let n = e.consts.len();
        for (i, c) in e.consts.iter().enumerate() {
            let last = i + 1 == n;
            let term = if last { Term::Ff } else { Term::Nl };
            if n > 1 {
                // Typed const spec: name \v type \v = value [\v comment].
                let mut cells = vec![
                    Cell::indent(),
                    Cell::new(c.name.clone(), Term::Vtab),
                    Cell::new(e.type_ident.clone(), Term::Vtab),
                    Cell::new(
                        format!("= {}", c.value),
                        if c.description.is_some() {
                            Term::Vtab
                        } else {
                            term
                        },
                    ),
                ];
                if let Some(d) = &c.description {
                    cells.push(Cell::new(format!("// {d}"), term));
                }
                doc.line(cells);
            } else {
                // A single spec prints with blank separators — no columns.
                let text = match &c.description {
                    Some(d) => format!("{} {} = {} // {d}", c.name, e.type_ident, c.value),
                    None => format!("{} {} = {}", c.name, e.type_ident, c.value),
                };
                doc.line(vec![Cell::indent(), Cell::new(text, term)]);
            }
        }
        let rendered = doc.finish();
        rendered.lines().map(str::to_string).collect()
    }
}

mod docfmt;
mod filters;
mod idents;
mod literals;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use docfmt::{literal_break_terms, Cell, Doc, Term};
pub use filters::register_filters;
pub(crate) use idents::{go_ident, package_ident, unique_ident};
pub(crate) use literals::{enum_backing, enum_value_literal, field_doc, json_tag, render_default};
pub(crate) use types::field_go_type;
