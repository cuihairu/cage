//! Tera filter library: the `cpp_type` / `cpp_default` filters and their registration (split out of lib.rs, LAY-011).

use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::HashMap;
use tera::Tera;

use super::literals::render_default;
use super::types::cpp_type;
use super::{CppTargetGenerator, EnumAlloc, Needs};

// ————— G4 filter library (design §22): `cpp_type` / `cpp_default` —————

pub(crate) fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// `{{ field | cpp_type }}` — the C++ type text the official generator
/// would emit (`std::optional<…>` shapes included). The per-file include
/// aggregation is a generator concern: each filter call sees a fresh
/// `Needs`, and the type text itself is unaffected.
pub(crate) struct CppTypeFilter {
    enum_by_key: HashMap<String, EnumAlloc>,
}

impl tera::Filter for CppTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, _field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let view: HashMap<&str, EnumAlloc> = self
            .enum_by_key
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        Ok(Value::String(cpp_type(&ft, &view, &mut Needs::default())))
    }
}

/// `{{ field | cpp_default }}` — the C++ initializer for the field's
/// default, or null when the field has no renderable default.
pub(crate) struct CppDefaultFilter {
    pub(crate) schema: Schema,
}

impl tera::Filter for CppDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the C++ filter library for user templates
/// (`options.lang_filters = "cpp"`).
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    let enum_by_key: HashMap<String, EnumAlloc> = CppTargetGenerator::allocate_names(schema)
        .1
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    tera.register_filter("cpp_type", CppTypeFilter { enum_by_key });
    tera.register_filter(
        "cpp_default",
        CppDefaultFilter {
            schema: schema.clone(),
        },
    );
}
