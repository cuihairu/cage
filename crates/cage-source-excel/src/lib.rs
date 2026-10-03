//! Excel Source Adapter - parses Excel files (.xlsx, .xls, .ods) into Cage's Canonical Model

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
use calamine::{open_workbook_auto, Data, DataType, Reader};
use indexmap::IndexMap;
use std::path::Path;

/// Excel Source Adapter
pub struct ExcelSourceAdapter {
    /// Row index (0-based) that contains headers
    pub header_row: usize,
    /// Whether to evaluate formulas (use cached values)
    pub eval_formulas: bool,
    /// Skip empty rows
    pub skip_empty_rows: bool,
    /// Treat first row as header if true, otherwise generate col0, col1...
    pub has_header: bool,
}

impl Default for ExcelSourceAdapter {
    fn default() -> Self {
        Self {
            header_row: 0,
            eval_formulas: true, // Use cached formula values
            skip_empty_rows: true,
            has_header: true,
        }
    }
}

impl ExcelSourceAdapter {
    /// Parse an Excel file into a Document
    pub fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        let mut diags = Diagnostics::new();
        let file_path = path.display().to_string();

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if !matches!(extension.as_str(), "xlsx" | "xlsm" | "ods") {
            diags.add(
                Diagnostic::error(
                    parse::E0001,
                    format!(
                        "Unsupported Excel format: .{extension} (expected .xlsx, .xlsm or .ods)"
                    ),
                )
                .with_source(file_path.clone()),
            );
            return Err(diags);
        }

        let mut workbook = match open_workbook_auto(path) {
            Ok(wb) => wb,
            Err(e) => {
                diags.add(
                    Diagnostic::error(internal::E9902, format!("Failed to open workbook: {e:?}"))
                        .with_source(file_path.clone()),
                );
                return Err(diags);
            }
        };

        let mut doc = Document::new();
        doc.metadata.format = "excel".to_string();
        doc.source_files.push(file_path.clone());

        let sheet_names = workbook.sheet_names();

        for sheet_name in &sheet_names {
            let range = match workbook.worksheet_range(sheet_name) {
                Ok(r) => r,
                Err(e) => {
                    diags.add(
                        Diagnostic::warning(
                            parse::E0001,
                            format!("Failed to read sheet '{sheet_name}': {e:?}"),
                        )
                        .with_source(file_path.clone()),
                    );
                    continue;
                }
            };

            if range.is_empty() {
                continue;
            }

            // Determine table name from sheet name
            let table_name = Self::sanitize_table_name(sheet_name);

            // Extract headers
            let headers = if self.has_header && range.height() > self.header_row {
                let header_row = range.rows().nth(self.header_row).unwrap();
                header_row
                    .iter()
                    .enumerate()
                    .map(|(col_idx, cell)| {
                        let val = Self::cell_to_string(cell);
                        if val.trim().is_empty() {
                            format!("col{col_idx}")
                        } else {
                            val.trim().to_string()
                        }
                    })
                    .collect::<Vec<String>>()
            } else {
                // Generate column names
                let width = range.width();
                (0..width).map(|i| format!("col{i}")).collect()
            };

            let mut table = Table {
                name: table_name.clone(),
                primary_key_fields: Vec::new(),
                rows: Vec::new(),
                source_file: file_path.clone(),
                sheet: Some(sheet_name.clone()),
            };

            // Process data rows
            let start_row = if self.has_header {
                self.header_row + 1
            } else {
                0
            };

            for (row_idx, row) in range.rows().skip(start_row).enumerate() {
                // Check if row is empty
                if self.skip_empty_rows && row.iter().all(Data::is_empty) {
                    continue;
                }

                let mut fields = IndexMap::new();
                let mut primary_key = Vec::new();

                for (col_idx, cell) in row.iter().enumerate() {
                    if col_idx >= headers.len() {
                        break;
                    }

                    let header = &headers[col_idx];
                    let cage_value = Self::cell_to_cage_value(cell);

                    // Excel column letter (A, B, C... AA, AB...)
                    let col_letter = Self::col_index_to_letter(col_idx);

                    let loc = SourceLocation::new(&file_path)
                        .with_sheet(sheet_name.clone())
                        .with_row(start_row + row_idx + 1) // 1-indexed for user
                        .with_column(col_letter)
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
                    location: SourceLocation::new(&file_path)
                        .with_sheet(sheet_name.clone())
                        .with_row(start_row + row_idx + 1),
                    index: table.rows.len(),
                });
            }

            if !table.rows.is_empty() || !headers.is_empty() {
                doc.add_table(table);
            }
        }

        if doc.tables.is_empty() {
            diags.add(
                Diagnostic::warning(parse::E0001, "No tables found in Excel file")
                    .with_source(file_path.clone()),
            );
        }

        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(doc)
        }
    }

    fn cell_to_cage_value(cell: &Data) -> Value {
        match cell {
            Data::Empty => Value::Null,
            Data::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    Value::Null
                } else {
                    // Try to infer type from string
                    Self::infer_string_type(trimmed)
                }
            }
            Data::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    Value::String(f.to_string())
                } else if f.fract() == 0.0 {
                    // Whole number float -> integer
                    Value::Int(*f as i64)
                } else {
                    Value::Float(*f)
                }
            }
            Data::Int(i) => Value::Int(*i),
            Data::Bool(b) => Value::Bool(*b),
            Data::Error(e) => Value::String(format!("#ERROR: {e:?}")),
            Data::DateTime(dt) => {
                // Excel serial date -> ISO string
                Value::String(format!("{dt:?}"))
            }
            Data::DateTimeIso(s) | Data::DurationIso(s) => Value::String(s.clone()),
        }
    }

    fn cell_to_string(cell: &Data) -> String {
        match cell {
            Data::Empty => String::new(),
            Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
            Data::Float(f) => f.to_string(),
            Data::Int(i) => i.to_string(),
            Data::Bool(b) => b.to_string(),
            Data::Error(e) => format!("#ERROR: {e:?}"),
            Data::DateTime(dt) => format!("{dt:?}"),
        }
    }

    fn infer_string_type(s: &str) -> Value {
        let lower = s.to_lowercase();

        // Boolean ("1"/"0" stay numeric: id cells must not flip to Bool)
        if matches!(
            lower.as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off"
        ) {
            return Value::Bool(matches!(lower.as_str(), "true" | "yes" | "on"));
        }

        // Integer
        if let Ok(i) = s.parse::<i64>() {
            return Value::Int(i);
        }

        // Unsigned
        if let Ok(u) = s.parse::<u64>() {
            return Value::UInt(u);
        }

        // Float
        if let Ok(f) = s.parse::<f64>() {
            return Value::Float(f);
        }

        // Default to string
        Value::String(s.to_string())
    }

    fn sanitize_table_name(name: &str) -> String {
        // Replace invalid characters for table names
        name.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string()
    }

    fn col_index_to_letter(mut idx: usize) -> String {
        let mut result = String::new();
        loop {
            let rem = idx % 26;
            result.push((b'A' + rem as u8) as char);
            if idx < 26 {
                break;
            }
            idx = idx / 26 - 1;
        }
        result.chars().rev().collect()
    }
}

/// Trait for source adapters
pub trait SourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics>;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn format_name(&self) -> &'static str;
}

impl SourceAdapter for ExcelSourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        self.parse_file(path)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["xlsx", "xlsm", "ods"]
    }

    fn format_name(&self) -> &'static str {
        "Excel"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::value::Value;

    #[test]
    fn test_col_index_to_letter() {
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(0), "A");
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(1), "B");
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(25), "Z");
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(26), "AA");
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(27), "AB");
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(51), "AZ");
        assert_eq!(ExcelSourceAdapter::col_index_to_letter(52), "BA");
    }

    #[test]
    fn test_sanitize_table_name() {
        assert_eq!(ExcelSourceAdapter::sanitize_table_name("Item"), "Item");
        assert_eq!(
            ExcelSourceAdapter::sanitize_table_name("Monster Data"),
            "Monster_Data"
        );
        assert_eq!(
            ExcelSourceAdapter::sanitize_table_name("Sheet-1"),
            "Sheet_1"
        );
        assert_eq!(ExcelSourceAdapter::sanitize_table_name("123"), "123");
    }

    #[test]
    fn test_cell_to_cage_value() {
        use calamine::Data;

        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Empty),
            Value::Null
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::String("hello".to_string())),
            Value::String("hello".to_string())
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Int(42)),
            Value::Int(42)
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Float(3.14)),
            Value::Float(3.14)
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Float(100.0)),
            Value::Int(100)
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Bool(true)),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_infer_string_type() {
        assert_eq!(ExcelSourceAdapter::infer_string_type("42"), Value::Int(42));
        assert_eq!(
            ExcelSourceAdapter::infer_string_type("3.14"),
            Value::Float(3.14)
        );
        assert_eq!(
            ExcelSourceAdapter::infer_string_type("true"),
            Value::Bool(true)
        );
        assert_eq!(
            ExcelSourceAdapter::infer_string_type("false"),
            Value::Bool(false)
        );
        assert_eq!(
            ExcelSourceAdapter::infer_string_type("hello"),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_source_adapter_trait() {
        let adapter = ExcelSourceAdapter::default();
        assert_eq!(adapter.supported_extensions(), &["xlsx", "xlsm", "ods"]);
        assert_eq!(adapter.format_name(), "Excel");
    }
}
