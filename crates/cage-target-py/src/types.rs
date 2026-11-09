//! Python type annotations: `FieldType` → Python annotation text (split
//! out of lib.rs, LAY-020).

use super::idents::py_ident;
use cage_core::schema::{FieldType, MapKeyType, Schema};
use std::collections::HashSet;

/// Python type annotation for a field type (no optionality suffix).
pub(crate) fn py_type(ft: &FieldType, schema: &Schema) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "Any".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => "int".to_string(),
        FieldType::Float32 | FieldType::Float64 => "float".to_string(),
        FieldType::String => "str".to_string(),
        FieldType::Bytes => "bytes".to_string(),
        FieldType::Array(inner) => format!("list[{}]", py_type(inner, schema)),
        // Tagged union renders as its runtime shape — a flat dict with the
        // discriminant key (L2 validates the branch; a Python union of
        // TypedDicts is a follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => "dict[str, Any]".to_string(),
        FieldType::Map(map) => format!(
            "dict[{}, {}]",
            map_key_annotation(map.key_type),
            py_type(&map.value_type, schema)
        ),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => py_ident(name),
            // Unresolved (or empty) enum: fall back to plain str.
            None => "str".to_string(),
        },
    }
}
/// Python annotation for a map key type: string keys → `str`, integer
/// keys → `int` (they live as numeric strings in the data model, but the
/// annotation is the semantic type).
fn map_key_annotation(key_type: MapKeyType) -> &'static str {
    match key_type {
        MapKeyType::String => "str",
        MapKeyType::Int => "int",
    }
}
/// Whether the Python annotation for this type mentions `Any` (Null/Any
/// kinds, untyped objects, or any Array/Map nesting thereof) — those
/// modules need `from typing import Any`.
pub(crate) fn annotation_uses_any(ft: &FieldType) -> bool {
    match ft {
        FieldType::Null | FieldType::Any | FieldType::Object(_) => true,
        FieldType::Array(inner) => annotation_uses_any(inner),
        FieldType::Map(map) => annotation_uses_any(&map.value_type),
        _ => false,
    }
}

/// Enum names a field's annotation spells out, collected through
/// `list[...]`/`dict[...]` nesting — the module must import exactly these,
/// or the generated annotation would name an undefined class.
pub(crate) fn annotation_enum_names<'a>(ft: &'a FieldType, out: &mut HashSet<&'a str>) {
    match ft {
        FieldType::Enum(name) => {
            out.insert(name.as_str());
        }
        FieldType::Array(inner) => annotation_enum_names(inner, out),
        FieldType::Map(map) => annotation_enum_names(&map.value_type, out),
        _ => {}
    }
}
