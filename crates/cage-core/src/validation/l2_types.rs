//! L2 type validation pass (value-vs-FieldType matching).
//! Split out of the validation module (LAY-003).

use super::{type_val, DiagnosticBuilder, FieldType, MapKeyType, ValidationContext, Value};

pub(crate) fn validate_type(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue; // Unknown table, already warned in L1
        };

        for row in &table.rows {
            for (field_name, typed_value) in &row.fields {
                let Some(field_schema) = table_schema.fields.get(field_name) else {
                    continue; // Unknown field, already warned in L1
                };

                // 回流-2: a nullable field accepts a present Null (Excel empty
                // cell / explicit null) without weakening the type to Any.
                let matches = (field_schema.nullable && typed_value.value == Value::Null)
                    || value_matches_type(&typed_value.value, &field_schema.field_type);
                if matches {
                    // Store resolved type for later stages
                    // (in practice, we'd mutate typed_value, but it's borrowed)
                } else {
                    let loc = typed_value.location.clone();
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(type_val::E1101, "Type mismatch")
                            .location(loc)
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .field(field_name)
                            .value(serde_json::to_value(&typed_value.value).unwrap_or_default())
                            .hint(type_mismatch_hint(
                                &field_schema.field_type,
                                typed_value.value.type_name(),
                            ))
                            .build(),
                    );
                }
            }
        }
    }
}

pub(crate) fn value_matches_type(value: &Value, expected: &FieldType) -> bool {
    match (value, expected) {
        (Value::Array(arr), FieldType::Array(inner)) => {
            arr.iter().all(|v| value_matches_type(v, inner))
        }
        (Value::Object(obj), FieldType::Object(fields)) => {
            obj.keys().all(|k| fields.contains_key(k))
                && obj
                    .iter()
                    .all(|(k, v)| fields.get(k).is_some_and(|ft| value_matches_type(v, ft)))
        }
        // Tagged union (回流-6): the discriminant key must hold a branch key,
        // and every other key must be a payload field of that branch with a
        // matching type. A key belonging to another branch is rejected — the
        // type safety the enum + Any-payload workaround loses.
        (Value::Object(obj), FieldType::TaggedUnion(tu)) => {
            let Some(Value::String(tag_val)) = obj.get(&tu.tag) else {
                return false; // discriminant missing or not a string
            };
            let Some(branch) = tu.branches.get(tag_val) else {
                return false; // unknown branch
            };
            obj.iter().all(|(k, v)| {
                k == &tu.tag || branch.get(k).is_some_and(|ft| value_matches_type(v, ft))
            })
        }
        // Map: an open-ended object — key_type gates which keys are legal
        // (int keys are numeric strings in the data model), value_type gates
        // every value recursively. An empty object is always a valid map.
        (Value::Object(obj), FieldType::Map(map)) => {
            obj.iter().all(|(k, _)| match map.key_type {
                MapKeyType::String => true, // Value::Object keys are String by construction
                MapKeyType::Int => k.parse::<i64>().is_ok(),
            }) && obj
                .iter()
                .all(|(_, v)| value_matches_type(v, &map.value_type))
        }
        // Enum membership itself is checked in L3 (value constraints)
        (Value::String(_), FieldType::Enum(_)) => true,
        // Scalars: only the family must match; width checking happens in L3.
        // The number tower is unified upward (回流-4): an integer value is a
        // valid float, so `Int`/`UInt` cells (an Excel whole-number cell loads
        // as `Int`) are accepted on `Float32`/`Float64` fields. The reverse
        // (`Float` into an integer field) is rejected.
        _ if matches!(
            (value, expected),
            (Value::Null, FieldType::Null)
                | (Value::Bool(_), FieldType::Bool)
                | (
                    Value::Int(_),
                    FieldType::Int8 | FieldType::Int16 | FieldType::Int32 | FieldType::Int64
                )
                | (
                    Value::UInt(_),
                    FieldType::UInt8 | FieldType::UInt16 | FieldType::UInt32 | FieldType::UInt64
                )
                | (
                    Value::Float(_) | Value::Int(_) | Value::UInt(_),
                    FieldType::Float32 | FieldType::Float64
                )
                | (Value::String(_), FieldType::String)
                | (Value::Bytes(_), FieldType::Bytes)
                | (_, FieldType::Any)
        ) =>
        {
            true
        }
        _ => false,
    }
}

/// Hint text for an E1101 type mismatch.
///
/// Non-map expectations keep the plain `Debug` rendering byte-for-byte
/// (`Expected type: Int32, got: string`); a map expectation is spelled
/// `map<string, Array(Int32)>`-style because `Debug` on the payload struct
/// (`Map(MapField { key_type: String, value_type: ... })`) reads poorly.
pub(crate) fn type_mismatch_hint(expected: &FieldType, got: &str) -> String {
    match expected {
        FieldType::Map(map) => format!(
            "Expected type: map<{}, {:?}>, got: {got}",
            match map.key_type {
                MapKeyType::String => "string",
                MapKeyType::Int => "int",
            },
            map.value_type
        ),
        FieldType::TaggedUnion(tu) => format!(
            "Expected type: tagged union on key {:?} with branches [{}], got: {got}",
            tu.tag,
            tu.branches.keys().cloned().collect::<Vec<_>>().join(", ")
        ),
        _ => format!("Expected type: {expected:?}, got: {got}"),
    }
}
