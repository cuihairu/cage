//! TypeScript type mapping: `FieldType` to type text and map key types (split out of lib.rs, LAY-010).

use cage_core::schema::{FieldType, MapKeyType, Schema};

use super::idents::ts_ident;
/// TypeScript type annotation for a field type (no optionality suffix).
///
/// `enum_ty` maps a resolved enum's schema name to how it is spelled in
/// this file: the (possibly aliased) local binding in TS mode / `.d.ts`,
/// or an inline `import("…")` type in JS-mode `JSDoc`.
pub(crate) fn ts_type(
    ft: &FieldType,
    schema: &Schema,
    enum_ty: &std::collections::HashMap<String, String>,
) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "unknown".to_string(),
        FieldType::Bool => "boolean".to_string(),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64
        | FieldType::Float32
        | FieldType::Float64 => "number".to_string(),
        FieldType::String => "string".to_string(),
        FieldType::Bytes => "Uint8Array".to_string(),
        FieldType::Array(inner) => format!("{}[]", ts_type(inner, schema, enum_ty)),
        FieldType::Map(map) => format!(
            "Map<{}, {}>",
            ts_map_key(map.key_type),
            ts_type(&map.value_type, schema, enum_ty)
        ),
        // Tagged union renders as its runtime shape — a flat record with the
        // discriminant key (L2 validates the branch; a TS discriminated
        // union is a follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => "Record<string, unknown>".to_string(),
        FieldType::Enum(name) => match schema.enums.get(name).filter(|e| !e.values.is_empty()) {
            Some(_) => enum_ty.get(name).cloned().unwrap_or_else(|| ts_ident(name)),
            // Unresolved (or empty) enum: fall back to plain string.
            None => "string".to_string(),
        },
    }
}

/// Map key annotation: `string` keys stay strings; int keys are typed as
/// `number` (the data model stores them as numeric strings, but the typed
/// surface spells them as numbers).
pub(crate) fn ts_map_key(key: MapKeyType) -> &'static str {
    match key {
        MapKeyType::String => "string",
        MapKeyType::Int => "number",
    }
}
