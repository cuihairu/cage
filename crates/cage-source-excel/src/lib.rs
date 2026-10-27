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
    convention::{
        decode, parse_response, run_script, CellValue, ExcelConvention, Grid, Preset, TableMeta,
        CONTRACT_VERSION,
    },
    diagnostics::{Diagnostic, Diagnostics},
    error::codes::{
        internal, parse,
        source_convention::{E2201, E2203, E2207},
    },
    value::{Document, Row, SourceLocation, Table, TypedValue, Value},
};
use calamine::{open_workbook_auto, Data, DataType, Range, Reader, SheetVisible, Sheets};
use indexmap::IndexMap;
use serde_json::{json, Value as JsonValue};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
    /// Meta-row convention declaration (`[source.excel]`, design §51).
    /// `None` is the baseline: no convention at all, today's byte-for-byte
    /// data plane.
    pub convention: Option<ExcelConvention>,
    /// Project root, used to resolve a declared script path (§51 Q2).
    pub project_root: Option<PathBuf>,
}

impl Default for ExcelSourceAdapter {
    fn default() -> Self {
        Self {
            header_row: 0,
            eval_formulas: true, // Use cached formula values
            skip_empty_rows: true,
            has_header: true,
            convention: None,
            project_root: None,
        }
    }
}

impl ExcelSourceAdapter {
    /// Parse an Excel file into a Document
    pub fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        self.parse_file_with_meta(path)
            .map(|(document, _)| document)
    }

    /// Parse one workbook into a document **and** the decoded per-table
    /// convention metadata (X1, design §51). [`Self::parse_file`] is the
    /// data-plane projection of this: the same code, the metadata dropped.
    /// With no `[source.excel]` declaration this is the byte-for-byte path
    /// that has always shipped.
    pub fn parse_file_with_meta(
        &self,
        path: &Path,
    ) -> Result<(Document, IndexMap<String, TableMeta>), Diagnostics> {
        let mut diags = Diagnostics::new();
        let file_path = path.display().to_string();
        let mut metas: IndexMap<String, TableMeta> = IndexMap::new();

        // Declaration-level coherence is checked once per workbook, before a
        // sheet is read (E2201).
        if let Some(message) = self.convention.as_ref().and_then(|c| c.validate().err()) {
            diags.add(Diagnostic::error(E2201, message).with_source(file_path.clone()));
            return Err(diags);
        }

        // A built-in preset has nothing to key columns by when the adapter
        // generates `col0, col1, ...` names, so it is refused at the workbook
        // level rather than failing cell by cell (E2201). A script declares
        // every column name itself and is unaffected.
        if !self.has_header && self.convention.as_ref().is_some_and(|c| !c.uses_script()) {
            diags.add(
                Diagnostic::error(
                    E2201,
                    "a preset cannot carry a convention with has_header = false: there is no \
                     header row to key columns by",
                )
                .with_source(file_path.clone()),
            );
            return Err(diags);
        }

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

        // Sheet metadata: a hidden sheet is authoring scratch — drafts and
        // staging never ship, so hidden/veryHidden sheets are skipped before
        // any parsing (2026-10-12 ruling, design §3.1). ODS workbooks
        // default to visible; calamine maps ODS `style:display` when set.
        let hidden: HashSet<String> = workbook
            .sheets_metadata()
            .iter()
            .filter(|s| s.visible != SheetVisible::Visible)
            .map(|s| s.name.clone())
            .collect();

        let sheet_names = workbook.sheet_names();

        // A `meta_sheet` workbook keeps its declarations in sibling sheets
        // named `<sheet>.meta`. Those are convention carriers, not tables, so
        // they are consumed by the meta sheet they serve and never parsed as
        // data (§51 Q1). The workbook file name is the key the convention
        // resolves against.
        let workbook_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let is_meta_sheet_workbook = self
            .convention
            .as_ref()
            .is_some_and(|c| !c.uses_script() && c.preset_for(&workbook_name) == Preset::MetaSheet);
        let carriers: HashSet<String> = if is_meta_sheet_workbook {
            let names: HashSet<&String> = sheet_names.iter().collect();
            sheet_names
                .iter()
                .filter_map(|name| {
                    let (stem, _) = name.split_once(".meta")?;
                    names.iter().any(|n| **n == stem).then(|| name.clone())
                })
                .collect()
        } else {
            HashSet::new()
        };

        for sheet_name in &sheet_names {
            if hidden.contains(sheet_name) || carriers.contains(sheet_name) {
                continue;
            }
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

            // Convention (§51): the script path needs the sheet grid before
            // any header row is known, so it runs first and may place the
            // data header row itself; a preset decodes the territory its
            // location chooses. With no declaration both are no-ops and the
            // data plane keeps exactly its configured header row.
            let configured_headers = if self.convention.is_some() {
                Self::headers_at(&range, &merge_anchors, self.header_row, self.has_header)
            } else {
                Vec::new()
            };
            let convention_meta = self.resolve_convention(
                &mut diags,
                &mut workbook,
                path,
                &file_path,
                sheet_name,
                &table_name,
                &range,
                &merge_anchors,
                &configured_headers,
            );
            let data_header_row = convention_meta
                .as_ref()
                .map_or(self.header_row, |meta| meta.header_row);

            // Extract headers (a covered header cell takes the anchor's text)
            let headers =
                Self::headers_at(&range, &merge_anchors, data_header_row, self.has_header);

            let mut table = Table {
                name: table_name.clone(),
                primary_key_fields: Vec::new(),
                rows: Vec::new(),
                source_file: file_path.clone(),
                sheet: Some(sheet_name.clone()),
            };

            // Process data rows
            let start_row = if self.has_header {
                data_header_row + 1
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
                        .with_format("excel")
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
                        .with_format("excel")
                        .with_sheet(sheet_name.clone())
                        .with_row(sheet_row),
                    index: table.rows.len(),
                });
            }

            if !table.rows.is_empty() || !headers.is_empty() {
                doc.add_table(table);
            }
            if let Some(meta) = convention_meta {
                metas.insert(table_name.clone(), meta);
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
            Ok((doc, metas))
        }
    }

    /// Resolve the convention for one sheet (design §51): a script adapter
    /// (subprocess, E2203–E2207) or a built-in preset, both decoded into the
    /// same [`TableMeta`]. Returns `None` and appends diagnostics on
    /// failure. `configured_headers` is the configured header row's names,
    /// used to validate a script's or a meta sheet's column declarations
    /// (E2207).
    #[allow(clippy::too_many_arguments)]
    fn resolve_convention<RS>(
        &self,
        diags: &mut Diagnostics,
        workbook: &mut Sheets<RS>,
        path: &Path,
        file_path: &str,
        sheet_name: &str,
        table_name: &str,
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        configured_headers: &[String],
    ) -> Option<TableMeta>
    where
        RS: std::io::Read + std::io::Seek,
    {
        let convention = self.convention.as_ref()?;

        // Script adapter (Q2): the grid goes to the script, the response is
        // gated by the contract and the vocabulary table.
        if convention.uses_script() {
            let script = convention.script.as_deref()?;
            let Some(root) = &self.project_root else {
                diags.add(
                    Diagnostic::error(
                        E2203,
                        format!(
                            "{E2203} a script adapter is declared but cage was not given the \
                             project root to resolve it against"
                        ),
                    )
                    .with_source(file_path.to_string()),
                );
                return None;
            };
            let grid = Self::grid_of(range, merge_anchors);
            let request = json!({
                "contract": CONTRACT_VERSION,
                "workbook": path
                    .strip_prefix(root)
                    .map_or_else(|_| path.to_path_buf(), Path::to_path_buf)
                    .display()
                    .to_string(),
                "sheet": sheet_name,
                "header_row": self.header_row,
                "grid": grid
                    .iter()
                    .map(|row| row.iter().map(Self::cell_to_json).collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                "config": convention.script_config,
            });
            let value = match run_script(
                root,
                script,
                &convention.script_args,
                std::time::Duration::from_secs(convention.script_timeout_secs.max(1)),
                &request,
            ) {
                Ok(value) => value,
                Err(d) => {
                    for d in &d {
                        diags.add(d.clone());
                    }
                    return None;
                }
            };
            return match parse_response(
                &value,
                table_name,
                configured_headers,
                file_path,
                sheet_name,
            ) {
                Ok(meta) => Some(meta),
                Err(d) => {
                    for d in &d {
                        diags.add(d.clone());
                    }
                    None
                }
            };
        }

        // Built-in presets (Q1): the territory is a location, not a rule.
        let workbook_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let preset = convention.preset_for(workbook_name);
        let grid = Self::grid_of(range, merge_anchors);
        let range_start = range.start().unwrap_or((0, 0));
        let (territory, meta_grid, meta_row0, meta_col0, meta_sheet): (
            Vec<usize>,
            Option<Grid>,
            usize,
            usize,
            Option<String>,
        ) = match preset {
            Preset::Data => return None,
            Preset::MarkerRows => (
                (0..self.header_row.min(grid.len())).collect(),
                None,
                0,
                0,
                None,
            ),
            Preset::HeaderBlock => {
                let mut start = self.header_row.min(grid.len());
                while start > 0 && grid[start - 1].iter().any(|c| !c.text().is_empty()) {
                    start -= 1;
                }
                (
                    (start..self.header_row.min(grid.len())).collect(),
                    None,
                    0,
                    0,
                    None,
                )
            }
            Preset::MetaSheet => {
                let meta_name = format!("{sheet_name}.meta");
                let meta_range = match workbook.worksheet_range(&meta_name) {
                    Ok(r) => r,
                    Err(e) => {
                        diags.add(
                            Diagnostic::error(
                                E2201,
                                format!(
                                    "{E2201} meta sheet {meta_name:?} for {sheet_name:?} could \
                                     not be read: {e:?}"
                                ),
                            )
                            .with_source(file_path.to_string()),
                        );
                        return None;
                    }
                };
                let meta_grid = Self::grid_of(&meta_range, &HashMap::new());
                let meta_start = meta_range.start().unwrap_or((0, 0));

                // The meta sheet's first row names the columns and must agree
                // with the data header row (E2207); the rows below are the
                // declarations.
                if let Some(names) = meta_grid.first() {
                    for (i, cell) in names.iter().enumerate() {
                        let name = cell.text();
                        if name.is_empty() {
                            continue;
                        }
                        if !configured_headers.iter().any(|h| h == &name) {
                            let loc = SourceLocation::new(file_path)
                                .with_format("excel")
                                .with_sheet(meta_name.clone())
                                .with_row(meta_start.0 as usize + 1)
                                .with_column(cage_core::convention::col_index_to_letter(i));
                            diags.add_error(
                                E2207,
                                format!(
                                    "{E2207} meta sheet {meta_name:?} names column {name:?}, which \
                                     is not in the data header row",
                                ),
                                loc,
                            );
                        }
                    }
                }
                (
                    (1..meta_grid.len()).collect(),
                    Some(meta_grid),
                    meta_start.0 as usize,
                    meta_start.1 as usize,
                    Some(meta_name),
                )
            }
        };

        let decoded = if let Some(meta_grid) = meta_grid.as_ref() {
            decode(
                meta_grid,
                0,
                &territory,
                meta_row0,
                meta_col0,
                &convention.marker,
                file_path,
                meta_sheet.as_deref().unwrap_or(sheet_name),
            )
        } else {
            decode(
                &grid,
                self.header_row,
                &territory,
                range_start.0 as usize,
                range_start.1 as usize,
                &convention.marker,
                file_path,
                sheet_name,
            )
        };
        match decoded {
            Ok(meta) if meta.is_empty() => None,
            Ok(meta) => Some(meta),
            Err(d) => {
                for d in &d {
                    diags.add(d.clone());
                }
                None
            }
        }
    }

    /// The sheet grid as the convention sees it: merge-resolved, and one
    /// [`CellValue`] per cell in the contract's shape.
    fn grid_of(range: &Range<Data>, merge_anchors: &HashMap<(u32, u32), (u32, u32)>) -> Grid {
        range
            .rows()
            .enumerate()
            .map(|(row_idx, row)| {
                row.iter()
                    .enumerate()
                    .map(|(col_idx, cell)| {
                        Self::cell_to_contract_value(Self::resolve_cell(
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
    fn headers_at(
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
    fn cell_to_contract_value(cell: &Data) -> CellValue {
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
    fn cell_to_json(cell: &CellValue) -> JsonValue {
        serde_json::to_value(cell).unwrap_or(JsonValue::Null)
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
    use cage_core::error::codes::source_convention::{E2202, E2204, E2205, E2206};
    use cage_core::schema::FieldType;
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

    /// Like [`minimal_xlsx`], but the caller supplies the workbook.xml (to
    /// exercise sheet metadata such as visibility states) plus one XML part
    /// per `rIdN` it declares.
    fn minimal_xlsx_with_workbook(workbook_xml: &str, sheets: &[(&str, &str)]) -> Vec<u8> {
        use std::fmt::Write as _;
        let package_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
        let mut rel = String::from(
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
        );
        let mut entries = vec![
            ("_rels/.rels".to_string(), package_rels.to_string()),
            ("xl/workbook.xml".to_string(), workbook_xml.to_string()),
        ];
        for (n, (part, xml)) in sheets.iter().enumerate() {
            if n > 0 {
                let _ = write!(
                    rel,
                    r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/{part}"/>"#,
                    n + 1
                );
            }
            entries.push((format!("xl/worksheets/{part}"), (*xml).to_string()));
        }
        let workbook_rels = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rel}</Relationships>"#
        );
        entries.insert(2, ("xl/_rels/workbook.xml.rels".to_string(), workbook_rels));
        let refs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(name, xml)| (name.as_str(), xml.as_str()))
            .collect();
        build_stored_zip(&refs)
    }

    /// A file in the system temp dir, removed on drop (the crate has no
    /// tempfile dev-dependency).
    struct TempWorkbook(PathBuf);

    impl TempWorkbook {
        fn new(bytes: &[u8], file_name: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "cage_excel_{}_{}_{}",
                std::process::id(),
                nanos,
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

    /// Sheet metadata: hidden and veryHidden sheets are authoring scratch —
    /// drafts never ship, so only the visible sheet reaches the canonical
    /// model (2026-10-12 ruling, design §3.1).
    #[test]
    fn test_handbuilt_xlsx_hidden_sheets_are_skipped() {
        let sheet = |cells: &str| {
            format!(
                concat!(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
                    r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
                    r#"<dimension ref="A1:C2"/>"#,
                    "<sheetData>",
                    r#"<row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="C1" t="inlineStr"><is><t>name</t></is></c></row>"#,
                    r#"<row r="2">"#,
                    "{}",
                    "</row></sheetData></worksheet>"
                ),
                cells
            )
        };
        let visible =
            sheet(r#"<c r="A2"><v>1001</v></c><c r="C2" t="inlineStr"><is><t>Sword</t></is></c>"#);
        let hidden = sheet(
            r#"<c r="A2"><v>9001</v></c><c r="C2" t="inlineStr"><is><t>DraftRow</t></is></c>"#,
        );
        let very_hidden = sheet(
            r#"<c r="A2"><v>9002</v></c><c r="C2" t="inlineStr"><is><t>SecretRow</t></is></c>"#,
        );
        let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Draft" sheetId="2" state="hidden" r:id="rId2"/><sheet name="Secrets" sheetId="3" state="veryHidden" r:id="rId3"/></sheets></workbook>"#;
        let bytes = minimal_xlsx_with_workbook(
            workbook,
            &[
                ("sheet1.xml", visible.as_str()),
                ("sheet2.xml", hidden.as_str()),
                ("sheet3.xml", very_hidden.as_str()),
            ],
        );
        let wb = TempWorkbook::new(&bytes, "staging.xlsx");

        let adapter = ExcelSourceAdapter::default();
        let doc = adapter.parse_file(wb.path()).expect("workbook parses");

        assert_eq!(doc.tables.len(), 1, "only the visible sheet is a table");
        let table = doc.tables.get("Data").expect("Data table");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(
            table.rows[0].fields["id"].value,
            Value::Int(1001),
            "the shipped row is the visible sheet's, not a hidden sheet's"
        );
        assert!(doc.tables.get("Draft").is_none());
        assert!(doc.tables.get("Secrets").is_none());
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

    // ---- Meta-row convention (X1, design §51) ------------------------------
    //
    // The presets read declarations from where the author put them. The
    // workbooks are built with the helpers above so the fixtures are inline
    // bytes — no generated file to check in, no OS-specific reader.

    /// A worksheet part from a 2-D array of cell texts; an empty string omits
    /// the cell, so a genuinely absent row or cell is representable.
    fn sheet_xml(rows: &[&[&str]], first_row: u32, dimension: &str) -> String {
        use std::fmt::Write as _;
        let mut xml = String::from(concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        ));
        let _ = write!(xml, "<dimension ref=\"{dimension}\"/>");
        let _ = write!(xml, "<sheetData>");
        for (i, row) in rows.iter().enumerate() {
            let r = first_row + i as u32;
            let _ = write!(xml, "<row r=\"{r}\">");
            for (j, text) in row.iter().enumerate() {
                if text.is_empty() {
                    continue;
                }
                let col = ExcelSourceAdapter::col_index_to_letter(j);
                let _ = write!(
                    xml,
                    "<c r=\"{col}{r}\" t=\"inlineStr\"><is><t>{text}</t></is></c>",
                    text = xml_escape(text),
                );
            }
            let _ = write!(xml, "</row>");
        }
        let _ = write!(xml, "</sheetData></worksheet>");
        xml
    }

    fn xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// A workbook with a Luban-style meta block: three declaration rows, the
    /// data header at row index 3, one data row.
    fn skill_workbook() -> TempWorkbook {
        let sheet = sheet_xml(
            &[
                &["##type Int32", "String", "Enum<Rarity>"][..],
                &["##description", "武器名", "稀有度"][..],
                &["##enum", "", "common,rare,epic"][..],
                &["id", "name", "rarity"][..],
                &["1", "sword", "common"][..],
            ],
            1,
            "A1:C5",
        );
        TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "skill.xlsx")
    }

    /// The convention gate: with no `[source.excel]` declaration
    /// declaration rows are ordinary data, exactly as before.
    #[test]
    fn test_convention_off_keeps_meta_rows_as_data() {
        let wb = skill_workbook();
        let adapter = ExcelSourceAdapter::default();

        let doc = adapter.parse_file(wb.path()).expect("workbook parses");
        let (table_name, table) = doc.tables.first().expect("one table");
        assert_eq!(table_name, "Data");
        assert_eq!(
            table.rows.len(),
            4,
            "the header row and every row below are data"
        );
        // The `##type Int32` row is the header row and the `##description`
        // row is ordinary data — nothing was decoded, nothing was dropped.
        assert!(
            table.rows[0]
                .fields
                .values()
                .any(|v| v.value == Value::String("##description".to_string())),
            "declaration text is ordinary data with no convention declared"
        );
        assert!(
            table.rows[1]
                .fields
                .values()
                .any(|v| v.value == Value::String("common,rare,epic".to_string())),
            "the enum row is data too"
        );
    }

    /// Preset B: every row above the header is a declaration row.
    #[test]
    fn test_marker_rows_preset_decodes_types_descriptions_and_enums() {
        let wb = skill_workbook();
        let adapter = ExcelSourceAdapter {
            header_row: 3,
            convention: Some(ExcelConvention {
                preset: Preset::MarkerRows,
                ..Default::default()
            }),
            ..Default::default()
        };

        let (doc, metas) = adapter
            .parse_file_with_meta(wb.path())
            .expect("convention decodes");
        let table = doc
            .tables
            .get("Data")
            .expect("the data table is still there");
        assert_eq!(table.rows.len(), 1, "only the row below the header is data");
        assert_eq!(
            table.rows[0].fields.get("name").unwrap().value,
            Value::String("sword".to_string())
        );

        let meta = metas.get("Data").expect("metadata for the table");
        assert_eq!(meta.header_row, 3);
        let id = meta.columns.get("id").expect("id declared");
        assert_eq!(id.cell, "A1");
        assert_eq!(id.field_type, Some(FieldType::Int32));
        assert_eq!(id.description, None);
        let name = meta.columns.get("name").expect("name declared");
        assert_eq!(name.cell, "B1");
        assert_eq!(name.field_type, Some(FieldType::String));
        assert_eq!(name.description.as_deref(), Some("武器名"));
        let rarity = meta.columns.get("rarity").expect("rarity declared");
        assert_eq!(rarity.cell, "C1");
        assert_eq!(
            rarity.field_type,
            Some(FieldType::Enum("Rarity".to_string()))
        );
        assert_eq!(
            rarity.enum_values.as_deref(),
            Some(["common", "rare", "epic"].map(String::from).as_slice())
        );
    }

    /// Preset A: only the contiguous non-empty run immediately above the
    /// header row is read — a blank separator row ends the block.
    #[test]
    fn test_header_block_preset_stops_at_the_blank_row() {
        let sheet = sheet_xml(
            &[
                &["##description", "序号说明", "名称说明"][..],
                &["", "", ""][..],
                &["##type Float64", "Float64", ""][..],
                &["id", "name", "note"][..],
                &["1", "x", ""][..],
            ],
            1,
            "A1:C5",
        );
        let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "item.xlsx");
        let adapter = ExcelSourceAdapter {
            header_row: 3,
            convention: Some(ExcelConvention {
                preset: Preset::HeaderBlock,
                ..Default::default()
            }),
            ..Default::default()
        };

        let (doc, metas) = adapter
            .parse_file_with_meta(wb.path())
            .expect("convention decodes");

        let meta = metas.get("Data").expect("metadata for the table");
        let id = meta.columns.get("id").expect("id declared");
        assert_eq!(
            id.field_type,
            Some(FieldType::Float64),
            "the block directly above the header"
        );
        assert_eq!(
            id.description, None,
            "the description row past the blank row is not in the block"
        );
        let table = doc.tables.get("Data").expect("data table");
        assert_eq!(table.rows.len(), 1);
    }

    /// Preset C: the sibling `<sheet>.meta` sheet carries the declarations and
    /// is never parsed as a table.
    #[test]
    fn test_meta_sheet_preset_reads_the_sibling_and_skips_it() {
        let data = sheet_xml(
            &[&["id", "name", "rarity"][..], &["1", "sword", "common"][..]],
            1,
            "A1:C2",
        );
        let meta_sheet = sheet_xml(
            &[
                &["id", "name", "rarity"][..],
                &["##type Int32", "String", "Enum<Rarity>"][..],
                &["##description", "武器名", "稀有度"][..],
                &["##enum", "", "common,rare,epic"][..],
            ],
            1,
            "A1:C4",
        );
        let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Skill" sheetId="1" r:id="rId1"/><sheet name="Skill.meta" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
        let wb = TempWorkbook::new(
            &minimal_xlsx_with_workbook(
                workbook,
                &[("sheet1.xml", &data), ("sheet2.xml", &meta_sheet)],
            ),
            "meta.xlsx",
        );
        let adapter = ExcelSourceAdapter {
            convention: Some(ExcelConvention {
                preset: Preset::MetaSheet,
                ..Default::default()
            }),
            ..Default::default()
        };

        let (doc, metas) = adapter
            .parse_file_with_meta(wb.path())
            .expect("convention decodes");

        assert!(doc.tables.contains_key("Skill"));
        assert!(
            !doc.tables.contains_key("Skill.meta"),
            "the meta sheet is a carrier, not a table"
        );
        let meta = metas.get("Skill").expect("metadata for the table");
        let id = meta.columns.get("id").expect("id declared");
        assert_eq!(id.field_type, Some(FieldType::Int32));
        let name = meta.columns.get("name").expect("name declared");
        assert_eq!(name.description.as_deref(), Some("武器名"));
        let rarity = meta.columns.get("rarity").expect("rarity declared");
        assert_eq!(
            rarity.enum_values.as_deref(),
            Some(["common", "rare", "epic"].map(String::from).as_slice())
        );
    }

    /// The meta sheet's name row must agree with the data header row.
    #[test]
    fn test_meta_sheet_name_row_mismatch_is_e2207() {
        let data = sheet_xml(&[&["id", "name"][..], &["1", "x"][..]], 1, "A1:B2");
        let meta_sheet = sheet_xml(
            &[
                &["id", "name", "ghost"][..],
                &["##type Int32", "String", "Float64"][..],
            ],
            1,
            "A1:C2",
        );
        let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Skill" sheetId="1" r:id="rId1"/><sheet name="Skill.meta" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
        let wb = TempWorkbook::new(
            &minimal_xlsx_with_workbook(
                workbook,
                &[("sheet1.xml", &data), ("sheet2.xml", &meta_sheet)],
            ),
            "mismatch.xlsx",
        );
        let adapter = ExcelSourceAdapter {
            convention: Some(ExcelConvention {
                preset: Preset::MetaSheet,
                ..Default::default()
            }),
            ..Default::default()
        };

        let diags = adapter.parse_file(wb.path()).unwrap_err();
        assert!(diags.errors().iter().any(|d| d.code == E2207));
        assert!(
            diags
                .errors()
                .iter()
                .any(|d| d.message.contains("ghost")
                    && d.message.contains("not in the data header row")),
            "{:?}",
            diags
                .errors()
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
    }

    /// A declaration cell is reported where the author put it: cell anchor in
    /// the message and in the location.
    #[test]
    fn test_convention_violation_is_anchored_to_the_cell() {
        let sheet = sheet_xml(
            &[
                &["##type Int32", "String"][..],
                &["name", "##description", "x"][..],
                &["id", "name"][..],
            ],
            1,
            "A1:C3",
        );
        let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "broken.xlsx");
        let adapter = ExcelSourceAdapter {
            header_row: 2,
            convention: Some(ExcelConvention {
                preset: Preset::MarkerRows,
                ..Default::default()
            }),
            ..Default::default()
        };

        let diags = adapter.parse_file(wb.path()).unwrap_err();
        assert!(diags.errors().iter().all(|d| d.code == E2201));
        let errors = diags.errors();
        let hit = errors
            .iter()
            .find(|d| d.message.contains("A2"))
            .expect("anchored error");
        assert!(
            hit.message.contains("before any keyword"),
            "{:?}",
            hit.message
        );
        let location = hit.location.as_ref().expect("cell location");
        assert_eq!(location.sheet.as_deref(), Some("Data"));
        assert_eq!(location.column.as_deref(), Some("A"));
        assert_eq!(location.row, Some(2));
        assert!(hit.source.contains("broken.xlsx"));
    }

    // ---- Script adapter (X1, §51 Q2) ---------------------------------------
    //
    // The subprocess contract, exercised with a real script. The script is a
    // repository file with a shebang, so nothing looks anything up on $PATH.
    // Windows cannot exec a shebang script; the same tests stand behind
    // `cfg(unix)` and a Windows project invokes a `.cmd`/`.bat` launcher,
    // which is a repository file too (contract §"script").

    /// Where python is; the script fixture uses `#!/usr/bin/env python3`.
    fn python() -> Option<String> {
        for candidate in ["python3", "python"] {
            if std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                return Some(candidate.to_string());
            }
        }
        None
    }

    /// A throwaway project root: `tools/` for scripts, a workbook inside it,
    /// so a declared script path stays inside the root (§51 Q2).
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "cage_convention_{}_{}_{}",
                std::process::id(),
                nanos,
                tag
            ));
            std::fs::create_dir_all(path.join("tools")).expect("tools dir");
            Self(path)
        }

        fn write_script(&self, rel: &str, body: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::write(&path, body).expect("script write");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mut perms = std::fs::metadata(&path)
                    .expect("script metadata")
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&path, perms).expect("script chmod");
            }
            path
        }

        /// One data sheet: a header row and one row.
        fn workbook(&self, name: &str) -> PathBuf {
            let sheet = sheet_xml(&[&["id", "name"][..], &["1", "x"][..]], 1, "A1:B2");
            let path = self.0.join(name);
            std::fs::write(&path, minimal_xlsx(Some(&sheet))).expect("workbook write");
            path
        }

        fn adapter(&self, script: &str, timeout: u64) -> ExcelSourceAdapter {
            ExcelSourceAdapter {
                header_row: 0,
                convention: Some(ExcelConvention {
                    script: Some(script.to_string()),
                    script_timeout_secs: timeout,
                    ..Default::default()
                }),
                project_root: Some(self.0.clone()),
                ..Default::default()
            }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The script the contract tests point at: it reads the request, checks
    /// the contract version, and answers two columns.
    const META_SCRIPT: &str = r#"#!/usr/bin/env python3
import json, sys
request = json.load(sys.stdin)
assert request["contract"] == "cage.excel-meta/1", request["contract"]
response = {
    "contract": "cage.excel-meta/1",
    "table": request["sheet"],
    "header_row": request["header_row"],
    "columns": {
        "id": {"cell": "A1", "type": {"kind": "Int32"}, "required": True},
        "name": {"cell": "B1", "type": {"kind": "String"}, "description": "武器名"},
    },
}
print(json.dumps(response, ensure_ascii=False))
"#;

    #[cfg(unix)]
    #[test]
    fn test_script_contract_delivers_metadata() {
        if python().is_none() {
            eprintln!("skip: no python interpreter");
            return;
        }

        let root = TempRoot::new("ok");
        let path = root.write_script("tools/meta.py", META_SCRIPT);
        let wb = root.workbook("Data.xlsx");

        let (doc, metas) = root
            .adapter("tools/meta.py", 10)
            .parse_file_with_meta(&wb)
            .expect("the contract round-trips");

        assert!(doc.tables.contains_key("Data"));
        let meta = metas.get("Data").expect("metadata for the table");
        let id = meta.columns.get("id").expect("id declared");
        assert_eq!(id.cell, "A1");
        assert_eq!(id.field_type, Some(FieldType::Int32));
        assert_eq!(id.required, Some(true));
        let name = meta.columns.get("name").expect("name declared");
        assert_eq!(name.field_type, Some(FieldType::String));
        assert_eq!(name.description.as_deref(), Some("武器名"));
        assert!(
            path.exists(),
            "the fixture still stands until it is dropped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_script_will_not_start_is_e2203() {
        for missing in [
            "tools/absent.py",
            "/absent/meta.py",
            "tools/../tools/meta.py",
        ] {
            let root = TempRoot::new(&format!("missing_{}", root_tag(missing)));
            let wb = root.workbook("Data.xlsx");
            let diags = root.adapter(missing, 10).parse_file(&wb).unwrap_err();
            assert!(diags.errors().iter().any(|d| d.code == E2203), "{missing}");
        }
    }

    /// A built-in preset cannot work without a header row to key columns by:
    /// workbook-level E2201, before any cell is read.
    #[test]
    fn test_preset_without_a_header_row_is_e2201() {
        let wb = TempWorkbook::new(&minimal_xlsx(None), "headerless.xlsx");
        let adapter = ExcelSourceAdapter {
            has_header: false,
            convention: Some(ExcelConvention {
                preset: Preset::HeaderBlock,
                ..Default::default()
            }),
            ..Default::default()
        };
        let diags = adapter.parse_file(wb.path()).unwrap_err();
        assert_eq!(diags.errors().len(), 1, "{:?}", diags.errors());
        let d = &diags.errors()[0];
        assert_eq!(d.code, E2201);
        assert!(d.message.contains("has_header = false"), "{}", d.message);
    }

    /// A declared script name that is not a file inside the root is refused.
    fn root_tag(missing: &str) -> String {
        missing
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn test_script_timeout_is_e2204() {
        if python().is_none() {
            eprintln!("skip: no python interpreter");
            return;
        }

        let root = TempRoot::new("timeout");
        root.write_script(
            "tools/slow.py",
            "#!/usr/bin/env python3\nimport time\ntime.sleep(30)\nprint('{}')\n",
        );
        let wb = root.workbook("Data.xlsx");

        let diags = root
            .adapter("tools/slow.py", 1)
            .parse_file(&wb)
            .unwrap_err();
        assert!(diags.errors().iter().any(|d| d.code == E2204), "{diags:?}");
        assert!(
            diags
                .errors()
                .iter()
                .any(|d| d.message.contains("timed out")),
            "{:?}",
            diags
                .errors()
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_script_failure_is_e2205_with_its_stderr() {
        if python().is_none() {
            eprintln!("skip: no python interpreter");
            return;
        }

        let root = TempRoot::new("crash");
        root.write_script(
            "tools/crash.py",
            "#!/usr/bin/env python3\nimport sys\nprint('authoring went wrong', file=sys.stderr)\nsys.exit(3)\n",
        );
        let wb = root.workbook("Data.xlsx");

        let diags = root
            .adapter("tools/crash.py", 10)
            .parse_file(&wb)
            .unwrap_err();
        assert!(diags.errors().iter().any(|d| d.code == E2205), "{diags:?}");
        assert!(
            diags
                .errors()
                .iter()
                .any(|d| d.message.contains("authoring went wrong")),
            "{:?}",
            diags
                .errors()
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_script_bad_output_is_e2206() {
        if python().is_none() {
            eprintln!("skip: no python interpreter");
            return;
        }

        for (tag, code, script) in [
            (
                "garbage",
                E2206,
                "#!/usr/bin/env python3\nprint('not json at all')\n",
            ),
            (
                "contract",
                E2206,
                "#!/usr/bin/env python3\nimport json, sys\njson.load(sys.stdin)\nprint('{\"contract\": \"cage.excel-meta/9\", \"table\": \"Data\", \"header_row\": 0, \"columns\": {}}')\n",
            ),
            (
                "table",
                E2207,
                "#!/usr/bin/env python3\nimport json, sys\njson.load(sys.stdin)\nprint('{\"contract\": \"cage.excel-meta/1\", \"table\": \"Other\", \"header_row\": 0, \"columns\": {\"id\": {\"cell\": \"A1\", \"type\": {\"kind\": \"Int32\"}}}}')\n",
            ),
        ] {
            let root = TempRoot::new(tag);
            root.write_script("tools/case.py", script);
            let wb = root.workbook("Data.xlsx");

            let diags = root.adapter("tools/case.py", 10).parse_file(&wb).unwrap_err();
            assert!(
                diags.errors().iter().any(|d| d.code == code),
                "{tag}: {diags:?}"
            );
        }
    }

    /// The response is gated by the Schema vocabulary, not by a whitelist.
    #[cfg(unix)]
    #[test]
    fn test_script_invented_keyword_is_e2202() {
        if python().is_none() {
            eprintln!("skip: no python interpreter");
            return;
        }

        let root = TempRoot::new("vocabulary");
        root.write_script(
            "tools/vocab.py",
            "#!/usr/bin/env python3\nimport json, sys\njson.load(sys.stdin)\nprint('{\"contract\": \"cage.excel-meta/1\", \"table\": \"Data\", \"header_row\": 0, \"columns\": {\"id\": {\"cell\": \"A1\", \"type\": {\"kind\": \"Int32\"}, \"note\": \"invented\"}}}')\n",
        );
        let wb = root.workbook("Data.xlsx");

        let diags = root
            .adapter("tools/vocab.py", 10)
            .parse_file(&wb)
            .unwrap_err();
        assert!(diags.errors().iter().any(|d| d.code == E2202), "{diags:?}");
    }
}
