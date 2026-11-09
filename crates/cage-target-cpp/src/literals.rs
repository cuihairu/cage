//! Defaults, literals, enum backing/values and field docs (split out of lib.rs, LAY-011).

use cage_core::schema::{
    EnumSchema, EnumValue, FieldSchema, FieldType, MapField, MapKeyType, Schema,
};
use std::fmt::Write as _;
/// Render a schema default as a C++ initializer expression; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, null,
/// enums, mismatched kinds — shared rule across all targets; arrays and maps
/// render for scalar element types only).
pub(crate) fn render_default(
    value: &serde_json::Value,
    ft: &FieldType,
    schema: &Schema,
) -> Option<String> {
    match ft {
        FieldType::Bool => value.as_bool().map(|b| b.to_string()),
        FieldType::Int8
        | FieldType::Int16
        | FieldType::Int32
        | FieldType::Int64
        | FieldType::UInt8
        | FieldType::UInt16
        | FieldType::UInt32
        | FieldType::UInt64 => value
            .as_i64()
            .map(cpp_int_literal)
            .or_else(|| value.as_u64().map(cpp_uint_literal)),
        FieldType::Float32 => value.as_f64().map(|f| cpp_float_literal(f, true)),
        FieldType::Float64 => value.as_f64().map(|f| cpp_float_literal(f, false)),
        FieldType::String => value.as_str().map(cpp_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| map_literal(entries, map, schema)),
        _ => None,
    }
}

/// Whether a default literal exists for this element type: scalars only —
/// the rule array and map defaults share with the other generators.
pub(crate) fn scalar_default_type(ft: &FieldType) -> bool {
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
/// the other generators); rendered as a brace initializer list.
pub(crate) fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    schema: &Schema,
) -> Option<String> {
    if !scalar_default_type(inner) {
        return None;
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(render_default(item, inner, schema)?);
    }
    Some(format!("{{{}}}", parts.join(", ")))
}

/// Map defaults: an empty object renders the empty brace-init list; a
/// non-empty one renders `{{key, value}, ...}` pairs, and only for scalar
/// value types (same rule as arrays). Int keys arrive in their data-model
/// form — numeric strings — and are parsed exactly the way L2 validates
/// them; a non-numeric key skips the whole default.
pub(crate) fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> Option<String> {
    if entries.is_empty() {
        return Some("{}".to_string());
    }
    if !scalar_default_type(&map.value_type) {
        return None;
    }
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        let key_lit = match map.key_type {
            MapKeyType::String => cpp_string_literal(key),
            MapKeyType::Int => cpp_int_literal(key.parse::<i64>().ok()?),
        };
        let value_lit = render_default(value, &map.value_type, schema)?;
        parts.push(format!("{{{key_lit}, {value_lit}}}"));
    }
    Some(format!("{{{}}}", parts.join(", ")))
}

/// Integer literal for `i64` values: plain decimal, except `i64::MIN`,
/// whose unsuffixed spelling (`-9223372036854775808`) is unary minus on an
/// unsigned constant and is rejected by `-Werror`.
pub(crate) fn cpp_int_literal(i: i64) -> String {
    if i == i64::MIN {
        "-9223372036854775807 - 1".to_string()
    } else {
        i.to_string()
    }
}

/// Integer literal for `u64` values: plain decimal up to `i64::MAX`, and a
/// `ULL`-suffixed decimal beyond it (an unsuffixed constant that does not
/// fit a signed type is ill-formed).
pub(crate) fn cpp_uint_literal(u: u64) -> String {
    if u > i64::MAX as u64 {
        format!("{u}ULL")
    } else {
        u.to_string()
    }
}

/// Float literal: finite values keep a decimal point; non-finite values map
/// to `std::numeric_limits<T>` expressions (C++ has no literal for them).
pub(crate) fn cpp_float_literal(f: f64, float32: bool) -> String {
    let type_name = if float32 { "float" } else { "double" };
    if f.is_nan() {
        return format!("std::numeric_limits<{type_name}>::quiet_NaN()");
    }
    if f.is_infinite() {
        let sign = if f.is_sign_negative() { "-" } else { "" };
        return format!("{sign}std::numeric_limits<{type_name}>::infinity()");
    }
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

pub(crate) fn cpp_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for C++ literals: `\\`, `\"`, `\n`, `\r`, `\t`;
/// other control characters (incl. U+007F) become 3-digit octal escapes —
/// C++ `\x` greedily consumes following hex digits, so octal is the safe
/// spelling. Printable Unicode passes through (UTF-8 source charset).
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
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// Whether every enum member carries an i64/u64 number (numeric bucket).
pub(crate) fn is_integral_enum(e: &EnumSchema) -> bool {
    !e.values.is_empty()
        && e.values.iter().all(|v| {
            matches!(
                &v.value,
                Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64()
            )
        })
}

/// Backing type of an `enum class`, selected by value range: fit in i32 →
/// `std::int32_t`, fit in i64 → `std::int64_t`, otherwise `std::uint64_t`.
pub(crate) fn enum_backing(e: &EnumSchema) -> &'static str {
    let mut min = i128::MAX;
    let mut max = i128::MIN;
    for v in &e.values {
        let n = match &v.value {
            Some(serde_json::Value::Number(num)) => {
                if let Some(i) = num.as_i64() {
                    i128::from(i)
                } else if let Some(u) = num.as_u64() {
                    i128::from(u)
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        min = min.min(n);
        max = max.max(n);
    }
    if min >= i128::from(i32::MIN) && max <= i128::from(i32::MAX) {
        "std::int32_t"
    } else if min >= i128::from(i64::MIN) && max <= i128::from(i64::MAX) {
        "std::int64_t"
    } else {
        "std::uint64_t"
    }
}

/// Enumerator value of a numeric-bucket enum member (decimal literal).
pub(crate) fn integral_value_literal(v: &EnumValue) -> String {
    match &v.value {
        Some(serde_json::Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                cpp_int_literal(i)
            } else if let Some(u) = n.as_u64() {
                cpp_uint_literal(u)
            } else {
                n.to_string()
            }
        }
        // Unreachable for the numeric bucket; keep the member usable.
        _ => cpp_string_literal(&v.name),
    }
}

/// Value of a string-bucket enum member: String as-is, Number as decimal
/// string, Bool as `"true"`/`"false"`, missing → member name.
pub(crate) fn string_bucket_value(v: &EnumValue) -> String {
    match &v.value {
        Some(serde_json::Value::String(s)) => cpp_string_literal(s),
        Some(serde_json::Value::Number(n)) => cpp_string_literal(&n.to_string()),
        Some(serde_json::Value::Bool(b)) => cpp_string_literal(if *b { "true" } else { "false" }),
        _ => cpp_string_literal(&v.name),
    }
}
/// Field comment body: description plus constraint summary; `None` when
/// there is nothing to say (same format as the other generators).
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
