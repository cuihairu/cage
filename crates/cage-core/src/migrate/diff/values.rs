//! Value-shaping helpers for draft rendering: JSON → typed/plain values
//! and the YAML block / scalar quoting forms (split out of diff.rs,
//! LAY-025).

use crate::schema::FieldType;
use crate::value::Value;
use indexmap::IndexMap;
use std::fmt::Write as _;

pub(crate) fn typed_json_value(json: &serde_json::Value, expected: &FieldType) -> Option<Value> {
    match expected {
        FieldType::UInt8 | FieldType::UInt16 | FieldType::UInt32 | FieldType::UInt64 => {
            match json {
                serde_json::Value::Number(n) if n.is_u64() => Some(Value::UInt(n.as_u64()?)),
                _ => None,
            }
        }
        FieldType::Int8 | FieldType::Int16 | FieldType::Int32 | FieldType::Int64 => match json {
            serde_json::Value::Number(n) if n.is_i64() => Some(Value::Int(n.as_i64()?)),
            _ => None,
        },
        FieldType::Float32 | FieldType::Float64 => match json {
            serde_json::Value::Number(n) => Some(Value::Float(n.as_f64()?)),
            _ => None,
        },
        FieldType::Bool => match json {
            serde_json::Value::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        },
        FieldType::String | FieldType::Enum(_) => match json {
            serde_json::Value::String(s) => Some(Value::String(s.clone())),
            _ => None,
        },
        FieldType::Null => match json {
            serde_json::Value::Null => Some(Value::Null),
            _ => None,
        },
        FieldType::Array(inner) => match json {
            serde_json::Value::Array(items) => Some(Value::Array(
                items
                    .iter()
                    .map(|v| typed_json_value(v, inner))
                    .collect::<Option<Vec<_>>>()?,
            )),
            _ => None,
        },
        // Objects, maps, bytes and `Any` fall back to the generic bridge —
        // a schema default for these is rare and the family check on
        // objects is structural anyway.
        _ => json_to_value(json),
    }
}

/// Plain JSON (the shape schema files carry defaults in) → canonical
/// value. The canonical model is adjacently tagged for serde, so this is
/// a hand-rolled mapping rather than a serde round-trip; `Bytes` and
/// anything else a schema default cannot literally be, maps to `None`.
pub(crate) fn json_to_value(json: &serde_json::Value) -> Option<Value> {
    Some(match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else {
                Value::Float(n.as_f64()?)
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(items) => Value::Array(
            items
                .iter()
                .map(json_to_value)
                .collect::<Option<Vec<_>>>()?,
        ),
        serde_json::Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Some((k.clone(), json_to_value(v)?)))
                .collect::<Option<IndexMap<String, Value>>>()?,
        ),
    })
}

pub(crate) fn quote_string(s: &str) -> Option<String> {
    if s.chars()
        .any(|c| c == '\n' || c == '\r' || (c.is_control() && c != '\t'))
    {
        return None;
    }
    Some(format!("'{}'", s.replace('\'', "''")))
}

/// True when plain rendering would lose the value's family: a bare
/// unsigned integer reads back as `Int` (`yaml_to_value` is i64-first),
/// so `UInt` anywhere in the value forces the canonical tagged form.
pub(crate) fn has_unsigned(value: &Value) -> bool {
    match value {
        Value::UInt(_) => true,
        Value::Array(items) => items.iter().any(has_unsigned),
        Value::Object(map) => map.values().any(has_unsigned),
        _ => false,
    }
}

/// Render a canonical value as inline-or-block YAML at the given indent
/// (the inverse of the rule parser's `yaml_to_value`, with quoting the
/// generic serializer cannot be trusted for). `Bytes` has no plain YAML
/// form and yields `None`.
pub(crate) fn value_to_yaml_block(value: &Value, pad: usize) -> Option<String> {
    let ind = " ".repeat(pad);
    Some(match value {
        Value::Null => "null\n".to_string(),
        Value::Bool(true) => "true\n".to_string(),
        Value::Bool(false) => "false\n".to_string(),
        Value::Int(i) => format!("{i}\n"),
        Value::UInt(u) => format!("{u}\n"),
        // serde formats floats with a `.0`-style fractional part, so the
        // round-trip cannot land back in the integer family.
        Value::Float(f) => {
            let mut text =
                serde_yaml::to_string(&serde_yaml::Value::Number((*f).into())).unwrap_or_default();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text
        }
        Value::String(s) => format!("{}\n", quote_string(s)?),
        Value::Bytes(_) => return None,
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                match item {
                    Value::Array(_) | Value::Object(_) => {
                        let nested = value_to_yaml_block(item, pad + 2)?;
                        write!(out, "{ind}-\n{nested}").unwrap();
                    }
                    scalar => {
                        let one = value_to_yaml_block(scalar, pad)?;
                        write!(out, "{ind}- {}", one.trim_end()).unwrap();
                        out.push('\n');
                    }
                }
            }
            out
        }
        Value::Object(map) => {
            let mut out = String::new();
            for (k, v) in map {
                let key = quote_string(k)?;
                match v {
                    Value::Array(_) | Value::Object(_) => {
                        let nested = value_to_yaml_block(v, pad + 2)?;
                        write!(out, "{ind}{key}:\n{nested}").unwrap();
                    }
                    scalar => {
                        let one = value_to_yaml_block(scalar, pad)?;
                        write!(out, "{ind}{key}: {}", one.trim_end()).unwrap();
                        out.push('\n');
                    }
                }
            }
            out
        }
    })
}
