//! Tera filter library: the `go_type` / `go_default` filters and their registration (split out of lib.rs, LAY-009).

use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::HashMap;
use tera::Tera;

use super::literals::render_default;
use super::types::field_go_type;
use super::GoTargetGenerator;

// ————— G4 filter library (design §22): `go_type` / `go_default` —————

pub(crate) fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// Borrowed `&str` view of the allocated enum types, as `field_go_type`
/// and `render_default` consume them.
pub(crate) fn enum_view(enum_types: &HashMap<String, String>) -> HashMap<&str, String> {
    enum_types
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect()
}

/// `{{ field | go_type }}` — the Go type text, enum references carrying
/// the ALLOCATED idents (the same names `generate` emits).
pub(crate) struct GoTypeFilter {
    enum_types: HashMap<String, String>,
}

impl tera::Filter for GoTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let required = field
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let has_default = cage_target_template::field_default(field).is_some();
        Ok(Value::String(field_go_type(
            &ft,
            &enum_view(&self.enum_types),
            required,
            has_default,
        )))
    }
}

/// `{{ field | go_default }}` — the Go literal for the field's default,
/// or null when the field has no renderable default.
pub(crate) struct GoDefaultFilter {
    enum_types: HashMap<String, String>,
}

impl tera::Filter for GoDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &enum_view(&self.enum_types)).map_or(Value::Null, Value::String))
    }
}

/// Register the Go filter library for user templates
/// (`options.lang_filters = "go"`). Enum references resolve through the
/// package-level allocation, so a filter sees the same type names the
/// official generator emits.
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    let enum_types: HashMap<String, String> = GoTargetGenerator::allocate_names(schema)
        .1
        .iter()
        .map(|e| (e.schema.name.clone(), e.type_ident.clone()))
        .collect();
    tera.register_filter(
        "go_type",
        GoTypeFilter {
            enum_types: enum_types.clone(),
        },
    );
    tera.register_filter("go_default", GoDefaultFilter { enum_types });
}
