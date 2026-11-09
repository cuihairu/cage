use crate::schema::{FieldType, MapField, MapKeyType, ReferenceSchema};

/// `true` / `false`, case-insensitive — nothing else (E2202).
pub(crate) fn parse_bool_text(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, got {value:?}")),
    }
}

/// A number, verbatim (E2202).
pub(crate) fn parse_number_text(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("{value:?} is not a number"))
}

/// A comma-separated value list, entries trimmed, empties dropped; an empty
/// result is an error (E2202).
pub(crate) fn parse_list_text(value: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if items.is_empty() {
        Err(format!("{value:?} is not a value list"))
    } else {
        Ok(items)
    }
}

/// `Table.Field[:cardinality]` — cardinality defaults to `one`, matching
/// `ReferenceSchema`'s own default (E2202).
pub(crate) fn parse_reference_text(value: &str) -> Result<ReferenceSchema, String> {
    let (target, cardinality) = match value.split_once(':') {
        Some((target, card)) => (target, card.trim().to_string()),
        None => (value, "one".to_string()),
    };
    if !matches!(cardinality.as_str(), "one" | "many" | "optional") {
        return Err(format!(
            "reference cardinality must be one | many | optional, got {cardinality:?}"
        ));
    }
    let Some((table, field)) = target.split_once('.') else {
        return Err(format!(
            "reference must be Table.field, got {value:?} (no '.')"
        ));
    };
    let (table, field) = (table.trim(), field.trim());
    if table.is_empty() || field.is_empty() {
        return Err(format!("reference must be Table.field, got {value:?}"));
    }
    Ok(ReferenceSchema {
        table: table.to_string(),
        field: field.to_string(),
        predicate: None,
        cardinality,
        compatible_with: None,
    })
}

/// The type cell accepts two spellings (design §51, cell text notation):
/// the Schema's own expression form — `{ kind: Int32 }`,
/// `{ kind: Array, value: { kind: Int32 } }` — parsed by the Schema's own
/// deserializer so there is no second vocabulary; and the cell shorthand —
/// `Int32`, `Array<Int32>`, `Enum<Rarity>`, `Map<string, Int32>` — which
/// renders into exactly that tagged form. An `Object` cannot be declared in
/// a cell (its value is a property map): it belongs in the Schema (E2202).
pub fn parse_type_text(text: &str) -> Result<FieldType, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("empty type cell".to_string());
    }
    if t.starts_with('{') {
        return serde_yaml::from_str::<FieldType>(t)
            .map_err(|e| format!("{t:?} is not a Schema type expression: {e}"));
    }
    if let Some(rest) = t.strip_prefix("Array<").and_then(|s| s.strip_suffix('>')) {
        return parse_type_text(rest)
            .map(|inner| FieldType::Array(Box::new(inner)))
            .map_err(|e| format!("{t:?}: {e}"));
    }
    if let Some(rest) = t.strip_prefix("Enum<").and_then(|s| s.strip_suffix('>')) {
        let name = rest.trim();
        if !is_identifier(name) {
            return Err(format!("{t:?} is not a valid Enum name"));
        }
        return Ok(FieldType::Enum(name.to_string()));
    }
    if let Some(rest) = t.strip_prefix("Map<").and_then(|s| s.strip_suffix('>')) {
        let Some((key, value)) = rest.split_once(',') else {
            return Err(format!(
                "{t:?}: a Map needs Map<key, value> with a comma between the types"
            ));
        };
        let key_type = match key.trim() {
            "string" => MapKeyType::String,
            "int" => MapKeyType::Int,
            other => {
                return Err(format!(
                    "{t:?}: map key type must be string or int, got {other:?}"
                ))
            }
        };
        let value_type = parse_type_text(value).map_err(|e| format!("{t:?}: {e}"))?;
        return Ok(FieldType::Map(MapField {
            key_type,
            value_type: Box::new(value_type),
        }));
    }
    if t.starts_with("Object") {
        return Err(format!(
            "{t:?}: an Object's property map cannot be declared in a cell — declare it in the \
             Schema"
        ));
    }
    scalar_kind(t).ok_or_else(|| format!("{t:?} is not a Schema type name"))
}

fn scalar_kind(t: &str) -> Option<FieldType> {
    Some(match t {
        "Null" => FieldType::Null,
        "Bool" => FieldType::Bool,
        "Int8" => FieldType::Int8,
        "Int16" => FieldType::Int16,
        "Int32" => FieldType::Int32,
        "Int64" => FieldType::Int64,
        "UInt8" => FieldType::UInt8,
        "UInt16" => FieldType::UInt16,
        "UInt32" => FieldType::UInt32,
        "UInt64" => FieldType::UInt64,
        "Float32" => FieldType::Float32,
        "Float64" => FieldType::Float64,
        "String" => FieldType::String,
        "Bytes" => FieldType::Bytes,
        "Any" => FieldType::Any,
        _ => return None,
    })
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
