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
    fn test_normalize_object_sorts_keys() {
        let mut obj = IndexMap::new();
        obj.insert("z".to_string(), Value::Int(1));
        obj.insert("a".to_string(), Value::Int(2));
        let normalized = normalize_value(&Value::Object(obj));

        if let Value::Object(norm_obj) = normalized {
            let keys: Vec<_> = norm_obj.keys().collect();
            assert_eq!(keys, vec!["a", "z"]);
        } else {
            panic!("Expected object");
        }
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
}
