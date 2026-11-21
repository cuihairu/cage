//! Schema refactorings: the safe rename face (P0-4).
//!
//! A rename is not a string edit. A table, field, or enum name appears in
//! several places beyond the thing itself — primary keys, unique
//! constraints, `order_by`, per-environment override keys, cross-table
//! `reference` targets, `Enum(name)` field types, and the field operands of
//! rule expressions (row asserts and reference predicates) — and each site
//! names it in its own context. This module is the one place that knows
//! every site, so the editor renames through it instead of rewriting its own
//! document keys (which left the entity's own `name` stale and every
//! reference pointing at nothing).
//!
//! [`preview`] and [`apply`] share a single implementation: the preview runs
//! that implementation against a clone, so the report an author reads can
//! never drift from what the apply does. [`find_references`] is the
//! read-only enumeration of the same sites, and a unit test pins the two to
//! the same list so a future site type cannot be added to one and forgotten
//! in the other.

use crate::schema::{
    ExpressionRule, FieldSchema, FieldType, MapField, ReferenceSchema, Schema, TaggedUnion,
};
use crate::validation::expr::{parse_assert, CmpOp, Comparison, Operand};
use crate::value::Value;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// What a rename addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RenameTarget {
    /// Rename a table (its map key, its `name`, and every cross-table
    /// `reference.table` that names it).
    Table {
        /// The table's current name.
        name: String,
    },
    /// Rename one field of one table (its map key, its `name`, primary-key
    /// entries, unique-constraint members, `order_by` entries,
    /// per-environment override keys, every `reference.field` that names it,
    /// and every rule-assert operand that names it).
    Field {
        /// The owning table.
        table: String,
        /// The field's current name.
        field: String,
    },
    /// Rename a shared enum (its map key, its `name`, and every
    /// `Enum(name)` field type that names it).
    Enum {
        /// The enum's current name.
        name: String,
    },
}

impl RenameTarget {
    /// `table Monster` / `field Monster.hp` / `enum Tier` — the form the
    /// report and the error messages use.
    pub fn describe(&self) -> String {
        match self {
            Self::Table { name } => format!("table '{name}'"),
            Self::Field { table, field } => format!("field '{field}' of table '{table}'"),
            Self::Enum { name } => format!("enum '{name}'"),
        }
    }

    fn current_name(&self) -> &str {
        match self {
            Self::Table { name } | Self::Enum { name } => name,
            Self::Field { field, .. } => field,
        }
    }
}

/// One site the rename edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefactorChange {
    /// JSON path of the site, e.g. `tables.Monster.fields.hp` or
    /// `tables.Monster.primary_key[0]` — the anchor the editor highlights.
    pub path: String,
    /// What the site is: a table/field/enum key, a reference target, a
    /// primary-key entry, a rule-assert operand, ...
    pub kind: String,
    /// The name currently written at the site.
    pub before: String,
    /// The name it becomes.
    pub after: String,
}

/// One place in the schema that names the target, read-only — the list the
/// editor shows before the author confirms a rename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceSite {
    /// JSON path of the site.
    pub path: String,
    /// What the site is (same `kind` vocabulary as [`RefactorChange`]).
    pub kind: String,
    /// The name written at the site.
    pub name: String,
}

/// What a rename did (or would do).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefactorReport {
    /// The site edits, in document order.
    pub changes: Vec<RefactorChange>,
    /// Sites the rename could not touch — e.g. a rule assert it could not
    /// parse, left byte-for-byte as written — recorded so a preview never
    /// silently disagrees with the apply.
    pub notes: Vec<String>,
}

impl RefactorReport {
    /// The new name appears at this many sites (the entity itself plus every
    /// reference).
    pub fn site_count(&self) -> usize {
        self.changes.len()
    }
}

/// Preview the rename against a clone: returns the same report [`apply`]
/// produces, without touching `schema`.
pub fn preview(
    schema: &Schema,
    target: &RenameTarget,
    new_name: &str,
) -> Result<RefactorReport, String> {
    let mut draft = schema.clone();
    apply(&mut draft, target, new_name)
}

/// Apply the rename to `schema`, reporting every site it edited.
///
/// The new name must be a bare identifier (`[A-Za-z_][A-Za-z0-9_]*`) — the
/// shape every downstream consumer (codegen identifiers, assert operands,
/// JSON paths) can carry — and must not already name a sibling of the same
/// kind. Anything else is `Err` with the reason; the schema is left
/// untouched.
pub fn apply(
    schema: &mut Schema,
    target: &RenameTarget,
    new_name: &str,
) -> Result<RefactorReport, String> {
    if !is_bare_ident(new_name) {
        return Err(format!(
            "new name '{new_name}' is not a bare identifier (letter or '_' first, then letters, \
             digits or '_')"
        ));
    }
    if new_name == target.current_name() {
        return Err(format!(
            "{} is already named '{new_name}'",
            target.describe()
        ));
    }
    match target {
        RenameTarget::Table { name } => rename_table(schema, name, new_name),
        RenameTarget::Field { table, field } => rename_field(schema, table, field, new_name),
        RenameTarget::Enum { name } => rename_enum(schema, name, new_name),
    }
}

/// Rename a table: its map key, its `name`, and every cross-table
/// `reference.table` that names it.
pub fn rename_table(schema: &mut Schema, old: &str, new: &str) -> Result<RefactorReport, String> {
    if !schema.tables.contains_key(old) {
        return Err(format!("no such table '{old}'"));
    }
    if schema.tables.contains_key(new) {
        return Err(format!("table '{new}' already exists"));
    }
    let mut report = RefactorReport::default();
    rename_map_key(
        &mut schema.tables,
        old,
        new,
        &mut report,
        "tables",
        "table_key",
    );
    // The table's own `name` member — the key and the member must agree
    // (schema insertion keys by `name`).
    let table = schema.tables.get_mut(new).expect("renamed key exists");
    table.name = new.to_string();
    report.changes.push(RefactorChange {
        path: format!("tables.{new}.name"),
        kind: "table_name".to_string(),
        before: old.to_string(),
        after: new.to_string(),
    });
    // Every reference into the table, from any table (its own included).
    for (owner, t) in &mut schema.tables {
        for (fname, field) in &mut t.fields {
            if let Some(reference) = field.reference.as_mut() {
                if reference.table == old {
                    report.changes.push(RefactorChange {
                        path: format!("tables.{owner}.fields.{fname}.reference.table"),
                        kind: "reference_table".to_string(),
                        before: reference.table.clone(),
                        after: new.to_string(),
                    });
                    reference.table = new.to_string();
                }
            }
        }
    }
    Ok(report)
}

/// Rename a field: its map key, its `name`, the primary-key entries,
/// unique-constraint members, `order_by` entries, per-environment override
/// keys, every `reference.field` that names it (from any table), every
/// row-assert operand in the owning table, and every reference predicate
/// that filters rows of the owning table.
pub fn rename_field(
    schema: &mut Schema,
    table_name: &str,
    old: &str,
    new: &str,
) -> Result<RefactorReport, String> {
    if !schema.tables.contains_key(table_name) {
        return Err(format!("no such table '{table_name}'"));
    }
    if !schema.tables[table_name].fields.contains_key(old) {
        return Err(format!("no such field '{old}' in table '{table_name}'"));
    }
    if schema.tables[table_name].fields.contains_key(new) {
        return Err(format!(
            "field '{new}' already exists in table '{table_name}'"
        ));
    }
    let mut report = RefactorReport::default();

    // ── Sites inside the owning table (borrow it once, then drop it) ──
    let table = schema.tables.get_mut(table_name).expect("table exists");
    rename_map_key(
        &mut table.fields,
        old,
        new,
        &mut report,
        &format!("tables.{table_name}.fields"),
        "field_key",
    );
    let field = table.fields.get_mut(new).expect("renamed field key exists");
    field.name = new.to_string();
    report.changes.push(RefactorChange {
        path: format!("tables.{table_name}.fields.{new}.name"),
        kind: "field_name".to_string(),
        before: old.to_string(),
        after: new.to_string(),
    });
    for (i, pk) in table.primary_key.iter_mut().enumerate() {
        if pk == old {
            report.changes.push(RefactorChange {
                path: format!("tables.{table_name}.primary_key[{i}]"),
                kind: "primary_key".to_string(),
                before: pk.clone(),
                after: new.to_string(),
            });
            *pk = new.to_string();
        }
    }
    for (i, constraint) in table.unique_constraints.iter_mut().enumerate() {
        for (j, member) in constraint.fields.iter_mut().enumerate() {
            if member == old {
                report.changes.push(RefactorChange {
                    path: format!("tables.{table_name}.unique_constraints[{i}].fields[{j}]"),
                    kind: "unique_constraint".to_string(),
                    before: member.clone(),
                    after: new.to_string(),
                });
                *member = new.to_string();
            }
        }
    }
    if let Some(order_by) = table.order_by.as_mut() {
        for (i, member) in order_by.iter_mut().enumerate() {
            if member == old {
                report.changes.push(RefactorChange {
                    path: format!("tables.{table_name}.order_by[{i}]"),
                    kind: "order_by".to_string(),
                    before: member.clone(),
                    after: new.to_string(),
                });
                *member = new.to_string();
            }
        }
    }
    for (env, overrides) in &mut table.env_overrides {
        rename_map_key(
            overrides,
            old,
            new,
            &mut report,
            &format!("tables.{table_name}.env_overrides.{env}"),
            "env_override",
        );
    }
    // Row asserts of the owning table: the operand names a field of the row
    // under evaluation, which for this table's rules is this field.
    for (fname, f) in &mut table.fields {
        let fpath = format!("tables.{table_name}.fields.{fname}");
        rewrite_row_rules(f, old, new, &fpath, &mut report);
    }
    // The `table` borrow ends here; the next pass re-borrows `schema.tables`.

    // ── Sites outside the owning table: reference targets into it, and the
    //    predicates that filter rows of it. The owning table's own
    //    references are walked too (self-references), so this pass is the
    //    single owner of `reference.field` / `reference.predicate`. ──
    for (owner, t) in &mut schema.tables {
        for (fname, field) in &mut t.fields {
            if let Some(reference) = field.reference.as_mut() {
                if reference.table == table_name {
                    rewrite_reference_field(reference, old, new, owner, fname, &mut report);
                }
            }
        }
    }
    Ok(report)
}

/// Rename a shared enum: its map key, its `name`, and every
/// `Enum(name)` field type that names it.
pub fn rename_enum(schema: &mut Schema, old: &str, new: &str) -> Result<RefactorReport, String> {
    if !schema.enums.contains_key(old) {
        return Err(format!("no such enum '{old}'"));
    }
    if schema.enums.contains_key(new) {
        return Err(format!("enum '{new}' already exists"));
    }
    let mut report = RefactorReport::default();
    rename_map_key(
        &mut schema.enums,
        old,
        new,
        &mut report,
        "enums",
        "enum_key",
    );
    let e = schema.enums.get_mut(new).expect("renamed enum key exists");
    e.name = new.to_string();
    report.changes.push(RefactorChange {
        path: format!("enums.{new}.name"),
        kind: "enum_name".to_string(),
        before: old.to_string(),
        after: new.to_string(),
    });
    for (owner, t) in &mut schema.tables {
        for (fname, field) in &mut t.fields {
            let fpath = format!("tables.{owner}.fields.{fname}.type");
            rewrite_field_type_enum(&mut field.field_type, old, new, &fpath, &mut report);
        }
    }
    Ok(report)
}

/// Every site in `schema` that names `target`, read-only — in document
/// order, with the same path/kind vocabulary the rename edits. A unit test
/// pins this list to the `changes` of [`apply`].
pub fn find_references(schema: &Schema, target: &RenameTarget) -> Vec<ReferenceSite> {
    let mut sites = Vec::new();
    match target {
        RenameTarget::Table { name } => {
            if schema.tables.contains_key(name) {
                sites.push(site(format!("tables.{name}"), "table_key", name));
                sites.push(site(format!("tables.{name}.name"), "table_name", name));
            }
            for (owner, t) in &schema.tables {
                for (fname, field) in &t.fields {
                    if let Some(reference) = &field.reference {
                        if &reference.table == name {
                            sites.push(site(
                                format!("tables.{owner}.fields.{fname}.reference.table"),
                                "reference_table",
                                &reference.table,
                            ));
                        }
                    }
                }
            }
        }
        RenameTarget::Field { table, field } => {
            let Some(t) = schema.tables.get(table) else {
                return sites;
            };
            if t.fields.contains_key(field) {
                sites.push(site(
                    format!("tables.{table}.fields.{field}"),
                    "field_key",
                    field,
                ));
                sites.push(site(
                    format!("tables.{table}.fields.{field}.name"),
                    "field_name",
                    field,
                ));
            }
            for (i, pk) in t.primary_key.iter().enumerate() {
                if pk == field {
                    sites.push(site(
                        format!("tables.{table}.primary_key[{i}]"),
                        "primary_key",
                        pk,
                    ));
                }
            }
            for (i, constraint) in t.unique_constraints.iter().enumerate() {
                for (j, member) in constraint.fields.iter().enumerate() {
                    if member == field {
                        sites.push(site(
                            format!("tables.{table}.unique_constraints[{i}].fields[{j}]"),
                            "unique_constraint",
                            member,
                        ));
                    }
                }
            }
            if let Some(order_by) = &t.order_by {
                for (i, member) in order_by.iter().enumerate() {
                    if member == field {
                        sites.push(site(
                            format!("tables.{table}.order_by[{i}]"),
                            "order_by",
                            member,
                        ));
                    }
                }
            }
            for (env, overrides) in &t.env_overrides {
                for (f, _) in overrides {
                    if f == field {
                        sites.push(site(
                            format!("tables.{table}.env_overrides.{env}.{f}"),
                            "env_override",
                            f,
                        ));
                    }
                }
            }
            for (fname, f) in &t.fields {
                let fpath = format!("tables.{table}.fields.{fname}");
                collect_row_rule_sites(f, field, &fpath, &mut sites);
            }
            for (owner, ot) in &schema.tables {
                for (fname, f) in &ot.fields {
                    if let Some(reference) = &f.reference {
                        if &reference.table == table {
                            if &reference.field == field {
                                sites.push(site(
                                    format!("tables.{owner}.fields.{fname}.reference.field"),
                                    "reference_field",
                                    &reference.field,
                                ));
                            }
                            collect_rule_sites(
                                reference.predicate.as_ref(),
                                field,
                                &format!(
                                    "tables.{owner}.fields.{fname}.reference.predicate.assert"
                                ),
                                &mut sites,
                            );
                        }
                    }
                }
            }
        }
        RenameTarget::Enum { name } => {
            if schema.enums.contains_key(name) {
                sites.push(site(format!("enums.{name}"), "enum_key", name));
            }
            for (owner, t) in &schema.tables {
                for (fname, field) in &t.fields {
                    let fpath = format!("tables.{owner}.fields.{fname}.type");
                    collect_field_type_enum(&field.field_type, name, &fpath, &mut sites);
                }
            }
        }
    }
    sites
}

fn site(path: String, kind: &str, name: &str) -> ReferenceSite {
    ReferenceSite {
        path,
        kind: kind.to_string(),
        name: name.to_string(),
    }
}

/// Rename an `IndexMap` key in place, preserving its position — the editor
/// contract keeps document order, and canonical YAML is order-sensitive.
fn rename_map_key<V>(
    map: &mut IndexMap<String, V>,
    old: &str,
    new: &str,
    report: &mut RefactorReport,
    prefix: &str,
    kind: &str,
) {
    let index = map.get_index_of(old).expect("caller checked key exists");
    // `shift_remove` (not `remove`/`swap_remove`): the map keeps document
    // order, so the following keys only shift down by one and the reinsert
    // lands back in the vacated slot.
    let value = map.shift_remove(old).expect("key exists");
    report.changes.push(RefactorChange {
        path: format!("{prefix}.{old}"),
        kind: kind.to_string(),
        before: old.to_string(),
        after: new.to_string(),
    });
    map.shift_insert(index, new.to_string(), value);
}

/// Rewrite every row-assert rule in `field` (recursively: nested
/// `items`/`properties` fields carry their own rules, all of which evaluate
/// against the outer row). The path is the field path plus the rule's index
/// in its own array — already unique without threading a counter.
fn rewrite_row_rules(
    field: &mut FieldSchema,
    from: &str,
    to: &str,
    fpath: &str,
    report: &mut RefactorReport,
) {
    for (i, rule) in field.rules.iter_mut().enumerate() {
        rewrite_rule(
            rule,
            from,
            to,
            &format!("{fpath}.rules[{i}].assert"),
            report,
        );
    }
    if let Some(items) = field.items.as_mut() {
        rewrite_row_rules(items, from, to, &format!("{fpath}.items"), report);
    }
    if let Some(properties) = field.properties.as_mut() {
        for (pname, p) in properties.iter_mut() {
            rewrite_row_rules(p, from, to, &format!("{fpath}.properties.{pname}"), report);
        }
    }
}

/// Rewrite one rule's assert: swap the `from` field operand for `to` and
/// re-render. An assert that names neither side stays untouched; an assert
/// that does not parse at all is left byte-for-byte (the L6 stage reports
/// the original parse defect) and recorded as a note.
fn rewrite_rule(
    rule: &mut ExpressionRule,
    from: &str,
    to: &str,
    path: &str,
    report: &mut RefactorReport,
) {
    let Ok(cmp) = parse_assert(&rule.assert) else {
        report.notes.push(format!(
            "rule '{}' at {path} could not be parsed; left unchanged",
            rule.name
        ));
        return;
    };
    let lhs = rename_operand(&cmp.lhs, from, to);
    let rhs = rename_operand(&cmp.rhs, from, to);
    if !lhs.1 && !rhs.1 {
        return;
    }
    report.changes.push(RefactorChange {
        path: path.to_string(),
        kind: "rule_assert".to_string(),
        before: from.to_string(),
        after: to.to_string(),
    });
    rule.assert = render_comparison(&Comparison {
        lhs: lhs.0,
        op: cmp.op,
        rhs: rhs.0,
    });
}

/// Rewrite a `reference.field` and, when the reference targets the renamed
/// field, its predicate (which filters rows of the referenced table, so its
/// operands name that table's fields).
fn rewrite_reference_field(
    reference: &mut ReferenceSchema,
    from: &str,
    to: &str,
    owner: &str,
    fname: &str,
    report: &mut RefactorReport,
) {
    if reference.field == from {
        report.changes.push(RefactorChange {
            path: format!("tables.{owner}.fields.{fname}.reference.field"),
            kind: "reference_field".to_string(),
            before: reference.field.clone(),
            after: to.to_string(),
        });
        reference.field = to.to_string();
    }
    if let Some(predicate) = reference.predicate.as_mut() {
        rewrite_rule(
            predicate,
            from,
            to,
            &format!("tables.{owner}.fields.{fname}.reference.predicate.assert"),
            report,
        );
    }
}

/// Rewrite an `Enum(name)` field type anywhere in the type tree.
fn rewrite_field_type_enum(
    field_type: &mut FieldType,
    from: &str,
    to: &str,
    path: &str,
    report: &mut RefactorReport,
) {
    match field_type {
        FieldType::Enum(name) if name == from => {
            report.changes.push(RefactorChange {
                path: path.to_string(),
                kind: "enum_ref".to_string(),
                before: name.clone(),
                after: to.to_string(),
            });
            *name = to.to_string();
        }
        FieldType::Enum(_)
        | FieldType::Any
        | FieldType::Null
        | FieldType::Bool
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
        | FieldType::Bytes => {}
        FieldType::Array(inner) => {
            rewrite_field_type_enum(inner, from, to, &format!("{path}.value"), report);
        }
        FieldType::Object(props) => {
            for (pname, p) in props.iter_mut() {
                rewrite_field_type_enum(p, from, to, &format!("{path}.value.{pname}"), report);
            }
        }
        FieldType::Map(MapField { value_type, .. }) => {
            rewrite_field_type_enum(value_type, from, to, &format!("{path}.value"), report);
        }
        FieldType::TaggedUnion(inner) => {
            let TaggedUnion { branches, .. } = inner.as_mut();
            for (branch, fields) in branches.iter_mut() {
                for (bname, b) in fields.iter_mut() {
                    rewrite_field_type_enum(
                        b,
                        from,
                        to,
                        &format!("{path}.value.{branch}.{bname}"),
                        report,
                    );
                }
            }
        }
    }
}

/// Collect every `Enum(from)` site in a field type tree.
fn collect_field_type_enum(
    field_type: &FieldType,
    from: &str,
    path: &str,
    sites: &mut Vec<ReferenceSite>,
) {
    match field_type {
        FieldType::Enum(name) if name == from => {
            sites.push(site(path.to_string(), "enum_ref", name));
        }
        FieldType::Enum(_)
        | FieldType::Any
        | FieldType::Null
        | FieldType::Bool
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
        | FieldType::Bytes => {}
        FieldType::Array(inner) => {
            collect_field_type_enum(inner, from, &format!("{path}.value"), sites);
        }
        FieldType::Object(props) => {
            for (pname, p) in props {
                collect_field_type_enum(p, from, &format!("{path}.value.{pname}"), sites);
            }
        }
        FieldType::Map(MapField { value_type, .. }) => {
            collect_field_type_enum(value_type, from, &format!("{path}.value"), sites);
        }
        FieldType::TaggedUnion(inner) => {
            let TaggedUnion { branches, .. } = inner.as_ref();
            for (branch, fields) in branches {
                for (bname, b) in fields {
                    collect_field_type_enum(
                        b,
                        from,
                        &format!("{path}.value.{branch}.{bname}"),
                        sites,
                    );
                }
            }
        }
    }
}

/// Collect every row-assert site naming `from` in a field (recursively).
fn collect_row_rule_sites(
    field: &FieldSchema,
    from: &str,
    fpath: &str,
    sites: &mut Vec<ReferenceSite>,
) {
    for (i, rule) in field.rules.iter().enumerate() {
        collect_rule_sites(
            Some(rule),
            from,
            &format!("{fpath}.rules[{i}].assert"),
            sites,
        );
    }
    if let Some(items) = &field.items {
        collect_row_rule_sites(items, from, &format!("{fpath}.items"), sites);
    }
    if let Some(properties) = &field.properties {
        for (pname, p) in properties {
            collect_row_rule_sites(p, from, &format!("{fpath}.properties.{pname}"), sites);
        }
    }
}

/// Collect the sites of one rule's assert naming `from`.
fn collect_rule_sites(
    rule: Option<&ExpressionRule>,
    from: &str,
    path: &str,
    sites: &mut Vec<ReferenceSite>,
) {
    let Some(rule) = rule else {
        return;
    };
    let Ok(cmp) = parse_assert(&rule.assert) else {
        return;
    };
    for name in cmp.field_refs() {
        if name == from {
            sites.push(site(path.to_string(), "rule_assert", name));
        }
    }
}

/// Rename the operand if it is a `from` field reference; returns the operand
/// and whether it changed.
fn rename_operand(operand: &Operand, from: &str, to: &str) -> (Operand, bool) {
    match operand {
        Operand::Field(name) if name == from => (Operand::Field(to.to_string()), true),
        _ => (operand.clone(), false),
    }
}

/// Render a comparison back to assert text, re-quoting any field name the
/// lexer could not re-read bare (punctuation, spaces, `true`/`false`).
fn render_comparison(cmp: &Comparison) -> String {
    format!(
        "{} {} {}",
        render_operand(&cmp.lhs),
        op_symbol(cmp.op),
        render_operand(&cmp.rhs)
    )
}

fn op_symbol(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
    }
}

fn render_operand(operand: &Operand) -> String {
    match operand {
        Operand::Field(name) => {
            if is_bare_ident(name) && name != "true" && name != "false" {
                name.clone()
            } else {
                format!("`{name}`")
            }
        }
        Operand::Lit(value) => match value {
            Value::Bool(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => {
                if f.is_finite() && f.fract() == 0.0 {
                    format!("{f}.0")
                } else {
                    format!("{f}")
                }
            }
            Value::String(s) => format!("'{s}'"),
            other => other.type_name().to_string(),
        },
    }
}

/// A bare identifier as the assert lexer and the convention layer define it:
/// letter or `_` first, then letters, digits, or `_`.
fn is_bare_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::from_editor_json;

    fn schema_of(doc: &str) -> Schema {
        from_editor_json(doc).expect("test schema parses")
    }

    /// Two-table schema with the field-rename surface spread across every
    /// site that names a field: primary key, unique constraint, `order_by`,
    /// an environment override, a row assert, a shared enum reference, and a
    /// cross-table reference with a predicate.
    const FIELD_FIXTURE: &str = r#"{
      "tables": {
        "Item": {
          "name": "Item",
          "primary_key": ["id", "tier"],
          "unique_constraints": [{"name": "u1", "fields": ["tier"]}],
          "order_by": ["tier"],
          "env_overrides": {"client": {"tier": {"min": 1}}},
          "fields": {
            "id": {"name": "id", "type": {"kind": "Int32"}},
            "tier": {"name": "tier", "type": {"kind": "Enum", "value": "Tier"}},
            "min_level": {
              "name": "min_level",
              "type": {"kind": "Int32"},
              "rules": [{"name": "r1", "assert": "tier >= 1"},
                        {"name": "r2", "assert": "min_level <= max_level"}]
            },
            "max_level": {"name": "max_level", "type": {"kind": "Int32"}}
          }
        },
        "Monster": {
          "name": "Monster",
          "primary_key": ["id"],
          "fields": {
            "id": {"name": "id", "type": {"kind": "Int32"}},
            "item_id": {
              "name": "item_id",
              "type": {"kind": "Int32"},
              "reference": {
                "table": "Item",
                "field": "tier",
                "predicate": {"name": "p", "assert": "tier > 1"}
              }
            }
          }
        }
      },
      "enums": {"Tier": {"name": "Tier", "values": [{"name": "Low", "literal": "low"}]}}
    }"#;

    #[test]
    fn renames_a_table_key_name_and_every_cross_table_reference() {
        let mut schema = schema_of(FIELD_FIXTURE);
        let report = apply(
            &mut schema,
            &RenameTarget::Table {
                name: "Item".into(),
            },
            "Reward",
        )
        .expect("rename succeeds");
        assert_eq!(
            schema.tables.get_index_of("Reward"),
            Some(0),
            "key renamed in place"
        );
        assert!(!schema.tables.contains_key("Item"));
        assert_eq!(schema.tables["Reward"].name, "Reward");
        assert_eq!(
            schema.tables["Monster"].fields["item_id"]
                .reference
                .as_ref()
                .unwrap()
                .table,
            "Reward"
        );
        assert_eq!(report.changes.len(), 3, "{report:?}");
        assert_eq!(report.notes, Vec::<String>::new());
    }

    #[test]
    fn renames_a_field_across_every_site_that_names_it() {
        let mut schema = schema_of(FIELD_FIXTURE);
        let report = apply(
            &mut schema,
            &RenameTarget::Field {
                table: "Item".into(),
                field: "tier".into(),
            },
            "rarity",
        )
        .expect("rename succeeds");

        let item = &schema.tables["Item"];
        assert_eq!(
            item.fields.get_index_of("rarity"),
            Some(1),
            "key renamed in place"
        );
        assert!(!item.fields.contains_key("tier"));
        assert_eq!(item.fields["rarity"].name, "rarity");
        assert_eq!(item.primary_key, vec!["id", "rarity"]);
        assert_eq!(item.unique_constraints[0].fields, vec!["rarity"]);
        assert_eq!(item.order_by, Some(vec!["rarity".to_string()]));
        assert!(item.env_overrides["client"].contains_key("rarity"));
        assert!(!item.env_overrides["client"].contains_key("tier"));

        // The cross-table reference and its row-filtering predicate.
        let reference = schema.tables["Monster"].fields["item_id"]
            .reference
            .as_ref()
            .unwrap();
        assert_eq!(reference.field, "rarity");
        assert_eq!(reference.predicate.as_ref().unwrap().assert, "rarity > 1");

        // The row assert that names the field; the sibling rule that names a
        // different field stays byte-identical.
        let rules = &item.fields["min_level"].rules;
        assert_eq!(rules[0].assert, "rarity >= 1");
        assert_eq!(rules[1].assert, "min_level <= max_level");

        assert_eq!(report.notes, Vec::<String>::new(), "{report:?}");
    }

    #[test]
    fn renames_an_enum_everywhere_it_is_referenced_including_nested_types() {
        let mut schema = schema_of(
            r#"{
          "tables": {
            "Item": {
              "name": "Item",
              "primary_key": ["id"],
              "fields": {
                "id": {"name": "id", "type": {"kind": "Int32"}},
                "tier": {"name": "tier", "type": {"kind": "Enum", "value": "Tier"}},
                "drop": {"name": "drop", "type": {"kind": "Array",
                        "value": {"kind": "Enum", "value": "Tier"}}},
                "box": {"name": "box", "type": {"kind": "Object",
                        "value": {"t": {"kind": "Enum", "value": "Tier"}}}}
              }
            }
          },
          "enums": {"Tier": {"name": "Tier", "values": []}}
        }"#,
        );
        apply(
            &mut schema,
            &RenameTarget::Enum {
                name: "Tier".into(),
            },
            "Rarity",
        )
        .expect("rename succeeds");
        assert_eq!(schema.enums.get_index_of("Rarity"), Some(0));
        assert_eq!(schema.enums["Rarity"].name, "Rarity");
        let fields = &schema.tables["Item"].fields;
        assert_eq!(fields["tier"].field_type, FieldType::Enum("Rarity".into()));
        assert_eq!(
            fields["drop"].field_type,
            FieldType::Array(Box::new(FieldType::Enum("Rarity".into())))
        );
        assert_eq!(
            fields["box"].field_type,
            FieldType::Object(indexmap::IndexMap::from([(
                "t".to_string(),
                FieldType::Enum("Rarity".into()),
            )]))
        );
    }

    #[test]
    fn find_references_lists_exactly_the_sites_the_rename_edits() {
        let schema = schema_of(FIELD_FIXTURE);
        let target = RenameTarget::Field {
            table: "Item".into(),
            field: "tier".into(),
        };
        let sites = find_references(&schema, &target);

        let mut draft = schema.clone();
        let report = apply(&mut draft, &target, "rarity").expect("rename succeeds");

        // Paths agree up to the rename itself (the new name is what the
        // report writes at the key and the name sites).
        let site_paths: Vec<String> = sites.iter().map(|s| s.path.clone()).collect();
        let change_paths: Vec<String> = report
            .changes
            .iter()
            .map(|c| c.path.replace("rarity", "tier"))
            .collect();
        assert_eq!(site_paths, change_paths);

        let site_kinds: Vec<(String, String)> = sites
            .iter()
            .map(|s| (s.kind.clone(), s.name.clone()))
            .collect();
        let change_kinds: Vec<(String, String)> = report
            .changes
            .iter()
            .map(|c| (c.kind.clone(), c.before.clone()))
            .collect();
        assert_eq!(site_kinds, change_kinds);
        for change in &report.changes {
            assert_eq!(change.after, "rarity");
        }
    }

    #[test]
    fn preview_reports_what_apply_edits_without_touching_the_schema() {
        let schema = schema_of(FIELD_FIXTURE);
        let target = RenameTarget::Field {
            table: "Item".into(),
            field: "tier".into(),
        };
        let report = preview(&schema, &target, "rarity").expect("preview succeeds");
        assert!(report.site_count() > 0, "{report:?}");
        // Nothing was written into `schema`.
        assert!(schema.tables["Item"].fields.contains_key("tier"));

        let mut draft = schema.clone();
        assert_eq!(
            report,
            apply(&mut draft, &target, "rarity").expect("rename succeeds")
        );
    }

    #[test]
    fn rename_keeps_document_order() {
        let mut schema = schema_of(
            r#"{
          "tables": {
            "Alpha": {"name": "Alpha", "primary_key": ["id"],
              "fields": {"id": {"name": "id", "type": {"kind": "Int32"}}}},
            "Beta": {"name": "Beta", "primary_key": ["id"],
              "fields": {"id": {"name": "id", "type": {"kind": "Int32"}}}},
            "Gamma": {"name": "Gamma", "primary_key": ["id"],
              "fields": {"id": {"name": "id", "type": {"kind": "Int32"}}}}
          },
          "enums": {}
        }"#,
        );
        apply(
            &mut schema,
            &RenameTarget::Table {
                name: "Gamma".into(),
            },
            "Delta",
        )
        .expect("rename succeeds");
        let keys: Vec<&String> = schema.tables.keys().collect();
        assert_eq!(keys, vec!["Alpha", "Beta", "Delta"]);
    }

    #[test]
    fn rejects_collisions_ghost_targets_and_non_identifier_names() {
        let mut schema = schema_of(FIELD_FIXTURE);
        let collide = apply(
            &mut schema,
            &RenameTarget::Table {
                name: "Item".into(),
            },
            "Monster",
        );
        assert!(collide.unwrap_err().contains("already exists"));
        let bad = apply(
            &mut schema,
            &RenameTarget::Table {
                name: "Item".into(),
            },
            "1bad",
        );
        assert!(bad.unwrap_err().contains("not a bare identifier"));
        let ghost = apply(
            &mut schema,
            &RenameTarget::Table {
                name: "Ghost".into(),
            },
            "Item",
        );
        assert!(ghost.unwrap_err().contains("no such table"));
        let same = apply(
            &mut schema,
            &RenameTarget::Enum {
                name: "Tier".into(),
            },
            "Tier",
        );
        assert!(same.unwrap_err().contains("already named"));
        // None of them landed.
        assert_eq!(schema.tables.get_index_of("Item"), Some(0));
        assert_eq!(schema.enums.get_index_of("Tier"), Some(0));
        assert_eq!(schema.tables["Item"].fields.len(), 4);
    }

    #[test]
    fn records_a_note_when_an_assert_will_not_parse() {
        let mut schema = schema_of(
            r#"{
          "tables": {
            "Item": {
              "name": "Item",
              "primary_key": ["id"],
              "fields": {
                "id": {"name": "id", "type": {"kind": "Int32"}},
                "tier": {"name": "tier", "type": {"kind": "Int32"},
                         "rules": [{"name": "r", "assert": "tier tier"}]}
              }
            }
          },
          "enums": {}
        }"#,
        );
        let report = apply(
            &mut schema,
            &RenameTarget::Field {
                table: "Item".into(),
                field: "tier".into(),
            },
            "rarity",
        )
        .expect("rename succeeds");
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("could not be parsed")),
            "{report:?}"
        );
        assert_eq!(
            schema.tables["Item"].fields["rarity"].rules[0].assert, "tier tier",
            "an unparseable assert is left byte-identical"
        );
    }

    #[test]
    fn renames_a_field_quoted_by_backticks_when_the_new_name_is_not_bare() {
        let mut schema = schema_of(
            r#"{
          "tables": {
            "Item": {
              "name": "Item",
              "primary_key": ["id"],
              "fields": {
                "id": {"name": "id", "type": {"kind": "Int32"}},
                "a_b": {"name": "a_b", "type": {"kind": "Int32"},
                        "rules": [{"name": "r", "assert": "a_b >= 1"}]}
              }
            }
          },
          "enums": {}
        }"#,
        );
        // `1x` is not a bare identifier, so apply rejects it outright.
        assert!(apply(
            &mut schema,
            &RenameTarget::Field {
                table: "Item".into(),
                field: "a_b".into()
            },
            "1x"
        )
        .is_err());
        apply(
            &mut schema,
            &RenameTarget::Field {
                table: "Item".into(),
                field: "a_b".into(),
            },
            "renamed",
        )
        .expect("rename succeeds");
        assert_eq!(
            schema.tables["Item"].fields["renamed"].rules[0].assert,
            "renamed >= 1"
        );
        // An already-backticked operand stays resolvable through the rename.
        let mut schema = schema_of(
            r#"{
          "tables": {
            "Item": {
              "name": "Item",
              "primary_key": ["id"],
              "fields": {
                "id": {"name": "id", "type": {"kind": "Int32"}},
                "min-max": {"name": "min-max", "type": {"kind": "Int32"},
                            "rules": [{"name": "r", "assert": "`min-max` <= 9"}]}
              }
            }
          },
          "enums": {}
        }"#,
        );
        apply(
            &mut schema,
            &RenameTarget::Field {
                table: "Item".into(),
                field: "min-max".into(),
            },
            "cap",
        )
        .expect("rename succeeds");
        assert_eq!(
            crate::validation::expr::parse_assert(
                &schema.tables["Item"].fields["cap"].rules[0].assert
            )
            .expect("rewritten assert parses")
            .field_refs(),
            vec!["cap"]
        );
    }
}
