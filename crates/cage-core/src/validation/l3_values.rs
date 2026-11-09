//! L3 value-constraint validation pass (field constraints).
//! Split out of the validation module (LAY-003).

use super::{
    value, DiagnosticBuilder, FieldSchema, FieldType, Row, TypedValue, ValidationContext, Value,
};

pub(crate) fn validate_value(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue;
        };

        for row in &table.rows {
            for (field_name, typed_value) in &row.fields {
                let Some(field_schema) = table_schema.fields.get(field_name) else {
                    continue;
                };

                validate_field_constraints(
                    ctx,
                    table_name,
                    row,
                    field_name,
                    typed_value,
                    field_schema,
                );
            }
        }
    }
}

pub(crate) fn validate_field_constraints(
    ctx: &mut ValidationContext,
    table_name: &str,
    row: &Row,
    field_name: &str,
    typed_value: &TypedValue,
    field_schema: &FieldSchema,
) {
    let value = &typed_value.value;
    let loc = typed_value.location.clone();

    // Numeric range
    if let Some(min) = field_schema.min {
        if let Some(num) = value.coerce_to_float() {
            if num < min {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1201, "Value below minimum")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Minimum allowed: {min}"))
                        .build(),
                );
            }
        }
    }
    if let Some(max) = field_schema.max {
        if let Some(num) = value.coerce_to_float() {
            if num > max {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1201, "Value exceeds maximum")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Maximum allowed: {max}"))
                        .build(),
                );
            }
        }
    }

    // String length
    if let Value::String(s) = value {
        if let Some(min_len) = field_schema.min_length {
            if s.len() < min_len {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1202, "String too short")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Minimum length: {min_len}"))
                        .build(),
                );
            }
        }
        if let Some(max_len) = field_schema.max_length {
            if s.len() > max_len {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1202, "String too long")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Maximum length: {max_len}"))
                        .build(),
                );
            }
        }
        // Pattern
        if let Some(pattern) = &field_schema.pattern {
            if let Ok(re) = regex::Regex::new(pattern) {
                if !re.is_match(s) {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(value::E1203, "String does not match pattern")
                            .location(loc.clone())
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .field(field_name)
                            .value(serde_json::to_value(value).unwrap_or_default())
                            .hint(format!("Pattern: {pattern}"))
                            .build(),
                    );
                }
            }
        }
    }

    // Enum values
    if let Some(enum_values) = &field_schema.enum_values {
        let val_str = value.coerce_to_string().unwrap_or_default();
        if !enum_values.contains(&val_str) {
            ctx.diagnostics.add(
                DiagnosticBuilder::error(value::E1204, "Value not in allowed enum")
                    .location(loc.clone())
                    .table(table_name)
                    .row(format!("{}", row.index))
                    .field(field_name)
                    .value(serde_json::to_value(value).unwrap_or_default())
                    .hint(format!("Allowed values: {}", enum_values.join(", ")))
                    .build(),
            );
        }
    }

    // Named enum membership (`type: {kind: Enum, value: X}`): the value
    // must be one of the declared members of the schema-level enum. This
    // is a disjoint surface from inline `enum_values` (above) and
    // previously had no check at all — L2 passes any String for Enum
    // (see `value_matches_type`). Only well-typed strings are checked:
    // a non-string value is an L2 type mismatch (E1101), already
    // reported there — no double diagnosis. A dangling enum name is an
    // L1 schema check (E1004), so an unknown name falls through here.
    if let FieldType::Enum(enum_name) = &field_schema.field_type {
        if let (Some(enum_schema), Value::String(val_str)) =
            (ctx.schema.schema.enums.get(enum_name), value)
        {
            let members: Vec<&str> = enum_schema.values.iter().map(|v| v.name.as_str()).collect();
            if !members.contains(&val_str.as_str()) {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1204, "Value not in allowed enum")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Allowed values: {}", members.join(", ")))
                        .build(),
                );
            }
        }
    }

    // Array length
    if let Value::Array(arr) = value {
        if let Some(min_items) = field_schema.min_items {
            if arr.len() < min_items {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1205, "Array too short")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Minimum items: {min_items}"))
                        .build(),
                );
            }
        }
        if let Some(max_items) = field_schema.max_items {
            if arr.len() > max_items {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1205, "Array too long")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Maximum items: {max_items}"))
                        .build(),
                );
            }
        }
    }
}
