//! Lua literal rendering: schema defaults, map/array table literals,
//! table keys and float/string quoting (split out of lib.rs, LAY-022).

use super::idents::{sanitize_ident, LUA_KEYWORDS};
use cage_core::schema::{FieldType, MapField, Schema};
use std::fmt::Write as _;

/// Render a schema default as a Lua initializer expression; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, mismatched
/// kinds, non-finite floats — Lua has no literal for nan/inf).
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
        FieldType::Float32 | FieldType::Float64 => value.as_f64().and_then(|f| {
            if f.is_finite() {
                Some(lua_float_literal(f))
            } else {
                None
            }
        }),
        FieldType::String => value.as_str().map(lua_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(mf) => value
            .as_object()
            .map(|entries| map_literal(entries, mf, schema)),
        _ => None,
    }
}

/// Map defaults render member-by-member with the existing scalar rules
/// (recursing through [`render_default`], so nested maps and scalar arrays
/// work too). Unlike arrays, a member with no safe literal is skipped
/// individually — one mismatched entry does not sink the whole default;
/// a fully-skipped or empty default renders as an empty table.
fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    mf: &MapField,
    schema: &Schema,
) -> String {
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        if let Some(rendered) = render_default(value, &mf.value_type, schema) {
            parts.push(format!("{} = {rendered}", lua_table_key(key)));
        }
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Lua table-constructor key: identifier-safe string keys take the short
/// `k = v` form; everything else (keywords, spaces, escapes, and the
/// numeric-string keys of int-keyed maps — kept verbatim because that is
/// the data-model spelling) goes bracketed `["k"]`.
fn lua_table_key(key: &str) -> String {
    if sanitize_ident(key) == key && !LUA_KEYWORDS.contains(&key) {
        key.to_string()
    } else {
        format!("[{}]", lua_string_literal(key))
    }
}

/// Array defaults are rendered only for scalar element types (same rule as
/// the C#/Python generators — keeps the three outputs aligned).
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
    Some(format!("{{ {} }}", parts.join(", ")))
}

fn lua_float_literal(f: f64) -> String {
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

pub(crate) fn lua_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Lua literals: control characters become `\n` /
/// `\ddd` (Lua has no `\xHH`); printable Unicode passes through.
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
                let _ = write!(out, "\\{code:03}");
            }
            c => out.push(c),
        }
    }
    out
}
