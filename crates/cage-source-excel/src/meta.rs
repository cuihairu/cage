//! Meta-row convention and workbook opening (split out of lib.rs, LAY-005).

use cage_core::{
    convention::{
        a1_column, decode, parse_response, run_script, ColumnMeta, Grid, Preset, TableMeta,
        CONTRACT_VERSION, DEFAULT_MARKER,
    },
    diagnostics::{Diagnostic, Diagnostics},
    error::codes::{
        internal, parse,
        source_convention::{E2201, E2203, E2207},
    },
    value::{Document, Row, SourceLocation, Table, TypedValue, Value},
};
use calamine::{open_workbook_auto, Data, DataType, Range, Reader, SheetVisible, Sheets};
use indexmap::{IndexMap, IndexSet};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{ExcelSourceAdapter, HeaderGroup};

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

            // Audit finding 2026-10-29: an empty used range was skipped with no
            // diagnostic at all, silently dropping the sheet. By §3.1 a visible
            // sheet is a table — hiding a sheet is the documented way to say it
            // is not — so an empty one is an authoring error, not a no-op.
            // (An error, not a warning: source-adapter warnings are discarded
            // on the Ok path, so only an error surfaces.)
            if range.is_empty() {
                diags.add(
                    Diagnostic::error(
                        parse::E0001,
                        format!(
                            "Sheet '{sheet_name}' has an empty used range; a visible \
                             sheet is a table, so remove it or hide it if it is not one"
                        ),
                    )
                    .with_source(file_path.clone()),
                );
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

            // Process data rows. Under Q6 ruling (a) a built-in `marker_rows`
            // table whose configured header row is `##var` skips the
            // declaration rows below it; every other preset starts directly
            // below its data header row as before.
            let marker = self
                .convention
                .as_ref()
                .map_or(DEFAULT_MARKER.to_string(), |c| c.marker.clone());
            let marker_grid = if self.has_header
                && self.convention.as_ref().is_some_and(|c| {
                    !c.uses_script() && c.preset_for(&workbook_name) == Preset::MarkerRows
                }) {
                Some(Self::grid_of(&range, &merge_anchors))
            } else {
                None
            };
            let is_var_header = marker_grid.as_ref().is_some_and(|grid| {
                grid.get(self.header_row).is_some_and(|row| {
                    row.iter().any(|cell| {
                        cell.text()
                            .strip_prefix(&marker)
                            .is_some_and(|rest| rest.trim() == "var")
                    })
                })
            });
            let has_nested_columns = convention_meta
                .as_ref()
                .is_some_and(|meta| meta.columns.values().any(|c| !c.sub_columns.is_empty()));
            let multi_row_columns = convention_meta
                .as_ref()
                .map(|meta| {
                    meta.columns
                        .iter()
                        .filter(|(_name, column)| column.multi_row)
                        .map(|(name, _column)| name.clone())
                        .collect::<IndexSet<String>>()
                })
                .unwrap_or_default();
            let has_structural =
                is_var_header || has_nested_columns || !multi_row_columns.is_empty();
            let header_groups = (self.has_header && has_structural)
                .then(|| Self::header_groups_at(&range, &merge_anchors, data_header_row, &marker));

            // Audit finding 2026-10-29: every path stores top-level fields by
            // header name (flat: `fields.insert(header, ...)`; structural:
            // `fields.insert(group.name, ...)`), so two columns with the same
            // top-level header would silently overwrite the earlier column and
            // lose its values. Report the collision as an error instead. The
            // flat path reads names from `headers`; the structural path reads
            // them from `header_groups`, whose names already skip `##`-marker
            // cells and strip a `*` layout modifier.
            let duplicate_headers = if let Some(groups) = &header_groups {
                groups
                    .iter()
                    .map(|g| (g.columns.first().copied().unwrap_or(0), g.name.as_str()))
                    .collect::<Vec<_>>()
            } else {
                headers
                    .iter()
                    .enumerate()
                    .map(|(i, h)| (range_start.1 as usize + i, h.as_str()))
                    .collect()
            };
            let mut seen: HashSet<&str> = HashSet::new();
            for (col_idx, header) in duplicate_headers {
                if !seen.insert(header) {
                    let col_letter = Self::col_index_to_letter(col_idx);
                    diags.add(
                        Diagnostic::error(
                            parse::E0001,
                            format!(
                                "Duplicate column name '{header}' in sheet \
                                 '{sheet_name}' (column {col_letter}); rename \
                                 one of the columns so every header is unique"
                            ),
                        )
                        .with_source(file_path.clone()),
                    );
                }
            }

            let start_row = if self.has_header {
                if is_var_header {
                    let grid = marker_grid
                        .as_ref()
                        .expect("var header implies a marker-rows grid");
                    Self::marker_rows_territory(grid, self.header_row, &marker).1
                } else {
                    data_header_row + 1
                }
            } else {
                0
            };

            // Multi-row records (回流-5, design §51 Q6): a top-level column
            // declared `*name` in the `##var` row folds contiguous data rows
            // into a `Value::Array` of the nested objects its sub-columns
            // declare. A row whose every root (non-multi-row) field is empty
            // continues the previous record; any other row starts a new one.
            let has_multi_row = !multi_row_columns.is_empty();
            let mut pending_multi_row: Option<Row> = None;

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

                let is_record_start = !has_multi_row
                    || pending_multi_row.is_none()
                    || header_groups.as_ref().is_some_and(|groups| {
                        groups.iter().any(|group| {
                            !multi_row_columns.contains(&group.name)
                                && group.columns.iter().any(|col| {
                                    Self::nested_cell_value(
                                        &range,
                                        &merge_anchors,
                                        row,
                                        rel_row,
                                        *col,
                                        range_start.1 as usize,
                                    ) != Value::Null
                                })
                        })
                    });

                let mut fields = if let Some(header_groups) = &header_groups {
                    Self::fields_for_nested_row(
                        header_groups,
                        &range,
                        &merge_anchors,
                        row,
                        convention_meta.as_ref(),
                        rel_row,
                        sheet_row,
                        range_start.1 as usize,
                        &file_path,
                        sheet_name,
                    )
                } else {
                    IndexMap::new()
                };
                let mut primary_key = Vec::new();

                if header_groups.is_none() {
                    for (col_idx, cell) in row.iter().enumerate() {
                        if col_idx >= headers.len() {
                            break;
                        }

                        let header = &headers[col_idx];
                        let cell =
                            Self::resolve_cell(&range, &merge_anchors, rel_row, col_idx, cell);
                        let cage_value = Self::cell_to_cage_value(cell);

                        // Excel column letter (A, B, C... AA, AB...) on the sheet grid
                        let col_letter =
                            Self::col_index_to_letter(range_start.1 as usize + col_idx);

                        let loc = SourceLocation::new(&file_path)
                            .with_format("excel")
                            .with_sheet(sheet_name.clone())
                            .with_row(sheet_row)
                            .with_column(col_letter)
                            .with_field(header.clone());

                        fields.insert(header.clone(), TypedValue::new(cage_value, loc));

                        // Heuristic: first column named "id" or "ID" is primary key
                        if primary_key.is_empty()
                            && (header.eq_ignore_ascii_case("id")
                                || header.eq_ignore_ascii_case("ID"))
                        {
                            primary_key.push(fields[header].value.clone());
                        }
                    }
                }

                // If no explicit ID field, use first field as primary key
                if primary_key.is_empty() {
                    if let Some((_first_key, first_value)) = fields.iter().next() {
                        primary_key.push(first_value.value.clone());
                    }
                }

                let row_to_push = Row {
                    primary_key,
                    fields,
                    location: SourceLocation::new(&file_path)
                        .with_format("excel")
                        .with_sheet(sheet_name.clone())
                        .with_row(sheet_row),
                    index: table.rows.len(),
                };

                if has_multi_row {
                    if pending_multi_row.is_none() || is_record_start {
                        let flushed = pending_multi_row.take();
                        let mut anchor = row_to_push;
                        for name in &multi_row_columns {
                            if let Some(entry) = anchor.fields.get_mut(name) {
                                if !matches!(entry.value, Value::Array(_)) {
                                    let value = std::mem::replace(&mut entry.value, Value::Null);
                                    entry.value = Value::Array(vec![value]);
                                }
                            }
                        }
                        pending_multi_row = Some(anchor);
                        if let Some(row) = flushed {
                            table.rows.push(row);
                        }
                        continue;
                    }
                    // Continuation: fold each multi-row column of this row
                    // into the pending record's array for that column.
                    for name in &multi_row_columns {
                        let Some(meta) = convention_meta
                            .as_ref()
                            .and_then(|meta| meta.columns.get(name))
                        else {
                            continue;
                        };
                        let object = Value::Object(Self::nested_object_value(
                            meta,
                            &range,
                            &merge_anchors,
                            row,
                            rel_row,
                            range_start.1 as usize,
                        ));
                        let pending = pending_multi_row.as_mut().expect("pending present");
                        let entry = pending
                            .fields
                            .get_mut(name)
                            .expect("multi-row field present after its anchor row");
                        match &mut entry.value {
                            Value::Array(items) => items.push(object),
                            value => {
                                let items = std::mem::replace(value, Value::Null);
                                *value = Value::Array(vec![items, object]);
                            }
                        }
                    }
                    continue;
                }

                table.rows.push(row_to_push);
            }

            if let Some(row) = pending_multi_row {
                table.rows.push(row);
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
    pub(crate) fn resolve_convention<RS>(
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
            Preset::MarkerRows => {
                let (territory_end, _) =
                    Self::marker_rows_territory(&grid, self.header_row, &convention.marker);
                ((0..territory_end).collect(), None, 0, 0, None)
            }
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

        let covered: HashSet<(usize, usize)> = merge_anchors
            .keys()
            .map(|(row, col)| (*row as usize, *col as usize))
            .collect();

        let decoded = if let Some(meta_grid) = meta_grid.as_ref() {
            decode(
                meta_grid,
                0,
                &territory,
                meta_row0,
                meta_col0,
                &convention.marker,
                &HashSet::new(),
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
                &covered,
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

    /// `marker_rows` territory (Q6 ruling (a)): if `header_row` is a `##var`
    /// row the territory ends at the first pure-data row below it; otherwise
    /// it is exactly the rows above the configured header. The returned pair
    /// is `(territory_end, first_data_row)`.
    pub(crate) fn marker_rows_territory(
        grid: &Grid,
        header_row: usize,
        marker: &str,
    ) -> (usize, usize) {
        let var_header = grid.get(header_row).is_some_and(|row| {
            row.iter().any(|cell| {
                cell.text()
                    .strip_prefix(marker)
                    .is_some_and(|rest| rest.trim() == "var")
            })
        });
        if !var_header {
            return (header_row.min(grid.len()), (header_row + 1).min(grid.len()));
        }
        let data_start = ((header_row + 1)..grid.len())
            .find(|row| {
                !grid[*row]
                    .iter()
                    .any(|cell| cell.text().starts_with(marker))
            })
            .unwrap_or(grid.len());
        (data_start, data_start)
    }

    /// Top-level column groups of a data header row. Consecutive columns that
    /// a merged region binds to one anchor form one group; a marker cell (the
    /// `##var` of a Luban field-name row) is the row's own keyword, not a
    /// column name.
    pub(crate) fn header_groups_at(
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        header_row: usize,
        marker: &str,
    ) -> Vec<HeaderGroup> {
        let mut groups: Vec<HeaderGroup> = Vec::new();
        let Some((range_start_row, range_start_col)) = range.start() else {
            return groups;
        };
        let Some(header) = range.rows().nth(header_row) else {
            return groups;
        };
        for (col_idx, cell) in header.iter().enumerate() {
            let abs_col = range_start_col as usize + col_idx;
            let abs_row = range_start_row as usize + header_row;
            // The `##var` row's marker cell carries no column name.
            if Self::cell_to_string(cell).strip_prefix(marker).is_some() {
                continue;
            }
            let anchor_col = merge_anchors
                .get(&(abs_row as u32, abs_col as u32))
                .map_or(abs_col, |(_row, col)| *col as usize);
            let name = Self::cell_to_string(Self::resolve_cell(
                range,
                merge_anchors,
                header_row,
                col_idx,
                cell,
            ))
            .trim()
            .to_string();
            // A `*name` header declares a multi-row list (回流-5 Q6): the `*`
            // is a layout modifier, not part of the field name.
            let name = name
                .strip_prefix('*')
                .filter(|n| !n.is_empty())
                .unwrap_or(&name)
                .to_string();
            // Audit finding 2026-10-30: the flat path names an empty header
            // cell `col<n>` (headers_at), so the structural path must do the
            // same; an empty group name would otherwise make two blank
            // top-level headers collide as "Duplicate column name ''".
            let name = if name.is_empty() {
                format!("col{abs_col}")
            } else {
                name
            };
            // One merged region binds every covered column to the same
            // anchor, so a multi-level parent (width >= 3, e.g. `*waves`
            // spanning D1:F1) stays a single group rather than being split
            // into sibling groups that share the parent's name (audit
            // finding 2026-10-29).
            if let Some(current) = groups.last_mut() {
                if current.columns.first() == Some(&anchor_col) {
                    current.columns.push(abs_col);
                    continue;
                }
            }
            groups.push(HeaderGroup {
                name,
                columns: vec![abs_col],
            });
        }
        groups
    }

    /// Assemble one data row under the nesting rules (design §51 Q6): a
    /// top-level column with `sub_columns` becomes a nested `Value::Object`
    /// (recursing through grandchildren), every other column keeps its cell
    /// value. The top-level field order follows the header groups.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fields_for_nested_row(
        header_groups: &[HeaderGroup],
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        row: &[Data],
        convention_meta: Option<&TableMeta>,
        rel_row: usize,
        sheet_row: usize,
        range_start_col: usize,
        file_path: &str,
        sheet_name: &str,
    ) -> IndexMap<String, TypedValue> {
        let mut fields = IndexMap::new();
        for group in header_groups {
            let nested = convention_meta
                .and_then(|meta| meta.columns.get(&group.name))
                .filter(|meta| !meta.sub_columns.is_empty());
            let value = if let Some(meta) = nested {
                Value::Object(Self::nested_object_value(
                    meta,
                    range,
                    merge_anchors,
                    row,
                    rel_row,
                    range_start_col,
                ))
            } else {
                Self::nested_cell_value(
                    range,
                    merge_anchors,
                    row,
                    rel_row,
                    group.columns.first().copied().unwrap_or(0),
                    range_start_col,
                )
            };
            let col_letter = Self::col_index_to_letter(group.columns.first().copied().unwrap_or(0));
            let loc = SourceLocation::new(file_path)
                .with_format("excel")
                .with_sheet(sheet_name.to_string())
                .with_row(sheet_row)
                .with_column(col_letter)
                .with_field(group.name.clone());
            fields.insert(group.name.clone(), TypedValue::new(value, loc));
        }
        fields
    }

    /// A nested bean's fields, in sub-column declaration order: a child with
    /// its own sub-columns is itself an object, a childless one is the cell
    /// at its declaration column.
    pub(crate) fn nested_object_value(
        meta: &ColumnMeta,
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        row: &[Data],
        rel_row: usize,
        range_start_col: usize,
    ) -> IndexMap<String, Value> {
        let mut value = IndexMap::new();
        for (name, child) in &meta.sub_columns {
            let Some(abs_col) = a1_column(&child.cell) else {
                continue;
            };
            let child_value = if child.sub_columns.is_empty() {
                Self::nested_cell_value(
                    range,
                    merge_anchors,
                    row,
                    rel_row,
                    abs_col,
                    range_start_col,
                )
            } else {
                Value::Object(Self::nested_object_value(
                    child,
                    range,
                    merge_anchors,
                    row,
                    rel_row,
                    range_start_col,
                ))
            };
            value.insert(name.clone(), child_value);
        }
        value
    }

    /// The cell value at an absolute sheet column on the current data row; a
    /// column past the row's width is an empty cell (`Null`).
    pub(crate) fn nested_cell_value(
        range: &Range<Data>,
        merge_anchors: &HashMap<(u32, u32), (u32, u32)>,
        row: &[Data],
        rel_row: usize,
        abs_col: usize,
        range_start_col: usize,
    ) -> Value {
        let Some(rel_col) = abs_col.checked_sub(range_start_col) else {
            return Value::Null;
        };
        let Some(cell) = row.get(rel_col) else {
            return Value::Null;
        };
        Self::cell_to_cage_value(Self::resolve_cell(
            range,
            merge_anchors,
            rel_row,
            rel_col,
            cell,
        ))
    }
}
