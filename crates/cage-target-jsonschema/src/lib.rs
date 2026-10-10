//! JSON Schema Target Generator - emits standard JSON Schema documents from
//! the Cage schema
//!
//! One self-contained document per table (`{table}.schema.json` by default):
//! `properties` follow the schema's field declaration order, `required`
//! collects the required fields, and `additionalProperties: false` pins rows
//! to the declared shape (mirrors the validators: unknown object keys fail,
//! named enums are checked as member-name strings, map int keys arrive as
//! numeric strings). Documents are pure standard JSON Schema — draft-07 by
//! default, 2020-12 via `options.draft` — so editors, ajv, and other
//! non-cage tooling consume them with no cage runtime. No shared enums file:
//! enums inline as `enum` arrays (instances are always member names — L2
//! types enum fields as strings regardless of the backing literals).
//!
//! Constraints map onto the closest JSON Schema keyword set; cross-table
//! references and semantic rules have no standard spelling and stay out
//! (they remain enforced by cage's own pipeline on the source of truth).

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Stage: crate-prefixed type names (JsonSchemaTargetGenerator, ...) are
// idiomatic across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Stage: doc-comment examples would need a Schema fixture builder; unit
// tests carry the executable documentation instead.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names
)]
// Index lengths render as JSON numbers (usize → u64); widths never wrap
// there in practice.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use cage_core::manifest::TargetConfig;
use cage_core::schema::{FieldSchema, FieldType, MapKeyType, Schema};
use indexmap::IndexMap;
use serde_json::{Map, Number, Value as Json};
use std::path::PathBuf;

/// `$schema` meta URI, draft-07 (the widest-supported dialect).
const DRAFT_07_META: &str = "http://json-schema.org/draft-07/schema#";
/// `$schema` meta URI, 2020-12 (the current standard).
const DRAFT_2020_12_META: &str = "https://json-schema.org/draft/2020-12/schema";

/// JSON Schema dialect of the emitted documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draft {
    /// draft-07 (`http://json-schema.org/draft-07/schema#`), the default —
    /// widest tooling support.
    V07,
    /// 2020-12 (`https://json-schema.org/draft/2020-12/schema`).
    V2020_12,
}

impl Draft {
    /// Meta URI stamped into `$schema`.
    fn meta_uri(self) -> &'static str {
        match self {
            Draft::V07 => DRAFT_07_META,
            Draft::V2020_12 => DRAFT_2020_12_META,
        }
    }

    /// Parse the `options.draft` string; unknown values are a config error
    /// surfaced from `generate` (`from_config` stays infallible like every
    /// bundled generator).
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "07" | "7" | "draft-07" | "draft7" => Ok(Draft::V07),
            "2020-12" | "202012" | "draft-2020-12" | "draft2020-12" => Ok(Draft::V2020_12),
            other => Err(format!(
                "jsonschema target: unknown draft '{other}' (supported: 07, 2020-12)"
            )),
        }
    }
}

/// JSON Schema Target Generator — one self-contained standard document per
/// table, `properties` in schema declaration order.
pub struct JsonSchemaTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.schema.json")
    pub file_template: String,
    /// `options.draft` raw value ("07" default); validated in `generate`
    draft: String,
}

impl Default for JsonSchemaTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/jsonschema"),
            file_template: "{table}.schema.json".to_string(),
            draft: "07".to_string(),
        }
    }
}

/// JSON number for a bound: integral f64 values render as integers
/// (`minimum: 1`, not `minimum: 1.0`); non-finite bounds are not JSON and
/// stay out.
fn json_number(f: f64) -> Option<Json> {
    // 2^53: the largest magnitude where every integral f64 converts to i64
    // exactly — beyond it the schema value stays a float.
    const MAX_EXACT_INT: f64 = 9_007_199_254_740_992.0;
    if f.fract() == 0.0 && f.abs() <= MAX_EXACT_INT {
        Some(Json::Number(Number::from(f as i64)))
    } else {
        Number::from_f64(f).map(Json::Number)
    }
}

impl JsonSchemaTargetGenerator {
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.schema.json".to_string()),
            draft: "07".to_string(),
        };
        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("draft") {
                if let Some(s) = v.as_str() {
                    gen.draft = s.to_string();
                }
            }
        }
        gen
    }

    /// Generate one JSON Schema document per table, tables in name order.
    /// Fails only on an unusable `options.draft` value.
    pub fn generate(
        &self,
        schema: &Schema,
        _schema_hash: Option<&str>,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let draft = Draft::parse(&self.draft)?;
        let mut table_names: Vec<&String> = schema.tables.keys().collect();
        table_names.sort();

        let mut artifacts = Vec::new();
        for name in table_names {
            let table = &schema.tables[name];
            let doc = self.render_table(name, table, schema, draft);
            // 2-space indent + trailing newline: diffable and newline-safe
            // like every other text artifact in the tree.
            let mut body = serde_json::to_string_pretty(&doc).unwrap_or_default();
            body.push('\n');
            let file_name = self.file_template.replace("{table}", name);
            artifacts.push((self.path(&file_name), body.into_bytes()));
        }
        Ok(artifacts)
    }

    fn path(&self, file_name: &str) -> String {
        self.output_dir
            .join(file_name)
            .to_string_lossy()
            .to_string()
    }

    /// One table document: meta, title/description, object shape with the
    /// declared fields, required list, closed additionalProperties.
    fn render_table(
        &self,
        name: &str,
        table: &cage_core::schema::TableSchema,
        schema: &Schema,
        draft: Draft,
    ) -> Json {
        let mut doc = Map::new();
        doc.insert("$schema".into(), Json::String(draft.meta_uri().into()));
        doc.insert("title".into(), Json::String(name.to_string()));
        if let Some(desc) = &table.description {
            doc.insert("description".into(), Json::String(desc.clone()));
        }
        doc.insert("type".into(), Json::String("object".into()));

        let mut properties = Map::new();
        let mut required = Vec::new();
        for (field_name, field) in &table.fields {
            properties.insert(field_name.clone(), self.render_field(field, schema, draft));
            if field.required {
                required.push(Json::String(field_name.clone()));
            }
        }
        doc.insert("properties".into(), Json::Object(properties));
        if !required.is_empty() {
            doc.insert("required".into(), Json::Array(required));
        }
        // Rows carry exactly the declared fields — the validators reject
        // unknown keys on typed objects, so the document closes the shape.
        doc.insert("additionalProperties".into(), Json::Bool(false));
        Json::Object(doc)
    }

    /// One property schema: type first, then description/default, then the
    /// constraint keywords in a fixed order — fixed insertion order is what
    /// keeps the bytes deterministic.
    fn render_field(&self, field: &FieldSchema, schema: &Schema, draft: Draft) -> Json {
        let mut obj = Map::new();
        Self::insert_type_keywords(&mut obj, &field.field_type, schema, draft);

        if let Some(desc) = &field.description {
            obj.insert("description".into(), Json::String(desc.clone()));
        }
        if let Some(default) = &field.default {
            obj.insert("default".into(), default.clone());
        }

        // Numeric range (E1201) → minimum/maximum. Non-finite bounds are not
        // JSON numbers and stay out rather than serializing to null; integral
        // bounds render as integers (schema YAML reads them as f64).
        if let Some(min) = field.min.and_then(json_number) {
            obj.insert("minimum".into(), min);
        }
        if let Some(max) = field.max.and_then(json_number) {
            obj.insert("maximum".into(), max);
        }
        if let Some(v) = field.min_length {
            obj.insert("minLength".into(), Json::Number(Number::from(v)));
        }
        if let Some(v) = field.max_length {
            obj.insert("maxLength".into(), Json::Number(Number::from(v)));
        }
        if let Some(pattern) = &field.pattern {
            obj.insert("pattern".into(), Json::String(pattern.clone()));
        }

        // Inline enum (E1204) → `enum`. Validation compares the coerced
        // string, so the domain is string-shaped; for integer-typed fields
        // the instances are JSON numbers, so all-integer domains are emitted
        // as numbers to keep the document matching actual rows.
        if let Some(values) = &field.enum_values {
            let all_int = !values.is_empty() && values.iter().all(|v| v.parse::<i64>().is_ok());
            let rendered: Vec<Json> = values
                .iter()
                .map(|v| match (all_int, v.parse::<i64>()) {
                    (true, Ok(n)) => Json::Number(Number::from(n)),
                    (_, _) => Json::String(v.clone()),
                })
                .collect();
            obj.insert("enum".into(), Json::Array(rendered));
        }

        self.insert_type_shape(&mut obj, field, schema, draft);
        Json::Object(obj)
    }

    /// Type keyword for the field's own type: the JSON Schema `type` plus
    /// the domains that live on the type itself (base64 annotation on
    /// bytes, named-enum membership on enums).
    fn insert_type_keywords(
        obj: &mut Map<String, Json>,
        field_type: &FieldType,
        schema: &Schema,
        draft: Draft,
    ) {
        match field_type {
            FieldType::Null => {
                obj.insert("type".into(), Json::String("null".into()));
            }
            FieldType::Bool => {
                obj.insert("type".into(), Json::String("boolean".into()));
            }
            FieldType::Int8
            | FieldType::Int16
            | FieldType::Int32
            | FieldType::Int64
            | FieldType::UInt8
            | FieldType::UInt16
            | FieldType::UInt32
            | FieldType::UInt64 => {
                obj.insert("type".into(), Json::String("integer".into()));
            }
            FieldType::Float32 | FieldType::Float64 => {
                obj.insert("type".into(), Json::String("number".into()));
            }
            FieldType::String => {
                obj.insert("type".into(), Json::String("string".into()));
            }
            FieldType::Bytes => {
                obj.insert("type".into(), Json::String("string".into()));
                // draft-07 carries contentEncoding as an annotation; 2020-12
                // removed the keyword, so it only lands there.
                if draft == Draft::V07 {
                    obj.insert("contentEncoding".into(), Json::String("base64".into()));
                }
            }
            FieldType::Array(_) => {
                obj.insert("type".into(), Json::String("array".into()));
            }
            FieldType::Object(_) | FieldType::Map(_) => {
                obj.insert("type".into(), Json::String("object".into()));
            }
            FieldType::Enum(enum_name) => {
                // Instances are always member names (L2 types enum fields as
                // strings; the backing literals are code-gen metadata only).
                obj.insert("type".into(), Json::String("string".into()));
                if let Some(def) = schema.enums.get(enum_name) {
                    let names: Vec<Json> = def
                        .values
                        .iter()
                        .map(|v| Json::String(v.name.clone()))
                        .collect();
                    if !names.is_empty() {
                        obj.insert("enum".into(), Json::Array(names));
                    }
                }
                // Dangling enum name: L1 reports it; the document degrades to
                // a plain string rather than inventing a domain.
            }
            FieldType::Any => {}
        }
    }

    /// Shape keywords that depend on the type's payload: array items and
    /// item-count range, typed-object properties, map value schema and int
    /// key constraint.
    fn insert_type_shape(
        &self,
        obj: &mut Map<String, Json>,
        field: &FieldSchema,
        schema: &Schema,
        draft: Draft,
    ) {
        match &field.field_type {
            FieldType::Array(inner) => {
                obj.insert(
                    "items".into(),
                    self.render_type_schema(inner, schema, draft),
                );
                if let Some(v) = field.min_items {
                    obj.insert("minItems".into(), Json::Number(Number::from(v)));
                }
                if let Some(v) = field.max_items {
                    obj.insert("maxItems".into(), Json::Number(Number::from(v)));
                }
            }
            FieldType::Object(props) if !props.is_empty() => {
                let mut properties = Map::new();
                for (prop_name, prop_type) in props {
                    properties.insert(
                        prop_name.clone(),
                        self.render_type_schema(prop_type, schema, draft),
                    );
                }
                obj.insert("properties".into(), Json::Object(properties));
                // Typed objects reject unknown keys (value_matches_type).
                obj.insert("additionalProperties".into(), Json::Bool(false));
            }
            FieldType::Map(map) => {
                obj.insert(
                    "additionalProperties".into(),
                    self.render_type_schema(&map.value_type, schema, draft),
                );
                // Int keys arrive as numeric strings in the data model —
                // constrain the key shape the same way the validators do.
                if map.key_type == MapKeyType::Int {
                    let mut names = Map::new();
                    names.insert("pattern".into(), Json::String("^-?[0-9]+$".into()));
                    obj.insert("propertyNames".into(), Json::Object(names));
                }
            }
            _ => {}
        }
    }

    /// Schema fragment for a bare `FieldType` (array items, object
    /// properties, map values) — no `FieldSchema` constraints at this level.
    fn render_type_schema(&self, field_type: &FieldType, schema: &Schema, draft: Draft) -> Json {
        let field = FieldSchema {
            name: String::new(),
            field_type: field_type.clone(),
            description: None,
            required: false,
            default: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            enum_values: None,
            min_items: None,
            max_items: None,
            items: None,
            properties: None,
            additional_properties: None,
            reference: None,
            targets: Vec::new(),
            rules: Vec::new(),
            metadata: IndexMap::default(),
        };
        self.render_field(&field, schema, draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(yaml: &str) -> Schema {
        serde_yaml::from_str(yaml).expect("test schema must parse")
    }

    fn test_schema() -> Schema {
        schema(
            r"
tables:
  Item:
    name: Item
    description: Equipment definitions.
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      name: { name: name, type: { kind: String }, required: true, min_length: 1, max_length: 24, pattern: '^[A-Z]' }
      kind: { name: kind, type: { kind: Enum, value: ItemKind }, required: true }
      rarity: { name: rarity, type: { kind: Enum, value: Rarity } }
      tags: { name: tags, type: { kind: Array, value: { kind: String } }, min_items: 1, max_items: 4 }
      prices: { name: prices, type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } }
      stock_by_day: { name: stock_by_day, type: { kind: Map, value: { key_type: int, value_type: { kind: Int32 } } } }
      meta: { name: meta, type: { kind: Object, value: { hp: { kind: Int32 }, note: { kind: String } } } }
      blob: { name: blob, type: { kind: Bytes } }
      payload: { name: payload, type: { kind: Any } }
      ghost: { name: ghost, type: { kind: Null } }
      flag: { name: flag, type: { kind: Bool }, default: true, description: Enabled flag }
      weight: { name: weight, type: { kind: Float64 }, min: 0.5, max: 99.5 }
      tier: { name: tier, type: { kind: Int32 }, enum_values: ['1', '2', '3'] }
  Empty:
    name: Empty
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt64 }, required: true }
enums:
  ItemKind:
    name: ItemKind
    description: Weapon category
    values:
      - { name: Sword, value: 1, description: Sword weapon }
      - { name: Shield, value: 2 }
  Rarity:
    name: Rarity
    values:
      - { name: common }
      - { name: rare }
",
        )
    }

    fn artifacts(draft: &str) -> Vec<(String, serde_json::Value)> {
        let mut config = TargetConfig {
            format: "jsonschema".into(),
            output_dir: "build/js".into(),
            file_template: None,
            options: None,
        };
        if !draft.is_empty() {
            config.options = Some(
                [("draft".to_string(), Json::String(draft.into()))]
                    .into_iter()
                    .collect(),
            );
        }
        let gen = JsonSchemaTargetGenerator::from_config(&config);
        gen.generate(&test_schema(), Some("abc123"))
            .expect("default draft is valid")
            .into_iter()
            .map(|(p, bytes)| {
                (
                    p,
                    serde_json::from_slice(&bytes).expect("output is valid JSON"),
                )
            })
            .collect()
    }

    fn doc<'a>(docs: &'a [(String, serde_json::Value)], suffix: &str) -> &'a serde_json::Value {
        &docs
            .iter()
            .find(|(p, _)| p.ends_with(suffix))
            .unwrap_or_else(|| panic!("doc {suffix} not found"))
            .1
    }

    #[test]
    fn test_artifact_paths_in_table_name_order() {
        let mut config = TargetConfig {
            format: "jsonschema".into(),
            output_dir: "build/schemas".into(),
            file_template: None,
            options: None,
        };
        config.options = Some(
            [("draft".to_string(), Json::String("07".into()))]
                .into_iter()
                .collect(),
        );
        let gen = JsonSchemaTargetGenerator::from_config(&config);
        let items = gen.generate(&test_schema(), Some("abc123")).expect("valid");
        let paths: Vec<&str> = items.iter().map(|(p, _)| p.as_str()).collect();
        // One self-contained document per table, tables in name order.
        assert_eq!(
            paths,
            vec![
                "build/schemas/Empty.schema.json",
                "build/schemas/Item.schema.json"
            ]
        );
    }

    #[test]
    fn test_document_shape_and_required_list() {
        let docs = artifacts("07");
        let item = doc(&docs, "Item.schema.json");
        assert_eq!(item["$schema"], "http://json-schema.org/draft-07/schema#");
        assert_eq!(item["title"], "Item");
        assert_eq!(item["description"], "Equipment definitions.");
        assert_eq!(item["type"], "object");
        assert_eq!(item["additionalProperties"], false);
        // Required collects exactly the required fields, declaration order.
        assert_eq!(item["required"], serde_json::json!(["id", "name", "kind"]));
        // Properties preserve declaration order (preserve_order feature).
        let keys: Vec<&String> = item["properties"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec![
                "id",
                "name",
                "kind",
                "rarity",
                "tags",
                "prices",
                "stock_by_day",
                "meta",
                "blob",
                "payload",
                "ghost",
                "flag",
                "weight",
                "tier"
            ]
        );
    }

    #[test]
    fn test_scalar_and_constraint_mapping() {
        let docs = artifacts("07");
        let props = &doc(&docs, "Item.schema.json")["properties"];
        assert_eq!(props["id"]["type"], "integer");
        assert_eq!(props["id"]["minimum"], 1);
        assert_eq!(props["name"]["type"], "string");
        assert_eq!(props["name"]["minLength"], 1);
        assert_eq!(props["name"]["maxLength"], 24);
        assert_eq!(props["name"]["pattern"], "^[A-Z]");
        assert_eq!(props["weight"]["type"], "number");
        assert_eq!(props["weight"]["minimum"], 0.5);
        assert_eq!(props["weight"]["maximum"], 99.5);
        assert_eq!(props["ghost"]["type"], "null");
        assert_eq!(props["flag"]["type"], "boolean");
        assert_eq!(props["flag"]["default"], true);
        assert_eq!(props["flag"]["description"], "Enabled flag");
        // Any imposes no constraint at all.
        assert_eq!(props["payload"], serde_json::json!({}));
    }

    #[test]
    fn test_enum_fields_are_member_name_strings() {
        let docs = artifacts("07");
        let props = &doc(&docs, "Item.schema.json")["properties"];
        // Integer-backed enum: instances are still member names (L2 types
        // enum fields as strings), so the domain is the names.
        assert_eq!(props["kind"]["type"], "string");
        assert_eq!(
            props["kind"]["enum"],
            serde_json::json!(["Sword", "Shield"])
        );
        // No-value enum: member names are the values.
        assert_eq!(
            props["rarity"]["enum"],
            serde_json::json!(["common", "rare"])
        );
    }

    #[test]
    fn test_composite_types() {
        let docs = artifacts("07");
        let props = &doc(&docs, "Item.schema.json")["properties"];
        // Array + item schema + item-count range.
        assert_eq!(props["tags"]["type"], "array");
        assert_eq!(props["tags"]["items"]["type"], "string");
        assert_eq!(props["tags"]["minItems"], 1);
        assert_eq!(props["tags"]["maxItems"], 4);
        // String-keyed map: open object over the value schema.
        assert_eq!(props["prices"]["type"], "object");
        assert_eq!(props["prices"]["additionalProperties"]["type"], "integer");
        assert!(props["prices"].get("propertyNames").is_none());
        // Int-keyed map: keys are numeric strings, constrained by pattern.
        assert_eq!(
            props["stock_by_day"]["propertyNames"]["pattern"],
            "^-?[0-9]+$"
        );
        // Typed object: declared properties, closed shape.
        assert_eq!(props["meta"]["type"], "object");
        assert_eq!(props["meta"]["properties"]["hp"]["type"], "integer");
        assert_eq!(props["meta"]["properties"]["note"]["type"], "string");
        assert_eq!(props["meta"]["additionalProperties"], false);
        // Bytes: base64 string annotation on draft-07.
        assert_eq!(props["blob"]["type"], "string");
        assert_eq!(props["blob"]["contentEncoding"], "base64");
        // Inline enum on an integer field: all-integer domain as numbers.
        assert_eq!(props["tier"]["type"], "integer");
        assert_eq!(props["tier"]["enum"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn test_dangling_enum_degrades_to_string() {
        let schema = schema(
            r"
tables:
  T:
    name: T
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      kind: { name: kind, type: { kind: Enum, value: Missing } }
enums: {}
",
        );
        let gen = JsonSchemaTargetGenerator::default();
        let items = gen.generate(&schema, None).expect("valid");
        let value: serde_json::Value = serde_json::from_slice(&items[0].1).unwrap();
        assert_eq!(value["properties"]["kind"]["type"], "string");
        assert!(value["properties"]["kind"].get("enum").is_none());
    }

    #[test]
    fn test_draft_2020_12() {
        let docs = artifacts("2020-12");
        let item = doc(&docs, "Item.schema.json");
        assert_eq!(
            item["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        // contentEncoding was removed in 2020-12 — the base64 annotation
        // only lands on draft-07 documents.
        assert!(item["properties"]["blob"].get("contentEncoding").is_none());
        assert_eq!(item["properties"]["blob"]["type"], "string");
    }

    #[test]
    fn test_unknown_draft_fails_at_generate() {
        let gen = JsonSchemaTargetGenerator::from_config(&TargetConfig {
            format: "jsonschema".into(),
            output_dir: "build/js".into(),
            file_template: None,
            options: Some(
                [("draft".to_string(), Json::String("draft-99".into()))]
                    .into_iter()
                    .collect(),
            ),
        });
        let err = gen
            .generate(&test_schema(), None)
            .expect_err("unknown draft must fail");
        assert!(err.contains("unknown draft 'draft-99'"), "{err}");
    }

    #[test]
    fn test_minimal_table_is_closed_object() {
        let docs = artifacts("07");
        let empty = doc(&docs, "Empty.schema.json");
        assert_eq!(empty["type"], "object");
        assert_eq!(empty["properties"]["id"]["type"], "integer");
        assert_eq!(empty["additionalProperties"], false);
        assert_eq!(empty["required"], serde_json::json!(["id"]));
    }

    #[test]
    fn test_deterministic_bytes_across_runs() {
        let gen = JsonSchemaTargetGenerator::default();
        let a = gen.generate(&test_schema(), Some("abc123")).expect("valid");
        let b = gen.generate(&test_schema(), Some("abc123")).expect("valid");
        assert_eq!(a, b);
        // Pretty-printed with a trailing newline.
        let item = &a
            .iter()
            .find(|(p, _)| p.ends_with("Item.schema.json"))
            .unwrap()
            .1;
        assert!(item.ends_with(b"\n"));
        assert!(std::str::from_utf8(item).unwrap().contains("\n  \"title\""));
    }

    #[test]
    fn test_from_config_defaults_and_options() {
        let gen = JsonSchemaTargetGenerator::from_config(&TargetConfig {
            format: "jsonschema".into(),
            output_dir: "out/x".into(),
            file_template: Some("{table}.json".into()),
            options: Some(
                [("draft".to_string(), Json::String("2020-12".into()))]
                    .into_iter()
                    .collect(),
            ),
        });
        assert_eq!(gen.output_dir, PathBuf::from("out/x"));
        assert_eq!(gen.file_template, "{table}.json");

        let bare = JsonSchemaTargetGenerator::from_config(&TargetConfig {
            format: "jsonschema".into(),
            output_dir: "out/x".into(),
            file_template: None,
            options: None,
        });
        assert_eq!(bare.file_template, "{table}.schema.json");
        assert_eq!(bare.draft, "07");
        assert_eq!(
            gen.generate(&test_schema(), None).expect("valid")[0].0,
            "out/x/Empty.json"
        );
    }
}
