//! Tera filter library: the `lua_type` / `lua_default` / `lua_string`
//! filters and their registration (split out of lib.rs, LAY-022).

use super::literals::{lua_string_literal, render_default};
use super::types::lua_type_label;
use cage_core::schema::{FieldType, Schema};
use serde_json::Value;
use std::collections::HashMap;
use tera::Tera;

/// Pure text conversion for the template: quote and escape a Lua string.
pub(crate) struct LuaStringFilter;

impl tera::Filter for LuaStringFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        match value {
            Value::String(s) => Ok(Value::String(lua_string_literal(s))),
            _ => Err("lua_string expects a string".into()),
        }
    }
}

// ————— G4 filter library (design §22): `lua_type` / `lua_default` —————

fn parse_field_type(ty: &serde_json::Value) -> tera::Result<FieldType> {
    serde_json::from_value(ty.clone()).map_err(|e| tera::Error::msg(e.to_string()))
}

/// `{{ field | lua_type }}` — the Lua type label the official generator
/// would print for this field.
struct LuaTypeFilter {
    schema: Schema,
}

impl tera::Filter for LuaTypeFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, _field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        Ok(Value::String(lua_type_label(&ft, &self.schema)))
    }
}

/// `{{ field | lua_default }}` — the Lua literal for the field's default,
/// or null when the field has no renderable default.
struct LuaDefaultFilter {
    schema: Schema,
}

impl tera::Filter for LuaDefaultFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let (ty, field) = cage_target_template::field_parts(value)?;
        let ft = parse_field_type(ty)?;
        let Some(d) = cage_target_template::field_default(field) else {
            return Ok(Value::Null);
        };
        Ok(render_default(d, &ft, &self.schema).map_or(Value::Null, Value::String))
    }
}

/// Register the Lua filter library for user templates
/// (`options.lang_filters = "lua"`).
pub fn register_filters(tera: &mut Tera, schema: &Schema) {
    tera.register_filter(
        "lua_type",
        LuaTypeFilter {
            schema: schema.clone(),
        },
    );
    tera.register_filter(
        "lua_default",
        LuaDefaultFilter {
            schema: schema.clone(),
        },
    );
}
