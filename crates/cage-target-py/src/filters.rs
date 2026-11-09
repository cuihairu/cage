//! Tera filter library: the `py_type` / `py_default` filters and their
//! registration (split out of lib.rs, LAY-020).

use super::literals::render_default;
use super::types::py_type;
use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::HashMap;
use tera::Tera;

// ————— G4 filter library (design §22): `py_type` / `py_default` —————

fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// `{{ field | py_type }}` — the Python annotation the official generator
/// would emit for this field.
struct PyTypeFilter {
    schema: Schema,
}

impl tera::Filter for PyTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, _field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        Ok(Value::String(py_type(&ft, &self.schema)))
    }
}

/// `{{ field | py_default }}` — the Python literal for the field's
/// default, or null when the field has no renderable default.
struct PyDefaultFilter {
    schema: Schema,
}

impl tera::Filter for PyDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the Python filter library for user templates
/// (`options.lang_filters = "py"`).
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    tera.register_filter(
        "py_type",
        PyTypeFilter {
            schema: schema.clone(),
        },
    );
    tera.register_filter(
        "py_default",
        PyDefaultFilter {
            schema: schema.clone(),
        },
    );
}
