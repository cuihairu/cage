//! Normalize - canonical representation for deterministic builds
//! Ensures semantically equivalent values produce identical canonical forms

use crate::value::{TypedValue, Value};
use indexmap::IndexMap;
use std::collections::BTreeMap;

/// Normalize a value to its canonical form
pub fn normalize_value(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Int(i) => normalize_int(*i),
        Value::UInt(u) => normalize_uint(*u),
        Value::Float(f) => normalize_float(*f),
        Value::String(s) => normalize_string(s),
        Value::Bytes(b) => Value::Bytes(b.clone()),
        Value::Array(arr) => normalize_array(arr),
        Value::Object(obj) => normalize_object(obj),
    }
}

fn normalize_int(i: i64) -> Value {
    // The canonical Value is width-untagged (i64); width checking happens at
    // L2 schema validation, so there is nothing to narrow here yet.
    Value::Int(i)
}

fn normalize_uint(u: u64) -> Value {
    // The canonical Value is width-untagged (u64); width checking happens at
    // L2 schema validation, so there is nothing to narrow here yet.
    Value::UInt(u)
}

fn normalize_float(f: f64) -> Value {
    // Canonicalize -0.0 to 0.0; NaN/Inf pass through unchanged for reproducibility
    Value::Float(if f == 0.0 { 0.0 } else { f })
}

fn normalize_string(s: &str) -> Value {
    // Trim whitespace, normalize line endings
    let normalized = s.trim().replace('\r', "");
    Value::String(normalized)
}

fn normalize_array(arr: &[Value]) -> Value {
    Value::Array(arr.iter().map(normalize_value).collect())
}

fn normalize_object(obj: &IndexMap<String, Value>) -> Value {
    // Sort keys for deterministic ordering
    let mut sorted: BTreeMap<String, Value> = BTreeMap::new();
    for (k, v) in obj {
        sorted.insert(k.clone(), normalize_value(v));
    }
    Value::Object(sorted.into_iter().collect())
}

/// Normalize a typed value (preserves location)
pub fn normalize_typed_value(typed: &TypedValue) -> TypedValue {
    TypedValue {
        value: normalize_value(&typed.value),
        location: typed.location.clone(),
        schema_type: typed.schema_type.clone(),
    }
}

/// Normalize entire document
pub fn normalize_document(doc: &crate::value::Document) -> crate::value::Document {
    let mut normalized = crate::value::Document::new();
    normalized.source_files.clone_from(&doc.source_files);
    normalized.metadata = doc.metadata.clone();

    for table in doc.tables.values() {
        let mut norm_table = crate::value::Table {
            name: table.name.clone(),
            primary_key_fields: table.primary_key_fields.clone(),
            rows: Vec::new(),
            source_file: table.source_file.clone(),
            sheet: table.sheet.clone(),
        };

        for row in &table.rows {
            let mut norm_fields = IndexMap::new();
            for (field_name, typed_value) in &row.fields {
                norm_fields.insert(field_name.clone(), normalize_typed_value(typed_value));
            }

            norm_table.rows.push(crate::value::Row {
                primary_key: row.primary_key.iter().map(normalize_value).collect(),
                fields: norm_fields,
                location: row.location.clone(),
                index: row.index,
            });
        }

        normalized.add_table(norm_table);
    }

    normalized
}

mod fields;
#[cfg(test)]
mod tests;

pub use fields::{coerce_to_type, fill_field_defaults, split_cell_arrays};
