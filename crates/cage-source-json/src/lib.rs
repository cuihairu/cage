//! JSON Source Adapter - parses JSON files into Cage's Canonical Model

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
use cage_core::{
    diagnostics::{Diagnostic, Diagnostics},
    error::codes::{internal, parse},
    value::{Document, Row, SourceLocation, Table, TypedValue, Value},
};
use indexmap::IndexMap;
use std::path::Path;

/// JSON Source Adapter
pub struct JsonSourceAdapter;

impl JsonSourceAdapter {
    /// Parse a JSON file into a Document
    pub fn parse_file(path: &Path) -> Result<Document, Diagnostics> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            let mut diags = Diagnostics::new();
            diags.add(
                Diagnostic::error(internal::E9902, format!("Failed to read file: {e}"))
                    .with_source(path.display().to_string()),
            );
            diags
        })?;

        Self::parse_str(&content, path)
    }

    /// Parse JSON string into a Document
    pub fn parse_str(content: &str, path: &Path) -> Result<Document, Diagnostics> {
        let mut diags = Diagnostics::new();
        let file_path = path.display().to_string();

        // Parse JSON with position tracking
        let value: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(e) => {
                let line_col = Self::byte_offset_to_line_col(content, e.line(), e.column());
                let loc = SourceLocation::new(&file_path)
                    .with_line_col(line_col.0, line_col.1)
                    .with_byte_offset(e.line());

                diags.add(Diagnostic::error(parse::E0001, format!("JSON syntax error: {e}"))
                    .with_location(loc)
                    .with_hint("Check JSON syntax - common issues: trailing commas, unquoted keys, mismatched brackets"));
                return Err(diags);
            }
        };

        let mut doc = Document::new();
        doc.metadata.format = "json".to_string();
        doc.source_files.push(file_path.clone());

        // Convert JSON value to Document
        match Self::json_to_document(value, &file_path, &mut diags) {
            Ok(tables) => {
                for table in tables {
                    doc.add_table(table);
                }
                if diags.has_errors() {
                    Err(diags)
                } else {
                    Ok(doc)
                }
            }
            Err(e) => {
                diags.add(e);
                Err(diags)
            }
        }
    }

    fn json_to_document(
        value: serde_json::Value,
        file_path: &str,
        diags: &mut Diagnostics,
    ) -> Result<Vec<Table>, Diagnostic> {
        let mut tables = Vec::new();

        match value {
            serde_json::Value::Object(obj) => {
                // Check if this is a multi-table format: { "TableName": [...], ... }
                let mut has_array_values = false;
                for v in obj.values() {
                    if v.is_array() {
                        has_array_values = true;
                        break;
                    }
                }

                if has_array_values && obj.values().all(|v| v.is_array() || v.is_null()) {
                    // Multi-table format
                    for (table_name, table_value) in obj {
                        if let serde_json::Value::Array(rows) = table_value {
                            if let Some(table) =
                                Self::array_to_table(&table_name, rows, file_path, 0, diags)
                            {
                                tables.push(table);
                            }
                        }
                    }
                } else {
                    // Single object - treat as single-row table
                    tables.push(Self::object_to_table("Root", &obj, file_path, 0, diags));
                }
            }
            serde_json::Value::Array(rows) => {
                // Top-level array - single table
                if let Some(table) = Self::array_to_table("Data", rows, file_path, 0, diags) {
                    tables.push(table);
                }
            }
            _ => {
                return Err(Diagnostic::error(
                    parse::E0004,
                    "JSON root must be an object or array",
                )
                .with_source(file_path));
            }
        }

        Ok(tables)
    }

    fn array_to_table(
        name: &str,
        rows: Vec<serde_json::Value>,
        file_path: &str,
        sheet_index: usize,
        diags: &mut Diagnostics,
    ) -> Option<Table> {
        if rows.is_empty() {
            return None;
        }

        let mut table = Table {
            name: name.to_string(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.to_string(),
            sheet: if sheet_index > 0 {
                Some(format!("Sheet{sheet_index}"))
            } else {
                None
            },
        };

        for (row_idx, row_value) in rows.into_iter().enumerate() {
            match row_value {
                serde_json::Value::Object(obj) => {
                    let row = Self::object_to_row(name, &obj, file_path, row_idx, diags);
                    table.rows.push(row);
                }
                _ => {
                    diags.add(
                        Diagnostic::warning(
                            parse::E0001,
                            format!("Row {row_idx} is not an object, skipping"),
                        )
                        .with_source(file_path)
                        .with_table(name)
                        .with_row(row_idx.to_string()),
                    );
                }
            }
        }

        if table.rows.is_empty() {
            None
        } else {
            Some(table)
        }
    }

    fn object_to_table(
        name: &str,
        obj: &serde_json::Map<String, serde_json::Value>,
        file_path: &str,
        sheet_index: usize,
        diags: &mut Diagnostics,
    ) -> Table {
        let mut table = Table {
            name: name.to_string(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.to_string(),
            sheet: if sheet_index > 0 {
                Some(format!("Sheet{sheet_index}"))
            } else {
                None
            },
        };

        let row = Self::object_to_row(name, obj, file_path, 0, diags);
        table.rows.push(row);
        table
    }

    fn object_to_row(
        _table_name: &str,
        obj: &serde_json::Map<String, serde_json::Value>,
        file_path: &str,
        row_idx: usize,
        _diags: &mut Diagnostics,
    ) -> Row {
        let mut fields = IndexMap::new();
        let mut primary_key = Vec::new();

        // Sort keys for deterministic ordering
        let mut keys: Vec<_> = obj.keys().collect();
        keys.sort();

        for key in keys {
            let value = obj[key].clone();
            let cage_value = Self::json_value_to_cage(value);
            let loc = SourceLocation::new(file_path)
                .with_row(row_idx + 1)
                .with_column(key.clone())
                .with_field(key.clone());

            fields.insert(key.clone(), TypedValue::new(cage_value, loc));

            // Heuristic: first field named "id" or "ID" is primary key
            if primary_key.is_empty()
                && (key.eq_ignore_ascii_case("id") || key.eq_ignore_ascii_case("ID"))
            {
                primary_key.push(fields[key].value.clone());
            }
        }

        // If no explicit ID field, use first field as primary key
        if primary_key.is_empty() {
            if let Some((_first_key, first_value)) = fields.iter().next() {
                primary_key.push(first_value.value.clone());
            }
        }

        Row {
            primary_key,
            fields,
            location: SourceLocation::new(file_path).with_row(row_idx + 1),
            index: row_idx,
        }
    }

    fn json_value_to_cage(value: serde_json::Value) -> Value {
        match value {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Value::UInt(u)
                } else if let Some(f) = n.as_f64() {
                    Value::Float(f)
                } else {
                    Value::String(n.to_string())
                }
            }
            serde_json::Value::String(s) => Value::String(s),
            serde_json::Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Self::json_value_to_cage).collect())
            }
            serde_json::Value::Object(obj) => {
                let mut map = IndexMap::new();
                for (k, v) in obj {
                    map.insert(k, Self::json_value_to_cage(v));
                }
                Value::Object(map)
            }
        }
    }

    fn byte_offset_to_line_col(_content: &str, line: usize, col: usize) -> (usize, usize) {
        // serde_json gives 1-indexed line/col, but we want to be precise
        // For simplicity, return as-is (serde_json uses 1-indexed)
        (line, col)
    }
}

/// Trait for source adapters (for future plugin system)
pub trait SourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics>;
    fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics>;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn format_name(&self) -> &'static str;
}

impl SourceAdapter for JsonSourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        Self::parse_file(path)
    }

    fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics> {
        Self::parse_str(content, path)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["json"]
    }

    fn format_name(&self) -> &'static str {
        "JSON"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::value::Value;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_json_array() {
        let json = r#"[
            {"id": 1, "name": "Item 1", "price": 100},
            {"id": 2, "name": "Item 2", "price": 200}
        ]"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let doc = JsonSourceAdapter::parse_file(file.path()).unwrap();
        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("Data").unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].fields["id"].value, Value::Int(1));
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("Item 1".to_string())
        );
    }

    #[test]
    fn test_parse_json_multi_table() {
        let json = r#"{
            "Item": [
                {"id": 1, "name": "Sword"},
                {"id": 2, "name": "Shield"}
            ],
            "Monster": [
                {"id": 100, "name": "Goblin", "hp": 50}
            ]
        }"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let doc = JsonSourceAdapter::parse_file(file.path()).unwrap();
        assert_eq!(doc.tables.len(), 2);
        assert!(doc.tables.contains_key("Item"));
        assert!(doc.tables.contains_key("Monster"));
    }

    #[test]
    fn test_parse_json_single_object() {
        let json = r#"{"id": 42, "name": "Single Item", "tags": ["weapon", "rare"]}"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let doc = JsonSourceAdapter::parse_file(file.path()).unwrap();
        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("Root").unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(
            table.rows[0].fields["tags"].value,
            Value::Array(vec![
                Value::String("weapon".to_string()),
                Value::String("rare".to_string()),
            ])
        );
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = r#"{"id": 1, "name": "Missing quote}"#;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let result = JsonSourceAdapter::parse_file(file.path());
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(diags.has_errors());
        assert_eq!(diags.errors()[0].code, parse::E0001);
    }

    #[test]
    fn test_source_adapter_trait() {
        let adapter = JsonSourceAdapter;
        assert_eq!(adapter.supported_extensions(), &["json"]);
        assert_eq!(adapter.format_name(), "JSON");
    }
}
