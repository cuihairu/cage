//! JSON Target Generator - generates JSON artifacts from validated Cage model

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: the Value/normalize layer is a numeric coercion engine —
// float<->int casts are its job, not a defect.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::approx_constant,
    clippy::checked_conversions
)]
// Stage: crate-prefixed type names (JsonTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: Diagnostics is the deliberate first-class error type,
// returned by value from every pipeline stage.
#![allow(clippy::result_large_err)]
// Restriction lints kept off for a 0.1.0 codebase.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names
)]
use base64::Engine;
use cage_core::{
    diagnostics::{Diagnostic, Diagnostics},
    error::codes::build,
    manifest::TargetConfig,
    normalize::normalize_value,
    value::{Document, Row, Table, Value},
};
use std::path::PathBuf;

/// JSON Target Generator
pub struct JsonTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.json")
    pub file_template: String,
    /// Pretty print output
    pub pretty: bool,
    /// Sort keys for deterministic output
    pub sort_keys: bool,
}

impl Default for JsonTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/json"),
            file_template: "{table}.json".to_string(),
            pretty: true,
            sort_keys: true,
        }
    }
}

impl JsonTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.json".to_string()),
            pretty: true,
            sort_keys: true,
        };

        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("pretty") {
                gen.pretty = v.as_bool().unwrap_or(true);
            }
            if let Some(v) = opts.get("sort_keys") {
                gen.sort_keys = v.as_bool().unwrap_or(true);
            }
        }

        gen
    }

    /// Generate JSON artifacts for all tables in document
    pub fn generate(
        &self,
        document: &Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics> {
        let mut diags = Diagnostics::new();
        let mut artifacts = Vec::new();

        // Filter tables by profile targets if specified
        let tables_to_generate: Vec<_> = if profile_targets.is_empty() {
            document.tables.values().collect()
        } else {
            document
                .tables
                .values()
                .filter(|t| {
                    profile_targets.contains(&t.name) || profile_targets.contains(&"*".to_string())
                })
                .collect()
        };

        for table in tables_to_generate {
            match self.generate_table(table) {
                Ok((path, content)) => artifacts.push((path, content)),
                Err(e) => {
                    diags.add(e);
                }
            }
        }

        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(artifacts)
        }
    }

    /// Generate JSON for a single table
    pub fn generate_table(&self, table: &Table) -> Result<(String, Vec<u8>), Diagnostic> {
        // Build JSON array of objects
        let rows_json: Vec<serde_json::Value> = table
            .rows
            .iter()
            .map(|row| serde_json::Value::Object(self.row_to_object(row)))
            .collect();

        let json_value = serde_json::Value::Array(rows_json);

        let content = if self.pretty {
            serde_json::to_vec_pretty(&json_value)
        } else {
            serde_json::to_vec(&json_value)
        }
        .map_err(|e| {
            Diagnostic::error(build::E9002, format!("JSON serialization failed: {e}"))
                .with_source("json-target")
                .with_table(&table.name)
        })?;

        let file_name = self.file_template.replace("{table}", &table.name);
        let path = self
            .output_dir
            .join(&file_name)
            .to_string_lossy()
            .to_string();

        Ok((path, content))
    }

    /// Convert one row into a JSON object.
    /// `sort_keys = true` sorts fields by name (fully deterministic);
    /// `sort_keys = false` keeps source field order.
    fn row_to_object(&self, row: &Row) -> serde_json::Map<String, serde_json::Value> {
        let mut entries: Vec<(String, serde_json::Value)> = row
            .fields
            .iter()
            .map(|(field_name, typed_value)| {
                let normalized = normalize_value(&typed_value.value);
                (field_name.clone(), Self::cage_value_to_json(normalized))
            })
            .collect();

        if self.sort_keys {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
        }

        serde_json::Map::from_iter(entries)
    }

    /// Generate single combined JSON file with all tables
    pub fn generate_combined(&self, document: &Document) -> Result<(String, Vec<u8>), Diagnostic> {
        let mut root = serde_json::Map::new();

        for (table_name, table) in &document.tables {
            let rows_json: Vec<serde_json::Value> = table
                .rows
                .iter()
                .map(|row| serde_json::Value::Object(self.row_to_object(row)))
                .collect();

            root.insert(table_name.clone(), serde_json::Value::Array(rows_json));
        }

        let entries: Vec<(String, serde_json::Value)> = if self.sort_keys {
            let mut sorted: Vec<(String, serde_json::Value)> = root.into_iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            sorted
        } else {
            root.into_iter().collect()
        };

        let json_value = serde_json::Value::Object(serde_json::Map::from_iter(entries));

        let content = if self.pretty {
            serde_json::to_vec_pretty(&json_value)
        } else {
            serde_json::to_vec(&json_value)
        }
        .map_err(|e| {
            Diagnostic::error(build::E9002, format!("JSON serialization failed: {e}"))
                .with_source("json-target")
        })?;

        let file_name = "all.json";
        let path = self
            .output_dir
            .join(file_name)
            .to_string_lossy()
            .to_string();

        Ok((path, content))
    }

    fn cage_value_to_json(value: Value) -> serde_json::Value {
        match value {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(b),
            Value::Int(i) => serde_json::Value::Number(serde_json::Number::from(i)),
            Value::UInt(u) => serde_json::Value::Number(serde_json::Number::from(u)),
            Value::Float(f) => serde_json::Number::from_f64(f)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            Value::String(s) => serde_json::Value::String(s),
            Value::Bytes(b) => {
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b))
            }
            Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(Self::cage_value_to_json).collect())
            }
            Value::Object(obj) => {
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    map.insert(k, Self::cage_value_to_json(v));
                }
                serde_json::Value::Object(map)
            }
        }
    }
}

/// Trait for target generators
pub trait TargetGenerator {
    fn generate(
        &self,
        document: &cage_core::value::Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics>;
    fn format_name(&self) -> &'static str;
    fn file_extension(&self) -> &'static str;
}

impl TargetGenerator for JsonTargetGenerator {
    fn generate(
        &self,
        document: &Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics> {
        self.generate(document, profile_targets)
    }

    fn format_name(&self) -> &'static str {
        "JSON"
    }

    fn file_extension(&self) -> &'static str {
        "json"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
    use indexmap::IndexMap;
    use tempfile::TempDir;

    fn make_test_doc() -> Document {
        let mut doc = Document::new();
        let mut table = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "items.json".to_string(),
            sheet: None,
        };
        table.rows.push(Row {
            primary_key: vec![Value::UInt(1)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(1), SourceLocation::new("items.json")),
                );
                f.insert(
                    "name".to_string(),
                    TypedValue::new(
                        Value::String("Sword".to_string()),
                        SourceLocation::new("items.json"),
                    ),
                );
                f.insert(
                    "price".to_string(),
                    TypedValue::new(Value::Int(100), SourceLocation::new("items.json")),
                );
                f
            },
            location: SourceLocation::new("items.json"),
            index: 0,
        });
        table.rows.push(Row {
            primary_key: vec![Value::UInt(2)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(2), SourceLocation::new("items.json")),
                );
                f.insert(
                    "name".to_string(),
                    TypedValue::new(
                        Value::String("Shield".to_string()),
                        SourceLocation::new("items.json"),
                    ),
                );
                f.insert(
                    "price".to_string(),
                    TypedValue::new(Value::Int(200), SourceLocation::new("items.json")),
                );
                f
            },
            location: SourceLocation::new("items.json"),
            index: 1,
        });
        doc.add_table(table);
        doc
    }

    #[test]
    fn test_generate_json() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = JsonTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            pretty: true,
            sort_keys: true,
            ..Default::default()
        };

        let artifacts = gen.generate(&doc, &[]).unwrap();
        assert_eq!(artifacts.len(), 1);

        let (path, content) = &artifacts[0];
        assert!(path.ends_with("Item.json"));

        let json: serde_json::Value = serde_json::from_slice(content).unwrap();
        assert!(json.is_array());
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["name"], "Sword");
        assert_eq!(arr[0]["price"], 100);
    }

    #[test]
    fn test_generate_combined() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = JsonTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let (path, content) = gen.generate_combined(&doc).unwrap();
        assert!(path.ends_with("all.json"));

        let json: serde_json::Value = serde_json::from_slice(&content).unwrap();
        assert!(json.is_object());
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("Item"));
    }

    #[test]
    fn test_deterministic_output() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = JsonTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            sort_keys: true,
            pretty: false,
            ..Default::default()
        };

        let artifacts1 = gen.generate(&doc, &[]).unwrap();
        let artifacts2 = gen.generate(&doc, &[]).unwrap();

        assert_eq!(artifacts1[0].1, artifacts2[0].1);
    }

    #[test]
    fn test_cage_value_to_json() {
        use cage_core::value::Value;

        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Null),
            serde_json::Value::Null
        );
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Bool(true)),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Int(42)),
            serde_json::Value::Number(42.into())
        );
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::String("test".to_string())),
            serde_json::Value::String("test".to_string())
        );
    }

    #[test]
    fn test_target_generator_trait() {
        let gen = JsonTargetGenerator::default();
        assert_eq!(gen.format_name(), "JSON");
        assert_eq!(gen.file_extension(), "json");
    }

    /// Build a `TargetConfig` directly — this crate has no `serde_yaml`
    /// dev-dependency; the struct is plain data.
    fn config(
        output_dir: &str,
        file_template: Option<&str>,
        options: Option<indexmap::IndexMap<String, serde_json::Value>>,
    ) -> TargetConfig {
        TargetConfig {
            format: "json".to_string(),
            output_dir: output_dir.to_string(),
            file_template: file_template.map(str::to_string),
            options,
        }
    }

    fn opts(pairs: &[(&str, serde_json::Value)]) -> indexmap::IndexMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_from_config_defaults() {
        let gen = JsonTargetGenerator::from_config(&config("build/j", None, None));
        assert_eq!(gen.output_dir, PathBuf::from("build/j"));
        assert_eq!(gen.file_template, "{table}.json");
        assert!(gen.pretty);
        assert!(gen.sort_keys);
    }

    #[test]
    fn test_from_config_option_overrides() {
        let gen = JsonTargetGenerator::from_config(&config(
            "build/j",
            Some("{table}_gen.json"),
            Some(opts(&[
                ("pretty", serde_json::json!(false)),
                ("sort_keys", serde_json::json!(false)),
            ])),
        ));
        assert_eq!(gen.file_template, "{table}_gen.json");
        assert!(!gen.pretty);
        assert!(!gen.sort_keys);

        // Non-bool option values fall back to the defaults.
        let gen = JsonTargetGenerator::from_config(&config(
            "build/j",
            None,
            Some(opts(&[
                ("pretty", serde_json::json!("yes")),
                ("sort_keys", serde_json::json!(1)),
            ])),
        ));
        assert!(gen.pretty);
        assert!(gen.sort_keys);

        // Options present but unrelated keys: defaults kept.
        let gen = JsonTargetGenerator::from_config(&config(
            "build/j",
            None,
            Some(opts(&[("unknown", serde_json::json!(true))])),
        ));
        assert!(gen.pretty);
        assert!(gen.sort_keys);
    }

    #[test]
    fn test_profile_target_filtering() {
        let doc = make_test_doc();
        let gen = JsonTargetGenerator::default();
        let matched = gen.generate(&doc, &["Item".to_string()]).unwrap();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].0.ends_with("Item.json"));
        let all = gen.generate(&doc, &["*".to_string()]).unwrap();
        assert_eq!(all.len(), 1);
        let none = gen.generate(&doc, &["Ghost".to_string()]).unwrap();
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn test_generate_unsorted_compact_preserves_source_order() {
        // Fields deliberately inserted in non-name order: sort_keys = false
        // keeps source order, pretty = false emits the compact form.
        let mut doc = Document::new();
        let mut table = Table {
            name: "Order".to_string(),
            primary_key_fields: vec!["zeta".to_string()],
            rows: vec![],
            source_file: "order.json".to_string(),
            sheet: None,
        };
        let mut fields = IndexMap::new();
        for name in ["zeta", "alpha"] {
            fields.insert(
                name.to_string(),
                TypedValue::new(
                    Value::String(name.to_string()),
                    SourceLocation::new("order.json"),
                ),
            );
        }
        table.rows.push(Row {
            primary_key: vec![Value::String("zeta".to_string())],
            fields,
            location: SourceLocation::new("order.json"),
            index: 0,
        });
        doc.add_table(table);

        let unsorted = JsonTargetGenerator {
            output_dir: PathBuf::from("build/j"),
            pretty: false,
            sort_keys: false,
            ..Default::default()
        };
        let artifacts = unsorted.generate(&doc, &[]).unwrap();
        assert_eq!(
            String::from_utf8(artifacts[0].1.clone()).unwrap(),
            "[{\"zeta\":\"zeta\",\"alpha\":\"alpha\"}]"
        );

        // Combined output with the same options keeps insertion order too.
        let (path, content) = unsorted.generate_combined(&doc).unwrap();
        assert!(path.ends_with("all.json"));
        assert_eq!(
            String::from_utf8(content).unwrap(),
            "{\"Order\":[{\"zeta\":\"zeta\",\"alpha\":\"alpha\"}]}"
        );

        // The sorted variant reorders the very same source data.
        let sorted = JsonTargetGenerator {
            sort_keys: true,
            ..unsorted
        };
        let artifacts = sorted.generate(&doc, &[]).unwrap();
        assert_eq!(
            String::from_utf8(artifacts[0].1.clone()).unwrap(),
            "[{\"alpha\":\"alpha\",\"zeta\":\"zeta\"}]"
        );
        let (_, content) = sorted.generate_combined(&doc).unwrap();
        assert_eq!(
            String::from_utf8(content).unwrap(),
            "{\"Order\":[{\"alpha\":\"alpha\",\"zeta\":\"zeta\"}]}"
        );
    }

    #[test]
    fn test_generate_combined_sorted_across_tables() {
        // Two tables so the sort comparator actually compares: a single
        // entry never calls it.
        let mut doc = make_test_doc();
        let mut extra = Table {
            name: "Alpha".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "alpha.json".to_string(),
            sheet: None,
        };
        extra.rows.push(Row {
            primary_key: vec![Value::UInt(9)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(9), SourceLocation::new("alpha.json")),
                );
                f
            },
            location: SourceLocation::new("alpha.json"),
            index: 0,
        });
        doc.add_table(extra);

        let gen = JsonTargetGenerator {
            output_dir: PathBuf::from("build/j"),
            pretty: false,
            sort_keys: true,
            ..Default::default()
        };
        let (_, content) = gen.generate_combined(&doc).unwrap();
        // Table keys sorted by name: Alpha before Item.
        assert_eq!(
            String::from_utf8(content).unwrap(),
            "{\"Alpha\":[{\"id\":9}],\"Item\":[{\"id\":1,\"name\":\"Sword\",\"price\":100},{\"id\":2,\"name\":\"Shield\",\"price\":200}]}"
        );
    }

    #[test]
    fn test_generate_row_with_all_value_kinds() {
        // Every Value variant through the real generate() pipeline:
        // Float non-finite → JSON null, Bytes → base64, nested composites,
        // unicode, and -0.0 canonicalization by normalize_value.
        let mut doc = Document::new();
        let mut table = Table {
            name: "Rich".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "rich.json".to_string(),
            sheet: None,
        };
        let mut fields = IndexMap::new();
        let mut add = |name: &str, value: Value| {
            fields.insert(
                name.to_string(),
                TypedValue::new(value, SourceLocation::new("rich.json")),
            );
        };
        add("id", Value::UInt(u64::MAX));
        add("nil", Value::Null);
        add("flag", Value::Bool(false));
        add("delta", Value::Int(i64::MIN));
        add("ratio", Value::Float(-0.0)); // normalize → 0.0
        add("huge", Value::Float(1.5e308));
        add("text", Value::String("  列 ⚔  ".to_string())); // trim + unicode
        add("blob", Value::Bytes(b"Hello".to_vec()));
        add("list", Value::Array(vec![Value::Int(1), Value::Null]));
        add("map", {
            let mut m = IndexMap::new();
            m.insert("k".to_string(), Value::Bool(true));
            m.insert("deep".to_string(), Value::Array(vec![Value::UInt(2)]));
            Value::Object(m)
        });
        table.rows.push(Row {
            primary_key: vec![Value::UInt(u64::MAX)],
            fields,
            location: SourceLocation::new("rich.json"),
            index: 0,
        });
        doc.add_table(table);

        let gen = JsonTargetGenerator {
            output_dir: PathBuf::from("build/j"),
            pretty: false,
            sort_keys: true,
            ..Default::default()
        };
        let artifacts = gen.generate(&doc, &[]).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&artifacts[0].1).unwrap();
        assert_eq!(json[0]["id"], u64::MAX);
        assert_eq!(json[0]["nil"], serde_json::Value::Null);
        assert_eq!(json[0]["flag"], false);
        assert_eq!(json[0]["delta"], i64::MIN);
        assert_eq!(json[0]["ratio"], serde_json::json!(0.0));
        assert_eq!(json[0]["huge"], serde_json::json!(1.5e308));
        assert_eq!(json[0]["text"], "列 ⚔");
        assert_eq!(json[0]["blob"], "SGVsbG8=");
        assert_eq!(json[0]["list"], serde_json::json!([1, null]));
        assert_eq!(json[0]["map"], serde_json::json!({"k": true, "deep": [2]}));
        // Byte-for-byte deterministic.
        let again = gen.generate(&doc, &[]).unwrap();
        assert_eq!(artifacts[0].1, again[0].1);
    }

    #[test]
    fn test_cage_value_to_json_numeric_composite_arms() {
        use cage_core::value::Value;

        // UInt / Float arms (Int/Bool/Null/String already covered above).
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::UInt(u64::MAX)),
            serde_json::Value::Number(serde_json::Number::from(u64::MAX))
        );
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Float(2.5)),
            serde_json::json!(2.5)
        );
        // Non-finite floats have no JSON number → null.
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Float(f64::NAN)),
            serde_json::Value::Null
        );
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Float(f64::NEG_INFINITY)),
            serde_json::Value::Null
        );
        // Bytes → base64 string.
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Bytes(b"Hello".to_vec())),
            serde_json::json!("SGVsbG8=")
        );
        // Nested array / object arms recurse.
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Array(vec![
                Value::Int(-1),
                Value::Array(vec![Value::Bool(true)]),
            ])),
            serde_json::json!([-1, [true]])
        );
        let mut obj = IndexMap::new();
        obj.insert("k".to_string(), Value::UInt(7));
        obj.insert("nil".to_string(), Value::Null);
        assert_eq!(
            JsonTargetGenerator::cage_value_to_json(Value::Object(obj)),
            serde_json::json!({"k": 7, "nil": null})
        );
    }

    #[test]
    fn test_target_generator_trait_dispatch() {
        let doc = make_test_doc();
        let gen = JsonTargetGenerator::default();
        let artifacts = TargetGenerator::generate(&gen, &doc, &[]).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].0.ends_with("Item.json"));
    }
}
