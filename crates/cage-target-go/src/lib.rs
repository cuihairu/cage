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
    schema::{EnumSchema, FieldSchema, FieldType, MapField, MapKeyType, Schema, TableSchema},
};
use cage_target_template::TemplateTargetGenerator;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
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

/// Go keywords (all lowercase). Only the `package` clause value needs the
/// trailing-`_` escape: exported Pascal-case identifiers can never collide
/// with an all-lowercase keyword.
const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

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
        // order (tables by name: struct type, then New func; then enums by
        // name: type, then member consts in schema order) so a `type Item`
        // can never collide with a `func NewItem` or an enum type.
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

// ---------------------------------------------------------------------------
// gofmt-shaped output model: cells, lines and tabwriter sections
// ---------------------------------------------------------------------------

/// Cell terminator — the separator go/printer feeds to text/tabwriter after
/// each cell. The final cell of every line carries the line break itself.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Term {
    /// '\t' — hard tab; the column can never be discarded (indent columns).
    Htab,
    /// '\v' — soft tab; a column of only-empty vtab cells is discarded.
    Vtab,
    /// '\n' — line break.
    Nl,
    /// '\f' — formfeed: line break that also ends the alignment section.
    Ff,
}

/// One tab-separated cell (text plus its terminator).
struct Cell {
    text: String,
    term: Term,
}

impl Cell {
    fn new(text: impl Into<String>, term: Term) -> Self {
        Self {
            text: text.into(),
            term,
        }
    }

    /// An empty hard-tab cell — one indentation level.
    fn indent() -> Self {
        Self::new(String::new(), Term::Htab)
    }

    /// Cell width in runes (tabwriter counts runes, not bytes).
    fn width(&self) -> usize {
        self.text.chars().count()
    }
}

/// A gofmt-shaped document: lines of tab-separated cells, split into the
/// sections the tabwriter would flush (at formfeeds and at lines that carry
/// no alignment cells at all).
#[derive(Default)]
struct Doc {
    sections: Vec<Vec<Vec<Cell>>>,
    current: Vec<Vec<Cell>>,
}

impl Doc {
    fn new() -> Self {
        Self::default()
    }

    /// Append a finished line. Lines that carry no alignment cells (a single
    /// cell), or that end in a formfeed, terminate the current section —
    /// exactly the conditions under which text/tabwriter flushes.
    fn line(&mut self, cells: Vec<Cell>) {
        let ends_section = cells.len() == 1 || cells[cells.len() - 1].term == Term::Ff;
        self.current.push(cells);
        if ends_section {
            self.sections.push(std::mem::take(&mut self.current));
        }
    }

    /// A single-cell line: no tabs, so the tabwriter flushes immediately.
    fn raw(&mut self, text: impl Into<String>) {
        self.line(vec![Cell::new(text, Term::Nl)]);
    }
    /// Render every section through the tabwriter port; each line ends in
    /// exactly one newline, so the file is newline-terminated.
    fn finish(mut self) -> String {
        if !self.current.is_empty() {
            self.sections.push(std::mem::take(&mut self.current));
        }
        let mut out = String::new();
        for section in &self.sections {
            format_section(section, 0, section.len(), &mut Vec::new(), &mut out);
        }
        out
    }
}

/// Port of text/tabwriter's `format` with gofmt's configuration
/// (minwidth 0, tabwidth 8, padding 1, padchar ' ', `DiscardEmptyColumns` |
/// `TabIndent`): contiguous runs of lines sharing a cell in a column form a
/// block; every column cell is padded (with spaces) to the block's widest
/// cell plus one, all-empty soft-tab columns are discarded, and the leading
/// empty indent cells are padded with tabs.
fn format_section(
    lines: &[Vec<Cell>],
    line0: usize,
    line1: usize,
    widths: &mut Vec<usize>,
    out: &mut String,
) {
    let mut line0 = line0;
    let mut this = line0;
    while this < line1 {
        let column = widths.len();
        if column + 1 >= lines[this].len() {
            this += 1;
            continue;
        }
        // print unprinted lines until beginning of block
        write_section_lines(lines, line0, this, widths, out);
        line0 = this;
        // column block: the contiguous run of lines with a cell at `column`
        let mut width = 0; // minwidth
        let mut discardable = true;
        while this < line1 {
            let line = &lines[this];
            if column + 1 >= line.len() {
                break;
            }
            let c = &line[column];
            width = width.max(c.width() + 1); // + padding
            if c.width() > 0 || c.term == Term::Htab {
                discardable = false;
            }
            this += 1;
        }
        if discardable {
            width = 0;
        }
        widths.push(width);
        format_section(lines, line0, this, widths, out);
        widths.pop();
        line0 = this;
        this += 1; // the line that broke the block lacks this cell too
    }
    write_section_lines(lines, line0, line1, widths, out);
}

/// Port of text/tabwriter's `writeLines` for one block of lines.
fn write_section_lines(
    lines: &[Vec<Cell>],
    line0: usize,
    line1: usize,
    widths: &[usize],
    out: &mut String,
) {
    for line in &lines[line0..line1] {
        let mut use_tabs = true; // TabIndent: tabs for leading empty cells
        for (j, c) in line.iter().enumerate() {
            if c.text.is_empty() {
                if j < widths.len() {
                    write_padding(c.width(), widths[j], use_tabs, out);
                }
            } else {
                use_tabs = false;
                out.push_str(&c.text);
                if j < widths.len() {
                    write_padding(c.width(), widths[j], false, out);
                }
            }
        }
        out.push('\n');
    }
}

/// Port of text/tabwriter's `writePadding`: tab padding rounds the cell up to
/// a multiple of the tab width; space padding pads exactly to the cell width.
fn write_padding(textw: usize, cellw: usize, use_tabs: bool, out: &mut String) {
    if use_tabs {
        let cellw = cellw.div_ceil(8) * 8;
        let n = cellw - textw;
        for _ in 0..n.div_ceil(8) {
            out.push('\t');
        }
    } else {
        for _ in 0..(cellw - textw) {
            out.push(' ');
        }
    }
}

/// Port of go/printer exprList's section-break heuristic for multi-line
/// composite literals: the line break after an entry becomes a formfeed
/// (splitting the alignment) when the key sizes involved differ enough
/// (smallSize 40, threshold r = 2.5, log2ish/exp2ish geometric mean).
/// The last entry's break is always the terminating formfeed.
fn literal_break_terms(members: &[&Member]) -> Vec<Term> {
    let n = members.len();
    let mut terms = vec![Term::Nl; n];
    // nodeSize of the whole `Key: Value` pair, in bytes; 0 = too big to fit.
    let pair_size = |m: &Member| -> i64 {
        let expr = m.default_expr.as_deref().unwrap_or_default();
        (m.ident.len() + 2 + expr.len()) as i64
    };
    let key_size = |m: &Member| -> i64 {
        if pair_size(m) > 1_000_000 {
            0
        } else {
            m.ident.len() as i64
        }
    };
    let mut log2sum = 0.0f64;
    let mut count = 0usize;
    let mut prev_size = 0i64;
    for i in 0..n {
        let size = key_size(members[i]);
        if i > 0 {
            let mut use_ff = true;
            if prev_size > 0 && size > 0 {
                if count == 0 || (prev_size <= 40 && size <= 40) {
                    use_ff = false;
                } else {
                    let geomean = exp2ish(log2sum / count as f64);
                    let ratio = size as f64 / geomean;
                    use_ff = 2.5 * ratio <= 1.0 || 2.5 <= ratio;
                }
            }
            terms[i - 1] = if use_ff { Term::Ff } else { Term::Nl };
            // a new section restarts the geometric-mean accumulation
            if use_ff {
                log2sum = 0.0;
                count = 0;
            }
        }
        if size > 0 {
            log2sum += log2ish(size as f64);
            count += 1;
        }
        prev_size = size;
    }
    if n > 0 {
        terms[n - 1] = Term::Ff; // terminating comma needs a line break
    }
    terms
}

/// Crude log₂ ported from go/printer (math.go) — identical on all platforms.
fn log2ish(x: f64) -> f64 {
    let (f, e) = frexp(x);
    f64::from(e) + 2.0 * (f - 1.0)
}

/// Crude 2^x ported from go/printer (math.go).
fn exp2ish(x: f64) -> f64 {
    let n = x.floor();
    let f = x - n;
    (1.0 + f) * (2.0f64).powi(n as i32)
}

/// frexp: x = f * 2^e with 0.5 <= f < 1 (loop form — exact for powers of 2,
/// and our inputs are small positive sizes).
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let mut f = x;
    let mut e = 0i32;
    while f.abs() >= 1.0 {
        f *= 0.5;
        e += 1;
    }
    while f.abs() < 0.5 {
        f *= 2.0;
        e -= 1;
    }
    (f, e)
}

// ---------------------------------------------------------------------------
// identifiers, types, literals
// ---------------------------------------------------------------------------

/// Sanitize a schema name into Go identifier characters (ASCII alnum + `_`).
fn sanitize_go_ident(name: &str) -> String {
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
    out
}

/// Exported Go identifier for a schema name: sanitize, split on `_`, then
/// Pascal-case each segment (first char uppercased, rest untouched —
/// `player_id` → `PlayerId`, `ID` → `ID`, `drop-item` → `DropItem`). The
/// result starts with an ASCII letter (`1st` → `X1st`) and is never empty
/// (`列` → `X`). Go keywords are all-lowercase, so no keyword escaping is
/// needed for exported names.
fn go_ident(name: &str) -> String {
    let mut out = String::new();
    for segment in sanitize_go_ident(name).split('_') {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        out.push('X');
    } else if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert(0, 'X');
    }
    out
}

/// `package` clause value with a trailing `_` for Go keywords.
fn package_ident(name: &str) -> String {
    if GO_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
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

/// Whether the Go type is nil-able without a pointer (slices, maps, `any`).
fn is_nilable_reference(ft: &FieldType) -> bool {
    matches!(
        ft,
        FieldType::Array(_)
            | FieldType::Bytes
            | FieldType::Map(_)
            | FieldType::Object(_)
            | FieldType::Null
            | FieldType::Any
    )
}

/// Go type for a field. Optionality rule: required or schema-defaulted
/// fields keep the plain value type; anything else must be nil-able, so value
/// types (bool, ints, floats, string, resolved enums) become pointers while
/// reference types ([]T, []byte, map, any) stay plain.
fn field_go_type(
    ft: &FieldType,
    enum_types: &HashMap<&str, String>,
    required: bool,
    has_default: bool,
) -> String {
    let base = go_type(ft, enum_types);
    if required || has_default || is_nilable_reference(ft) {
        base
    } else {
        format!("*{base}")
    }
}

/// Go type for a schema field type (unresolved or empty enums → `string`).
fn go_type(ft: &FieldType, enum_types: &HashMap<&str, String>) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "any".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Int8 => "int8".to_string(),
        FieldType::Int16 => "int16".to_string(),
        FieldType::Int32 => "int32".to_string(),
        FieldType::Int64 => "int64".to_string(),
        FieldType::UInt8 => "uint8".to_string(),
        FieldType::UInt16 => "uint16".to_string(),
        FieldType::UInt32 => "uint32".to_string(),
        FieldType::UInt64 => "uint64".to_string(),
        FieldType::Float32 => "float32".to_string(),
        FieldType::Float64 => "float64".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "[]byte".to_string(),
        FieldType::Array(inner) => format!("[]{}", go_type(inner, enum_types)),
        FieldType::Object(_) => "map[string]any".to_string(),
        FieldType::Map(map) => format!(
            "map[{}]{}",
            go_map_key_type(map.key_type),
            go_type(&map.value_type, enum_types)
        ),
        FieldType::Enum(name) => enum_types
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| "string".to_string()),
    }
}

/// Go type for a map key: string keys → `string`; signed-integer keys →
/// `int64` (the schema's key semantics are i64, and encoding/json decodes
/// the quoted-string keys of integer-keyed maps natively).
fn go_map_key_type(key_type: MapKeyType) -> &'static str {
    match key_type {
        MapKeyType::String => "string",
        MapKeyType::Int => "int64",
    }
}

/// Render a schema default as a Go literal expression; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, enum
/// kinds, mismatched kinds).
fn render_default(
    value: &serde_json::Value,
    ft: &FieldType,
    enum_types: &HashMap<&str, String>,
) -> Option<String> {
    match ft {
        FieldType::Bool => value
            .as_bool()
            .map(|b| if b { "true" } else { "false" }.to_string()),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => value
            .as_i64()
            .map(|i| i.to_string())
            .or_else(|| value.as_u64().map(|u| u.to_string())),
        FieldType::Float32 | FieldType::Float64 => value.as_f64().map(go_float_literal),
        FieldType::String => value.as_str().map(go_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, enum_types)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| map_literal(entries, map, enum_types)),
        _ => None,
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Python/Lua generators — keeps the outputs aligned).
fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    enum_types: &HashMap<&str, String>,
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
        parts.push(render_default(item, inner, enum_types)?);
    }
    Some(format!(
        "[]{}{{{}}}",
        go_type(inner, enum_types),
        parts.join(", ")
    ))
}

/// Map defaults render each entry's value through [`render_default`] under
/// the same compile-safe rule as array defaults (one unrenderable entry drops
/// the whole default). An empty default renders the typed empty map literal —
/// the constructor evaluates it on every call, so rows never share one map.
/// Entry order follows the default object's (`serde_json`'s lexicographic)
/// key order, which is deterministic.
fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    enum_types: &HashMap<&str, String>,
) -> Option<String> {
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        parts.push(format!(
            "{}: {}",
            map_key_literal(key, map.key_type)?,
            render_default(value, &map.value_type, enum_types)?
        ));
    }
    Some(format!(
        "map[{}]{}{{{}}}",
        go_map_key_type(map.key_type),
        go_type(&map.value_type, enum_types),
        parts.join(", ")
    ))
}

/// Map key literal: string keys render as Go string literals; integer keys
/// are stored as numeric strings in the data model and parse as i64, so they
/// render bare to match the `int64` map key type (non-numeric keys — already
/// rejected at L2 for data — drop the default rather than mis-render).
fn map_key_literal(key: &str, key_type: MapKeyType) -> Option<String> {
    match key_type {
        MapKeyType::String => Some(go_string_literal(key)),
        MapKeyType::Int => key.parse::<i64>().ok().map(|i| i.to_string()),
    }
}

/// Go float literal: integral values render bare (`100` is a valid untyped
/// constant), non-finite values need the `math` package.
fn go_float_literal(f: f64) -> String {
    if f.is_nan() {
        "math.NaN()".to_string()
    } else if f.is_infinite() {
        if f < 0.0 {
            "math.Inf(-1)".to_string()
        } else {
            "math.Inf(1)".to_string()
        }
    } else {
        f.to_string()
    }
}

/// Struct tag carrying the ORIGINAL field name; `"` inside the name is JSON-
/// escaped inside the raw-string tag. (Field names containing a backtick
/// cannot be represented — see crate docs.)
fn json_tag(field_name: &str) -> String {
    format!("`json:\"{}\"`", field_name.replace('"', "\\\""))
}

fn go_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Go literals: `\\`, `\"` and the short escapes
/// `\n` / `\r` / `\t`; other control characters (including U+007F) become
/// `\xHH` with exactly two hex digits; printable Unicode passes through.
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
                let _ = write!(out, "\\x{code:02x}");
            }
            c => out.push(c),
        }
    }
    out
}

/// Enum backing type: an enum where EVERY member has an integral value is
/// numeric, with the smallest range that fits (int32 → int64 → uint64);
/// anything else is a string enum.
fn enum_backing(e: &EnumSchema) -> &'static str {
    let all_integral = !e.values.is_empty()
        && e.values.iter().all(
            |v| matches!(&v.value, Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64()),
        );
    if !all_integral {
        return "string";
    }
    let mut lo = 0i128;
    let mut hi = 0i128;
    for v in &e.values {
        let x = match &v.value {
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from)),
            _ => None,
        };
        if let Some(x) = x {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if lo >= i128::from(i32::MIN) && hi <= i128::from(i32::MAX) {
        "int32"
    } else if lo >= i128::from(i64::MIN) && hi <= i128::from(i64::MAX) {
        "int64"
    } else {
        "uint64"
    }
}

/// Enum member literal: numeric enums keep their decimal value; string enums
/// stringify the value (String as-is, Number as decimal, Bool as
/// `"true"`/`"false"`, value-less members fall back to their name).
fn enum_value_literal(v: &cage_core::schema::EnumValue, backing: &str) -> String {
    use serde_json::Value as J;
    match backing {
        "string" => {
            let s = match &v.value {
                Some(J::String(s)) => s.clone(),
                Some(J::Number(n)) => n.to_string(),
                Some(J::Bool(b)) => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                _ => v.name.clone(),
            };
            go_string_literal(&s)
        }
        _ => match &v.value {
            Some(J::Number(n)) => n.to_string(),
            _ => v.name.clone(), // unreachable: all-integral was checked
        },
    }
}

/// Field comment body: description plus constraint summary; `None` when
/// there is nothing to say (same format as the C#/Python/Lua generators).
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
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
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

    fn gen() -> GoTargetGenerator {
        GoTargetGenerator::default()
    }

    /// Collapse each line's whitespace (tabs + alignment padding) so tests
    /// assert structure, not gofmt's byte-level column padding.
    fn normalized(src: &str) -> Vec<String> {
        src.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    #[test]
    fn test_generate_artifact_paths() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // Tables in name order, shared enums file last.
        assert_eq!(
            paths,
            vec![
                "build/go/Drop.go",
                "build/go/Item.go",
                "build/go/cage_enums.go",
            ]
        );
    }

    #[test]
    fn test_table_rendering() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let item = String::from_utf8(artifacts[1].1.clone()).unwrap();

        // Tooling marker on line 1, then the standard Cage header block.
        assert!(item.starts_with("// Code generated by Cage — DO NOT EDIT.\n"));
        assert!(item.contains("// <auto-generated>\n"));
        assert!(item.contains("//   Generated by Cage — do not edit.\n"));
        assert!(item.contains("//   schema: abc123\n"));
        assert!(item.contains("//   table:  Item\n"));
        assert!(item.contains("// </auto-generated>\n"));
        assert!(item.contains("package config\n"));

        // Banner: name + primary key, then description.
        assert!(item.contains("// Item — primary key: id\n"));
        assert!(item.contains("// Equipment definitions.\n"));
        assert!(item.contains("type Item struct {\n"));

        // Struct fields in pure name order (id < kind < name < note < owner <
        // price < rarity < tags < weight), original names in the json tags.
        let tag_pos = |tag: &str| item.find(tag).unwrap();
        assert!(tag_pos("`json:\"id\"`") < tag_pos("`json:\"kind\"`"));
        assert!(tag_pos("`json:\"kind\"`") < tag_pos("`json:\"name\"`"));
        assert!(tag_pos("`json:\"name\"`") < tag_pos("`json:\"note\"`"));
        assert!(tag_pos("`json:\"note\"`") < tag_pos("`json:\"owner\"`"));
        assert!(tag_pos("`json:\"owner\"`") < tag_pos("`json:\"price\"`"));
        assert!(tag_pos("`json:\"price\"`") < tag_pos("`json:\"rarity\"`"));
        assert!(tag_pos("`json:\"rarity\"`") < tag_pos("`json:\"tags\"`"));
        assert!(tag_pos("`json:\"tags\"`") < tag_pos("`json:\"weight\"`"));

        // Types + optionality pointers. Required or defaulted fields keep
        // plain value types; optional value fields become pointers; slices
        // and maps stay plain.
        let lines = normalized(&item);
        for expected in [
            "Id int32 `json:\"id\"`",
            "Kind *ItemKind `json:\"kind\"`",
            "Name string `json:\"name\"`",
            "Note *string `json:\"note\"`",
            "Owner *string `json:\"owner\"`",
            "Price *int32 `json:\"price\"`",
            "Rarity *string `json:\"rarity\"`",
            "Tags []string `json:\"tags\"`",
            "Weight float64 `json:\"weight\"`",
        ] {
            assert!(lines.contains(&expected.to_string()), "missing: {expected}");
        }

        // Constraint docs render as // comments above the fields.
        assert!(item.contains("// Identifier, required\n"));
        assert!(item.contains("// Display name, required\n"));
        assert!(item.contains("// Price in gold, min: 0\n"));
        assert!(item.contains("// allowed: common | rare\n"));
        assert!(item.contains("// → Player.id\n"));

        // New{Table} constructor: doc, header, only renderable defaults.
        assert!(item.contains("// NewItem returns an Item with the schema defaults applied.\n"));
        assert!(item.contains("func NewItem() Item {\n"));
        assert!(lines.contains(&"Tags: []string{\"pvp\"},".to_string()));
        assert!(lines.contains(&"Weight: 1.5,".to_string()));
        assert!(lines.contains(&"return Item{".to_string()));
        // No math import in this schema.
        assert!(!item.contains("import \"math\""));
        // Newline-terminated file.
        assert!(item.ends_with("}\n"));
    }

    #[test]
    fn test_enums_rendering() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let enums_src = String::from_utf8(artifacts[2].1.clone()).unwrap();

        assert!(enums_src.contains("//   enums:  shared definitions\n"));
        assert!(enums_src.contains("package config\n"));
        let lines = normalized(&enums_src);

        // Integral enum: typed constants with prefixed names; descriptions
        // become trailing comments on the const line.
        assert!(lines.contains(&"type ItemKind int32".to_string()));
        assert!(lines.contains(&"ItemKindSword ItemKind = 1 // Sword weapon".to_string()));
        assert!(lines.contains(&"ItemKindShield ItemKind = 2".to_string()));

        // Value-less enum → string enum, member names as values.
        assert!(lines.contains(&"type Rarity string".to_string()));
        assert!(lines.contains(&"RarityCommon Rarity = \"common\"".to_string()));
        assert!(lines.contains(&"RarityRare Rarity = \"rare\"".to_string()));

        // Empty enums are not emitted; enums keep name order.
        assert!(!enums_src.contains("EmptyEnum"));
        assert!(enums_src.find("type ItemKind").unwrap() < enums_src.find("type Rarity").unwrap());
    }

    #[test]
    fn test_unresolved_enum_falls_back_to_string() {
        let schema = test_schema();
        let artifacts = gen().generate(&schema, None);
        let drop_src = String::from_utf8(artifacts[0].1.clone()).unwrap();
        assert!(drop_src.contains("//   schema: (unavailable)\n"));
        let lines = normalized(&drop_src);
        // Unresolved enum field: string fallback, pointer for optionality.
        assert!(lines.contains(&"Id int64 `json:\"id\"`".to_string()));
        assert!(lines.contains(&"Item *string `json:\"item\"`".to_string()));
        assert!(drop_src.contains("// unresolved enum: MissingEnum\n"));
        // Constructor exists even with no defaults.
        assert!(drop_src.contains("func NewDrop() Drop {\n"));
        assert!(lines.contains(&"return Drop{}".to_string()));
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
    fn test_from_config_defaults_without_options() {
        let config: TargetConfig = serde_yaml::from_str(
            r"
format: go
output_dir: build/game
",
        )
        .expect("target config");
        let gen = GoTargetGenerator::from_config(&config);
        // No `options` block → every generator knob keeps its default.
        assert_eq!(gen.output_dir, PathBuf::from("build/game"));
        assert_eq!(gen.file_template, "{table}.go");
        assert_eq!(gen.enums_file, "cage_enums.go");
        assert_eq!(gen.package, "config");
    }

    #[test]
    fn test_from_config_options() {
        let config: TargetConfig = serde_yaml::from_str(
            r#"
format: go
output_dir: build/game
file_template: "{table}_gen.go"
options:
  enums_file: shared_enums.go
  package: gamecfg
"#,
        )
        .expect("target config");
        let gen = GoTargetGenerator::from_config(&config);
        assert_eq!(gen.output_dir, PathBuf::from("build/game"));
        assert_eq!(gen.file_template, "{table}_gen.go");
        assert_eq!(gen.enums_file, "shared_enums.go");
        assert_eq!(gen.package, "gamecfg");
    }

    #[test]
    fn test_ident_sanitization() {
        assert_eq!(go_ident("Item"), "Item");
        assert_eq!(go_ident("player_id"), "PlayerId");
        assert_eq!(go_ident("ID"), "ID");
        assert_eq!(go_ident("drop-item"), "DropItem");
        // Non-letter start gets an X prefix; nothing left becomes X.
        assert_eq!(go_ident("1st"), "X1st");
        assert_eq!(go_ident("列"), "X");
        assert_eq!(go_ident(""), "X");
        // Package clause escapes Go keywords (exported idents never need to).
        assert_eq!(package_ident("func"), "func_");
        assert_eq!(package_ident("config"), "config");
    }

    #[test]
    fn test_field_collisions_get_unique_members() {
        let mut used = HashSet::new();
        assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b");
        assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b_");
        assert_eq!(unique_ident("a_b".to_string(), &mut used), "a_b__");
    }

    #[test]
    fn test_package_level_name_collisions() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: String }, required: true }
  NewItem:
    name: NewItem
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: String }, required: true }
  Status:
    name: Status
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: String }, required: true }
      st: { name: st, type: { kind: Enum, value: Status } }
enums:
  Status:
    name: Status
    values:
      - { name: Active }
",
        )
        .expect("collision schema");
        let artifacts = gen().generate(&schema, Some("deadbeef"));
        // Tables in name order: Item, NewItem, Status, then the enums file.
        assert_eq!(artifacts[0].0, "build/go/Item.go");
        let new_item = normalized(&String::from_utf8(artifacts[1].1.clone()).unwrap());
        // `func NewItem` was claimed by Item, so the NewItem table gets a
        // suffix — and its own constructor follows the suffixed type name.
        assert!(new_item.contains(&"type NewItem_ struct {".to_string()));
        assert!(new_item.contains(&"func NewNewItem_() NewItem_ {".to_string()));
        // The table claimed `Status` first, so the enum type is suffixed and
        // fields referencing it use the suffixed ident.
        let status = normalized(&String::from_utf8(artifacts[2].1.clone()).unwrap());
        assert!(status.contains(&"St *Status_ `json:\"st\"`".to_string()));
        let enums_src = normalized(&String::from_utf8(artifacts[3].1.clone()).unwrap());
        assert!(enums_src.contains(&"type Status_ string".to_string()));
        assert!(enums_src.contains(&"Status_Active Status_ = \"Active\"".to_string()));
    }

    #[test]
    fn test_render_default_edge_cases() {
        let schema = Schema::new();
        let enums = enum_type_map(&schema);
        // Integral floats render bare: `100` is a valid untyped constant.
        assert_eq!(
            render_default(&serde_json::json!(100.0), &FieldType::Float64, &enums).unwrap(),
            "100"
        );
        // Non-finite floats need the math package.
        assert_eq!(go_float_literal(f64::NAN), "math.NaN()");
        assert_eq!(go_float_literal(f64::INFINITY), "math.Inf(1)");
        assert_eq!(go_float_literal(f64::NEG_INFINITY), "math.Inf(-1)");
        // Kind mismatch → None.
        assert!(render_default(&serde_json::json!("x"), &FieldType::Int32, &enums).is_none());
        // Object defaults are not rendered (shared rule across targets).
        assert!(render_default(
            &serde_json::json!({"a": 1}),
            &FieldType::Object(indexmap::IndexMap::default()),
            &enums
        )
        .is_none());
        // Array defaults carry the element type.
        assert_eq!(
            render_default(
                &serde_json::json!([1, 2]),
                &FieldType::Array(Box::new(FieldType::Int32)),
                &enums
            )
            .unwrap(),
            "[]int32{1, 2}"
        );
        // Control characters use Go's \xHH escapes (two hex digits).
        assert_eq!(go_string_literal("a\u{1}b"), "\"a\\x01b\"");
        assert_eq!(go_string_literal("a\u{7f}b"), "\"a\\x7fb\"");
        // The five short escapes pass through verbatim.
        assert_eq!(
            go_string_literal("q\"r\\s\n\t\rz"),
            "\"q\\\"r\\\\s\\n\\t\\rz\""
        );
        // Nested array defaults are not scalar — skipped (shared rule).
        assert!(render_default(
            &serde_json::json!([[1]]),
            &FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Int32)))),
            &enums
        )
        .is_none());
    }

    #[test]
    fn test_map_type_rendering() {
        let schema: Schema = serde_yaml::from_str(
            r#"
tables:
  Loot:
    name: Loot
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      drops: { name: drops, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } } }
      weights: { name: weights, type: { kind: Map, value: { key_type: int, value_type: { kind: Float32 } } } }
      nested: { name: nested, type: { kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } } }
      kinds: { name: kinds, type: { kind: Map, value: { key_type: string, value_type: { kind: Enum, value: ItemKind } } } }
      counts: { name: counts, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }, default: {} }
      scores: { name: scores, type: { kind: Map, value: { key_type: int, value_type: { kind: String } } }, default: {"1": a, "-2": b} }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
"#,
        )
        .expect("map schema");
        let artifacts = gen().generate(&schema, Some("abc123"));
        assert_eq!(artifacts.len(), 2); // Loot.go + cage_enums.go
        let src = String::from_utf8(artifacts[0].1.clone()).unwrap();
        let lines = normalized(&src);

        // Typed maps: key type (string / int64 for int keys), then the value
        // type — arrays and nested maps nest, enum values resolve to the
        // allocated type name.
        for expected in [
            "Drops map[string][]int32 `json:\"drops\"`",
            "Weights map[int64]float32 `json:\"weights\"`",
            "Nested map[string]map[string]int32 `json:\"nested\"`",
            "Kinds map[string]ItemKind `json:\"kinds\"`",
            "Counts map[string]int32 `json:\"counts\"`",
        ] {
            assert!(lines.contains(&expected.to_string()), "missing: {expected}");
        }
        // Maps are nil-able already: optional map fields never get a pointer.
        assert!(!src.contains("*map["));

        // New{Table}: the empty map default renders the typed empty literal,
        // and entry defaults render keys per key type — quoted strings, bare
        // i64 — in the default object's key order ("-2" sorts before "1").
        assert!(src.contains("func NewLoot() Loot {\n"));
        assert!(lines.contains(&"Counts: map[string]int32{},".to_string()));
        assert!(lines.contains(&"Scores: map[int64]string{-2: \"b\", 1: \"a\"},".to_string()));
    }

    #[test]
    fn test_map_default_rendering_rules() {
        let schema = test_schema();
        let enums = enum_type_map(&schema);
        let map = |k: MapKeyType, v: FieldType| {
            FieldType::Map(MapField {
                key_type: k,
                value_type: Box::new(v),
            })
        };
        // Key spelling: string → string, int → int64 (i64 key semantics).
        assert_eq!(go_map_key_type(MapKeyType::String), "string");
        assert_eq!(go_map_key_type(MapKeyType::Int), "int64");
        // Empty default → the typed empty literal.
        assert_eq!(
            render_default(
                &serde_json::json!({}),
                &map(MapKeyType::String, FieldType::Int32),
                &enums
            )
            .unwrap(),
            "map[string]int32{}"
        );
        // Entries: string keys quoted, values per the existing scalar rules.
        assert_eq!(
            render_default(
                &serde_json::json!({"a": 1, "b": 2}),
                &map(MapKeyType::String, FieldType::Int32),
                &enums
            )
            .unwrap(),
            "map[string]int32{\"a\": 1, \"b\": 2}"
        );
        // Integer keys are numeric strings in the data model; they parse as
        // i64 and render bare.
        assert_eq!(
            render_default(
                &serde_json::json!({"1": 0.5, "-2": 0.25}),
                &map(MapKeyType::Int, FieldType::Float32),
                &enums
            )
            .unwrap(),
            "map[int64]float32{-2: 0.25, 1: 0.5}"
        );
        // Array values render under the array rule; nested maps nest.
        assert_eq!(
            render_default(
                &serde_json::json!({"common": [1, 2]}),
                &map(
                    MapKeyType::String,
                    FieldType::Array(Box::new(FieldType::Int32))
                ),
                &enums
            )
            .unwrap(),
            "map[string][]int32{\"common\": []int32{1, 2}}"
        );
        assert_eq!(
            render_default(
                &serde_json::json!({"x": {"3": 9}}),
                &map(MapKeyType::String, map(MapKeyType::Int, FieldType::Int32)),
                &enums
            )
            .unwrap(),
            "map[string]map[int64]int32{\"x\": map[int64]int32{3: 9}}"
        );
        // Kind mismatch → None (a map default must be an object).
        assert!(render_default(
            &serde_json::json!([1]),
            &map(MapKeyType::String, FieldType::Int32),
            &enums
        )
        .is_none());
        // One unrenderable entry drops the whole default (array rule).
        assert!(render_default(
            &serde_json::json!({"a": {}}),
            &map(
                MapKeyType::String,
                FieldType::Object(indexmap::IndexMap::default())
            ),
            &enums
        )
        .is_none());
        // Enum values keep the shared no-enum-literals rule.
        assert!(render_default(
            &serde_json::json!({"x": 1}),
            &map(MapKeyType::String, FieldType::Enum("ItemKind".to_string())),
            &enums
        )
        .is_none());
        // Non-numeric or out-of-i64-range integer keys cannot render.
        assert!(render_default(
            &serde_json::json!({"x": 1}),
            &map(MapKeyType::Int, FieldType::Int32),
            &enums
        )
        .is_none());
        assert!(render_default(
            &serde_json::json!({"9223372036854775808": 1}),
            &map(MapKeyType::Int, FieldType::Int32),
            &enums
        )
        .is_none());
    }

    /// Empty-map helper matching `generate`'s enum type resolution.
    fn enum_type_map(schema: &Schema) -> HashMap<&str, String> {
        GoTargetGenerator::emitted_enums(schema)
            .into_iter()
            .map(|e| {
                let mut used = HashSet::new();
                let ty = unique_ident(go_ident(&e.name), &mut used);
                (e.name.as_str(), ty)
            })
            .collect()
    }

    #[test]
    fn test_enum_type_map_allocates_unique_idents() {
        let schema = test_schema();
        let map = enum_type_map(&schema);
        assert_eq!(map.get("ItemKind").map(String::as_str), Some("ItemKind"));
        assert_eq!(map.get("Rarity").map(String::as_str), Some("Rarity"));
        // Empty enums never resolve to a type.
        assert!(!map.contains_key("EmptyEnum"));
    }

    #[test]
    fn test_doc_finish_flushes_open_section() {
        let mut doc = Doc::new();
        doc.line(vec![
            Cell::new("a".to_string(), Term::Vtab),
            Cell::new("b".to_string(), Term::Nl),
        ]);
        // No terminating formfeed: finish() must still flush the open
        // section (a two-cell line pads column 0 to its block width + 1).
        assert_eq!(doc.finish(), "a b\n");
    }

    #[test]
    fn test_literal_break_terms_oversize_pairs_count_as_zero() {
        let member = |ident: &str, expr: &str| Member {
            field_name: ident.to_string(),
            ident: ident.to_string(),
            ty: "int32".to_string(),
            doc: None,
            default_expr: Some(expr.to_string()),
        };
        // A `Key: Value` pair beyond go/printer's 1 MB cutoff counts as key
        // size 0: it never joins the geometric-mean comparison (the false
        // side of `prev_size > 0 && size > 0`) and the break stays a section
        // split either way.
        let huge = member("k", &"x".repeat(1_000_001));
        let small = member("b", "1");
        let terms = literal_break_terms(&[&huge, &small]);
        assert!(terms == vec![Term::Ff, Term::Ff]);
    }

    #[test]
    fn test_frexp_edges() {
        // Zero and non-finite inputs pass through untouched.
        assert_eq!(frexp(0.0), (0.0, 0));
        let (f, e) = frexp(f64::NAN);
        assert!(f.is_nan() && e == 0);
        // Values below 0.5 scale up in the second loop; powers of two stay
        // exact through both loops.
        assert_eq!(frexp(0.25), (0.5, -1));
        assert_eq!(frexp(1.0), (0.5, 1));
        assert_eq!(frexp(6.0), (0.75, 3));
    }

    #[test]
    fn test_enum_value_literal_numeric_fallback_uses_name() {
        // Defensive: a value-less member inside a numeric enum renders its
        // name (generate feeds the numeric bucket only all-integral enums,
        // so this arm is otherwise unreachable).
        let v = cage_core::schema::EnumValue {
            name: "Only".to_string(),
            value: None,
            description: None,
        };
        assert_eq!(enum_value_literal(&v, "int64"), "Only");
        assert_eq!(enum_value_literal(&v, "string"), "\"Only\"");
    }

    /// Sample schema shared by the dev-only dump and the edge rendering
    /// test: every field kind (incl. Null/Any/Bytes/Object/Map), unsigned
    /// widths, string escapes, constraint docs, empty and no-primary-key
    /// tables, a single-field table, int64/uint64 and string-bucket enums,
    /// and a defaulted 44-char ident (the go/printer geometric-mean path
    /// that splits constructor alignment into sections).
    fn sample_schema() -> Schema {
        serde_yaml::from_str(
            r#"
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
      flag: { name: flag, type: { kind: Bool }, default: true }
      i8: { name: i8, type: { kind: Int8 }, required: true }
      i16: { name: i16, type: { kind: Int16 }, default: 1000 }
      u8: { name: u8, type: { kind: UInt8 }, default: 200 }
      u16: { name: u16, type: { kind: UInt16 }, default: 65535 }
      u32: { name: u32, type: { kind: UInt32 }, required: true }
      u64: { name: u64, type: { kind: UInt64 }, default: 42 }
      f32: { name: f32, type: { kind: Float32 }, default: 1.5 }
      raw: { name: raw, type: { kind: Null } }
      anyx: { name: anyx, type: { kind: Any } }
      blob: { name: blob, type: { kind: Bytes } }
      meta: { name: meta, type: { kind: Object, value: {} } }
      drops: { name: drops, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } } }
      weights: { name: weights, type: { kind: Map, value: { key_type: int, value_type: { kind: Float32 } } }, default: {} }
      qty: { name: qty, type: { kind: Int32 }, required: true, max: 99, description: Stock }
      code: { name: code, type: { kind: String }, min_length: 1, max_length: 10, pattern: "^[a-z]+$" }
      desc: { name: desc, type: { kind: String }, default: "q\"r\\s\n\t\rz" }
      unreasonably_long_field_name_exceeding_forty_chars: { name: unreasonably_long_field_name_exceeding_forty_chars, type: { kind: Int32 }, default: 1 }
  Drop:
    name: Drop
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int64 }, required: true }
      item: { name: item, type: { kind: Enum, value: MissingEnum } }
  Empty:
    name: Empty
    primary_key: [id]
    fields: {}
  NoKey:
    name: NoKey
    primary_key: []
    fields: {}
  Solo:
    name: Solo
    primary_key: [id]
    fields:
      only: { name: only, type: { kind: Enum, value: SoloKind }, required: true }
      note: { name: note, type: { kind: String }, default: hi }
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
  SoloKind:
    name: SoloKind
    values:
      - { name: Only, value: 7, description: The one }
  BigKind:
    name: BigKind
    description: Values past the 32-bit range.
    values:
      - { name: Large, value: 3000000000 }
  HugeKind:
    name: HugeKind
    values:
      - { name: Top, value: 18446744073709551615 }
  MixKind:
    name: MixKind
    values:
      - { name: S, value: "x y" }
      - { name: N, value: 42 }
      - { name: T, value: true }
      - { name: F, value: false }
      - { name: M }
  EmptyEnum:
    name: EmptyEnum
    values: []
"#,
        )
        .expect("sample schema must parse")
    }

    fn write_artifacts(dir: &std::path::Path, artifacts: &[(String, Vec<u8>)]) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).expect("create sample dir");
        for (p, content) in artifacts {
            let name = std::path::Path::new(p)
                .file_name()
                .expect("artifact file name");
            std::fs::write(dir.join(name), content).expect("write sample artifact");
        }
    }

    #[test]
    fn test_sample_schema_rendering() {
        let schema = sample_schema();
        let artifacts = gen().generate(&schema, Some("abc123"));
        let paths: Vec<&str> = artifacts.iter().map(|(p, _)| p.as_str()).collect();
        // Tables in name order (Drop, Empty, Item, NoKey, Solo), shared
        // enums file last.
        assert_eq!(
            paths,
            vec![
                "build/go/Drop.go",
                "build/go/Empty.go",
                "build/go/Item.go",
                "build/go/NoKey.go",
                "build/go/Solo.go",
                "build/go/cage_enums.go",
            ]
        );

        // Every remaining field kind + optionality pointers; no math import.
        let item = String::from_utf8(artifacts[2].1.clone()).unwrap();
        assert!(!item.contains("import \"math\""));
        let lines = normalized(&item);
        for expected in [
            "Flag bool `json:\"flag\"`",
            "I8 int8 `json:\"i8\"`",
            "I16 int16 `json:\"i16\"`",
            "U8 uint8 `json:\"u8\"`",
            "U16 uint16 `json:\"u16\"`",
            "U32 uint32 `json:\"u32\"`",
            "U64 uint64 `json:\"u64\"`",
            "F32 float32 `json:\"f32\"`",
            "Raw any `json:\"raw\"`",
            "Anyx any `json:\"anyx\"`",
            "Blob []byte `json:\"blob\"`",
            "Meta map[string]any `json:\"meta\"`",
            "Drops map[string][]int32 `json:\"drops\"`",
            "Weights map[int64]float32 `json:\"weights\"`",
            "Qty int32 `json:\"qty\"`",
            "Code *string `json:\"code\"`",
        ] {
            assert!(lines.contains(&expected.to_string()), "missing: {expected}");
        }
        // Constraint docs: description+required+max, and the string
        // length/pattern trio.
        assert!(lines.contains(&"// Stock, required, max: 99".to_string()));
        assert!(lines.contains(&"// min_length: 1, max_length: 10, pattern: ^[a-z]+$".to_string()));
        // Constructor: bool default, all five short string escapes, and the
        // 44-char ident whose break splits the alignment sections.
        assert!(lines.contains(&"Flag: true,".to_string()));
        assert!(lines.contains(&"Desc: \"q\\\"r\\\\s\\n\\t\\rz\",".to_string()));
        assert!(lines.contains(&"UnreasonablyLongFieldNameExceedingFortyChars: 1,".to_string()));
        // Empty map default: the typed empty literal, built per call.
        assert!(lines.contains(&"Weights: map[int64]float32{},".to_string()));
        assert!(lines.contains(&"return Item{".to_string()));

        // Empty field set: gofmt's one-liner struct.
        let empty = String::from_utf8(artifacts[1].1.clone()).unwrap();
        let empty_lines = normalized(&empty);
        assert!(empty_lines.contains(&"// Empty — primary key: id".to_string()));
        assert!(empty_lines.contains(&"type Empty struct{}".to_string()));
        assert!(empty_lines.contains(&"return Empty{}".to_string()));

        // Empty primary key: the raw name is the whole banner head.
        let nokey = normalized(&String::from_utf8(artifacts[3].1.clone()).unwrap());
        assert!(nokey.contains(&"// NoKey".to_string()));
        assert!(nokey.contains(&"type NoKey struct{}".to_string()));

        // A table with exactly one defaulted field renders that constructor
        // entry as a single blank-separated line.
        let solo = normalized(&String::from_utf8(artifacts[4].1.clone()).unwrap());
        assert!(solo.contains(&"Only SoloKind `json:\"only\"`".to_string()));
        assert!(solo.contains(&"Note string `json:\"note\"`".to_string()));
        assert!(solo.contains(&"Note: \"hi\",".to_string()));

        // Enums: description banner, int64/uint64 backing, the single-spec
        // trailing comment, string-bucket values from String/Number/Bool
        // and the name fallback.
        let enums = normalized(&String::from_utf8(artifacts[5].1.clone()).unwrap());
        assert!(enums.contains(&"// BigKind — Values past the 32-bit range.".to_string()));
        assert!(enums.contains(&"type BigKind int64".to_string()));
        assert!(enums.contains(&"BigKindLarge BigKind = 3000000000".to_string()));
        assert!(enums.contains(&"type HugeKind uint64".to_string()));
        assert!(enums.contains(&"HugeKindTop HugeKind = 18446744073709551615".to_string()));
        assert!(enums.contains(&"SoloKindOnly SoloKind = 7 // The one".to_string()));
        assert!(enums.contains(&"MixKindS MixKind = \"x y\"".to_string()));
        assert!(enums.contains(&"MixKindN MixKind = \"42\"".to_string()));
        assert!(enums.contains(&"MixKindT MixKind = \"true\"".to_string()));
        assert!(enums.contains(&"MixKindF MixKind = \"false\"".to_string()));
        assert!(enums.contains(&"MixKindM MixKind = \"M\"".to_string()));
        assert!(!enums.contains(&"EmptyEnum".to_string()));
    }

    #[test]
    fn test_write_artifacts() {
        let dir = std::env::temp_dir().join(format!("cage-go-sample-{}", std::process::id()));
        write_artifacts(&dir, &gen().generate(&test_schema(), Some("abc123")));
        for name in ["Drop.go", "Item.go", "cage_enums.go"] {
            assert!(dir.join(name).is_file(), "missing {name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dev-only: dump the sample schema output under `/tmp/cage-go-sample`
    /// for a manual gofmt / go build / go vet run.
    #[test]
    #[ignore = "dev-only: dumps sample output for gofmt / go build / go vet checks"]
    fn dump_sample_output() {
        let artifacts = gen().generate(&sample_schema(), Some("abc123"));
        write_artifacts(std::path::Path::new("/tmp/cage-go-sample"), &artifacts);
    }
}
