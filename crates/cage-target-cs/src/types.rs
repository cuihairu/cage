//! C# type-text mapping: schema `FieldType` → C# type strings (split out
//! of lib.rs, LAY-019).

use super::idents::cs_ident;
use cage_core::schema::{FieldType, MapKeyType, Schema};

/// C# type for a field type (no optionality suffix).
pub(crate) fn cs_type_inner(ft: &FieldType, schema: &Schema) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "object?".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Int8 => "sbyte".to_string(),
        FieldType::Int16 => "short".to_string(),
        FieldType::Int32 => "int".to_string(),
        FieldType::Int64 => "long".to_string(),
        FieldType::UInt8 => "byte".to_string(),
        FieldType::UInt16 => "ushort".to_string(),
        FieldType::UInt32 => "uint".to_string(),
        FieldType::UInt64 => "ulong".to_string(),
        FieldType::Float32 => "float".to_string(),
        FieldType::Float64 => "double".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "byte[]".to_string(),
        FieldType::Array(inner) => format!("IReadOnlyList<{}>", cs_type_inner(inner, schema)),
        // Tagged union renders as its runtime shape — a flat dictionary with
        // the discriminant key (L2 validates the branch; a C#
        // discriminated union is a follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => {
            "IReadOnlyDictionary<string, object?>".to_string()
        }
        // Unlike Object, a Map knows its exact K/V — the concrete
        // Dictionary (not IReadOnlyDictionary) so `= new();` defaults and
        // empty initializers compile against it.
        FieldType::Map(map) => format!(
            "Dictionary<{}, {}>",
            cs_map_key_type(map.key_type),
            cs_type_inner(&map.value_type, schema)
        ),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => cs_ident(name),
            // Unresolved (or empty) enum: fall back to plain string.
            None => "string".to_string(),
        },
    }
}

/// Map key type: string keys stay `string`; signed-integer keys (stored as
/// numeric strings in the data model, e.g. `"42"` / `"-7"`) widen to `long`.
pub(crate) fn cs_map_key_type(key: MapKeyType) -> &'static str {
    match key {
        MapKeyType::String => "string",
        MapKeyType::Int => "long",
    }
}

/// The filter-facing alias of `cs_type_inner` (same mapping the official
/// generator emits).
pub(crate) fn cs_type(ft: &FieldType, schema: &Schema) -> String {
    cs_type_inner(ft, schema)
}
