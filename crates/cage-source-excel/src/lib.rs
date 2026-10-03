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
use calamine::{open_workbook_auto, Data, DataType, Range, Reader, Sheets};
use indexmap::IndexMap;
use std::collections::HashMap;
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

            // The used range may not start at A1 — anchor row numbers, column
            // letters and merge coordinates to the sheet grid, not to the range.
            let range_start = range.start().unwrap_or((0, 0));

            // T2.4: merged-cell regions — map every covered cell to its anchor.
            // calamine exposes merge regions as inherent methods on Xlsx only
            // (not on `Sheets`/`Reader`), so ODS workbooks skip the fill.
            let mut merge_anchors: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
            if let Sheets::Xlsx(xlsx) = &mut workbook {
                match xlsx.merge_cells_by_sheet_name(sheet_name) {
                    Ok(dims) => {
                        for dim in dims {
                            for row in dim.start.0..=dim.end.0 {
                                for col in dim.start.1..=dim.end.1 {
                                    if (row, col) != dim.start {
                                        merge_anchors.insert((row, col), dim.start);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => diags.add(
                        Diagnostic::warning(
                            parse::E0001,
                            format!("Failed to read merged cells in sheet '{sheet_name}': {e}"),
                        )
                        .with_source(file_path.clone()),
                    ),
                }
            }

            // Determine table name from sheet name
            let table_name = Self::sanitize_table_name(sheet_name);

            // Extract headers (a covered header cell takes the anchor's text)
            let headers = if self.has_header && range.height() > self.header_row {
                let header_row = range.rows().nth(self.header_row).unwrap();
                header_row
                    .iter()
                    .enumerate()
                    .map(|(col_idx, cell)| {
                        let cell = Self::resolve_cell(
                            &range,
                            &merge_anchors,
                            self.header_row,
                            col_idx,
                            cell,
                        );
                        let val = Self::cell_to_string(cell);
                        if val.trim().is_empty() {
                            format!("col{}", range_start.1 as usize + col_idx)
                        } else {
                            val.trim().to_string()
                        }
                    })
                    .collect::<Vec<String>>()
            } else {
                // Generate column names from the absolute column index (col2 ↔ C)
                let width = range.width();
                (0..width)
                    .map(|i| format!("col{}", range_start.1 as usize + i))
                    .collect()
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
                let rel_row = start_row + row_idx;
                // 1-indexed sheet row (G27-style: the used range may start at B2)
                let sheet_row = range_start.0 as usize + rel_row + 1;

                // Check if row is empty — through merges: a row fully covered
                // by a vertical merge still carries the anchor's value
                if self.skip_empty_rows
                    && row.iter().enumerate().all(|(col_idx, cell)| {
                        Self::resolve_cell(&range, &merge_anchors, rel_row, col_idx, cell)
                            .is_empty()
                    })
                {
                    continue;
                }

                let mut fields = IndexMap::new();
                let mut primary_key = Vec::new();

                for (col_idx, cell) in row.iter().enumerate() {
                    if col_idx >= headers.len() {
                        break;
                    }

                    let header = &headers[col_idx];
                    let cell = Self::resolve_cell(&range, &merge_anchors, rel_row, col_idx, cell);
                    let cage_value = Self::cell_to_cage_value(cell);

                    // Excel column letter (A, B, C... AA, AB...) on the sheet grid
                    let col_letter = Self::col_index_to_letter(range_start.1 as usize + col_idx);

                    let loc = SourceLocation::new(&file_path)
                        .with_sheet(sheet_name.clone())
                        .with_row(sheet_row)
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
                        .with_row(sheet_row),
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

    /// Resolve a cell through merged regions (T2.4): a covered cell takes its
    /// anchor's value, so a vertical merge reads as the same value on every
    /// covered row. `rel_row`/`rel_col` are positions within `range` (as
    /// yielded by `rows()`); merge coordinates are absolute sheet cells.
    fn resolve_cell<'a>(
        range: &'a Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        rel_row: usize,
        rel_col: usize,
        cell: &'a Data,
    ) -> &'a Data {
        let Some((start_row, start_col)) = range.start() else {
            return cell;
        };
        let abs = (start_row + rel_row as u32, start_col + rel_col as u32);
        let Some(&(anchor_row, anchor_col)) = merge_anchors.get(&abs) else {
            return cell;
        };
        // Anchor outside the used range (merge overhangs the data) — keep the raw cell
        let (Some(anchor_rel_row), Some(anchor_rel_col)) = (
            anchor_row.checked_sub(start_row),
            anchor_col.checked_sub(start_col),
        ) else {
            return cell;
        };
        range
            .get((anchor_rel_row as usize, anchor_rel_col as usize))
            .unwrap_or(cell)
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

    /// T2.4 end-to-end: vertical merge fill + sheet-grid addressing.
    /// Fixture (`tests/fixtures/gen_merged_cells.py`) has its used range start
    /// at B2 and merges D3:D4 with no value stored in D4.
    #[test]
    fn test_merged_cells_fill_and_sheet_addressing() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/merged_cells.xlsx");
        let adapter = ExcelSourceAdapter::default();
        let doc = adapter.parse_file(&path).expect("fixture must parse");

        let table = doc.tables.get("Items").expect("Items table");
        assert_eq!(table.rows.len(), 3, "all three data rows present");

        // Primary key heuristic picks the id column
        assert_eq!(table.rows[0].primary_key, vec![Value::Int(1001)]);
        assert_eq!(table.rows[1].primary_key, vec![Value::Int(1002)]);

        // Vertical merge: D4 carries no value in the file, takes D3's anchor
        assert_eq!(
            table.rows[0].fields["type"].value,
            Value::String("weapon".to_string())
        );
        assert_eq!(
            table.rows[1].fields["type"].value,
            Value::String("weapon".to_string()),
            "covered cell D4 must be filled from anchor D3"
        );
        assert_eq!(
            table.rows[2].fields["type"].value,
            Value::String("material".to_string())
        );

        // Sheet-grid addressing (G27-style): used range starts at B2, so the
        // anchor row of the merge is sheet row 3, the covered row is sheet row 4
        // and the type column is D — not range-relative offsets.
        let anchor_loc = &table.rows[0].fields["type"].location;
        assert_eq!(anchor_loc.row, Some(3));
        assert_eq!(anchor_loc.column.as_deref(), Some("D"));
        let covered_loc = &table.rows[1].fields["type"].location;
        assert_eq!(covered_loc.row, Some(4));
        assert_eq!(covered_loc.column.as_deref(), Some("D"));

        // Empty covered cells elsewhere stay Null; plain cells unaffected
        assert_eq!(table.rows[1].fields["note"].value, Value::Null);
        assert_eq!(
            table.rows[2].fields["note"].value,
            Value::String("rare".to_string())
        );
        assert_eq!(table.rows[1].fields["price"].value, Value::Int(250));
    }

    /// Horizontal merge: covered cell takes the anchor to its left; anchors,
    /// uncovered cells and anchors outside the used range stay raw.
    #[test]
    fn test_resolve_cell_horizontal_and_out_of_range() {
        // Sheet rows 3..5, columns B..F — start deliberately not A1
        let mut range: Range<Data> = Range::new((2, 1), (4, 5));
        range.set_value((3, 4), Data::String("wide".to_string())); // E4 anchor

        let mut anchors = HashMap::new();
        anchors.insert((3, 5), (3, 4)); // F4 covered by merge E4:F4

        // F4 (rel 1,4) is covered → E4's value
        let covered = ExcelSourceAdapter::resolve_cell(&range, &anchors, 1, 4, &Data::Empty);
        assert_eq!(covered, &Data::String("wide".to_string()));

        // E4 itself is the anchor → raw value kept
        let anchor_raw = Data::String("wide".to_string());
        let anchor = ExcelSourceAdapter::resolve_cell(&range, &anchors, 1, 3, &anchor_raw);
        assert_eq!(anchor, &anchor_raw);

        // B3 not covered → raw value kept
        let raw = ExcelSourceAdapter::resolve_cell(&range, &anchors, 0, 0, &Data::Int(7));
        assert_eq!(raw, &Data::Int(7));

        // Merge anchor sits before range.start() → no bogus substitution
        let mut outside = HashMap::new();
        outside.insert((3, 5), (1, 0)); // anchor at sheet col A (before B)
        let kept = ExcelSourceAdapter::resolve_cell(&range, &outside, 1, 4, &Data::Int(9));
        assert_eq!(kept, &Data::Int(9));
    }
}
