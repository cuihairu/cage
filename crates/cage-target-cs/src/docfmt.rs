//! C# XML doc-comment text: XML escaping and field doc bodies (split out
//! of lib.rs, LAY-019).

use cage_core::schema::{FieldSchema, FieldType, Schema};

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Field doc comment body: description plus constraint summary (already
/// XML-escaped); `None` when there is nothing to say.
pub(crate) fn field_doc(schema: &Schema, field: &FieldSchema) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = &field.description {
        parts.push(xml_escape(d));
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
        parts.push(format!("pattern: {}", xml_escape(p)));
    }
    if let Some(vals) = &field.enum_values {
        let allowed: Vec<String> = vals.iter().map(|v| xml_escape(v)).collect();
        parts.push(format!("allowed: {}", allowed.join(" | ")));
    }
    if let Some(r) = &field.reference {
        parts.push(format!(
            "→ {}.{}",
            xml_escape(&r.table),
            xml_escape(&r.field)
        ));
    }
    if let FieldType::Enum(name) = &field.field_type {
        if schema.enums.get(name).is_none_or(|e| e.values.is_empty()) {
            parts.push(format!("unresolved enum: {}", xml_escape(name)));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}
