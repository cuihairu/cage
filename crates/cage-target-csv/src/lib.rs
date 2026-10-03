//! CSV Target Generator - generates CSV artifacts from validated Cage model

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
    value::{Document, Table, Value},
};
use csv::WriterBuilder;
use std::path::PathBuf;

/// CSV Target Generator
pub struct CsvTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.csv")
    pub file_template: String,
    /// Delimiter character
    pub delimiter: u8,
    /// Whether to write header row
    pub write_header: bool,
    /// Quote style
    pub quote_style: csv::QuoteStyle,
}

impl Default for CsvTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/csv"),
            file_template: "{table}.csv".to_string(),
            delimiter: b',',
            write_header: true,
            quote_style: csv::QuoteStyle::Necessary,
        }
    }
}

impl CsvTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.csv".to_string()),
            delimiter: b',',
            write_header: true,
            quote_style: csv::QuoteStyle::Necessary,
        };

        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("delimiter") {
                if let Some(s) = v.as_str() {
                    if s.len() == 1 {
                        gen.delimiter = s.as_bytes()[0];
                    }
                }
            }
            if let Some(v) = opts.get("write_header") {
                gen.write_header = v.as_bool().unwrap_or(true);
            }
            if let Some(v) = opts.get("quote_style") {
                if let Some(s) = v.as_str() {
                    gen.quote_style = match s {
                        "always" => csv::QuoteStyle::Always,
                        "never" => csv::QuoteStyle::Never,
                        "non_numeric" => csv::QuoteStyle::NonNumeric,
                        _ => csv::QuoteStyle::Necessary,
                    };
                }
            }
        }

        gen
    }

    /// Generate CSV artifacts for all tables in document
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

    /// Generate CSV for a single table
    pub fn generate_table(&self, table: &Table) -> Result<(String, Vec<u8>), Diagnostic> {
        // Collect all field names in order of first appearance
        let mut field_names: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for row in &table.rows {
            for field_name in row.fields.keys() {
                if seen.insert(field_name.clone()) {
                    field_names.push(field_name.clone());
                }
            }
        }

        // Empty table: fall back to the primary key fields as headers
        if field_names.is_empty() && !table.primary_key_fields.is_empty() {
            field_names.clone_from(&table.primary_key_fields);
        }

        let mut buffer = Vec::new();
        {
            let mut writer = WriterBuilder::new()
                .delimiter(self.delimiter)
                .quote_style(self.quote_style)
                .from_writer(&mut buffer);

            // Write header
            if self.write_header && !field_names.is_empty() {
                writer.write_record(&field_names).map_err(|e| {
                    Diagnostic::error(build::E9002, format!("CSV write header failed: {e}"))
                        .with_source("csv-target")
                        .with_table(&table.name)
                })?;
            }

            // Write rows
            for row in &table.rows {
                let mut record = Vec::with_capacity(field_names.len());
                for field_name in &field_names {
                    let value = row
                        .fields
                        .get(field_name)
                        .map_or(&Value::Null, |tv| &tv.value);
                    let normalized = normalize_value(value);
                    record.push(Self::cage_value_to_string(normalized));
                }
                writer.write_record(&record).map_err(|e| {
                    Diagnostic::error(build::E9002, format!("CSV write row failed: {e}"))
                        .with_source("csv-target")
                        .with_table(&table.name)
                        .with_row(row.index.to_string())
                })?;
            }

            writer.flush().map_err(|e| {
                Diagnostic::error(build::E9002, format!("CSV flush failed: {e}"))
                    .with_source("csv-target")
                    .with_table(&table.name)
            })?;
        }

        let file_name = self.file_template.replace("{table}", &table.name);
        let path = self
            .output_dir
            .join(&file_name)
            .to_string_lossy()
            .to_string();

        Ok((path, buffer))
    }

    fn cage_value_to_string(value: Value) -> String {
        match value {
            Value::Null => String::new(),
            Value::Bool(b) => if b { "true" } else { "false" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => {
                // Use fixed precision for determinism
                if f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    format!("{f:.10}").trim_end_matches('0').to_string()
                }
            }
            Value::String(s) => s,
            Value::Bytes(b) => base64::engine::general_purpose::STANDARD.encode(b),
            Value::Array(arr) => {
                // Serialize arrays as JSON strings for CSV
                serde_json::to_string(&arr.iter().map(Self::cage_value_to_json).collect::<Vec<_>>())
                    .unwrap_or_default()
            }
            Value::Object(obj) => {
                // Serialize objects as JSON strings for CSV
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    map.insert(k.clone(), Self::cage_value_to_json(&v));
                }
                serde_json::to_string(&map).unwrap_or_default()
            }
        }
    }

    fn cage_value_to_json(value: &Value) -> serde_json::Value {
        match value {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Int(i) => serde_json::Value::Number((*i).into()),
            Value::UInt(u) => serde_json::Value::Number((*u).into()),
            Value::Float(f) => serde_json::Number::from_f64(*f)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::Bytes(b) => {
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b))
            }
            Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::cage_value_to_json).collect())
            }
            Value::Object(obj) => {
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    map.insert(k.clone(), Self::cage_value_to_json(v));
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

impl TargetGenerator for CsvTargetGenerator {
    fn generate(
        &self,
        document: &Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics> {
        self.generate(document, profile_targets)
    }

    fn format_name(&self) -> &'static str {
        "CSV"
    }

    fn file_extension(&self) -> &'static str {
        "csv"
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
            source_file: "items.csv".to_string(),
            sheet: None,
        };
        table.rows.push(Row {
            primary_key: vec![Value::UInt(1)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(1), SourceLocation::new("items.csv")),
                );
                f.insert(
                    "name".to_string(),
                    TypedValue::new(
                        Value::String("Sword".to_string()),
                        SourceLocation::new("items.csv"),
                    ),
                );
                f.insert(
                    "price".to_string(),
                    TypedValue::new(Value::Int(100), SourceLocation::new("items.csv")),
                );
                f
            },
            location: SourceLocation::new("items.csv"),
            index: 0,
        });
        table.rows.push(Row {
            primary_key: vec![Value::UInt(2)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(2), SourceLocation::new("items.csv")),
                );
                f.insert(
                    "name".to_string(),
                    TypedValue::new(
                        Value::String("Shield".to_string()),
                        SourceLocation::new("items.csv"),
                    ),
                );
                f.insert(
                    "price".to_string(),
                    TypedValue::new(Value::Int(200), SourceLocation::new("items.csv")),
                );
                f
            },
            location: SourceLocation::new("items.csv"),
            index: 1,
        });
        doc.add_table(table);
        doc
    }

    #[test]
    fn test_generate_csv() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = CsvTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            write_header: true,
            ..Default::default()
        };

        let artifacts = gen.generate(&doc, &[]).unwrap();
        assert_eq!(artifacts.len(), 1);

        let (path, content) = &artifacts[0];
        assert!(path.ends_with("Item.csv"));

        let csv_str = String::from_utf8(content.clone()).unwrap();
        let lines: Vec<&str> = csv_str.lines().collect();
        assert_eq!(lines.len(), 3); // header + 2 rows
        assert_eq!(lines[0], "id,name,price");
        assert_eq!(lines[1], "1,Sword,100");
        assert_eq!(lines[2], "2,Shield,200");
    }

    #[test]
    fn test_generate_csv_no_header() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = CsvTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            write_header: false,
            ..Default::default()
        };

        let artifacts = gen.generate(&doc, &[]).unwrap();
        let (_, content) = &artifacts[0];

        let csv_str = String::from_utf8(content.clone()).unwrap();
        let lines: Vec<&str> = csv_str.lines().collect();
        assert_eq!(lines.len(), 2); // no header
        assert_eq!(lines[0], "1,Sword,100");
    }

    #[test]
    fn test_generate_tsv() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = CsvTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            delimiter: b'\t',
            ..Default::default()
        };

        let artifacts = gen.generate(&doc, &[]).unwrap();
        let (_, content) = &artifacts[0];

        let tsv_str = String::from_utf8(content.clone()).unwrap();
        assert!(tsv_str.contains('\t'));
        assert!(!tsv_str.contains(','));
    }

    #[test]
    fn test_deterministic_output() {
        let doc = make_test_doc();
        let temp_dir = TempDir::new().unwrap();
        let gen = CsvTargetGenerator {
            output_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let artifacts1 = gen.generate(&doc, &[]).unwrap();
        let artifacts2 = gen.generate(&doc, &[]).unwrap();

        assert_eq!(artifacts1[0].1, artifacts2[0].1);
    }

    #[test]
    fn test_cage_value_to_string() {
        use cage_core::value::Value;

        assert_eq!(CsvTargetGenerator::cage_value_to_string(Value::Null), "");
        assert_eq!(
            CsvTargetGenerator::cage_value_to_string(Value::Bool(true)),
            "true"
        );
        assert_eq!(
            CsvTargetGenerator::cage_value_to_string(Value::Bool(false)),
            "false"
        );
        assert_eq!(
            CsvTargetGenerator::cage_value_to_string(Value::Int(42)),
            "42"
        );
        assert_eq!(
            CsvTargetGenerator::cage_value_to_string(Value::Float(3.14)),
            "3.14"
        );
    }

    #[test]
    fn test_target_generator_trait() {
        let gen = CsvTargetGenerator::default();
        assert_eq!(gen.format_name(), "CSV");
        assert_eq!(gen.file_extension(), "csv");
    }
}
