//! CSV Source Adapter - parses CSV files into Cage's Canonical Model

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
use csv::ReaderBuilder;
use indexmap::IndexMap;
use std::path::Path;

/// CSV Source Adapter
pub struct CsvSourceAdapter {
    /// Whether the first row is a header row
    pub has_header: bool,
    /// Delimiter character
    pub delimiter: u8,
    /// Whether to trim whitespace from fields
    pub trim: bool,
    /// Flexible parsing (allow variable field counts)
    pub flexible: bool,
}

impl Default for CsvSourceAdapter {
    fn default() -> Self {
        Self {
            has_header: true,
            delimiter: b',',
            trim: true,
            flexible: false,
        }
    }
}

impl CsvSourceAdapter {
    /// Parse a CSV file into a Document
    pub fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        let mut diags = Diagnostics::new();
        let file_path = path.display().to_string();

        let mut reader = ReaderBuilder::new()
            .has_headers(self.has_header)
            .delimiter(self.delimiter)
            .trim(if self.trim {
                csv::Trim::All
            } else {
                csv::Trim::None
            })
            .flexible(self.flexible)
            .from_path(path)
            .map_err(|e| {
                let mut diags = Diagnostics::new();
                diags.add(
                    Diagnostic::error(internal::E9902, format!("Failed to open CSV: {e}"))
                        .with_source(file_path.clone()),
                );
                diags
            })?;

        // Materialize all records first: the csv Reader is stateful, so any
        // lookahead (even peek()) consumes a record
        let records: Vec<csv::Result<csv::StringRecord>> = reader.records().collect();

        let headers = if self.has_header {
            reader
                .headers()
                .map_err(|e| {
                    diags.add(
                        Diagnostic::error(parse::E0001, format!("Failed to read CSV headers: {e}"))
                            .with_source(file_path.clone()),
                    );
                    diags.clone()
                })?
                .clone()
        } else {
            // Generate column names from the first record
            let count = records
                .first()
                .and_then(|r| r.as_ref().ok().map(csv::StringRecord::len))
                .unwrap_or(0);
            (0..count)
                .map(|i| format!("col{i}"))
                .collect::<csv::StringRecord>()
        };

        let header_strings: Vec<String> = headers
            .iter()
            .map(std::string::ToString::to_string)
            .collect();

        let mut doc = Document::new();
        doc.metadata.format = "csv".to_string();
        doc.source_files.push(file_path.clone());

        let table_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Data")
            .to_string();

        let mut table = Table {
            name: table_name.clone(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.clone(),
            sheet: None,
        };

        for (row_idx, record) in records.into_iter().enumerate() {
            let record = match record {
                Ok(r) => r,
                Err(e) => {
                    diags.add(
                        Diagnostic::error(
                            parse::E0001,
                            format!("CSV parse error at row {}: {}", row_idx + 1, e),
                        )
                        .with_source(file_path.clone())
                        .with_table(&table_name)
                        .with_row((row_idx + 1).to_string()),
                    );
                    continue;
                }
            };

            if record.len() != header_strings.len() && !self.flexible {
                diags.add(
                    Diagnostic::warning(
                        parse::E0001,
                        format!(
                            "Row {} has {} fields, expected {}",
                            row_idx + 1,
                            record.len(),
                            header_strings.len()
                        ),
                    )
                    .with_source(file_path.clone())
                    .with_table(&table_name)
                    .with_row((row_idx + 1).to_string()),
                );
            }

            let mut fields = IndexMap::new();
            let mut primary_key = Vec::new();

            for (col_idx, header) in header_strings.iter().enumerate() {
                let value_str = record.get(col_idx).unwrap_or("");
                let cage_value = Self::infer_and_parse_value(value_str);

                let loc = SourceLocation::new(&file_path)
                    .with_row(row_idx + 1)
                    .with_column(header.clone())
                    .with_field(header.clone());

                fields.insert(header.clone(), TypedValue::new(cage_value, loc));

                // Heuristic: first column named "id" or "ID" is primary key
                if primary_key.is_empty()
                    && (header.eq_ignore_ascii_case("id") || header.eq_ignore_ascii_case("ID"))
                {
                    primary_key.push(fields[header].value.clone());
                }
            }

            // If no explicit ID field, use first field as primary key
            if primary_key.is_empty() {
                if let Some((_first_key, first_value)) = fields.iter().next() {
                    primary_key.push(first_value.value.clone());
                }
            }

            table.rows.push(Row {
                primary_key,
                fields,
                location: SourceLocation::new(&file_path).with_row(row_idx + 1),
                index: row_idx,
            });
        }

        if table.rows.is_empty() {
            // Still add empty table for schema validation
        }

        doc.add_table(table);

        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(doc)
        }
    }

    fn infer_and_parse_value(s: &str) -> Value {
        let trimmed = s.trim();

        if trimmed.is_empty() {
            return Value::Null;
        }

        // Try boolean ("1"/"0" stay numeric: id columns must not flip to Bool)
        let lower = trimmed.to_lowercase();
        if matches!(
            lower.as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off"
        ) {
            return Value::Bool(matches!(lower.as_str(), "true" | "yes" | "on"));
        }

        // Try integer
        if let Ok(i) = trimmed.parse::<i64>() {
            return Value::Int(i);
        }

        // Try unsigned
        if let Ok(u) = trimmed.parse::<u64>() {
            return Value::UInt(u);
        }

        // Try float
        if let Ok(f) = trimmed.parse::<f64>() {
            return Value::Float(f);
        }

        // Default to string
        Value::String(trimmed.to_string())
    }

    /// Parse CSV from string
    pub fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics> {
        let mut diags = Diagnostics::new();
        let file_path = path.display().to_string();

        let mut reader = ReaderBuilder::new()
            .has_headers(self.has_header)
            .delimiter(self.delimiter)
            .trim(if self.trim {
                csv::Trim::All
            } else {
                csv::Trim::None
            })
            .flexible(self.flexible)
            .from_reader(content.as_bytes());

        // Materialize all records first: the csv Reader is stateful, so any
        // lookahead (even peek()) consumes a record
        let records: Vec<csv::Result<csv::StringRecord>> = reader.records().collect();

        let headers = if self.has_header {
            reader
                .headers()
                .map_err(|e| {
                    diags.add(
                        Diagnostic::error(parse::E0001, format!("Failed to read CSV headers: {e}"))
                            .with_source(file_path.clone()),
                    );
                    diags.clone()
                })?
                .clone()
        } else {
            // Generate column names from the first record
            let count = records
                .first()
                .and_then(|r| r.as_ref().ok().map(csv::StringRecord::len))
                .unwrap_or(0);
            (0..count)
                .map(|i| format!("col{i}"))
                .collect::<csv::StringRecord>()
        };

        let header_strings: Vec<String> = headers
            .iter()
            .map(std::string::ToString::to_string)
            .collect();

        let mut doc = Document::new();
        doc.metadata.format = "csv".to_string();
        doc.source_files.push(file_path.clone());

        let table_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Data")
            .to_string();

        let mut table = Table {
            name: table_name.clone(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.clone(),
            sheet: None,
        };

        for (row_idx, record) in records.into_iter().enumerate() {
            let record = match record {
                Ok(r) => r,
                Err(e) => {
                    diags.add(
                        Diagnostic::error(
                            parse::E0001,
                            format!("CSV parse error at row {}: {}", row_idx + 1, e),
                        )
                        .with_source(file_path.clone())
                        .with_table(&table_name)
                        .with_row((row_idx + 1).to_string()),
                    );
                    continue;
                }
            };

            let mut fields = IndexMap::new();
            let mut primary_key = Vec::new();

            for (col_idx, header) in header_strings.iter().enumerate() {
                let value_str = record.get(col_idx).unwrap_or("");
                let cage_value = Self::infer_and_parse_value(value_str);

                let loc = SourceLocation::new(&file_path)
                    .with_row(row_idx + 1)
                    .with_column(header.clone())
                    .with_field(header.clone());

                fields.insert(header.clone(), TypedValue::new(cage_value, loc));

                if primary_key.is_empty()
                    && (header.eq_ignore_ascii_case("id") || header.eq_ignore_ascii_case("ID"))
                {
                    primary_key.push(fields[header].value.clone());
                }
            }

            if primary_key.is_empty() {
                if let Some((_first_key, first_value)) = fields.iter().next() {
                    primary_key.push(first_value.value.clone());
                }
            }

            table.rows.push(Row {
                primary_key,
                fields,
                location: SourceLocation::new(&file_path).with_row(row_idx + 1),
                index: row_idx,
            });
        }

        doc.add_table(table);

        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(doc)
        }
    }
}

/// Trait for source adapters
pub trait SourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics>;
    fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics>;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn format_name(&self) -> &'static str;
}

impl SourceAdapter for CsvSourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        self.parse_file(path)
    }

    fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics> {
        self.parse_str(content, path)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["csv", "tsv"]
    }

    fn format_name(&self) -> &'static str {
        "CSV"
    }
}

impl CsvSourceAdapter {
    /// Create adapter for TSV (tab-separated values)
    pub fn tsv() -> Self {
        Self {
            delimiter: b'\t',
            ..Default::default()
        }
    }

    /// Create adapter without header row
    pub fn no_header() -> Self {
        Self {
            has_header: false,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::value::Value;
    use std::path::PathBuf;

    /// Write content to <tempdir>/<`file_name`> and return (keepalive dir, path)
    fn write_temp_file(content: &str, file_name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(file_name);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn test_parse_csv_with_header() {
        let csv = "id,name,price\n1,Sword,100\n2,Shield,200\n";

        let (_dir, file) = write_temp_file(csv, "test.csv");

        let adapter = CsvSourceAdapter::default();
        let doc = adapter.parse_file(&file).unwrap();

        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("test").unwrap(); // tempfile name
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].fields["id"].value, Value::Int(1));
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("Sword".to_string())
        );
        assert_eq!(table.rows[0].fields["price"].value, Value::Int(100));
    }

    #[test]
    fn test_parse_tsv() {
        let tsv = "id\tname\tprice\n1\tSword\t100\n2\tShield\t200\n";

        let (_dir, file) = write_temp_file(tsv, "test.tsv");

        let adapter = CsvSourceAdapter::tsv();
        let doc = adapter.parse_file(&file).unwrap();

        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("test").unwrap();
        assert_eq!(table.rows.len(), 2);
    }

    #[test]
    fn test_parse_csv_no_header() {
        let csv = "1,Sword,100\n2,Shield,200\n";

        let (_dir, file) = write_temp_file(csv, "test.csv");

        let adapter = CsvSourceAdapter::no_header();
        let doc = adapter.parse_file(&file).unwrap();

        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("test").unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].fields["col0"].value, Value::Int(1));
        assert_eq!(
            table.rows[0].fields["col1"].value,
            Value::String("Sword".to_string())
        );
    }

    #[test]
    fn test_parse_csv_type_inference() {
        let csv = "id,flag,count,ratio,name\n1,true,42,3.14,Item\n2,false,100,2.5,Another\n";

        let (_dir, file) = write_temp_file(csv, "test.csv");

        let adapter = CsvSourceAdapter::default();
        let doc = adapter.parse_file(&file).unwrap();

        let table = doc.tables.get("test").unwrap();
        assert_eq!(table.rows[0].fields["flag"].value, Value::Bool(true));
        assert_eq!(table.rows[0].fields["count"].value, Value::Int(42));
        assert_eq!(table.rows[0].fields["ratio"].value, Value::Float(3.14));
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("Item".to_string())
        );
    }

    #[test]
    fn test_parse_csv_empty_values() {
        let csv = "id,name,value\n1,Item,\n2,,100\n3,Test,200\n";

        let (_dir, file) = write_temp_file(csv, "test.csv");

        let adapter = CsvSourceAdapter::default();
        let doc = adapter.parse_file(&file).unwrap();

        let table = doc.tables.get("test").unwrap();
        assert_eq!(table.rows[0].fields["value"].value, Value::Null);
        assert_eq!(table.rows[1].fields["name"].value, Value::Null);
    }

    #[test]
    fn test_parse_csv_flexible() {
        let csv = "id,name\n1,Item,extra\n2,Item\n";

        let (_dir, file) = write_temp_file(csv, "test.csv");

        let adapter = CsvSourceAdapter {
            flexible: true,
            ..Default::default()
        };
        let doc = adapter.parse_file(&file).unwrap();

        let table = doc.tables.get("test").unwrap();
        assert_eq!(table.rows.len(), 2);
    }

    #[test]
    fn test_parse_csv_string() {
        let csv = "id,name\n1,Test\n";
        let adapter = CsvSourceAdapter::default();
        let path = Path::new("test.csv");
        let doc = adapter.parse_str(csv, path).unwrap();

        assert_eq!(doc.tables.len(), 1);
    }

    #[test]
    fn test_source_adapter_trait() {
        let adapter = CsvSourceAdapter::default();
        assert_eq!(adapter.supported_extensions(), &["csv", "tsv"]);
        assert_eq!(adapter.format_name(), "CSV");
    }

    /// Opening a path the reader cannot read reports E9902 with the file path.
    #[test]
    fn test_parse_file_open_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("no_such_file.csv");

        let adapter = CsvSourceAdapter::default();
        let diags = adapter.parse_file(&missing).unwrap_err();

        assert_eq!(diags.errors().len(), 1);
        assert_eq!(diags.errors()[0].code, internal::E9902);
        assert!(diags.errors()[0].message.contains("Failed to open CSV"));
        assert!(diags.errors()[0].source.contains("no_such_file.csv"));
    }

    /// `trim: false` keeps field padding: header cells and lookup keys are raw.
    #[test]
    fn test_parse_file_trim_disabled_keeps_field_padding() {
        let csv = " id , name \n 1 , Sword \n";
        let (_dir, file) = write_temp_file(csv, "notrim.csv");

        let adapter = CsvSourceAdapter {
            trim: false,
            ..Default::default()
        };
        let doc = adapter.parse_file(&file).unwrap();

        let table = doc.tables.get("notrim").unwrap();
        assert!(table.rows[0].fields.contains_key(" id "));
        // Value inference trims before type detection
        assert_eq!(
            table.rows[0].fields[" name "].value,
            Value::String("Sword".to_string())
        );
        // The padded header never equals "id", so the first field is the key
        assert_eq!(table.rows[0].primary_key, vec![Value::Int(1)]);
    }

    /// A header row that is not valid UTF-8 makes `headers()` fail (E0001)
    /// even when the data rows themselves decode.
    #[test]
    fn test_parse_file_invalid_utf8_header() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad_header.csv");
        std::fs::write(&path, b"id,\xffname\n1,Sword\n").unwrap();

        let adapter = CsvSourceAdapter::default();
        let diags = adapter.parse_file(&path).unwrap_err();

        assert_eq!(diags.errors().len(), 1);
        assert_eq!(diags.errors()[0].code, parse::E0001);
        assert!(diags.errors()[0]
            .message
            .contains("Failed to read CSV headers"));
    }

    /// A row whose field count differs from the header (flexible=false) is a
    /// csv reader error → row-level E0001, and the document is rejected.
    #[test]
    fn test_parse_file_ragged_row_reports_row_error() {
        let csv = "id,name\n1,Sword\n2\n";
        let (_dir, file) = write_temp_file(csv, "ragged.csv");

        let adapter = CsvSourceAdapter::default();
        let diags = adapter.parse_file(&file).unwrap_err();

        assert!(diags.has_errors());
        assert_eq!(diags.errors().len(), 1);
        let err = diags.errors()[0];
        assert_eq!(err.code, parse::E0001);
        assert!(err.message.contains("CSV parse error at row 2"));
        assert_eq!(err.row.as_deref(), Some("2"));
        assert_eq!(err.table.as_deref(), Some("ragged"));
    }

    /// Header-only input: no data rows, the empty-table branch still builds a
    /// document (schema validation later sees an empty table, not an error).
    #[test]
    fn test_parse_file_header_only_yields_empty_table() {
        let csv = "id,name,price\n";
        let (_dir, file) = write_temp_file(csv, "header_only.csv");

        let adapter = CsvSourceAdapter::default();
        let doc = adapter.parse_file(&file).unwrap();

        let table = doc.tables.get("header_only").unwrap();
        assert!(table.rows.is_empty());
        assert_eq!(doc.metadata.format, "csv");
        assert!(doc
            .source_files
            .iter()
            .any(|f| f.ends_with("header_only.csv")));
    }

    /// Headerless file whose first row cannot be decoded: the generated column
    /// names collapse to zero, so the first decodable row is flagged against
    /// "expected 0" fields while the undecodable row is a row-level error.
    #[test]
    fn test_parse_file_headerless_undecodable_first_row() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("undecodable.csv");
        std::fs::write(&path, b"\xff\xfe,2\n1,Sword\n").unwrap();

        let adapter = CsvSourceAdapter::no_header();
        let diags = adapter.parse_file(&path).unwrap_err();

        assert!(diags.has_errors());
        assert_eq!(diags.errors().len(), 1);
        assert!(diags.errors()[0]
            .message
            .contains("CSV parse error at row 1"));

        let warnings = diags.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].message, "Row 2 has 2 fields, expected 0");
    }

    /// Inference boundaries: u64 beyond i64, negative i64, exponent floats,
    /// boolean spellings and blank → Null.
    #[test]
    fn test_infer_and_parse_value_boundaries() {
        assert_eq!(
            CsvSourceAdapter::infer_and_parse_value("18446744073709551615"),
            Value::UInt(u64::MAX)
        );
        assert_eq!(
            CsvSourceAdapter::infer_and_parse_value("-9223372036854775808"),
            Value::Int(i64::MIN)
        );
        assert_eq!(
            CsvSourceAdapter::infer_and_parse_value("1e3"),
            Value::Float(1e3)
        );
        assert_eq!(
            CsvSourceAdapter::infer_and_parse_value("YES"),
            Value::Bool(true)
        );
        assert_eq!(
            CsvSourceAdapter::infer_and_parse_value("Off"),
            Value::Bool(false)
        );
        assert_eq!(CsvSourceAdapter::infer_and_parse_value("   "), Value::Null);
        assert_eq!(
            CsvSourceAdapter::infer_and_parse_value("  spaced  "),
            Value::String("spaced".to_string())
        );
    }

    /// `parse_str` with trim disabled keeps the raw header cells as keys.
    #[test]
    fn test_parse_str_trim_disabled_keeps_field_padding() {
        let csv = " id , name \n 1 , Sword \n";
        let adapter = CsvSourceAdapter {
            trim: false,
            ..Default::default()
        };
        let doc = adapter.parse_str(csv, Path::new("notrim.csv")).unwrap();

        let table = doc.tables.get("notrim").unwrap();
        assert!(table.rows[0].fields.contains_key(" id "));
        assert_eq!(
            table.rows[0].fields[" name "].value,
            Value::String("Sword".to_string())
        );
    }

    /// `parse_str` + no header: column names are generated from the first row.
    #[test]
    fn test_parse_str_no_header_generates_column_names() {
        let csv = "1,Sword,100\n2,Shield,200\n";
        let adapter = CsvSourceAdapter::no_header();
        let doc = adapter.parse_str(csv, Path::new("nohdr.csv")).unwrap();

        let table = doc.tables.get("nohdr").unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].fields["col0"].value, Value::Int(1));
        assert_eq!(
            table.rows[1].fields["col1"].value,
            Value::String("Shield".to_string())
        );
        assert_eq!(table.rows[0].fields["col2"].value, Value::Int(100));
    }

    /// `parse_str` surfaces the same ragged-row error path as `parse_file`.
    #[test]
    fn test_parse_str_ragged_row_reports_error() {
        let csv = "id,name\n1,Sword\n2\n";
        let adapter = CsvSourceAdapter::default();
        let diags = adapter.parse_str(csv, Path::new("ragged.csv")).unwrap_err();

        assert_eq!(diags.errors().len(), 1);
        assert_eq!(diags.errors()[0].code, parse::E0001);
        assert!(diags.errors()[0]
            .message
            .contains("CSV parse error at row 2"));
    }

    /// Without an `id` column the first field becomes the primary key.
    #[test]
    fn test_parse_str_primary_key_falls_back_to_first_field() {
        let csv = "name,qty\nSword,3\n";
        let adapter = CsvSourceAdapter::default();
        let doc = adapter.parse_str(csv, Path::new("weapons.csv")).unwrap();

        let table = doc.tables.get("weapons").unwrap();
        assert_eq!(
            table.rows[0].primary_key,
            vec![Value::String("Sword".to_string())]
        );
    }

    /// The `SourceAdapter` trait forwards to the inherent parse entry points.
    #[test]
    fn test_source_adapter_trait_parse_delegation() {
        let adapter = CsvSourceAdapter::default();
        let csv = "id,name\n1,Sword\n";
        let (_dir, file) = write_temp_file(csv, "trait.csv");

        let doc = SourceAdapter::parse_file(&adapter, &file).unwrap();
        assert_eq!(doc.tables.get("trait").unwrap().rows.len(), 1);

        let doc = SourceAdapter::parse_str(&adapter, csv, Path::new("inline.csv")).unwrap();
        assert_eq!(doc.tables.get("inline").unwrap().rows.len(), 1);
    }
}
