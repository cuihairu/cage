//! Field-level normalization: schema-default fill, cell-array split and
//! type coercion (split out of mod.rs, LAY-015).

use crate::value::{TypedValue, Value};
use indexmap::IndexMap;

/// Fill absent-or-null fields with their schema default (回流-1).
///
/// Mirrors Luban's `#default`: a missing key **or** an Excel empty cell
/// (which loads as `Value::Null`) both become the declared default, so a
/// field with a default always carries a value. Without this an Excel empty
/// cell in a typed field dies as E1101 (`Expected type: X, got: null`) even
/// though the schema provides a default. The fill runs on the full document
/// before profile projection; fields hidden by the profile are dropped after,
/// and E9006 (required + no default + hidden) is unaffected.
///
/// Fields whose default cannot be represented in the field's own type family
/// (`migrate::diff::schema_default` returns `None`, e.g. a bytes literal) are
/// left as-is — the L2 type check surfaces them instead of silently dropping.
pub fn fill_field_defaults(schema: &crate::schema::Schema, document: &mut crate::value::Document) {
    for table in document.tables.values_mut() {
        let Some(schema_table) = schema.tables.get(&table.name) else {
            continue;
        };
        for row in &mut table.rows {
            for (name, field) in &schema_table.fields {
                let Some(default) = crate::migrate::diff::schema_default(field) else {
                    continue;
                };
                let needs_fill = match row.fields.get(name) {
                    None => true,
                    Some(tv) => tv.value == Value::Null,
                };
                if needs_fill {
                    row.fields.insert(
                        name.clone(),
                        TypedValue::new(default, row.location.clone().with_field(name)),
                    );
                }
            }
        }
    }
}

/// Split scalar table cells for fields with a `separator` (回流-3 / 回流-6,
/// Luban's `(list#sep=,)` and single-cell `sep="|"` polymorphism).
///
/// Excel/CSV cells are scalars, so an `Array` field fed from a table source
/// cannot arrive as an array by itself, and a `TaggedUnion` field (whose
/// value is a flat object) has no object surface either. When the schema
/// declares a `separator`:
///
/// - `Array`: the cell is split, each token trimmed and coerced to the
///   element type (via [`coerce_to_type`]), replaced by the resulting array.
/// - `TaggedUnion`: the first token is the discriminant, the remaining
///   tokens map positionally to the selected branch's payload field
///   declaration order (Luban's `"Damage|410|wind"` → `{"$type":"Damage",
///   "amount":410,"element":"wind"}`). Exactly one token per payload field.
///
/// Cells that are already the right shape (arrays / objects from JSON/YAML)
/// pass through untouched. If any token cannot be coerced the cell is left
/// as the original `String`, so the L2 type check reports the mismatch
/// instead of silently dropping data.
pub fn split_cell_arrays(schema: &crate::schema::Schema, document: &mut crate::value::Document) {
    use crate::schema::FieldType;

    for table in document.tables.values_mut() {
        let Some(schema_table) = schema.tables.get(&table.name) else {
            continue;
        };
        for row in &mut table.rows {
            for (name, field) in &schema_table.fields {
                let Some(sep) = &field.separator else {
                    continue;
                };
                let Some(tv) = row.fields.get_mut(name) else {
                    continue;
                };
                let Value::String(cell) = &tv.value else {
                    continue; // already an array/object (or null) — no split
                };
                if cell.is_empty() {
                    continue;
                }
                match &field.field_type {
                    FieldType::Array(inner) => {
                        let mut items = Vec::new();
                        let mut ok = true;
                        for token in cell.split(sep.as_str()) {
                            let token = token.trim();
                            let Some(v) = coerce_to_type(&Value::String(token.to_string()), inner)
                            else {
                                ok = false;
                                break;
                            };
                            items.push(v);
                        }
                        if ok {
                            tv.value = Value::Array(items);
                        }
                    }
                    FieldType::TaggedUnion(tu) => {
                        let tokens: Vec<&str> = cell.split(sep.as_str()).map(str::trim).collect();
                        let Some(tag) = tokens.first() else {
                            continue;
                        };
                        let Some(branch) = tu.branches.get(*tag) else {
                            continue; // unknown branch — leave as-is for L2
                        };
                        if tokens.len() - 1 != branch.len() {
                            continue; // not one token per payload field
                        }
                        let mut obj = IndexMap::new();
                        obj.insert(tu.tag.clone(), Value::String((*tag).to_string()));
                        let mut ok = true;
                        for ((k, ft), token) in branch.iter().zip(tokens.iter().skip(1)) {
                            let Some(v) = coerce_to_type(&Value::String(token.to_string()), ft)
                            else {
                                ok = false;
                                break;
                            };
                            obj.insert(k.clone(), v);
                        }
                        if ok {
                            tv.value = Value::Object(obj);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
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

        // Tagged-union coercion (回流-6): the discriminant selects a branch,
        // and every payload key is coerced per that branch (a key from
        // another branch makes the object uncoercible).
        (Value::Object(obj), crate::schema::FieldType::TaggedUnion(tu)) => {
            let Some(Value::String(tag_val)) = obj.get(&tu.tag) else {
                return None;
            };
            let branch = tu.branches.get(tag_val)?;
            let mut result = IndexMap::new();
            for (k, v) in obj {
                if k == &tu.tag {
                    result.insert(k.clone(), v.clone());
                } else {
                    result.insert(k.clone(), coerce_to_type(v, branch.get(k)?)?);
                }
            }
            Some(Value::Object(result))
        }

        // Map coercion: open-ended — every entry's value is coerced by
        // value_type, keys pass through untouched (normalize_object sorts
        // them for determinism) and missing/extra keys are all legal.
        (Value::Object(obj), crate::schema::FieldType::Map(map)) => {
            let mut result = IndexMap::new();
            for (k, v) in obj {
                result.insert(k.clone(), coerce_to_type(v, &map.value_type)?);
            }
            Some(Value::Object(result))
        }

        _ => None,
    }
}
