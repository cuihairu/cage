//! Default and literal rendering: schema defaults to Java initializers, scalar/array/map literals, enum value literals and field Javadoc (split out of lib.rs, LAY-008).

use super::types::field_enum;
use cage_core::schema::{EnumValue, FieldSchema, FieldType, MapField, MapKeyType, Schema};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Resolve an enum-typed field to the holder's allocated nested ident;
/// `None` when the enum is unknown or has no values (falls back to String).
pub(crate) fn resolve_enum<'e>(
    schema: &Schema,
    ft: &FieldType,
    enum_idents: &'e HashMap<String, String>,
) -> Option<&'e str> {
    let name = field_enum(ft)?;
    let e = schema.enums.get(name)?;
    if e.values.is_empty() {
        return None;
    }
    enum_idents.get(e.name.as_str()).map(String::as_str)
}

/// Render a schema default as a Java field initializer; `None` when the
/// default does not map to a compile-safe literal (objects, bytes, mismatched
/// or out-of-range kinds, Enum/Null/Any, maps with non-scalar values or more
/// entries than `Map.of` accepts — the field then keeps its wrapper or
/// reference type instead).
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
        | FieldType::UInt64 => value.as_i64().and_then(|i| int_literal(i, ft)),
        FieldType::Float32 => value.as_f64().map(|f| java_float_literal(f, true)),
        FieldType::Float64 => value.as_f64().map(|f| java_float_literal(f, false)),
        FieldType::String => value.as_str().map(java_string_literal),
        FieldType::Array(inner) => value
            .as_array()
            .and_then(|items| array_literal(items, inner, schema)),
        FieldType::Map(map) => value
            .as_object()
            .and_then(|entries| map_literal(entries, map, schema)),
        _ => None,
    }
}

/// Integer literal for a field's Java type: `long`-typed fields carry the
/// `L` suffix (`int`/`short`/`byte` widen silently), and a value that does
/// not fit the target type has no Java literal — it returns `None`, the
/// same "no safe literal" bucket as kind mismatches. A u64 above
/// `i64::MAX` never reaches here (`as_i64` fails) — Java is signed-only.
pub(crate) fn int_literal(i: i64, ft: &FieldType) -> Option<String> {
    let (fits, long) = match ft {
        FieldType::Int8 => (i8::try_from(i).is_ok(), false),
        FieldType::UInt8 => (u8::try_from(i).is_ok(), false),
        FieldType::Int16 => (i16::try_from(i).is_ok(), false),
        FieldType::UInt16 => (u16::try_from(i).is_ok(), false),
        FieldType::Int32 => (i32::try_from(i).is_ok(), false),
        FieldType::UInt32 => (u32::try_from(i).is_ok(), true),
        FieldType::Int64 => (true, true),
        FieldType::UInt64 => (i >= 0, true),
        _ => return None,
    };
    if !fits {
        return None;
    }
    Some(if long { format!("{i}L") } else { i.to_string() })
}

/// Whether a field type is a scalar with a renderable literal — the shared
/// rule that gates container defaults (array elements, map values) across
/// targets.
pub(crate) fn is_scalar_value(ft: &FieldType) -> bool {
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

/// Array defaults render only for scalar element types (the shared rule
/// across targets) as a fresh mutable list per instance — Java arrays and
/// `List.of` are immutable, but field initializers re-run per instance, so
/// `new ArrayList<>(List.of(…))` never shares state between rows.
pub(crate) fn array_literal(
    items: &[serde_json::Value],
    inner: &FieldType,
    schema: &Schema,
) -> Option<String> {
    if !is_scalar_value(inner) {
        return None;
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(render_default(item, inner, schema)?);
    }
    Some(format!("new ArrayList<>(List.of({}))", parts.join(", ")))
}

/// Map defaults follow the array-default rules: a fresh mutable instance
/// per row (`Map.of(…)` is immutable, but field initializers re-run per
/// instance, so `new HashMap<>(…)` never shares state between rows). The
/// empty default is the bare `new HashMap<>()`; entries render only for
/// scalar value types (the shared rule) and only when every key parses
/// under the declared key type — otherwise the initializer is skipped
/// entirely, the same all-or-nothing bucket as array defaults. Entries are
/// emitted in `serde_json`'s key order (sorted without `preserve_order`),
/// and more than ten pairs have no `Map.of` overload — also skipped.
pub(crate) fn map_literal(
    entries: &serde_json::Map<String, serde_json::Value>,
    map: &MapField,
    schema: &Schema,
) -> Option<String> {
    if entries.is_empty() {
        return Some("new HashMap<>()".to_string());
    }
    if entries.len() > 10 || !is_scalar_value(&map.value_type) {
        return None;
    }
    let mut parts = Vec::with_capacity(entries.len());
    // Sort keys: serde_json's map order follows feature unification
    // (BTreeMap by default, insertion order with preserve_order).
    let mut sorted: Vec<(&String, &serde_json::Value)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in sorted {
        // String keys quote; int keys are stored as numeric strings
        // ("42", "-7") in the data model and render as `long` literals —
        // a bare int would infer Map<Integer, …>, which does not copy
        // into a HashMap<Long, …>.
        let key_lit = match map.key_type {
            MapKeyType::String => java_string_literal(key),
            MapKeyType::Int => {
                let n: i64 = key.parse().ok()?;
                format!("{n}L")
            }
        };
        let value_lit = render_default(value, &map.value_type, schema)?;
        parts.push(format!("{key_lit}, {value_lit}"));
    }
    Some(format!("new HashMap<>(Map.of({}))", parts.join(", ")))
}

/// Java float literal. Non-finite values have no literal — they use the
/// `java.lang` constants (`Float.NaN`, `Double.POSITIVE_INFINITY`, …, no
/// import needed); finite values keep a decimal point, and `float`-typed
/// literals carry the mandatory `f` suffix.
pub(crate) fn java_float_literal(f: f64, float32: bool) -> String {
    if f.is_nan() {
        return if float32 {
            "Float.NaN".to_string()
        } else {
            "Double.NaN".to_string()
        };
    }
    if f.is_infinite() {
        let class = if float32 { "Float" } else { "Double" };
        let sign = if f < 0.0 { "NEGATIVE" } else { "POSITIVE" };
        return format!("{class}.{sign}_INFINITY");
    }
    let mut s = f.to_string();
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        s.push_str(".0");
    }
    if float32 {
        s.push('f');
    }
    s
}

pub(crate) fn java_string_literal(s: &str) -> String {
    format!("\"{}\"", escape_string_content(s))
}

/// Escape string content for Java literals: `\\` / `\"` / `\n` / `\r` /
/// `\t` keep their short escapes, other control characters (incl. DEL)
/// become `\u00XX` with exactly four hex digits — Java processes `\uXXXX`
/// even before parsing, so a raw `\u000a` would inject a newline; CR/LF/TAB
/// therefore must use the short forms. Printable Unicode passes through.
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
                let _ = write!(out, "\\u{code:04x}");
            }
            c => out.push(c),
        }
    }
    out
}

/// Field Javadoc body: description plus constraint summary (identical part
/// list/order/format as the Python/Lua generators), plus the Java-specific
/// caveat for `UInt64` fields; `None` when there is nothing to say.
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
    if matches!(field.field_type, FieldType::UInt64) {
        parts.push("values above Long.MAX_VALUE do not fit Java's long".to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Whether one enum member's integral value fits Java's `int` payload
/// (u64 above `i64::MAX` does not — it forces the `long` backing).
pub(crate) fn value_fits_i32(v: &EnumValue) -> bool {
    match &v.value {
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .is_some_and(|i| i >= i64::from(i32::MIN) && i <= i64::from(i32::MAX)),
        _ => false,
    }
}

/// Payload literal for one integral member, plus an optional trailing
/// note. Values above `i64::MAX` have no Java literal — they are wrapped
/// to their two's-complement `long` and flagged (Java has no unsigned
/// 64-bit type, so the wrapped negative is the faithful bit pattern).
pub(crate) fn integral_literal(v: &EnumValue, long_backing: bool) -> (String, Option<String>) {
    let Some(serde_json::Value::Number(n)) = &v.value else {
        return (v.name.clone(), None);
    };
    if let Some(i) = n.as_i64() {
        let lit = if long_backing {
            format!("{i}L")
        } else {
            i.to_string()
        };
        return (lit, None);
    }
    if let Some(u) = n.as_u64() {
        let wrapped = i64::from_ne_bytes(u.to_ne_bytes());
        let lit = if long_backing {
            format!("{wrapped}L")
        } else {
            wrapped.to_string()
        };
        return (lit, Some(format!("wrapped from unsigned {u}")));
    }
    (v.name.clone(), None)
}

/// String-bucket member value: String as-is / Number as a decimal string /
/// Bool as `"true"`/`"false"` / missing values fall back to the member
/// name (Cage compares enums as strings — same rule as py/lua).
pub(crate) fn string_bucket_value(v: &EnumValue) -> String {
    match &v.value {
        Some(serde_json::Value::String(s)) => java_string_literal(s),
        Some(serde_json::Value::Number(n)) => java_string_literal(&n.to_string()),
        Some(serde_json::Value::Bool(b)) => java_string_literal(if *b { "true" } else { "false" }),
        _ => java_string_literal(&v.name),
    }
}
