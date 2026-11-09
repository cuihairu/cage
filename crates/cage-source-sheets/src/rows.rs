//! Rows and convention application: grid building, meta resolution and canonical
//! conversion for the Sheets adapter (split out of lib.rs, LAY-014).

use cage_core::convention::{
    self, a1_column, convention_territory, decode, parse_response, run_script, CellValue,
    ColumnMeta, Grid, Preset, SheetsConvention, TableMeta, CONTRACT_VERSION,
};
use cage_core::error::codes::remote::E1903;
use cage_core::error::codes::source_convention::{E2201, E2207};
use std::collections::HashSet;
use std::path::Path;

pub(crate) fn cell_to_cellvalue(cell: &serde_json::Value) -> CellValue {
    match cell {
        serde_json::Value::Null => CellValue::Null,
        serde_json::Value::Bool(b) => CellValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CellValue::Int(i)
            } else {
                CellValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) if s.is_empty() => CellValue::Null,
        serde_json::Value::String(s) => CellValue::Str(s.clone()),
        other => CellValue::Str(other.to_string()),
    }
}

/// One fetched row into a convention grid row.
pub(crate) fn row_to_grid_row(row: &serde_json::Value) -> Vec<CellValue> {
    row.as_array()
        .map(|cells| cells.iter().map(cell_to_cellvalue).collect())
        .unwrap_or_default()
}

/// The fetched rows as a convention grid (the `values` array is already the
/// tab's used range from its top-left, so row 0 is the grid's row 0).
pub(crate) fn values_grid(values: &[serde_json::Value]) -> Grid {
    values.iter().map(row_to_grid_row).collect()
}

/// A data cell's canonical form: numbers and booleans keep their JSON text
/// (UNFORMATTED_VALUE's cache value), NULL stays null, strings stay strings —
/// the same rendering the baseline `sheets_json_to_canonical` uses.
pub(crate) fn render_cell(cell: &serde_json::Value) -> serde_json::Value {
    match cell {
        serde_json::Value::Null => serde_json::Value::Null,
        serde_json::Value::String(s) => serde_json::Value::String(s.clone()),
        other => serde_json::Value::String(other.to_string()),
    }
}

/// The cell at absolute column `abs_col` in one fetched row (ragged rows read
/// as null past their width, like the baseline).
pub(crate) fn cell_at(row: &serde_json::Value, abs_col: usize) -> serde_json::Value {
    row.as_array()
        .and_then(|cells| cells.get(abs_col))
        .map(render_cell)
        .unwrap_or(serde_json::Value::Null)
}

/// One top-level column's value for a data row: a column with sub-columns is
/// a nested bean (object of its children), a childless column is its cell.
pub(crate) fn column_value(meta: &ColumnMeta, row: &serde_json::Value) -> serde_json::Value {
    if meta.sub_columns.is_empty() {
        return cell_at(row, a1_column(&meta.cell).unwrap_or(0));
    }
    let mut obj = serde_json::Map::new();
    for (name, child) in &meta.sub_columns {
        obj.insert(name.clone(), column_value(child, row));
    }
    serde_json::Value::Object(obj)
}

/// Whether a value is a present record field (the Excel adapter's "root field
/// non-empty" test, at the JSON level).
pub(crate) fn value_nonempty(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
        _ => true,
    }
}

/// One data row as a canonical row object, keyed by the (stripped) top-level
/// column names.
pub(crate) fn row_object(meta: &TableMeta, row: &serde_json::Value) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (name, col) in &meta.columns {
        obj.insert(name.clone(), column_value(col, row));
    }
    serde_json::Value::Object(obj)
}

/// A fully blank row (all cells null or empty string) is a separator, not an
/// element — the sheets baseline's `any_value` rule.
pub(crate) fn row_blank(row: &serde_json::Value) -> bool {
    row.as_array()
        .map(|cells| {
            cells.iter().all(|c| {
                matches!(c, serde_json::Value::Null)
                    || matches!(c, serde_json::Value::String(s) if s.is_empty())
            })
        })
        .unwrap_or(true)
}

/// Apply the decoded structure to the data rows: `sub_columns` group into
/// nested objects and `multi_row` columns fold contiguous rows into arrays,
/// mirroring the Excel adapter's assembly (回流-5). `data_start` is the first
/// data row (the `##var` territory may leave marker rows between the header
/// row and the data).
pub(crate) fn structured_rows(
    values: &[serde_json::Value],
    data_start: usize,
    meta: &TableMeta,
) -> Vec<serde_json::Value> {
    let multi_row_columns: Vec<String> = meta
        .columns
        .iter()
        .filter(|(_, c)| c.multi_row)
        .map(|(n, _)| n.clone())
        .collect();
    let has_multi_row = !multi_row_columns.is_empty();
    let mut out = Vec::new();
    let mut pending: Option<serde_json::Value> = None;

    for row in values.iter().skip(data_start) {
        if row_blank(row) {
            continue;
        }
        if has_multi_row {
            let is_start = pending.is_none()
                || meta
                    .columns
                    .iter()
                    .any(|(_, col)| !col.multi_row && value_nonempty(&column_value(col, row)));
            if pending.is_none() || is_start {
                let flushed = pending.take();
                let mut anchor = row_object(meta, row);
                for name in &multi_row_columns {
                    let entry = anchor.get_mut(name).expect("multi-row field present");
                    let value = std::mem::replace(entry, serde_json::Value::Null);
                    *entry = serde_json::Value::Array(vec![value]);
                }
                pending = Some(anchor);
                if let Some(r) = flushed {
                    out.push(r);
                }
                continue;
            }
            let pending_obj = pending.as_mut().expect("pending record present");
            for name in &multi_row_columns {
                let element = column_value(&meta.columns[name], row);
                let entry = pending_obj.get_mut(name).expect("multi-row field present");
                if let serde_json::Value::Array(items) = entry {
                    items.push(element);
                } else {
                    let old = std::mem::replace(entry, serde_json::Value::Null);
                    *entry = serde_json::Value::Array(vec![old, element]);
                }
            }
            continue;
        }
        out.push(row_object(meta, row));
    }
    if let Some(r) = pending {
        out.push(r);
    }
    out
}

/// Decode the declared convention for one tab into [`TableMeta`], plus the
/// first data row. `meta_grid` is the fetched `<tab>.meta` grid when the
/// preset is `meta_sheet`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_meta(
    main_grid: &Grid,
    header_row: usize,
    convention: &SheetsConvention,
    project_root: &Path,
    table_name: &str,
    sheet: &str,
    workbook: &str,
    meta_grid: Option<&Grid>,
) -> Result<(TableMeta, usize), String> {
    let excel = convention.as_excel_convention();
    let marker = &excel.marker;

    if excel.uses_script() {
        let script = excel.script.as_deref().expect("uses_script");
        let request = serde_json::json!({
            "contract": CONTRACT_VERSION,
            "workbook": workbook,
            "sheet": sheet,
            "header_row": header_row,
            "grid": main_grid
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            "config": excel.script_config,
        });
        let value = run_script(
            project_root,
            script,
            &excel.script_args,
            std::time::Duration::from_secs(excel.script_timeout_secs.max(1)),
            &request,
        )
        .map_err(|diags| diags.render(false))?;
        let header_names: Vec<String> = main_grid
            .get(header_row)
            .map(|row| row.iter().map(CellValue::text).collect())
            .unwrap_or_default();
        let meta = parse_response(&value, table_name, &header_names, workbook, sheet)
            .map_err(|diags| diags.render(false))?;
        let data_start = meta.header_row.saturating_add(1);
        return Ok((meta, data_start));
    }

    match convention.preset {
        Preset::Data => Ok((TableMeta::default(), header_row.saturating_add(1))),
        Preset::MetaSheet => {
            let meta_grid = meta_grid.ok_or_else(|| {
                format!(
                    "{E2201} meta_sheet preset needs the <tab>.meta grid, which was not fetched"
                )
            })?;
            let header_names: Vec<String> = main_grid
                .get(header_row)
                .map(|row| row.iter().map(CellValue::text).collect())
                .unwrap_or_default();
            if let Some(names) = meta_grid.first() {
                for (i, cell) in names.iter().enumerate() {
                    let name = cell.text();
                    if !name.is_empty() && !header_names.iter().any(|h| h == &name) {
                        return Err(format!(
                            "{E2207} meta sheet names column {name:?} at {}, which is not in \
                             the data header row",
                            convention::col_index_to_letter(i)
                        ));
                    }
                }
            }
            let territory: Vec<usize> = (1..meta_grid.len()).collect();
            let meta = decode(
                meta_grid,
                0,
                &territory,
                0,
                0,
                marker,
                &HashSet::new(),
                workbook,
                sheet,
            )
            .map_err(|diags| diags.render(false))?;
            Ok((meta, header_row.saturating_add(1)))
        }
        preset => {
            let (territory, data_start) =
                convention_territory(main_grid, header_row, marker, preset);
            let meta = decode(
                main_grid,
                header_row,
                &territory,
                0,
                0,
                marker,
                &HashSet::new(),
                workbook,
                sheet,
            )
            .map_err(|diags| diags.render(false))?;
            Ok((meta, data_start))
        }
    }
}

/// The canonical JSON for a convention-declared tab: decode the convention,
/// apply the structure, serialize `{tab: [row objects]}`. `meta_values` is
/// the fetched `<tab>.meta` rows for the `meta_sheet` preset.
#[allow(clippy::too_many_arguments)]
pub(crate) fn canonical_with_convention(
    table_name: &str,
    values: &[serde_json::Value],
    header_row: usize,
    convention: &SheetsConvention,
    project_root: &Path,
    workbook: &str,
    sheet: &str,
    meta_values: Option<&[serde_json::Value]>,
) -> Result<Vec<u8>, String> {
    let main_grid = values_grid(values);
    let (meta, data_start) = resolve_meta(
        &main_grid,
        header_row,
        convention,
        project_root,
        table_name,
        sheet,
        workbook,
        meta_values.map(values_grid).as_ref(),
    )?;
    let rows = structured_rows(values, data_start, &meta);
    let mut doc = serde_json::Map::new();
    doc.insert(table_name.to_string(), serde_json::Value::Array(rows));
    serde_json::to_vec_pretty(&serde_json::Value::Object(doc))
        .map_err(|e| format!("{E1903} cannot serialize ({e})"))
}
