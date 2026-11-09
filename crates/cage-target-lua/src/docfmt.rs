//! Field comment rendering for Lua metadata, with the Map/Array
//! table-shape notes (split out of lib.rs, LAY-022).

use cage_core::schema::{FieldSchema, FieldType, MapKeyType, Schema};

/// Field comment body: description plus constraint summary; `None` when
/// there is nothing to say (same format as the C#/Python generators).
pub(crate) fn field_doc(schema: &Schema, field: &FieldSchema) -> Option<String> {
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
    // Map and Array are both `table` in Lua — the doc must say which shape
    // the field carries, and int-keyed maps how to read their keys.
    if let FieldType::Map(mf) = &field.field_type {
        match mf.key_type {
            MapKeyType::String => parts.push(MAP_NOTE.to_string()),
            MapKeyType::Int => parts.push(format!("{MAP_NOTE}；{MAP_INT_KEY_NOTE}")),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Doc note for Map fields: in Lua both Map and Array are `table`, so the
/// metadata comment must say which shape the field carries — a Map lives in
/// the hash part, an Array is a sequence part.
pub(crate) const MAP_NOTE: &str = "键值表（hash part），与 Array 的数组 table（sequence part）不同";
/// Extra note for integer-keyed maps: the data model stores the keys as
/// numeric strings (`"42"`), so consumers convert them with `tonumber`.
pub(crate) const MAP_INT_KEY_NOTE: &str = "int 键在数据里是数字字符串，消费端用 tonumber 转换";
