//! X2: the decoded convention metadata (`TableMeta`) as a schema, and the
//! drift check against an adopted one (design §51).
//!
//! The mapping is a field-for-field copy: `ColumnMeta` already mirrors the
//! Schema DSL fields (`type`, `enum`, `reference`, …), so there is no
//! translation and no place for the two spellings to drift. Structural
//! nesting (回流-5/6) is the one addition: `sub_columns` become a nested
//! `Object` (with `properties`), `multi_row` wraps that in an `Array` (with
//! `items`), both keyed off the same sub-column declarations.
//!
//! Two consumers share this module:
//! - the derived chain — a table whose workbook declares meta rows and has
//!   no adopted schema entry validates against the schema derived here;
//! - `schema-draft` renders the derived schema as a reviewable YAML draft.
//! And [`drift`] is the adopted-schema cross-check (E2208, cell-anchored).

use super::{ColumnMeta, TableMeta};
use crate::diagnostics::{Diagnostic, Diagnostics};
use crate::error::codes::source_convention::E2208;
use crate::schema::{EnumSchema, EnumValue, FieldSchema, FieldType, Schema, TableSchema};
use crate::value::SourceLocation;
use indexmap::IndexMap;

/// An all-defaults field (the schema structs do not derive `Default`; the
/// literal sets every field so a new constraint key cannot be forgotten
/// silently — the compiler names it).
fn empty_field() -> FieldSchema {
    FieldSchema {
        name: String::new(),
        field_type: FieldType::String,
        description: None,
        required: false,
        nullable: false,
        default: None,
        min: None,
        max: None,
        min_length: None,
        max_length: None,
        pattern: None,
        enum_values: None,
        min_items: None,
        max_items: None,
        items: None,
        separator: None,
        properties: None,
        additional_properties: None,
        reference: None,
        targets: Vec::new(),
        rules: Vec::new(),
        metadata: IndexMap::new(),
    }
}

fn empty_table() -> TableSchema {
    TableSchema {
        name: String::new(),
        description: None,
        primary_key: Vec::new(),
        fields: IndexMap::new(),
        unique_constraints: Vec::new(),
        order_by: None,
        targets: Vec::new(),
        env_overrides: IndexMap::new(),
    }
}

/// The field type a column declares: a column with sub-columns is a nested
/// bean by construction (the `Object` type is implied, never written), and a
/// `multi_row` column is the `Array` of those nested beans. A childless
/// column is its declared type, or `String` when the declaration is silent.
pub fn field_type_of(column: &ColumnMeta) -> FieldType {
    if column.sub_columns.is_empty() {
        return column.field_type.clone().unwrap_or(FieldType::String);
    }
    let mut props = IndexMap::new();
    for (name, child) in &column.sub_columns {
        props.insert(name.clone(), field_type_of(child));
    }
    let bean = FieldType::Object(props);
    if column.multi_row {
        FieldType::Array(Box::new(bean))
    } else {
        bean
    }
}

/// The `Object` element schema for a column's sub-columns (the `items`
/// element of a `multi_row` array, or the `properties` of a nested bean).
fn bean_schema(name: &str, column: &ColumnMeta) -> FieldSchema {
    let mut props = IndexMap::new();
    for (child_name, child) in &column.sub_columns {
        props.insert(child_name.clone(), field_schema(child_name, child));
    }
    FieldSchema {
        name: name.to_string(),
        // The bean element is the Object, never the multi_row Array: a
        // multi_row column's `items` is this bean and its own type wraps it.
        field_type: FieldType::Object(
            column
                .sub_columns
                .iter()
                .map(|(n, c)| (n.clone(), field_type_of(c)))
                .collect(),
        ),
        description: column.description.clone(),
        properties: Some(props),
        additional_properties: Some(false),
        ..empty_field()
    }
}

/// The field-for-field copy of one declared column into the Schema DSL. The
/// meta-row vocabulary is a subset of the DSL: only the keys a meta row can
/// declare are carried, so an author who later adds `pattern`/`min_length`
/// in the schema is not contradicting the workbook — and the drift check
/// compares exactly this subset.
pub fn field_schema(name: &str, column: &ColumnMeta) -> FieldSchema {
    if !column.sub_columns.is_empty() {
        let bean = bean_schema(name, column);
        if column.multi_row {
            return FieldSchema {
                name: name.to_string(),
                field_type: FieldType::Array(Box::new(bean.field_type.clone())),
                items: Some(Box::new(bean)),
                ..empty_field()
            };
        }
        return bean;
    }
    FieldSchema {
        name: name.to_string(),
        field_type: column.field_type.clone().unwrap_or(FieldType::String),
        description: column.description.clone(),
        required: column.required.unwrap_or(false),
        default: column.default.clone(),
        min: column.min,
        max: column.max,
        enum_values: column.enum_values.clone(),
        reference: column.reference.clone(),
        targets: column.targets.clone(),
        ..empty_field()
    }
}

/// The table schema a decoded table derives to: the declared columns in
/// declaration order, and the primary key columns the meta rows named.
pub fn to_table_schema(table_name: &str, meta: &TableMeta) -> TableSchema {
    let mut fields = IndexMap::new();
    let mut primary_key = Vec::new();
    for (name, column) in &meta.columns {
        if column.primary_key == Some(true) {
            primary_key.push(name.clone());
        }
        fields.insert(name.clone(), field_schema(name, column));
    }
    TableSchema {
        name: table_name.to_string(),
        primary_key,
        fields,
        ..empty_table()
    }
}

/// Collect the named enums the declared columns reference (`Enum<Name>`
/// plus the `##enum` values), into a `Schema`. Deterministic: first
/// declaration wins, later values that repeat are dropped.
fn collect_enums(meta: &TableMeta, enums: &mut IndexMap<String, EnumSchema>) {
    fn walk(column: &ColumnMeta, enums: &mut IndexMap<String, EnumSchema>) {
        if let Some(FieldType::Enum(name)) = &column.field_type {
            if let Some(values) = &column.enum_values {
                let entry = enums.entry(name.clone()).or_insert_with(|| EnumSchema {
                    name: name.clone(),
                    values: Vec::new(),
                    description: None,
                });
                for value in values {
                    if !entry.values.iter().any(|v| &v.name == value) {
                        entry.values.push(EnumValue {
                            name: value.clone(),
                            value: None,
                            description: None,
                        });
                    }
                }
            }
        }
        for child in column.sub_columns.values() {
            walk(child, enums);
        }
    }
    for column in meta.columns.values() {
        walk(column, enums);
    }
}

/// The full schema a set of decoded tables derives to — the derived chain.
pub fn derive_schema(metas: &IndexMap<String, TableMeta>) -> Schema {
    let mut tables = IndexMap::new();
    let mut enums = IndexMap::new();
    for (name, meta) in metas {
        tables.insert(name.clone(), to_table_schema(name, meta));
        collect_enums(meta, &mut enums);
    }
    Schema {
        tables,
        enums,
        metadata: None,
    }
}

/// Whether a `FieldSchema` carries a declaration the meta row made — used
/// by the drift check to compare only what the workbook actually declared.
fn declared_mismatch(
    column: &ColumnMeta,
    derived: &FieldSchema,
    adopted: &FieldSchema,
) -> Option<String> {
    if column.field_type.is_some() && derived.field_type != adopted.field_type {
        return Some(format!(
            "declares type {} but the schema has {}",
            field_type_label(&derived.field_type),
            field_type_label(&adopted.field_type)
        ));
    }
    if let Some(v) = column.required {
        if v != adopted.required {
            return Some(format!(
                "declares required {v} but the schema has {}",
                adopted.required
            ));
        }
    }
    if column.default.is_some() && column.default != adopted.default {
        return Some("declares a default but the schema has a different one".to_string());
    }
    if column.enum_values.is_some() && column.enum_values != adopted.enum_values {
        return Some("declares enum values that differ from the schema".to_string());
    }
    if column.min.is_some() && column.min != adopted.min {
        return Some("declares a min that differs from the schema".to_string());
    }
    if column.max.is_some() && column.max != adopted.max {
        return Some("declares a max that differs from the schema".to_string());
    }
    if let Some(declared) = &column.reference {
        // A meta row declares where a column points (table + field); the
        // predicate and cardinality are schema-side and are not drift.
        match &adopted.reference {
            None => return Some("declares a reference but the schema has none".to_string()),
            Some(adopted) => {
                if declared.table != adopted.table || declared.field != adopted.field {
                    return Some("declares a reference that differs from the schema".to_string());
                }
            }
        }
    }
    None
}

fn field_type_label(t: &FieldType) -> String {
    serde_json::to_string(t).unwrap_or_else(|_| "?".to_string())
}

/// Whether the meta rows name any primary key at all (the drift check only
/// compares the primary key when the workbook declared one).
fn declares_primary_key(meta: &TableMeta) -> bool {
    meta.columns.values().any(|c| c.primary_key.is_some())
}

/// The source location of a declaration cell: the column letter and row
/// number the A1 anchor carries.
fn cell_location(source_file: &str, sheet: &str, cell: &str) -> SourceLocation {
    let letters: String = cell.chars().take_while(char::is_ascii_alphabetic).collect();
    let digits: String = cell.chars().skip_while(char::is_ascii_alphabetic).collect();
    let mut loc = SourceLocation::new(source_file).with_sheet(sheet);
    if !letters.is_empty() {
        loc = loc.with_column(letters);
    }
    if let Ok(row) = digits.parse::<usize>() {
        loc = loc.with_row(row);
    }
    loc
}

/// A drift diagnostic, anchored at the exact declaration cell.
fn drift_diag(
    source_file: &str,
    sheet: &str,
    table_name: &str,
    cell: &str,
    message: String,
) -> Diagnostic {
    Diagnostic::error(E2208, message)
        .with_table(table_name)
        .with_location(cell_location(source_file, sheet, cell))
}

/// The derived chain (X2): a project that adopts no schema at all
/// (`schema_path: None`) validates its convention Excel against a schema
/// derived here — the "no YAML" acceptance. Tables and enums the workbook
/// declares are inserted only when the schema has no such entry, so an
/// author who later adopts a partial schema keeps the derived remainder.
pub fn fill_derived(schema: &mut Schema, metas: &IndexMap<String, TableMeta>) {
    for (name, meta) in metas {
        if !schema.tables.contains_key(name) {
            schema
                .tables
                .insert(name.clone(), to_table_schema(name, meta));
        }
        collect_enums(meta, &mut schema.enums);
    }
}

/// The drift check (X2): a project that adopts a schema (`schema_path:
/// Some`) cross-checks every convention Excel table against it. Every column
/// the meta rows declare must exist in the adopted schema with the same
/// shape on the declaration-carried subset; the workbook's primary key, when
/// declared, must match; a table the workbook declares but the schema lacks
/// is drift too. Adopted fields the meta rows never mention are the author's
/// own and are not drift. Each mismatch is an E2208 anchored at the exact
/// declaration cell.
pub fn drift(
    metas: &IndexMap<String, TableMeta>,
    adopted: &Schema,
    source_file: &str,
) -> Diagnostics {
    let mut diags = Diagnostics::new();
    for (table_name, meta) in metas {
        let sheet = table_name.clone();
        let Some(adopted_table) = adopted.tables.get(table_name) else {
            let cell = meta
                .columns
                .values()
                .find(|c| !c.cell.is_empty())
                .map(|c| c.cell.clone())
                .unwrap_or_default();
            diags.add(drift_diag(
                source_file,
                &sheet,
                table_name,
                &cell,
                format!(
                    "table '{table_name}' is declared in the workbook but has no entry in \
                     the adopted schema"
                ),
            ));
            continue;
        };
        if declares_primary_key(meta) {
            let derived = to_table_schema(table_name, meta);
            let mut a = derived.primary_key.clone();
            let mut b = adopted_table.primary_key.clone();
            a.sort();
            b.sort();
            if a != b {
                let cell = meta
                    .columns
                    .values()
                    .find(|c| c.primary_key == Some(true))
                    .map(|c| c.cell.clone())
                    .unwrap_or_default();
                diags.add(drift_diag(
                    source_file,
                    &sheet,
                    table_name,
                    &cell,
                    format!(
                        "declares primary key [{}] but the schema has [{}]",
                        derived.primary_key.join(", "),
                        adopted_table.primary_key.join(", ")
                    ),
                ));
            }
        }
        for (name, column) in &meta.columns {
            let derived = field_schema(name, column);
            let Some(adopted_field) = adopted_table.fields.get(name) else {
                diags.add(drift_diag(
                    source_file,
                    &sheet,
                    table_name,
                    &column.cell,
                    format!("declares field '{name}' which is not in the adopted schema"),
                ));
                continue;
            };
            if let Some(why) = declared_mismatch(column, &derived, adopted_field) {
                diags.add(drift_diag(
                    source_file,
                    &sheet,
                    table_name,
                    &column.cell,
                    format!("field '{name}' {why}"),
                ));
            }
            // Recurse into sub-columns when the adopted field is a bean too.
            if let (Some(a_props), Some(b_props)) = (&adopted_field.properties, &derived.properties)
            {
                for (child_name, child) in &column.sub_columns {
                    match (a_props.get(child_name), b_props.get(child_name)) {
                        (Some(a_child), Some(_)) => {
                            let d = field_schema(child_name, child);
                            if let Some(why) = declared_mismatch(child, &d, a_child) {
                                diags.add(drift_diag(
                                    source_file,
                                    &sheet,
                                    table_name,
                                    &child.cell,
                                    format!("field '{name}.{child_name}' {why}"),
                                ));
                            }
                        }
                        (None, _) => {
                            diags.add(drift_diag(
                                source_file,
                                &sheet,
                                table_name,
                                &child.cell,
                                format!(
                                    "declares sub-field '{name}.{child_name}' which is not \
                                     in the adopted schema"
                                ),
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    diags
}

/// Render the derived schema as a reviewable YAML draft (X2 `schema-draft`).
/// The body is the derived [`Schema`] serialized through its own serde — a
/// round trip is exact by construction — with a comment header naming the
/// source. The author reviews the semantic constraints before adopting.
pub fn render_draft(source_spec: &str, derived: &Schema) -> String {
    let mut out = String::new();
    out.push_str("# schema draft — derived from Excel meta rows (");
    out.push_str(source_spec);
    out.push_str(")\n");
    out.push_str(
        "# review constraints (min / max / pattern / enum_values / references) before adopting\n",
    );
    let body =
        serde_yaml::to_string(derived).unwrap_or_else(|_| "tables: {}\nenums: {}\n".to_string());
    out.push_str(&body);
    out
}
