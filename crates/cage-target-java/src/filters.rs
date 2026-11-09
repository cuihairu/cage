//! Tera filter library: the `java_type` / `java_default` filters and their registration (split out of lib.rs, LAY-008).

use super::idents::{java_ident, unique_ident};
use super::literals::{render_default, resolve_enum};
use super::types::java_type;
use super::JavaTargetGenerator;
use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tera::Tera;

// ————— G4 filter library (design §22): `java_type` / `java_default` —————

pub(crate) fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// The nested enum idents a user-template filter sees: allocated against
/// the holder's namespace seeded with the holder name — the same rule
/// `generate` applies (allocation runs in name order over the emitted
/// enums).
pub(crate) fn filter_enum_idents(schema: &Schema, holder: Option<&str>) -> HashMap<String, String> {
    let mut used: HashSet<String> = HashSet::new();
    if let Some(h) = holder {
        used.insert(h.to_string());
    }
    JavaTargetGenerator::emitted_enums(schema)
        .into_iter()
        .map(|e| {
            let ident = unique_ident(java_ident(&e.name), &mut used);
            (e.name.clone(), ident)
        })
        .collect()
}

/// `{{ field | java_type }}` — the Java type text the official generator
/// would emit: optionality resolved from the field itself (not required,
/// no default), enum references qualified `Holder.Enum` where needed.
pub(crate) struct JavaTypeFilter {
    schema: Schema,
    enum_idents: HashMap<String, String>,
}

impl tera::Filter for JavaTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let optional = !field
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && cage_target_template::field_default(field).is_none();
        let enum_text = resolve_enum(&self.schema, &ft, &self.enum_idents).map(str::to_string);
        Ok(Value::String(java_type(
            &ft,
            optional,
            enum_text.as_deref(),
        )))
    }
}

/// `{{ field | java_default }}` — the Java literal for the field's
/// default, or null when the field has no renderable default.
pub(crate) struct JavaDefaultFilter {
    pub(crate) schema: Schema,
}

impl tera::Filter for JavaDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the Java filter library for user templates
/// (`options.lang_filters = "java"`). `holder` is the shared-enums holder
/// class name (the CLI derives it from `options.enums_file`'s stem) — it
/// seeds the nested-enum ident namespace, and enum type text spells
/// `Holder.Enum` where the official generator would.
pub fn register_filters(tera: &mut Tera, schema: &Schema, holder: Option<&str>) {
    let enum_idents = filter_enum_idents(schema, holder);
    tera.register_filter(
        "java_type",
        JavaTypeFilter {
            schema: schema.clone(),
            enum_idents,
        },
    );
    tera.register_filter(
        "java_default",
        JavaDefaultFilter {
            schema: schema.clone(),
        },
    );
}
