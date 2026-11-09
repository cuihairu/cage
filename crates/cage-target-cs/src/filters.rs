//! Tera filter library: the `cs_type` / `cs_default` filters and their
//! registration (split out of lib.rs, LAY-019).

use super::literals::render_default;
use super::types::cs_type;
use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::HashMap;
use tera::Tera;

// ————— G4 filter library (design §22): `cs_type` / `cs_default` —————

fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// `{{ field | cs_type }}` — the C# type text the official generator
/// would emit for this field.
struct CsTypeFilter {
    schema: Schema,
}

impl tera::Filter for CsTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, _field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        Ok(Value::String(cs_type(&ft, &self.schema)))
    }
}

/// `{{ field | cs_default }}` — the C# literal for the field's default,
/// or null when the field has no renderable default.
struct CsDefaultFilter {
    schema: Schema,
}

impl tera::Filter for CsDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the C# filter library for user templates
/// (`options.lang_filters = "cs"`).
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    tera.register_filter(
        "cs_type",
        CsTypeFilter {
            schema: schema.clone(),
        },
    );
    tera.register_filter(
        "cs_default",
        CsDefaultFilter {
            schema: schema.clone(),
        },
    );
}
