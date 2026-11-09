//! Defaults, literals, string escaping and field docs (split out of lib.rs, LAY-010).

use cage_core::schema::{FieldSchema, FieldType, MapField, MapKeyType, Schema};
use std::fmt::Write as _;
/// Field comment body: description plus constraint summary; `None` when
/// there is nothing to say (same format as the C#/Python/Lua generators).
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
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}
/// Render a schema default as a TS/JS initializer expression; `None` when
/// the default does not map to a literal (objects, bytes, mismatched kinds).
/// Non-finite floats ARE rendered — `NaN` / `Infinity` are real TS values.
pub(crate) fn render_default(
    value: &serde_json::Value,
    ft: &FieldType,
    schema: &Schema,
) -> Option<String> {
    match ft {
        FieldType::Bool => value.as_bool().map(|b| {
            if b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }),
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
        FieldType::Float32 | FieldType::Float64 => value.as_f64().map(ts_float_literal),
        FieldType::String => value.as_str().map(ts_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| map_literal(entries, map, schema)),
        _ => None,
    }
}

/// Value types a collection default can render (`array_literal` and
/// `map_literal` share this gate — same rule as the C#/Python/Lua
/// generators, keeps the outputs aligned).
pub(crate) fn is_defaultable_scalar(ft: &FieldType) -> bool {
    matches!(
        ft,
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
    )
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Python/Lua generators — keeps the outputs aligned).
pub(crate) fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    schema: &Schema,
) -> Option<String> {
    if !is_defaultable_scalar(inner) {
        return None;
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(render_default(item, inner, schema)?);
    }
    Some(format!("[{}]", parts.join(", ")))
}

/// Map defaults render as `new Map(...)` entries (never an object literal —
/// the typed surface is `Map<K, V>`): an empty object gives `new Map()`,
/// other values follow the array rule (scalar value types only, and any
/// unrenderable entry sinks the whole default). Keys are strings on the
/// wire; an int-keyed map's numeric-string keys are promoted to number
/// literals (a non-numeric key is a mismatched kind → dropped).
pub(crate) fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> Option<String> {
    if !is_defaultable_scalar(&map.value_type) {
        return None;
    }
    if entries.is_empty() {
        return Some("new Map()".to_string());
    }
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        let key_lit = match map.key_type {
            MapKeyType::String => ts_string_literal(key),
            MapKeyType::Int => key.parse::<i64>().ok()?.to_string(),
        };
        let value_lit = render_default(value, &map.value_type, schema)?;
        parts.push(format!("[{key_lit}, {value_lit}]"));
    }
    Some(format!("new Map([{}])", parts.join(", ")))
}

/// TS/JS float literal: non-finite values render as the real `NaN` /
/// `Infinity` / `-Infinity` identifiers; finite values keep a `.`/`e`.
pub(crate) fn ts_float_literal(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }
    f.to_string()
}

pub(crate) fn ts_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for TS/JS literals: control characters become
/// `\n` / `\t` / `\xHH` (valid in both); printable Unicode passes through.
pub(crate) fn escape_string_content(s: &str) -> String {
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
