//! Go type mapping: `FieldType` to Go type text, nil-ability and map key types (split out of lib.rs, LAY-009).

use cage_core::schema::{FieldType, MapKeyType};
use std::collections::HashMap;

/// Whether the Go type is nil-able without a pointer (slices, maps, `any`).
pub(crate) fn is_nilable_reference(ft: &FieldType) -> bool {
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
pub(crate) fn field_go_type(
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
pub(crate) fn go_type(ft: &FieldType, enum_types: &HashMap<&str, String>) -> String {
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
        // Tagged union renders as its runtime shape — a flat map with the
        // discriminant key (L2 validates the branch; a Go interface hierarchy
        // is a follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => "map[string]any".to_string(),
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
pub(crate) fn go_map_key_type(key_type: MapKeyType) -> &'static str {
    match key_type {
        MapKeyType::String => "string",
        MapKeyType::Int => "int64",
    }
}
