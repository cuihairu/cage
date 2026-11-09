//! Defaults, literals, enum backing/literals and field docs (split out of lib.rs, LAY-009).

use cage_core::schema::{EnumSchema, FieldSchema, FieldType, MapField, MapKeyType, Schema};
use std::collections::HashMap;
use std::fmt::Write as _;

use super::types::{go_map_key_type, go_type};

/// Render a schema default as a Go literal expression; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, enum
/// kinds, mismatched kinds).
pub(crate) fn render_default(
    value: &serde_json::Value,
    ft: &FieldType,
    enum_types: &HashMap<&str, String>,
) -> Option<String> {
    match ft {
        FieldType::Bool => value
            .as_bool()
            .map(|b| if b { "true" } else { "false" }.to_string()),
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
        FieldType::Float32 | FieldType::Float64 => value.as_f64().map(go_float_literal),
        FieldType::String => value.as_str().map(go_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, enum_types)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| map_literal(entries, map, enum_types)),
        _ => None,
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Python/Lua generators — keeps the outputs aligned).
pub(crate) fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    enum_types: &HashMap<&str, String>,
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
        parts.push(render_default(item, inner, enum_types)?);
    }
    Some(format!(
        "[]{}{{{}}}",
        go_type(inner, enum_types),
        parts.join(", ")
    ))
}

/// Map defaults render each entry's value through [`render_default`] under
/// the same compile-safe rule as array defaults (one unrenderable entry drops
/// the whole default). An empty default renders the typed empty map literal —
/// the constructor evaluates it on every call, so rows never share one map.
/// Entry order follows the default object's (`serde_json`'s lexicographic)
/// key order, which is deterministic.
pub(crate) fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    enum_types: &HashMap<&str, String>,
) -> Option<String> {
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        parts.push(format!(
            "{}: {}",
            map_key_literal(key, map.key_type)?,
            render_default(value, &map.value_type, enum_types)?
        ));
    }
    Some(format!(
        "map[{}]{}{{{}}}",
        go_map_key_type(map.key_type),
        go_type(&map.value_type, enum_types),
        parts.join(", ")
    ))
}

/// Map key literal: string keys render as Go string literals; integer keys
/// are stored as numeric strings in the data model and parse as i64, so they
/// render bare to match the `int64` map key type (non-numeric keys — already
/// rejected at L2 for data — drop the default rather than mis-render).
pub(crate) fn map_key_literal(key: &str, key_type: MapKeyType) -> Option<String> {
    match key_type {
        MapKeyType::String => Some(go_string_literal(key)),
        MapKeyType::Int => key.parse::<i64>().ok().map(|i| i.to_string()),
    }
}

/// Go float literal: integral values render bare (`100` is a valid untyped
/// constant), non-finite values need the `math` package.
pub(crate) fn go_float_literal(f: f64) -> String {
    if f.is_nan() {
        "math.NaN()".to_string()
    } else if f.is_infinite() {
        if f < 0.0 {
            "math.Inf(-1)".to_string()
        } else {
            "math.Inf(1)".to_string()
        }
    } else {
        f.to_string()
    }
}

/// Struct tag carrying the ORIGINAL field name; `"` inside the name is JSON-
/// escaped inside the raw-string tag. (Field names containing a backtick
/// cannot be represented — see crate docs.)
pub(crate) fn json_tag(field_name: &str) -> String {
    format!("`json:\"{}\"`", field_name.replace('"', "\\\""))
}

pub(crate) fn go_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Go literals: `\\`, `\"` and the short escapes
/// `\n` / `\r` / `\t`; other control characters (including U+007F) become
/// `\xHH` with exactly two hex digits; printable Unicode passes through.
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

/// Enum backing type: an enum where EVERY member has an integral value is
/// numeric, with the smallest range that fits (int32 → int64 → uint64);
/// anything else is a string enum.
pub(crate) fn enum_backing(e: &EnumSchema) -> &'static str {
    let all_integral = !e.values.is_empty()
        && e.values.iter().all(
            |v| matches!(&v.value, Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64()),
        );
    if !all_integral {
        return "string";
    }
    let mut lo = 0i128;
    let mut hi = 0i128;
    for v in &e.values {
        let x = match &v.value {
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from)),
            _ => None,
        };
        if let Some(x) = x {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if lo >= i128::from(i32::MIN) && hi <= i128::from(i32::MAX) {
        "int32"
    } else if lo >= i128::from(i64::MIN) && hi <= i128::from(i64::MAX) {
        "int64"
    } else {
        "uint64"
    }
}

/// Enum member literal: numeric enums keep their decimal value; string enums
/// stringify the value (String as-is, Number as decimal, Bool as
/// `"true"`/`"false"`, value-less members fall back to their name).
pub(crate) fn enum_value_literal(v: &cage_core::schema::EnumValue, backing: &str) -> String {
    use serde_json::Value as J;
    match backing {
        "string" => {
            let s = match &v.value {
                Some(J::String(s)) => s.clone(),
                Some(J::Number(n)) => n.to_string(),
                Some(J::Bool(b)) => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                _ => v.name.clone(),
            };
            go_string_literal(&s)
        }
        _ => match &v.value {
            Some(J::Number(n)) => n.to_string(),
            _ => v.name.clone(), // unreachable: all-integral was checked
        },
    }
}

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
