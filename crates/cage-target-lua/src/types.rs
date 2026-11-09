//! Lua type labels: the metadata spelling of schema field types and
//! map key types — not a real type system (split out of lib.rs, LAY-022).

use cage_core::schema::{FieldType, MapKeyType, Schema};

/// Lua type label for a field type (metadata string, not a real type system).
pub(crate) fn lua_type_label(ft: &FieldType, schema: &Schema) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "any".to_string(),
        FieldType::Bool => "boolean".to_string(),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => "integer".to_string(),
        FieldType::Float32 | FieldType::Float64 => "number".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "bytes".to_string(),
        FieldType::Array(inner) => format!("array<{}>", lua_type_label(inner, schema)),
        FieldType::Map(mf) => format!(
            "map<{}, {}>",
            map_key_label(mf.key_type),
            lua_type_label(&mf.value_type, schema)
        ),
        // Tagged union renders as its runtime shape — a flat table with the
        // discriminant key (L2 validates the branch; per-language
        // discriminated unions are a follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => "table".to_string(),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => name.clone(),
            // Unresolved (or empty) enum: fall back to plain string.
            None => "string".to_string(),
        },
    }
}

/// Label for the key half of `map<K, V>`: `string` | `int` (the schema's
/// `MapKeyType` spelling, mirroring the YAML wire format).
fn map_key_label(key: MapKeyType) -> &'static str {
    match key {
        MapKeyType::String => "string",
        MapKeyType::Int => "int",
    }
}
