use super::types::{
    parse_bool_text, parse_list_text, parse_number_text, parse_reference_text, parse_type_text,
};
use super::{a1_column, a1_row_col, anchor, error, loc};
use super::{CellValue, ColumnMeta, Grid, Preset, TableMeta};
use crate::diagnostics::Diagnostics;
use crate::error::codes::source_convention::{E2201, E2202};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use std::collections::HashSet;

/// Convention territory for a built-in preset on one grid, plus the row
/// where the data begins. `marker_rows` follows Q6 ruling (a): when the data
/// header row is a `##var` row the territory runs from row 0 to the first
/// pure-data row below it (the marker rows at and below the field-name row);
/// otherwise it is exactly the rows above the configured header. `data` and
/// `meta_sheet` carry no in-grid territory (a `meta_sheet` territory is the
/// fetched sibling sheet's rows, decided by the caller).
pub fn convention_territory(
    grid: &Grid,
    header_row: usize,
    marker: &str,
    preset: Preset,
) -> (Vec<usize>, usize) {
    let header = header_row.min(grid.len());
    match preset {
        Preset::Data | Preset::MetaSheet => (Vec::new(), header.saturating_add(1).min(grid.len())),
        Preset::MarkerRows => {
            let var_header = grid.get(header_row).is_some_and(|row| {
                row.iter().any(|cell| {
                    cell.text()
                        .strip_prefix(marker)
                        .is_some_and(|rest| rest.trim() == "var")
                })
            });
            if !var_header {
                return (
                    (0..header).collect(),
                    header.saturating_add(1).min(grid.len()),
                );
            }
            let data_start = ((header_row + 1)..grid.len())
                .find(|row| !grid[*row].iter().any(|c| c.text().starts_with(marker)))
                .unwrap_or(grid.len());
            ((0..data_start).collect(), data_start)
        }
        Preset::HeaderBlock => {
            let mut start = header;
            while start > 0 && grid[start - 1].iter().any(|c| !c.text().is_empty()) {
                start -= 1;
            }
            (
                (start..header).collect(),
                header.saturating_add(1).min(grid.len()),
            )
        }
    }
}

/// Decode a territory of rows into column declarations (the built-in
/// preset path).
///
/// `grid` is the whole sheet (or the meta sheet) in row order; `header_row`
/// keys columns by name; `territory` lists the grid rows to scan; `row0` /
/// `col0` are the absolute sheet indices of the grid's first cell, so every
/// anchor is a real A1 address. Malformed cells are errors (E2201/E2202)
/// with the cell anchor — nothing is silently dropped, and nothing silently
/// degrades to data (design §51 Q3).
#[allow(clippy::too_many_arguments, clippy::implicit_hasher)]
pub fn decode(
    grid: &Grid,
    header_row: usize,
    territory: &[usize],
    row0: usize,
    col0: usize,
    marker: &str,
    covered: &HashSet<(usize, usize)>,
    source_file: &str,
    sheet: &str,
) -> Result<TableMeta, Diagnostics> {
    let mut diags = Diagnostics::new();
    let mut meta = TableMeta {
        header_row,
        columns: IndexMap::new(),
    };
    if territory.is_empty() {
        return Ok(meta);
    }

    // Column identity comes from the data header row: absolute column index
    // → name. A declaration cell with no header above it cannot attach. Under
    // ruling (a) a `##var` row is the data header row, and a `*name` header
    // there declares a multi-row list (回流-5 Q6) — the `*` is a layout
    // modifier, not part of the field name, so every downstream lookup
    // (##type, ##+, assembly) uses the bare name. A plain header row is never
    // re-read through the `*` lens: a column literally named `*foo` keeps its
    // name outside the Luban `##var` shape.
    let header_is_var = grid.get(header_row).is_some_and(|row| {
        first_marker_keyword(row, marker).is_some_and(|(_, keyword)| keyword == "var")
    });
    let headers: Vec<(usize, String)> = match grid.get(header_row) {
        Some(row) => {
            // Under ruling (a) the `##var` marker cell is the row's own
            // keyword, not a column name (design §51 Q6): a phantom column
            // named `##var` would otherwise be declared like any other and
            // leak into the derived schema (X2). Everything else — a plain
            // name, a `*name` layout modifier stripped to its bare name —
            // is a data column.
            let marker_cell = header_is_var
                .then(|| first_marker_keyword(row, marker).map(|(i, _)| i))
                .flatten();
            row.iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != marker_cell)
                .map(|(i, c)| {
                    let text = c.text();
                    let name = if header_is_var {
                        text.strip_prefix('*').filter(|n| !n.is_empty())
                    } else {
                        None
                    };
                    let name = name.unwrap_or(&text).to_string();
                    // Audit finding 2026-10-30: a blank header cell gets the
                    // same `col<n>` fallback the data plane's headers_at uses,
                    // so two blank headers don't collapse onto the same empty
                    // column name (and collide in the duplicate check).
                    let name = if name.is_empty() {
                        format!("col{}", col0 + i)
                    } else {
                        name
                    };
                    (col0 + i, name)
                })
                .collect()
        }
        None => Vec::new(),
    };

    for &row in territory {
        let Some(cells) = grid.get(row) else {
            continue; // a territory past the grid is empty by definition
        };
        let abs_row = row0 + row;
        let first = first_marker_keyword(cells, marker);

        // Under ruling (a) the `##var` row is the data header row: it names
        // the columns but carries no declaration values to `apply`.
        if row == header_row && first.as_ref().is_some_and(|(_, keyword)| keyword == "var") {
            for (rel_col, cell) in cells.iter().enumerate() {
                let abs_col = col0 + rel_col;
                let text = cell.text();
                let Some(name) = text.strip_prefix('*') else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let meta = meta.columns.entry(name.to_string()).or_default();
                if meta.cell.is_empty() {
                    meta.cell = anchor(abs_row, abs_col);
                }
                meta.multi_row = true;
            }
            continue;
        }

        // A row whose first marker cell is `+` declares sub-columns; the
        // marker cell carries no keyword value.
        if first.as_ref().is_some_and(|(_, keyword)| keyword == "+") {
            for (rel_col, cell) in cells.iter().enumerate() {
                let abs_col = col0 + rel_col;
                if covered.contains(&(abs_row, abs_col)) {
                    continue;
                }
                if rel_col == first.as_ref().expect("first marker present").0 {
                    continue;
                }
                let text = cell.text();
                if text.is_empty() {
                    continue;
                }
                if text.starts_with(marker) {
                    diags.add(
                        error(
                            E2201,
                            format!(
                                "sub-column row at {anchor} carries a second marker cell",
                                anchor = anchor(abs_row, abs_col),
                            ),
                        )
                        .with_location(loc(
                            source_file,
                            sheet,
                            abs_row,
                            abs_col,
                        )),
                    );
                    continue;
                }
                add_sub_column(
                    &mut meta.columns,
                    &mut diags,
                    &headers,
                    &text,
                    abs_row,
                    abs_col,
                    source_file,
                    sheet,
                );
            }
            continue;
        }

        let mut current: Option<String> = None;
        for (rel_col, cell) in cells.iter().enumerate() {
            let abs_col = col0 + rel_col;
            if covered.contains(&(abs_row, abs_col)) {
                continue;
            }
            let text = cell.text();
            if let Some(rest) = text.strip_prefix(marker) {
                let rest = rest.trim().to_string();
                if rest.is_empty() {
                    diags.add(
                        error(
                            E2201,
                            format!(
                                "marker cell {anchor} carries no keyword",
                                anchor = anchor(abs_row, abs_col)
                            ),
                        )
                        .with_location(loc(
                            source_file,
                            sheet,
                            abs_row,
                            abs_col,
                        )),
                    );
                    current = None;
                    continue;
                }
                let (keyword, own) = split_keyword(&rest);
                current = Some(keyword.clone());
                if let Some(own) = own {
                    apply(
                        &mut meta.columns,
                        &mut diags,
                        &headers,
                        &keyword,
                        &own,
                        abs_row,
                        abs_col,
                        source_file,
                        sheet,
                    );
                }
            } else if !text.is_empty() {
                let Some(keyword) = current.as_deref() else {
                    diags.add(
                        error(
                            E2201,
                            format!(
                                "value {text:?} at {anchor} appears before any keyword cell in \
                                 its row",
                                anchor = anchor(abs_row, abs_col)
                            ),
                        )
                        .with_location(loc(
                            source_file,
                            sheet,
                            abs_row,
                            abs_col,
                        )),
                    );
                    continue;
                };
                apply(
                    &mut meta.columns,
                    &mut diags,
                    &headers,
                    keyword,
                    &text,
                    abs_row,
                    abs_col,
                    source_file,
                    sheet,
                );
            }
        }
    }

    if diags.has_errors() {
        Err(diags)
    } else {
        for (name, column) in &meta.columns {
            if column.multi_row && column.sub_columns.is_empty() {
                let (cell_row, cell_col) = a1_row_col(&column.cell).unwrap_or((row0, col0));
                diags.add(
                    error(
                        E2201,
                        format!(
                            "multi-row column {name:?} declares no sub-columns; declare them \
                             with a + sub-column row"
                        ),
                    )
                    .with_location(loc(source_file, sheet, cell_row, cell_col)),
                );
            }
        }
        if diags.has_errors() {
            return Err(diags);
        }
        Ok(meta)
    }
}

/// First marker cell of a row: `(relative column, keyword)`.
fn first_marker_keyword(cells: &[CellValue], marker: &str) -> Option<(usize, String)> {
    cells.iter().enumerate().find_map(|(rel_col, cell)| {
        let rest = cell.text().strip_prefix(marker)?.trim().to_string();
        if rest.is_empty() {
            return None;
        }
        let (keyword, _) = split_keyword(&rest);
        Some((rel_col, keyword))
    })
}

/// Add one `##+` cell as a sub-column, descending into the deepest
/// declaration already opened at this column (nested sub-column scope).
#[allow(clippy::too_many_arguments)]
fn add_sub_column(
    columns: &mut IndexMap<String, ColumnMeta>,
    diags: &mut Diagnostics,
    headers: &[(usize, String)],
    name: &str,
    abs_row: usize,
    abs_col: usize,
    source_file: &str,
    sheet: &str,
) {
    let anchor_text = anchor(abs_row, abs_col);
    let Some((_header_col, parent_name)) = headers.iter().find(|(column, _)| *column == abs_col)
    else {
        diags.add(
            error(
                E2201,
                format!("sub-column declaration at {anchor_text} has no column in the header row"),
            )
            .with_location(loc(source_file, sheet, abs_row, abs_col)),
        );
        return;
    };
    let parent_name = parent_name.clone();
    let path = columns
        .get(&parent_name)
        .and_then(|meta| deepest_sub_column_path(meta, abs_col));
    let parent = match &path {
        Some(path) => target_mut(columns, &parent_name, Some(path)),
        None => columns.entry(parent_name.clone()).or_default(),
    };
    if parent.cell.is_empty() {
        parent.cell = anchor(abs_row, abs_col);
    }
    if parent.sub_columns.contains_key(name) {
        diags.add(
            error(
                E2201,
                format!(
                    "sub-column {name:?} of {parent_name:?} is declared twice at {anchor_text}"
                ),
            )
            .with_location(loc(source_file, sheet, abs_row, abs_col)),
        );
        return;
    }
    parent.sub_columns.insert(
        name.to_string(),
        ColumnMeta {
            cell: anchor_text,
            ..ColumnMeta::default()
        },
    );
}

/// Deepest declaration path (sub-column names) whose cell sits at `abs_col`.
fn deepest_sub_column_path(meta: &ColumnMeta, abs_col: usize) -> Option<Vec<String>> {
    for (name, child) in &meta.sub_columns {
        if a1_column(&child.cell) == Some(abs_col) {
            if let Some(mut path) = deepest_sub_column_path(child, abs_col) {
                path.insert(0, name.clone());
                return Some(path);
            }
            return Some(vec![name.clone()]);
        }
    }
    None
}

/// Mutably resolve a top-level column, then a sub-column path.
fn target_mut<'a>(
    columns: &'a mut IndexMap<String, ColumnMeta>,
    top_name: &str,
    path: Option<&[String]>,
) -> &'a mut ColumnMeta {
    let top = columns
        .get_mut(top_name)
        .expect("top-level column must exist");
    let Some(path) = path else {
        return top;
    };
    let mut current = top;
    for name in path {
        current = current
            .sub_columns
            .get_mut(name)
            .expect("sub-column path must exist");
    }
    current
}

/// Split a keyword cell's text into the keyword (up to the first space) and
/// the keyword's own value for its column, if any (`"##type Int32"` →
/// `("type", Some("Int32"))`).
fn split_keyword(rest: &str) -> (String, Option<String>) {
    match rest.split_once(char::is_whitespace) {
        Some((keyword, own)) => (keyword.to_string(), Some(own.trim().to_string())),
        None => (rest.to_string(), None),
    }
}

/// Apply one keyword/value pair to one column. `abs_row`/`abs_col` are
/// absolute sheet coordinates for the anchor.
#[allow(clippy::too_many_arguments)]
fn apply(
    columns: &mut IndexMap<String, ColumnMeta>,
    diags: &mut Diagnostics,
    headers: &[(usize, String)],
    keyword: &str,
    value: &str,
    abs_row: usize,
    abs_col: usize,
    source_file: &str,
    sheet: &str,
) {
    let anchor_text = anchor(abs_row, abs_col);
    let error_at = |code: &str, message: String| {
        error(code, format!("{message} (at {anchor_text})")).with_location(loc(
            source_file,
            sheet,
            abs_row,
            abs_col,
        ))
    };
    let Some((_abs_header_col, name)) = headers.iter().find(|(c, _)| *c == abs_col) else {
        diags.add(error_at(
            E2201,
            format!("declaration {keyword:?} at {anchor_text} has no column in the header row"),
        ));
        return;
    };
    let name = name.clone();

    let path = columns
        .get(&name)
        .and_then(|meta| deepest_sub_column_path(meta, abs_col));
    let meta = match &path {
        Some(_) => target_mut(
            columns,
            &name,
            Some(path.as_deref().expect("path is present")),
        ),
        None => columns.entry(name.clone()).or_default(),
    };
    if meta.cell.is_empty() {
        meta.cell.clone_from(&anchor_text);
    }
    let slot_is_set = match keyword {
        "type" => meta.field_type.is_some(),
        "required" => meta.required.is_some(),
        "primary_key" => meta.primary_key.is_some(),
        "default" => meta.default.is_some(),
        "enum" => meta.enum_values.is_some(),
        "min" => meta.min.is_some(),
        "max" => meta.max.is_some(),
        "unique" => meta.unique.is_some(),
        "reference" => meta.reference.is_some(),
        "description" => meta.description.is_some(),
        "targets" => !meta.targets.is_empty(),
        other => {
            diags.add(error_at(
                E2201,
                format!("unknown keyword {other:?} at {anchor_text} is not in the vocabulary"),
            ));
            return;
        }
    };
    if slot_is_set {
        diags.add(error_at(
            E2201,
            format!(
                "column {name:?} declares {keyword:?} twice; one keyword, one value per column"
            ),
        ));
        return;
    }

    macro_rules! set {
        ($slot:expr, $parse:expr) => {
            match $parse {
                Ok(parsed) => $slot = Some(parsed),
                Err(message) => diags.add(error_at(E2202, format!("column {name:?}: {message}"))),
            }
        };
    }

    match keyword {
        "type" => set!(meta.field_type, parse_type_text(value)),
        "required" => set!(meta.required, parse_bool_text(value)),
        "primary_key" => set!(meta.primary_key, parse_bool_text(value)),
        "unique" => set!(meta.unique, parse_bool_text(value)),
        "min" => set!(meta.min, parse_number_text(value)),
        "max" => set!(meta.max, parse_number_text(value)),
        "enum" => set!(meta.enum_values, parse_list_text(value)),
        "targets" => match parse_list_text(value) {
            Ok(parsed) => meta.targets = parsed,
            Err(message) => diags.add(error_at(E2202, format!("column {name:?}: {message}"))),
        },
        "default" => set!(meta.default, serde_json::from_str::<JsonValue>(value)),
        "reference" => set!(meta.reference, parse_reference_text(value)),
        "description" => meta.description = Some(value.to_string()),
        // unreachable: unknown keywords and the vocabulary gate are handled
        // in the slot_is_set match above
        _ => {}
    }
}
