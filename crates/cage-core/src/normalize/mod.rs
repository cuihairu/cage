//! Normalize - canonical representation for deterministic builds
//! Ensures semantically equivalent values produce identical canonical forms

use crate::value::{TypedValue, Value};
use indexmap::IndexMap;
use std::collections::BTreeMap;

/// Normalize a value to its canonical form
pub fn normalize_value(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Int(i) => normalize_int(*i),
        Value::UInt(u) => normalize_uint(*u),
        Value::Float(f) => normalize_float(*f),
        Value::String(s) => normalize_string(s),
        Value::Bytes(b) => Value::Bytes(b.clone()),
        Value::Array(arr) => normalize_array(arr),
        Value::Object(obj) => normalize_object(obj),
    }
}

fn normalize_int(i: i64) -> Value {
    // The canonical Value is width-untagged (i64); width checking happens at
    // L2 schema validation, so there is nothing to narrow here yet.
    Value::Int(i)
}

fn normalize_uint(u: u64) -> Value {
    // The canonical Value is width-untagged (u64); width checking happens at
    // L2 schema validation, so there is nothing to narrow here yet.
    Value::UInt(u)
}

fn normalize_float(f: f64) -> Value {
    // Canonicalize -0.0 to 0.0; NaN/Inf pass through unchanged for reproducibility
    Value::Float(if f == 0.0 { 0.0 } else { f })
}

fn normalize_string(s: &str) -> Value {
    // Trim whitespace, normalize line endings
    let normalized = s.trim().replace('\r', "");
    Value::String(normalized)
}

fn normalize_array(arr: &[Value]) -> Value {
    Value::Array(arr.iter().map(normalize_value).collect())
}

fn normalize_object(obj: &IndexMap<String, Value>) -> Value {
    // Sort keys for deterministic ordering
    let mut sorted: BTreeMap<String, Value> = BTreeMap::new();
    for (k, v) in obj {
        sorted.insert(k.clone(), normalize_value(v));
    }
    Value::Object(sorted.into_iter().collect())
}

/// Normalize a typed value (preserves location)
pub fn normalize_typed_value(typed: &TypedValue) -> TypedValue {
    TypedValue {
        value: normalize_value(&typed.value),
        location: typed.location.clone(),
        schema_type: typed.schema_type.clone(),
    }
}

/// Normalize entire document
pub fn normalize_document(doc: &crate::value::Document) -> crate::value::Document {
    let mut normalized = crate::value::Document::new();
    normalized.source_files.clone_from(&doc.source_files);
    normalized.metadata = doc.metadata.clone();

    for table in doc.tables.values() {
        let mut norm_table = crate::value::Table {
            name: table.name.clone(),
            primary_key_fields: table.primary_key_fields.clone(),
            rows: Vec::new(),
            source_file: table.source_file.clone(),
            sheet: table.sheet.clone(),
        };

        for row in &table.rows {
            let mut norm_fields = IndexMap::new();
            for (field_name, typed_value) in &row.fields {
                norm_fields.insert(field_name.clone(), normalize_typed_value(typed_value));
            }

            norm_table.rows.push(crate::value::Row {
                primary_key: row.primary_key.iter().map(normalize_value).collect(),
                fields: norm_fields,
                location: row.location.clone(),
                index: row.index,
            });
        }

        normalized.add_table(norm_table);
    }

    normalized
}

/// Coerce value to target type based on schema (for normalization phase)
pub fn coerce_to_type(value: &Value, target_type: &crate::schema::FieldType) -> Option<Value> {
    match (value, target_type) {
        // Already correct type
        (Value::Null, crate::schema::FieldType::Null) => Some(Value::Null),
        (Value::Bool(_), crate::schema::FieldType::Bool)
        | (
            Value::Int(_),
            crate::schema::FieldType::Int8
            | crate::schema::FieldType::Int16
            | crate::schema::FieldType::Int32
            | crate::schema::FieldType::Int64,
        )
        | (
            Value::UInt(_),
            crate::schema::FieldType::UInt8
            | crate::schema::FieldType::UInt16
            | crate::schema::FieldType::UInt32
            | crate::schema::FieldType::UInt64,
        )
        | (
            Value::Float(_),
            crate::schema::FieldType::Float32 | crate::schema::FieldType::Float64,
        )
        | (Value::String(_), crate::schema::FieldType::String)
        | (Value::Bytes(_), crate::schema::FieldType::Bytes) => Some(value.clone()),

        // String -> numeric coercion
        (
            Value::String(s),
            crate::schema::FieldType::Int8
            | crate::schema::FieldType::Int16
            | crate::schema::FieldType::Int32
            | crate::schema::FieldType::Int64,
        ) => s.parse::<i64>().ok().map(Value::Int),
        (
            Value::String(s),
            crate::schema::FieldType::UInt8
            | crate::schema::FieldType::UInt16
            | crate::schema::FieldType::UInt32
            | crate::schema::FieldType::UInt64,
        ) => s.parse::<u64>().ok().map(Value::UInt),
        (
            Value::String(s),
            crate::schema::FieldType::Float32 | crate::schema::FieldType::Float64,
        ) => s.parse::<f64>().ok().map(Value::Float),

        // String -> bool coercion (common in Excel/CSV)
        (Value::String(s), crate::schema::FieldType::Bool) => {
            let s = s.trim().to_lowercase();
            match s.as_str() {
                "true" | "yes" | "1" | "on" | "enabled" | "y" | "t" => Some(Value::Bool(true)),
                "false" | "no" | "0" | "off" | "disabled" | "n" | "f" => Some(Value::Bool(false)),
                _ => None,
            }
        }

        // Numeric -> bool
        (Value::Int(i), crate::schema::FieldType::Bool) => Some(Value::Bool(*i != 0)),
        (Value::UInt(u), crate::schema::FieldType::Bool) => Some(Value::Bool(*u != 0)),
        (Value::Float(f), crate::schema::FieldType::Bool) => Some(Value::Bool(*f != 0.0)),

        // Int -> UInt (if non-negative)
        (
            Value::Int(i),
            crate::schema::FieldType::UInt8
            | crate::schema::FieldType::UInt16
            | crate::schema::FieldType::UInt32
            | crate::schema::FieldType::UInt64,
        ) if *i >= 0 => Some(Value::UInt(*i as u64)),

        // UInt -> Int (if fits)
        (
            Value::UInt(u),
            crate::schema::FieldType::Int8
            | crate::schema::FieldType::Int16
            | crate::schema::FieldType::Int32
            | crate::schema::FieldType::Int64,
        ) if *u <= i64::MAX as u64 => Some(Value::Int(*u as i64)),

        // Float -> Int/UInt (if whole number)
        (
            Value::Float(f),
            crate::schema::FieldType::Int8
            | crate::schema::FieldType::Int16
            | crate::schema::FieldType::Int32
            | crate::schema::FieldType::Int64,
        ) if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 => {
            Some(Value::Int(*f as i64))
        }
        (
            Value::Float(f),
            crate::schema::FieldType::UInt8
            | crate::schema::FieldType::UInt16
            | crate::schema::FieldType::UInt32
            | crate::schema::FieldType::UInt64,
        ) if f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64 => Some(Value::UInt(*f as u64)),

        // Array coercion
        (Value::Array(arr), crate::schema::FieldType::Array(inner)) => {
            let mut result = Vec::with_capacity(arr.len());
            for item in arr {
                result.push(coerce_to_type(item, inner)?);
            }
            Some(Value::Array(result))
        }

        // Object coercion
        (Value::Object(obj), crate::schema::FieldType::Object(fields)) => {
            let mut result = IndexMap::new();
            for (k, expected_type) in fields {
                {
                    let v = obj.get(k)?;
                    result.insert(k.clone(), coerce_to_type(v, expected_type)?);
                }
            }
            Some(Value::Object(result))
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldType;
    use crate::value::Value;

    #[test]
    fn test_normalize_string() {
        assert_eq!(
            normalize_value(&Value::String("  hello  ".to_string())),
            Value::String("hello".to_string())
        );
        assert_eq!(
            normalize_value(&Value::String("hello\r\nworld".to_string())),
            Value::String("hello\nworld".to_string())
        );
    }

    #[test]
    fn test_normalize_float() {
        assert_eq!(normalize_value(&Value::Float(-0.0)), Value::Float(0.0));
        assert_eq!(normalize_value(&Value::Float(1.5)), Value::Float(1.5));
    }

    #[test]
    fn test_normalize_passthrough_variants() {
        // Null, Bool and Bytes have no canonicalization to do; Int/UInt are
        // width-untagged in the canonical Value and pass through unchanged.
        assert_eq!(normalize_value(&Value::Null), Value::Null);
        assert_eq!(normalize_value(&Value::Bool(true)), Value::Bool(true));
        assert_eq!(normalize_value(&Value::Bool(false)), Value::Bool(false));
        assert_eq!(normalize_value(&Value::Int(-5)), Value::Int(-5));
        assert_eq!(normalize_value(&Value::Int(i64::MIN)), Value::Int(i64::MIN));
        assert_eq!(normalize_value(&Value::UInt(7)), Value::UInt(7));
        assert_eq!(
            normalize_value(&Value::UInt(u64::MAX)),
            Value::UInt(u64::MAX)
        );
        assert_eq!(
            normalize_value(&Value::Bytes(vec![1, 2, 3])),
            Value::Bytes(vec![1, 2, 3])
        );
    }

    #[test]
    fn test_normalize_object_sorts_keys() {
        let mut obj = IndexMap::new();
        obj.insert("z".to_string(), Value::Int(1));
        obj.insert("a".to_string(), Value::Int(2));
        let normalized = normalize_value(&Value::Object(obj));

        // IndexMap equality ignores order, so assert the iteration order
        // directly through the Debug rendering of the normalized object.
        let debug = format!("{normalized:?}");
        let a = debug.find(r#""a": Int(2)"#).expect("a key must exist");
        let z = debug.find(r#""z": Int(1)"#).expect("z key must exist");
        assert!(a < z, "keys must be sorted, got: {debug}");
    }

    #[test]
    fn test_normalize_array_recursively_in_order() {
        // Element order is preserved (it is data, not key ordering)
        let arr = Value::Array(vec![
            Value::String("  a  ".to_string()),
            Value::Float(-0.0),
            Value::Array(vec![Value::String("b\r\n".to_string())]),
        ]);
        assert_eq!(
            normalize_value(&arr),
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::Float(0.0),
                // trim() strips the trailing newline, and interior CR is dropped
                Value::Array(vec![Value::String("b".to_string())]),
            ])
        );
        // Interior carriage returns are removed, leading/trailing space trimmed
        assert_eq!(
            normalize_value(&Value::String(" a\r\nb ".to_string())),
            Value::String("a\nb".to_string())
        );
        // Empty containers normalize to themselves
        assert_eq!(normalize_value(&Value::Array(vec![])), Value::Array(vec![]));
        let mut empty_obj = IndexMap::new();
        empty_obj.insert("only".to_string(), Value::Null);
        assert_eq!(
            normalize_value(&Value::Object(empty_obj)),
            Value::Object({
                let mut m = IndexMap::new();
                m.insert("only".to_string(), Value::Null);
                m
            })
        );
    }

    #[test]
    fn test_normalize_typed_value_preserves_metadata() {
        let typed = TypedValue {
            value: Value::String("  pad  ".to_string()),
            location: crate::value::SourceLocation::new("cells.xlsx").with_row(4),
            schema_type: Some("String".to_string()),
        };
        let normalized = normalize_typed_value(&typed);
        assert_eq!(normalized.value, Value::String("pad".to_string()));
        assert_eq!(normalized.location, typed.location);
        assert_eq!(normalized.schema_type.as_deref(), Some("String"));

        // Without schema type, it stays absent
        let plain = normalize_typed_value(&TypedValue::new(
            Value::Float(-0.0),
            crate::value::SourceLocation::new("cells.xlsx"),
        ));
        assert_eq!(plain.value, Value::Float(0.0));
        assert!(plain.schema_type.is_none());
    }

    #[test]
    fn test_normalize_document_canonicalizes_and_is_deterministic() {
        use crate::value::{Document, Row, Table};

        let mut doc = Document::new();
        doc.metadata.format = "json".to_string();
        doc.metadata.parsed_at = Some("2024-01-01T00:00:00Z".to_string());
        doc.metadata
            .custom
            .insert("origin".to_string(), Value::String("xlsx".to_string()));

        let mut meta_obj = IndexMap::new();
        meta_obj.insert("z".to_string(), Value::Int(1));
        meta_obj.insert("a".to_string(), Value::Int(2));
        let mut fields = IndexMap::new();
        fields.insert(
            "name".to_string(),
            TypedValue::new(
                Value::String("  Sword \r\n".to_string()),
                crate::value::SourceLocation::new("items.json").with_row(1),
            ),
        );
        fields.insert(
            "ratio".to_string(),
            TypedValue::new(
                Value::Float(-0.0),
                crate::value::SourceLocation::new("items.json").with_row(1),
            ),
        );
        fields.insert(
            "tags".to_string(),
            TypedValue::new(
                Value::Array(vec![Value::String(" x ".to_string()), Value::Float(-0.0)]),
                crate::value::SourceLocation::new("items.json").with_row(1),
            ),
        );
        fields.insert(
            "meta".to_string(),
            TypedValue::new(
                Value::Object(meta_obj),
                crate::value::SourceLocation::new("items.json").with_row(1),
            ),
        );

        let mut table = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![Row {
                primary_key: vec![Value::String("  1 ".to_string())],
                fields,
                location: crate::value::SourceLocation::new("items.json").with_row(1),
                index: 0,
            }],
            source_file: "items.json".to_string(),
            sheet: None,
        };
        // A second row keeps its index; an empty table survives as-is
        table.rows.push(Row {
            primary_key: vec![Value::UInt(2)],
            fields: IndexMap::new(),
            location: crate::value::SourceLocation::new("items.json").with_row(2),
            index: 1,
        });
        doc.add_table(table);
        doc.add_table(Table {
            name: "Empty".to_string(),
            primary_key_fields: vec![],
            rows: vec![],
            source_file: "empty.json".to_string(),
            sheet: Some("Sheet1".to_string()),
        });

        let normalized = normalize_document(&doc);

        assert_eq!(normalized.tables.len(), 2);
        // add_table appends each table's source_file, so the normalized
        // document re-records one entry per table; every original entry must
        // still survive normalization.
        for f in &doc.source_files {
            assert!(normalized.source_files.contains(f), "lost source file {f}");
        }
        assert_eq!(normalized.metadata.format, "json");
        assert_eq!(
            normalized.metadata.parsed_at.as_deref(),
            Some("2024-01-01T00:00:00Z")
        );

        let item = &normalized.tables["Item"];
        assert_eq!(item.name, "Item");
        assert_eq!(item.source_file, "items.json");
        assert!(item.sheet.is_none());
        assert_eq!(item.rows.len(), 2);
        assert_eq!(item.rows[0].index, 0);

        let row = &item.rows[0];
        // Primary key values are normalized too
        assert_eq!(row.primary_key, vec![Value::String("1".to_string())]);
        // Field insertion order is preserved
        assert_eq!(
            row.fields.keys().collect::<Vec<_>>(),
            vec!["name", "ratio", "tags", "meta"]
        );
        assert_eq!(
            row.fields["name"].value,
            // trim() removes the trailing CRLF entirely
            Value::String("Sword".to_string())
        );
        assert_eq!(row.fields["ratio"].value, Value::Float(0.0));
        assert_eq!(
            row.fields["tags"].value,
            Value::Array(vec![Value::String("x".to_string()), Value::Float(0.0)])
        );
        // Nested object keys are sorted
        assert!(format!("{:?}", row.fields["meta"].value).contains(r#""a": Int(2), "z": Int(1)"#));
        // Locations survive normalization
        assert_eq!(row.fields["name"].location, row.location);

        let empty = &normalized.tables["Empty"];
        assert!(empty.rows.is_empty());
        assert_eq!(empty.sheet.as_deref(), Some("Sheet1"));

        // Same document normalized twice yields byte-identical output
        let again = normalize_document(&doc);
        assert_eq!(
            serde_json::to_string(&normalized).unwrap(),
            serde_json::to_string(&again).unwrap()
        );
    }

    #[test]
    fn test_coerce_identity_types() {
        assert_eq!(
            coerce_to_type(&Value::Null, &FieldType::Null),
            Some(Value::Null)
        );
        assert_eq!(
            coerce_to_type(&Value::Bool(false), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        for ft in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
        ] {
            assert_eq!(coerce_to_type(&Value::Int(-3), &ft), Some(Value::Int(-3)));
        }
        for ft in [
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
        ] {
            assert_eq!(coerce_to_type(&Value::UInt(3), &ft), Some(Value::UInt(3)));
        }
        for ft in [FieldType::Float32, FieldType::Float64] {
            assert_eq!(
                coerce_to_type(&Value::Float(2.5), &ft),
                Some(Value::Float(2.5))
            );
        }
        assert_eq!(
            coerce_to_type(&Value::String("x".to_string()), &FieldType::String),
            Some(Value::String("x".to_string()))
        );
        assert_eq!(
            coerce_to_type(&Value::Bytes(vec![9]), &FieldType::Bytes),
            Some(Value::Bytes(vec![9]))
        );
    }

    #[test]
    fn test_coerce_string_to_int() {
        assert_eq!(
            coerce_to_type(&Value::String("42".to_string()), &FieldType::Int32),
            Some(Value::Int(42))
        );
        assert_eq!(
            coerce_to_type(&Value::String("42".to_string()), &FieldType::UInt32),
            Some(Value::UInt(42))
        );
        assert_eq!(
            coerce_to_type(&Value::String("3.14".to_string()), &FieldType::Float64),
            Some(Value::Float(3.14))
        );
    }

    #[test]
    fn test_coerce_string_to_bool() {
        assert_eq!(
            coerce_to_type(&Value::String("true".to_string()), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::String("false".to_string()), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::String("yes".to_string()), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::String("no".to_string()), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::String("1".to_string()), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::String("0".to_string()), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::String("on".to_string()), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::String("off".to_string()), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::String("enabled".to_string()), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::String("disabled".to_string()), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::String("invalid".to_string()), &FieldType::Bool),
            None
        );
    }

    #[test]
    fn test_coerce_numeric_to_bool() {
        assert_eq!(
            coerce_to_type(&Value::Int(0), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::Int(1), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::UInt(0), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::UInt(42), &FieldType::Bool),
            Some(Value::Bool(true))
        );
        assert_eq!(
            coerce_to_type(&Value::Float(0.0), &FieldType::Bool),
            Some(Value::Bool(false))
        );
        assert_eq!(
            coerce_to_type(&Value::Float(1.5), &FieldType::Bool),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn test_coerce_int_to_uint() {
        assert_eq!(
            coerce_to_type(&Value::Int(42), &FieldType::UInt32),
            Some(Value::UInt(42))
        );
        assert_eq!(coerce_to_type(&Value::Int(-1), &FieldType::UInt32), None);
    }

    #[test]
    fn test_coerce_float_to_int() {
        assert_eq!(
            coerce_to_type(&Value::Float(42.0), &FieldType::Int32),
            Some(Value::Int(42))
        );
        assert_eq!(coerce_to_type(&Value::Float(3.14), &FieldType::Int32), None);
    }

    #[test]
    fn test_coerce_string_bool_case_and_whitespace_insensitive() {
        for s in ["YES", "Yes", " TRUE ", "\ty", "t", "ON", "Enabled"] {
            assert_eq!(
                coerce_to_type(&Value::String(s.to_string()), &FieldType::Bool),
                Some(Value::Bool(true)),
                "expected {s:?} -> true"
            );
        }
        for s in ["NO", "No", " FALSE ", "n", "f", "OFF", "Disabled"] {
            assert_eq!(
                coerce_to_type(&Value::String(s.to_string()), &FieldType::Bool),
                Some(Value::Bool(false)),
                "expected {s:?} -> false"
            );
        }
    }

    #[test]
    fn test_coerce_string_parse_failures() {
        for s in ["abc", "", "1.5", "12x", " 12 , 34"] {
            assert_eq!(
                coerce_to_type(&Value::String(s.to_string()), &FieldType::Int32),
                None,
                "expected {s:?} -> no Int"
            );
        }
        for s in ["abc", "", "-1", "1.5"] {
            assert_eq!(
                coerce_to_type(&Value::String(s.to_string()), &FieldType::UInt32),
                None,
                "expected {s:?} -> no UInt"
            );
        }
        for s in ["abc", "", "1,5"] {
            assert_eq!(
                coerce_to_type(&Value::String(s.to_string()), &FieldType::Float64),
                None,
                "expected {s:?} -> no Float"
            );
        }
        // Every width parses (or rejects) the same textual forms
        for ft in [FieldType::Int8, FieldType::Int16, FieldType::Int64] {
            assert_eq!(
                coerce_to_type(&Value::String("-7".to_string()), &ft),
                Some(Value::Int(-7))
            );
        }
        for ft in [FieldType::UInt8, FieldType::UInt16, FieldType::UInt64] {
            assert_eq!(
                coerce_to_type(&Value::String("7".to_string()), &ft),
                Some(Value::UInt(7))
            );
        }
        for ft in [FieldType::Float32, FieldType::Float64] {
            assert_eq!(
                coerce_to_type(&Value::String("-0.5".to_string()), &ft),
                Some(Value::Float(-0.5))
            );
        }
    }

    #[test]
    fn test_coerce_int_to_uint_all_widths() {
        for ft in [
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
        ] {
            assert_eq!(
                coerce_to_type(&Value::Int(7), &ft),
                Some(Value::UInt(7)),
                "expected Int(7) -> UInt for {ft:?}"
            );
            // Negative values are rejected by the non-negative guard
            assert_eq!(coerce_to_type(&Value::Int(-7), &ft), None);
        }
        // Zero is non-negative and coerces
        assert_eq!(
            coerce_to_type(&Value::Int(0), &FieldType::UInt8),
            Some(Value::UInt(0))
        );
    }

    #[test]
    fn test_coerce_uint_to_int() {
        for ft in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
        ] {
            assert_eq!(
                coerce_to_type(&Value::UInt(42), &ft),
                Some(Value::Int(42)),
                "expected UInt(42) -> Int for {ft:?}"
            );
        }
        // Out-of-range unsigned values do not fit any signed width
        let huge = Value::UInt(i64::MAX as u64 + 1);
        for ft in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
        ] {
            assert_eq!(
                coerce_to_type(&huge, &ft),
                None,
                "i64::MAX+1 must not coerce"
            );
        }
        // The largest in-range value still fits
        assert_eq!(
            coerce_to_type(&Value::UInt(i64::MAX as u64), &FieldType::Int64),
            Some(Value::Int(i64::MAX))
        );
    }

    #[test]
    fn test_coerce_float_to_integer_boundaries() {
        // Whole floats coerce in every width
        for ft in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
        ] {
            assert_eq!(
                coerce_to_type(&Value::Float(-3.0), &ft),
                Some(Value::Int(-3)),
                "expected -3.0 -> Int for {ft:?}"
            );
        }
        for ft in [
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
        ] {
            assert_eq!(
                coerce_to_type(&Value::Float(100.0), &ft),
                Some(Value::UInt(100)),
                "expected 100.0 -> UInt for {ft:?}"
            );
        }
        // Fractional floats never coerce to integers
        assert_eq!(coerce_to_type(&Value::Float(-0.5), &FieldType::Int32), None);
        assert_eq!(coerce_to_type(&Value::Float(0.5), &FieldType::UInt8), None);
        // Beyond the signed/unsigned ranges
        assert_eq!(
            coerce_to_type(&Value::Float(9.3e18), &FieldType::Int64),
            None
        );
        assert_eq!(
            coerce_to_type(&Value::Float(-9.3e18), &FieldType::Int64),
            None
        );
        assert_eq!(
            coerce_to_type(&Value::Float(1.9e19), &FieldType::UInt64),
            None
        );
        assert_eq!(coerce_to_type(&Value::Float(-1.0), &FieldType::UInt8), None);
        // Boundary values on both sides of the guard
        assert_eq!(
            coerce_to_type(&Value::Float(i64::MAX as f64), &FieldType::Int64),
            Some(Value::Int(i64::MAX as f64 as i64))
        );
        assert_eq!(
            coerce_to_type(&Value::Float(0.0), &FieldType::UInt8),
            Some(Value::UInt(0))
        );
    }

    #[test]
    fn test_coerce_array_elements() {
        let arr = Value::Array(vec![
            Value::String("1".to_string()),
            Value::String("2".to_string()),
        ]);
        assert_eq!(
            coerce_to_type(&arr, &FieldType::Array(Box::new(FieldType::Int32))),
            Some(Value::Array(vec![Value::Int(1), Value::Int(2)]))
        );
        // Empty array coerces to an empty array
        assert_eq!(
            coerce_to_type(
                &Value::Array(vec![]),
                &FieldType::Array(Box::new(FieldType::Int32))
            ),
            Some(Value::Array(vec![]))
        );
        // Nested arrays coerce recursively
        let nested = Value::Array(vec![Value::Array(vec![Value::Float(1.0)])]);
        assert_eq!(
            coerce_to_type(
                &nested,
                &FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float32))))
            ),
            Some(Value::Array(vec![Value::Array(vec![Value::Float(1.0)])]))
        );
        // One bad element fails the whole array
        let bad = Value::Array(vec![
            Value::String("1".to_string()),
            Value::String("x".to_string()),
        ]);
        assert_eq!(
            coerce_to_type(&bad, &FieldType::Array(Box::new(FieldType::Int32))),
            None
        );
        // Element type mismatch (array vs scalar target) is rejected
        assert_eq!(coerce_to_type(&arr, &FieldType::Int32), None);
        assert_eq!(
            coerce_to_type(
                &Value::Int(1),
                &FieldType::Array(Box::new(FieldType::Int32))
            ),
            None
        );
    }

    #[test]
    fn test_coerce_object_fields() {
        let mut obj = IndexMap::new();
        obj.insert("count".to_string(), Value::String("10".to_string()));
        obj.insert("flag".to_string(), Value::String("yes".to_string()));
        // Extra source keys are not part of the schema and are dropped
        obj.insert("extra".to_string(), Value::String("ignored".to_string()));

        let mut fields = IndexMap::new();
        fields.insert("count".to_string(), FieldType::Int32);
        fields.insert("flag".to_string(), FieldType::Bool);

        let coerced = coerce_to_type(&Value::Object(obj), &FieldType::Object(fields));
        let mut expected = IndexMap::new();
        expected.insert("count".to_string(), Value::Int(10));
        expected.insert("flag".to_string(), Value::Bool(true));
        assert_eq!(coerced, Some(Value::Object(expected)));

        // Missing required key fails
        let mut partial = IndexMap::new();
        partial.insert("count".to_string(), Value::String("10".to_string()));
        let mut fields2 = IndexMap::new();
        fields2.insert("count".to_string(), FieldType::Int32);
        fields2.insert("flag".to_string(), FieldType::Bool);
        assert_eq!(
            coerce_to_type(&Value::Object(partial), &FieldType::Object(fields2)),
            None
        );

        // Inner coercion failure fails the object
        let mut bad = IndexMap::new();
        bad.insert("count".to_string(), Value::String("NaN".to_string()));
        let mut fields3 = IndexMap::new();
        fields3.insert("count".to_string(), FieldType::Int32);
        assert_eq!(
            coerce_to_type(&Value::Object(bad), &FieldType::Object(fields3)),
            None
        );

        // Empty schema object accepts and produces an empty object
        let mut empty_val = IndexMap::new();
        empty_val.insert("anything".to_string(), Value::Int(1));
        assert_eq!(
            coerce_to_type(
                &Value::Object(empty_val),
                &FieldType::Object(IndexMap::new())
            ),
            Some(Value::Object(IndexMap::new()))
        );
        // Scalar against an object target is rejected
        assert_eq!(
            coerce_to_type(&Value::Int(1), &FieldType::Object(IndexMap::new())),
            None
        );
    }

    #[test]
    fn test_coerce_rejects_unrelated_pairs() {
        // The catch-all arm: no coercion exists for these combinations
        assert_eq!(coerce_to_type(&Value::Null, &FieldType::Int32), None);
        assert_eq!(coerce_to_type(&Value::Null, &FieldType::Bool), None);
        assert_eq!(coerce_to_type(&Value::Bool(true), &FieldType::Int32), None);
        assert_eq!(
            coerce_to_type(&Value::Bytes(vec![1]), &FieldType::String),
            None
        );
        assert_eq!(
            coerce_to_type(&Value::String("1".to_string()), &FieldType::Bytes),
            None
        );
    }
}
