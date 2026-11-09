//! C# default-value rendering and string/float literals (split out of
//! lib.rs, LAY-019).

use super::types::{cs_map_key_type, cs_type_inner};
use cage_core::schema::{FieldType, MapField, MapKeyType, Schema};
use std::fmt::Write as _;

/// A non-nullable reference type with no schema default needs an empty
/// initializer so the file compiles warning-free under `#nullable enable`.
pub(crate) fn needs_ref_init(ty: &str) -> bool {
    !ty.ends_with('?')
        && (ty == "string"
            || ty.starts_with("byte[")
            || ty.starts_with("Dictionary<")
            || ty.starts_with("IReadOnlyList<")
            || ty.starts_with("IReadOnlyDictionary<"))
}

pub(crate) fn ref_empty_init(ty: &str) -> String {
    if ty == "string" {
        "string.Empty".to_string()
    } else if ty.starts_with("byte[") {
        "Array.Empty<byte>()".to_string()
    } else if ty.starts_with("Dictionary<") {
        // Target-typed `new()` — K/V type arguments come from the field.
        "new()".to_string()
    } else if let Some(inner) = ty
        .strip_prefix("IReadOnlyList<")
        .and_then(|s| s.strip_suffix('>'))
    {
        format!("Array.Empty<{inner}>()")
    } else {
        "new Dictionary<string, object?>()".to_string()
    }
}

/// Render a schema default as a C# initializer literal; `None` when the
/// default does not map to a compile-safe literal (objects, mismatched
/// kinds) — the field then stays optional/nullable.
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
        FieldType::Float32 => value.as_f64().map(|f| cs_float_literal(f, true)),
        FieldType::Float64 => value.as_f64().map(|f| cs_float_literal(f, false)),
        FieldType::String => value.as_str().map(cs_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| render_array_default(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| render_map_default(entries, map, schema)),
        _ => None,
    }
}

/// Scalar types whose defaults render as literals — the only element/value
/// types array and map defaults accept (same rule as the Python/Lua
/// generators — keeps the outputs aligned).
fn is_defaultable_scalar(ft: &FieldType) -> bool {
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

/// Array defaults are rendered only for scalar element types (see
/// [`is_defaultable_scalar`]).
fn render_array_default(
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
    let elem = cs_type_inner(inner, schema);
    Some(format!("new {elem}[] {{ {} }}", parts.join(", ")))
}

/// Map defaults follow the array rule: only scalar value types render, and
/// any unrenderable entry sinks the whole default. Keys are strings on the
/// wire; an int-keyed map's numeric-string keys are promoted to `long`
/// literals (a non-numeric key is a mismatched kind → dropped). An empty
/// object renders as the target-typed `new()`.
fn render_map_default(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> Option<String> {
    if !is_defaultable_scalar(&map.value_type) {
        return None;
    }
    if entries.is_empty() {
        return Some("new()".to_string());
    }
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        let key_lit = match map.key_type {
            MapKeyType::String => cs_string_literal(key),
            MapKeyType::Int => key.parse::<i64>().ok()?.to_string(),
        };
        let value_lit = render_default(value, &map.value_type, schema)?;
        parts.push(format!("{{ {key_lit}, {value_lit} }}"));
    }
    let ty = format!(
        "Dictionary<{}, {}>",
        cs_map_key_type(map.key_type),
        cs_type_inner(&map.value_type, schema)
    );
    Some(format!("new {ty} {{ {} }}", parts.join(", ")))
}

pub(crate) fn cs_float_literal(f: f64, float32: bool) -> String {
    if f.is_nan() {
        return if float32 {
            "float.NaN".to_string()
        } else {
            "double.NaN".to_string()
        };
    }
    if f.is_infinite() {
        let ty = if float32 { "float" } else { "double" };
        let sign = if f < 0.0 { "Negative" } else { "Positive" };
        return format!("{ty}.{sign}Infinity");
    }
    let s = f.to_string();
    if float32 {
        format!("{s}f")
    } else {
        s
    }
}

pub(crate) fn cs_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for C# (and Lua-compatible) literals: control
/// characters become `\n` / `\uXXXX`; printable Unicode passes through.
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
                let _ = write!(out, "\\u{code:04x}");
            }
            c => out.push(c),
        }
    }
    out
}
