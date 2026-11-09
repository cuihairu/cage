//! Tera filter library: the `ts_type` / `ts_default` filters and their registration (split out of lib.rs, LAY-010).

use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::HashMap;
use tera::Tera;

use super::enum_exports;
use super::literals::render_default;
use super::types::ts_type;
// ————— G4 filter library (design §22): `ts_type` / `ts_default` —————

pub(crate) fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// `{{ field | ts_type }}` — the TypeScript type text (Local enum
/// spellings, as on the `.ts` / `.d.ts` surface); enum references resolve
/// through the shared export names.
pub(crate) struct TsTypeFilter {
    schema: Schema,
    enum_ty: HashMap<String, String>,
}

impl tera::Filter for TsTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, _field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        Ok(Value::String(ts_type(&ft, &self.schema, &self.enum_ty)))
    }
}

/// `{{ field | ts_default }}` — the TypeScript literal for the field's
/// default, or null when the field has no renderable default.
pub(crate) struct TsDefaultFilter {
    schema: Schema,
}

impl tera::Filter for TsDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the TypeScript filter library for user templates
/// (`options.lang_filters = "ts"`).
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    let enum_ty: HashMap<String, String> = enum_exports(schema)
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    tera.register_filter(
        "ts_type",
        TsTypeFilter {
            schema: schema.clone(),
            enum_ty,
        },
    );
    tera.register_filter(
        "ts_default",
        TsDefaultFilter {
            schema: schema.clone(),
        },
    );
}
