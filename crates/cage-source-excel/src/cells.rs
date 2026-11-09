//! Cell/grid conversion helpers (split out of lib.rs, LAY-005).

use super::ExcelSourceAdapter;
use cage_core::convention::{CellValue, Grid};
use cage_core::value::Value;
use calamine::{Data, Range};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

impl ExcelSourceAdapter {
    /// The sheet grid as the convention sees it: merge-resolved, and one
    /// [`CellValue`] per cell in the contract's shape.
    pub(crate) fn grid_of(
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
    ) -> Grid {
        range
            .rows()
            .enumerate()
            .map(|(row_idx, row)| {
                row.iter()
                    .enumerate()
                    .map(|(col_idx, cell)| {
                        ExcelSourceAdapter::cell_to_contract_value(Self::resolve_cell(
                            range,
                            merge_anchors,
                            row_idx,
                            col_idx,
                            cell,
                        ))
                    })
                    .collect()
            })
            .collect()
    }

    /// Column names at a header row, the same resolution the data plane has
    /// always used (covered cells take the merge anchor's text, empty names
    /// fall back to `col<n>` on the absolute column index).
    pub(crate) fn headers_at(
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        header_row: usize,
        has_header: bool,
    ) -> Vec<String> {
        let range_start = range.start().unwrap_or((0, 0));
        if has_header && range.height() > header_row {
            let row = range.rows().nth(header_row).unwrap();
            row.iter()
                .enumerate()
                .map(|(col_idx, cell)| {
                    let cell = Self::resolve_cell(range, merge_anchors, header_row, col_idx, cell);
                    let val = Self::cell_to_string(cell);
                    if val.trim().is_empty() {
                        format!("col{}", range_start.1 as usize + col_idx)
                    } else {
                        val.trim().to_string()
                    }
                })
                .collect()
        } else {
            let width = range.width();
            (0..width)
                .map(|i| format!("col{}", range_start.1 as usize + i))
                .collect()
        }
    }

    /// One contract cell: the raw text, number, or boolean — no type
    /// inference, no trimming (the convention layer decides what a cell
    /// says). Dates render the way the data plane renders them.
    pub(crate) fn cell_to_contract_value(cell: &Data) -> CellValue {
        match cell {
            Data::Empty => CellValue::Null,
            Data::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    CellValue::Str(f.to_string())
                } else {
                    CellValue::Float(*f)
                }
            }
            Data::Int(i) => CellValue::Int(*i),
            Data::Bool(b) => CellValue::Bool(*b),
            Data::Error(e) => CellValue::Str(format!("#ERROR: {e:?}")),
            Data::DateTime(dt) => CellValue::Str(format!("{dt:?}")),
            Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => {
                CellValue::Str(s.clone())
            }
        }
    }

    /// [`CellValue`] as the contract JSON sends it.
    pub(crate) fn cell_to_json(cell: &CellValue) -> JsonValue {
        serde_json::to_value(cell).unwrap_or(JsonValue::Null)
    }

    pub(crate) fn cell_to_cage_value(cell: &Data) -> Value {
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
                // Excel serial date -> ISO-8601 string (the documented
                // contract; was accidentally rendering the `ExcelDateTime`
                // struct dump into the IR).
                if dt.is_duration() {
                    // Duration format cell (`[h]:mm:ss`): the serial is a day
                    // count; render as an ISO-8601 duration to match the
                    // `DurationIso` passthrough.
                    let total_ms = (dt.as_f64() * 86_400_000.0).round() as i64;
                    let (ms, s, m, h) = (
                        total_ms % 1000,
                        total_ms / 1000 % 60,
                        total_ms / 60_000 % 60,
                        total_ms / 3_600_000,
                    );
                    Value::String(format!("PT{h}H{m}M{s}.{ms:03}S"))
                } else {
                    let (y, mo, d, h, mi, s, ms) = dt.to_ymd_hms_milli();
                    if ms == 0 {
                        Value::String(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
                    } else {
                        Value::String(format!(
                            "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}"
                        ))
                    }
                }
            }
            Data::DateTimeIso(s) | Data::DurationIso(s) => Value::String(s.clone()),
        }
    }

    /// Resolve a cell through merged regions (T2.4): a covered cell takes its
    /// anchor's value, so a vertical merge reads as the same value on every
    /// covered row. `rel_row`/`rel_col` are positions within `range` (as
    /// yielded by `rows()`); merge coordinates are absolute sheet cells.
    pub(crate) fn resolve_cell<'a>(
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

    pub(crate) fn cell_to_string(cell: &Data) -> String {
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

    pub(crate) fn infer_string_type(s: &str) -> Value {
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

    pub(crate) fn sanitize_table_name(name: &str) -> String {
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

    pub(crate) fn col_index_to_letter(mut idx: usize) -> String {
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
