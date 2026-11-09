//! Python default-value rendering and string/float literals (split out
//! of lib.rs, LAY-020).

use cage_core::schema::{FieldType, MapField, MapKeyType, Schema};
use std::fmt::Write as _;

/// Render a schema default as a Python initializer expression; `None` when
/// the default does not map to a compile-safe literal (objects, bytes,
/// mismatched kinds) — the field then defaults to None instead.
pub(crate) fn render_default(
    value: &serde_json::Value,
    ft: &FieldType,
    schema: &Schema,
) -> Option<String> {
    match ft {
        FieldType::Bool => value.as_bool().map(|b| {
            if b {
                "True".to_string()
            } else {
                "False".to_string()
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
        FieldType::Float32 | FieldType::Float64 => value.as_f64().map(py_float_literal),
        FieldType::String => value.as_str().map(py_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .map(|entries| map_literal(entries, map, schema)),
        _ => None,
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Lua generators — keeps the three outputs aligned).
fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    schema: &Schema,
) -> Option<String> {
    if !matches!(
        inner,
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
    ) {
        return None;
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(render_default(item, inner, schema)?);
    }
    Some(format!("[{}]", parts.join(", ")))
}
/// Map defaults render entry by entry through the existing rules; entries
/// whose key or value kind does not match are skipped (unlike arrays, where
/// one bad element sinks the whole default), so a partially renderable map
/// still yields its renderable members.
fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> String {
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        let Some(rendered_key) = map_key_literal(key, map.key_type) else {
            continue;
        };
        let Some(rendered_value) = render_default(value, &map.value_type, schema) else {
            continue;
        };
        parts.push(format!("{rendered_key}: {rendered_value}"));
    }
    format!("{{{}}}", parts.join(", "))
}

/// Python literal for one map key. Integer keys arrive as numeric strings
/// (`"42"`, `"-7"`) and become int literals; a non-numeric key does not
/// match an `int` key type and is skipped by the caller.
fn map_key_literal(key: &str, key_type: MapKeyType) -> Option<String> {
    match key_type {
        MapKeyType::String => Some(py_string_literal(key)),
        MapKeyType::Int => key.parse::<i64>().ok().map(|i| i.to_string()),
    }
}

pub(crate) fn py_float_literal(f: f64) -> String {
    if f.is_nan() {
        return "float(\"nan\")".to_string();
    }
    if f.is_infinite() {
        return if f < 0.0 {
            "float(\"-inf\")".to_string()
        } else {
            "float(\"inf\")".to_string()
        };
    }
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

pub(crate) fn py_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Python literals: control characters become
/// `\n` / `\xHH`; printable Unicode passes through.
fn escape_string_content(s: &str) -> String {
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
