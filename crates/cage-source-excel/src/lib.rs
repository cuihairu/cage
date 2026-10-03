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
    use std::path::PathBuf;

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

    // ---- Hand-built minimal .xlsx fixtures ---------------------------------
    //
    // calamine opens a workbook from four zip members: `_rels/.rels`,
    // `xl/workbook.xml`, `xl/_rels/workbook.xml.rels` and the sheet XML
    // (sharedStrings/styles/[Content_Types] are optional). The builders below
    // assemble a STORED zip — no compression, hand-rolled CRC-32 — so the
    // sheet-read error paths can be exercised with inline bytes.

    /// CRC-32 (IEEE, reflected) of `data`; zip readers verify it even for
    /// STORED entries.
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF_u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    /// Assemble a STORED (uncompressed) zip archive from name/content pairs.
    fn build_stored_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, contents) in entries {
            let data = contents.as_bytes();
            let crc = crc32(data).to_le_bytes();
            let size = (data.len() as u32).to_le_bytes();
            let name_len = (name.len() as u16).to_le_bytes();
            let offset = (out.len() as u32).to_le_bytes();

            // Local file header
            out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&[0; 2]); // flags
            out.extend_from_slice(&[0; 2]); // method: stored
            out.extend_from_slice(&[0; 2]); // mod time
            out.extend_from_slice(&0x21u16.to_le_bytes()); // mod date (1980-01-01)
            out.extend_from_slice(&crc);
            out.extend_from_slice(&size); // compressed size
            out.extend_from_slice(&size); // uncompressed size
            out.extend_from_slice(&name_len);
            out.extend_from_slice(&[0; 2]); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            // Central directory record
            central.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&[0; 2]); // flags
            central.extend_from_slice(&[0; 2]); // method
            central.extend_from_slice(&[0; 2]); // mod time
            central.extend_from_slice(&0x21u16.to_le_bytes()); // mod date
            central.extend_from_slice(&crc);
            central.extend_from_slice(&size);
            central.extend_from_slice(&size);
            central.extend_from_slice(&name_len);
            central.extend_from_slice(&[0; 2]); // extra len
            central.extend_from_slice(&[0; 2]); // comment len
            central.extend_from_slice(&[0; 2]); // disk number
            central.extend_from_slice(&[0; 2]); // internal attrs
            central.extend_from_slice(&[0; 4]); // external attrs
            central.extend_from_slice(&offset);
            central.extend_from_slice(name.as_bytes());
        }

        let cd_offset = (out.len() as u32).to_le_bytes();
        let cd_size = (central.len() as u32).to_le_bytes();
        out.extend_from_slice(&central);
        // End of central directory
        out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
        out.extend_from_slice(&[0; 2]); // disk number
        out.extend_from_slice(&[0; 2]); // cd start disk
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size);
        out.extend_from_slice(&cd_offset);
        out.extend_from_slice(&[0; 2]); // comment len
        out
    }

    /// One-sheet workbook ("Data"); `sheet_xml: None` omits the sheet part
    /// from the zip (sheet referenced but not shipped).
    fn minimal_xlsx(sheet_xml: Option<&str>) -> Vec<u8> {
        let package_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
        let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let workbook_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;

        let mut entries = vec![
            ("_rels/.rels", package_rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels),
        ];
        if let Some(xml) = sheet_xml {
            entries.push(("xl/worksheets/sheet1.xml", xml));
        }
        build_stored_zip(&entries)
    }

    /// A file in the system temp dir, removed on drop (the crate has no
    /// tempfile dev-dependency).
    struct TempWorkbook(PathBuf);

    impl TempWorkbook {
        fn new(bytes: &[u8], file_name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "cage_excel_{}_{}",
                std::process::id(),
                file_name
            ));
            std::fs::write(&path, bytes).expect("temp workbook write");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempWorkbook {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A non-Excel extension (or none at all) is rejected with E0001 before
    /// any file is opened.
    #[test]
    fn test_unsupported_extension_rejected_before_open() {
        let adapter = ExcelSourceAdapter::default();

        for name in ["book.xls", "book"] {
            let diags = adapter.parse_file(Path::new(name)).unwrap_err();
            assert_eq!(diags.errors().len(), 1);
            assert_eq!(diags.errors()[0].code, parse::E0001);
            assert!(diags.errors()[0]
                .message
                .contains("Unsupported Excel format"));
            assert!(diags.errors()[0].source.contains(name));
        }
    }

    /// Bytes that are not a zip at all fail workbook opening with E9902.
    #[test]
    fn test_open_failure_reports_io_error() {
        let wb = TempWorkbook::new(b"definitely not a zip", "corrupt.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let diags = adapter.parse_file(wb.path()).unwrap_err();

        assert_eq!(diags.errors().len(), 1);
        assert_eq!(diags.errors()[0].code, internal::E9902);
        assert!(diags.errors()[0]
            .message
            .contains("Failed to open workbook"));
        assert!(diags.errors()[0].source.contains("corrupt.xlsx"));
    }

    /// Gaps in the used range: a missing header cell (B1) is named `col1` by
    /// its absolute grid column, and an entirely absent middle row is skipped
    /// by `skip_empty_rows` — leaving a sheet-row gap between data rows.
    #[test]
    fn test_handbuilt_xlsx_empty_header_cell_and_blank_row() {
        let sheet = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
            r#"<dimension ref="A1:C4"/>"#,
            "<sheetData>",
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="C1" t="inlineStr"><is><t>name</t></is></c></row>"#,
            r#"<row r="2"><c r="A2"><v>1001</v></c><c r="C2" t="inlineStr"><is><t>Sword</t></is></c></row>"#,
            r#"<row r="4"><c r="A4"><v>1002</v></c><c r="C4" t="inlineStr"><is><t>Shield</t></is></c></row>"#,
            "</sheetData></worksheet>"
        );
        let wb = TempWorkbook::new(&minimal_xlsx(Some(sheet)), "gaps.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let doc = adapter.parse_file(wb.path()).expect("workbook parses");

        let table = doc.tables.get("Data").expect("Data table");
        assert_eq!(table.rows.len(), 2, "blank sheet row 3 is skipped");

        // Header row: A1 "id", missing B1 → generated col1, C1 "name"
        let names: Vec<&str> = table.rows[0].fields.keys().map(String::as_str).collect();
        assert_eq!(names, ["id", "col1", "name"]);
        assert_eq!(table.rows[0].fields["id"].value, Value::Int(1001));
        assert_eq!(table.rows[0].fields["col1"].value, Value::Null);
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("Sword".to_string())
        );

        // col1 addresses sheet column B; the id column drives the primary key
        assert_eq!(
            table.rows[0].fields["col1"].location.column.as_deref(),
            Some("B")
        );
        assert_eq!(table.rows[0].fields["col1"].location.row, Some(2));
        assert_eq!(table.rows[0].primary_key, vec![Value::Int(1001)]);

        // The skipped blank row leaves a sheet-row gap before the next datum
        assert_eq!(table.rows[1].fields["id"].value, Value::Int(1002));
        assert_eq!(table.rows[1].location.row, Some(4));
    }

    /// A malformed `<mergeCell ref>` warns but does not abort the sheet: the
    /// merge-fill map stays empty and the data is still tabulated.
    #[test]
    fn test_handbuilt_xlsx_bad_merge_ref_still_parses() {
        let sheet = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
            r#"<dimension ref="A1:B2"/>"#,
            "<sheetData>",
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="B1" t="inlineStr"><is><t>name</t></is></c></row>"#,
            r#"<row r="2"><c r="A2"><v>1</v></c><c r="B2" t="inlineStr"><is><t>Sword</t></is></c></row>"#,
            "</sheetData>",
            r#"<mergeCells count="1"><mergeCell ref="not-a-ref"></mergeCell></mergeCells>"#,
            "</worksheet>"
        );
        let wb = TempWorkbook::new(&minimal_xlsx(Some(sheet)), "badmerge.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let doc = adapter
            .parse_file(wb.path())
            .expect("merge-table failure is a warning");

        let table = doc.tables.get("Data").unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].fields["id"].value, Value::Int(1));
    }

    /// A sheet with no cells yields an empty range: the sheet contributes no
    /// table and the document ends up table-less (with a warning, not an error).
    #[test]
    fn test_handbuilt_xlsx_empty_sheet_yields_no_tables() {
        let sheet = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
            "<sheetData></sheetData></worksheet>"
        );
        let wb = TempWorkbook::new(&minimal_xlsx(Some(sheet)), "emptysheet.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let doc = adapter
            .parse_file(wb.path())
            .expect("empty sheet is not an error");

        assert!(doc.tables.is_empty());
    }

    /// A workbook whose sheet part is missing from the zip: reading the sheet
    /// fails, the sheet is skipped and no table is produced.
    #[test]
    fn test_handbuilt_xlsx_missing_sheet_part_skips_sheet() {
        let wb = TempWorkbook::new(&minimal_xlsx(None), "nosheet.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let doc = adapter
            .parse_file(wb.path())
            .expect("missing sheet part is a warning");

        assert!(doc.tables.is_empty());
    }

    /// An ODS workbook takes the non-Xlsx arm: sheet data is still read, but
    /// the merged-cell fill (an Xlsx-only calamine API) is skipped.
    #[test]
    fn test_handbuilt_ods_workbook_parses_without_merge_fill() {
        // Rows must contain nothing but cell elements: any stray text node
        // inside a row is a parse error for the ODS reader.
        let content = concat!(
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
            r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
            r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
            "<office:body><office:spreadsheet>",
            r#"<table:table table:name="Data">"#,
            r#"<table:table-row><table:table-cell office:value-type="string"><text:p>id</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>name</text:p></table:table-cell></table:table-row>"#,
            r#"<table:table-row><table:table-cell office:value="1001"></table:table-cell><table:table-cell office:value-type="string"><text:p>Sword</text:p></table:table-cell></table:table-row>"#,
            "</table:table></office:spreadsheet></office:body></office:document-content>"
        );
        let ods = build_stored_zip(&[
            ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
            ("META-INF/manifest.xml", "<manifest:manifest/>"),
            ("content.xml", content),
        ]);
        let wb = TempWorkbook::new(&ods, "sheet.ods");

        let adapter = ExcelSourceAdapter::default();
        let doc = adapter.parse_file(wb.path()).expect("ods workbook parses");

        let table = doc.tables.get("Data").expect("Data table");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].fields["id"].value, Value::Int(1001));
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("Sword".to_string())
        );
    }

    /// `has_header: false` names columns from the absolute grid position and
    /// treats every used row (including the former header row) as data; with
    /// no `id` header each row's primary key falls back to its first field.
    #[test]
    fn test_has_header_false_generates_column_names() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/merged_cells.xlsx");
        let adapter = ExcelSourceAdapter {
            has_header: false,
            ..Default::default()
        };
        let doc = adapter.parse_file(&path).expect("fixture must parse");

        let table = doc.tables.get("Items").unwrap();
        assert_eq!(table.rows.len(), 4, "header row is now a data row");

        // Used range starts at B → generated names are col1..col5
        let names: Vec<&str> = table.rows[0].fields.keys().map(String::as_str).collect();
        assert_eq!(names, ["col1", "col2", "col3", "col4", "col5"]);

        // No id header → primary key falls back to the first field
        assert_eq!(
            table.rows[0].primary_key,
            vec![Value::String("id".to_string())]
        );
        assert_eq!(table.rows[1].primary_key, vec![Value::Int(1001)]);

        // Locations keep sheet-grid columns (B..F) and rows (2..)
        assert_eq!(
            table.rows[0].fields["col1"].location.column.as_deref(),
            Some("B")
        );
        assert_eq!(
            table.rows[0].fields["col5"].location.column.as_deref(),
            Some("F")
        );
        assert_eq!(table.rows[0].location.row, Some(2));
    }

    /// Exotic cell conversions: whitespace-only strings, trimmed numeric
    /// strings, non-finite floats, error cells, date and duration passthroughs.
    #[test]
    fn test_cell_to_cage_value_exotic() {
        use calamine::{CellErrorType, ExcelDateTime, ExcelDateTimeType};

        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::String("  ".to_string())),
            Value::Null
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::String(" 42 ".to_string())),
            Value::Int(42)
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Float(f64::NAN)),
            Value::String("NaN".to_string())
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Float(f64::INFINITY)),
            Value::String("inf".to_string())
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::Error(CellErrorType::Div0)),
            Value::String("#ERROR: Div0".to_string())
        );
        let dt = ExcelDateTime::new(44_927.0, ExcelDateTimeType::DateTime, false);
        let converted = ExcelSourceAdapter::cell_to_cage_value(&Data::DateTime(dt));
        assert!(
            matches!(converted, Value::String(ref s) if s.contains("44927")),
            "serial date renders as its Debug form: {converted:?}"
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::DateTimeIso(
                "2024-01-02T03:04:05Z".to_string()
            )),
            Value::String("2024-01-02T03:04:05Z".to_string())
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_cage_value(&Data::DurationIso("PT1H".to_string())),
            Value::String("PT1H".to_string())
        );
    }

    /// `cell_to_string` renders every cell kind for header extraction.
    #[test]
    fn test_cell_to_string_matrix() {
        use calamine::{CellErrorType, ExcelDateTime, ExcelDateTimeType};

        assert_eq!(ExcelSourceAdapter::cell_to_string(&Data::Empty), "");
        assert_eq!(ExcelSourceAdapter::cell_to_string(&Data::Float(3.5)), "3.5");
        assert_eq!(ExcelSourceAdapter::cell_to_string(&Data::Int(7)), "7");
        assert_eq!(
            ExcelSourceAdapter::cell_to_string(&Data::Bool(true)),
            "true"
        );
        assert_eq!(
            ExcelSourceAdapter::cell_to_string(&Data::Error(CellErrorType::NA)),
            "#ERROR: NA"
        );
        let dt = ExcelDateTime::new(0.5, ExcelDateTimeType::TimeDelta, false);
        let rendered = ExcelSourceAdapter::cell_to_string(&Data::DateTime(dt));
        assert!(rendered.contains("0.5"), "got: {rendered}");
    }

    /// Unsigned integers past `i64::MAX` keep their width in string inference.
    #[test]
    fn test_infer_string_type_unsigned() {
        assert_eq!(
            ExcelSourceAdapter::infer_string_type("18446744073709551615"),
            Value::UInt(u64::MAX)
        );
        assert_eq!(
            ExcelSourceAdapter::infer_string_type("-9223372036854775808"),
            Value::Int(i64::MIN)
        );
    }

    /// An empty used range has no anchor cell: `resolve_cell` must return the
    /// cell it was handed instead of indexing into the range.
    #[test]
    fn test_resolve_cell_empty_range_keeps_raw_cell() {
        let range = Range::<Data>::empty();
        let raw = Data::Int(5);

        let resolved = ExcelSourceAdapter::resolve_cell(&range, &HashMap::new(), 0, 0, &raw);

        assert_eq!(resolved, &raw);
    }

    /// The `SourceAdapter` trait forwards `parse_file` to the inherent method.
    #[test]
    fn test_source_adapter_trait_parse_delegation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/merged_cells.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let doc = SourceAdapter::parse_file(&adapter, &path).expect("fixture must parse");
        assert!(doc.tables.contains_key("Items"));
    }
}
