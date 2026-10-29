//! Excel meta-row convention engine (X series, design §51).
//!
//! Layout-agnostic by construction: this module knows nothing about
//! workbooks. It takes a 2-D grid of cells, the data header row, and a
//! territory of row indices to scan (chosen by the source adapter from the
//! declared preset), and decodes the territory into [`TableMeta`] — the same
//! structured metadata the script adapter returns, so both paths feed the
//! same downstream (schema-draft / drift check, X2).
//!
//! Everything here is gated behind an explicit `[source.excel]` declaration;
//! a table with no declaration never reaches this code and reads byte for
//! byte as before. Meta rows carry declarations only: no rule semantics are
//! invented here, and the decoded metadata never enters the IR data plane
//! (design §51, X3).

use crate::diagnostics::{Diagnostic, Diagnostics};
use crate::error::codes::source_convention::{E2201, E2202, E2203, E2204, E2205, E2206, E2207};
use crate::schema::{FieldType, MapField, MapKeyType, ReferenceSchema};
use crate::value::SourceLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Contract version spoken between cage and a user script (design §51 Q2).
/// Both sides check it; an unknown version fails loudly (E2206).
pub const CONTRACT_VERSION: &str = "cage.excel-meta/1";

/// Default stdout timeout for the script adapter (`script_timeout_secs`).
pub const DEFAULT_SCRIPT_TIMEOUT_SECS: u64 = 10;

/// Default marker that introduces a convention keyword cell.
pub const DEFAULT_MARKER: &str = "##";

/// One cell of the decoded sheet grid, in the same shape the script
/// contract sends (null / boolean / number / string; JSON has a single
/// numeric type, so integers and non-integers are one thing textually).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellValue {
    /// Empty cell
    Null,
    /// Boolean cell
    Bool(bool),
    /// Numeric cell with no fractional part
    Int(i64),
    /// Numeric cell with a fractional part
    Float(f64),
    /// Text cell (dates arrive as ISO-8601 strings, per the contract)
    Str(String),
}

impl CellValue {
    /// Cell text with surrounding whitespace trimmed; an empty cell is `""`.
    pub fn text(&self) -> String {
        match self {
            CellValue::Null => String::new(),
            CellValue::Bool(b) => b.to_string(),
            CellValue::Int(i) => i.to_string(),
            CellValue::Float(f) => f.to_string(),
            CellValue::Str(s) => s.trim().to_string(),
        }
    }
}

/// A whole sheet as rows of cells, 0-based in both directions.
pub type Grid = Vec<Vec<CellValue>>;

/// Location preset (design §51 Q1). One grammar, four territories; `Data`
/// is the default and means no convention at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// No convention — today's behavior, byte for byte.
    #[default]
    Data,
    /// The contiguous non-empty run of rows immediately above the data
    /// header row (stops at the first empty row).
    HeaderBlock,
    /// Every row above the data header row, scanned in full.
    MarkerRows,
    /// A sibling `<sheet>.meta` sheet carries the declarations.
    MetaSheet,
}

impl Preset {
    /// Whether this preset declares a convention at all (anything but the
    /// `data` baseline).
    pub fn is_data(&self) -> bool {
        matches!(self, Preset::Data)
    }
}

/// One column's decoded declaration. Names and shapes mirror the Schema DSL
/// fields (`type`, `enum`, `reference`), so the downstream mapping (X2) is a
/// field-for-field copy, not a translation. `deny_unknown_fields` is the
/// vocabulary gate: a script (or a preset cell) that invents a keyword fails.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColumnMeta {
    /// A1 anchor inside the decoded sheet (`"G27"`), so every later
    /// diagnostic can point at the cell that declared it.
    pub cell: String,
    /// Schema DSL `type`
    #[serde(rename = "type")]
    pub field_type: Option<FieldType>,
    /// Schema DSL `required`
    pub required: Option<bool>,
    /// Schema DSL `primary_key` (column-level declaration; the table-level
    /// list is assembled on the Schema side, not here)
    pub primary_key: Option<bool>,
    /// Schema DSL `default`
    pub default: Option<JsonValue>,
    /// Schema DSL `enum`
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
    /// Schema DSL `min`
    pub min: Option<f64>,
    /// Schema DSL `max`
    pub max: Option<f64>,
    /// Schema DSL `unique`
    pub unique: Option<bool>,
    /// Schema DSL `reference` (`predicate` is deliberately out: a meta row
    /// declares where a column points, any extra predicate lives in the
    /// Schema)
    pub reference: Option<ReferenceSchema>,
    /// Schema DSL `description`
    pub description: Option<String>,
    /// Schema DSL `targets`
    pub targets: Vec<String>,
    /// Child columns declared by a `##+` sub-column row. A non-empty map
    /// makes the parent a nested bean by construction.
    pub sub_columns: IndexMap<String, ColumnMeta>,
    /// Whether a Luban `*name` header folds adjacent data rows into an array
    /// of the nested objects declared by `sub_columns`.
    pub multi_row: bool,
}

/// Decoded metadata for one table: the header row the decoder resolved, and
/// the per-column declarations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableMeta {
    /// 0-based index of the data header row as the decoder resolved it (the
    /// script adapter may answer a different row than the config guessed).
    pub header_row: usize,
    /// Per-column declarations keyed by data column name.
    pub columns: IndexMap<String, ColumnMeta>,
}

impl TableMeta {
    /// Whether anything was declared at all.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

/// The `[source.excel]` table of cage.toml (design §51 Q1/Q2). Everything is
/// optional; an absent table means no convention and no behavior change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExcelConvention {
    /// Location preset for workbooks without a `workbooks` override.
    pub preset: Preset,
    /// Marker that introduces a keyword cell (`marker_rows` / the meta
    /// sheet's rows). Must start with `#`.
    pub marker: String,
    /// Per-workbook preset override, keyed by workbook file name (with
    /// extension). Ignored when `script` is set.
    pub workbooks: IndexMap<String, Preset>,
    /// Script adapter: path relative to the project root, forward slashes.
    /// Mutually exclusive with `preset`.
    pub script: Option<String>,
    /// Extra argv passed to the script verbatim (how an interpreted script
    /// gets its interpreter without a shell: `script = "python3"`,
    /// `script_args = ["tools/excel-meta.py"]`).
    pub script_args: Vec<String>,
    /// Script stdout timeout in seconds.
    pub script_timeout_secs: u64,
    /// Any TOML table, passed to the script verbatim in the request's
    /// `config` field.
    pub script_config: JsonValue,
}

impl Default for ExcelConvention {
    fn default() -> Self {
        Self {
            preset: Preset::Data,
            marker: DEFAULT_MARKER.to_string(),
            workbooks: IndexMap::new(),
            script: None,
            script_args: Vec::new(),
            script_timeout_secs: DEFAULT_SCRIPT_TIMEOUT_SECS,
            script_config: serde_json::json!({}),
        }
    }
}

impl ExcelConvention {
    /// Declaration-level coherence checks (E2201). Cell-level syntax is
    /// checked per sheet by [`decode`].
    pub fn validate(&self) -> Result<(), String> {
        if !self.marker.starts_with('#') {
            return Err(format!(
                "{E2201} marker must start with '#', got {:?}",
                self.marker
            ));
        }
        if self.script.is_some() {
            if !self.preset.is_data() {
                return Err(format!(
                    "{E2201} preset and script are mutually exclusive: the script replaces \
                     the built-in presets entirely"
                ));
            }
            if !self.workbooks.is_empty() {
                return Err(format!(
                    "{E2201} workbooks overrides do not apply when a script is declared"
                ));
            }
            if self.script.as_deref().is_none_or(str::is_empty) {
                return Err(format!("{E2201} script must not be empty"));
            }
        }
        Ok(())
    }

    /// The preset that applies to one workbook (file name with extension).
    pub fn preset_for(&self, workbook_name: &str) -> Preset {
        if self.script.is_some() {
            return Preset::Data; // the script replaces presets; see `uses_script`
        }
        self.workbooks
            .get(workbook_name)
            .copied()
            .unwrap_or(self.preset)
    }

    /// Whether the script adapter handles this workbook instead of the
    /// built-in presets.
    pub fn uses_script(&self) -> bool {
        self.script.is_some()
    }
}

/// The source fingerprint input for a script adapter (design §51 Q2):
/// blake3 of the script's bytes + resolved path + canonical `script_config`
/// + contract version. Editing the script or its config changes the string,
///   so `source_hash` / `build_id` rotate even when the parsed model is byte-identical.
pub fn script_fingerprint(
    root: &Path,
    convention: &ExcelConvention,
) -> Result<Option<String>, Diagnostics> {
    let Some(rel) = convention.script.as_deref() else {
        return Ok(None);
    };
    let script = resolve_script(root, rel)?;
    let bytes = std::fs::read(&script).map_err(|e| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            crate::error::codes::source_convention::E2203,
            format!("failed to read script {}: {e}", script.display()),
        ));
        diags
    })?;
    let script_hash = blake3::hash(&bytes).to_hex().to_string();
    // Canonical `script_config`: serialize with object keys sorted so that
    // semantically equal configs (differing only in key order) hash the
    // same, matching the "canonical form" determinism contract.
    let config = canonical_json(&convention.script_config);
    Ok(Some(format!(
        "{CONTRACT_VERSION}|{script_hash}|{}|{config}",
        script.to_string_lossy()
    )))
}

/// Serialize a JSON value with every object's keys sorted lexicographically
/// (recursively), so the bytes are order-independent — the "canonical form"
/// the deterministic-build contract requires. Arrays keep their order (order
/// is meaningful there); only object key order is normalized.
fn canonical_json(value: &JsonValue) -> String {
    fn sort(value: &JsonValue) -> JsonValue {
        match value {
            JsonValue::Object(map) => {
                let mut entries: Vec<(String, JsonValue)> =
                    map.iter().map(|(k, v)| (k.clone(), sort(v))).collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                JsonValue::Object(entries.into_iter().collect())
            }
            JsonValue::Array(items) => JsonValue::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(value)).unwrap_or_else(|_| value.to_string())
}

/// `[source.sheets]` (design §51 Q7): the shared §51 fields plus the
/// sheets-side `header_row` knob, in one flat table. `None` on
/// `ProjectConfig` — or a default value — is the baseline: the first
/// fetched row is the header and no meta-row convention is read at all.
///
/// The fields mirror `ExcelConvention` exactly (same grammar, vocabulary and
/// E-codes); they are repeated here rather than flattened because serde
/// `flatten` disables `deny_unknown_fields`, so a config typo (e.g.
/// `presett = "marker_rows"`) would silently mean "no convention" instead of
/// erroring the way `[source.excel]` does.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SheetsConvention {
    /// 0-based index of the data header row in the fetched tab. Default 0
    /// is today's assumption; `marker_rows` / `header_block` read the
    /// convention territory above it (Q7).
    pub header_row: usize,
    /// Location preset, as in `ExcelConvention.preset`.
    pub preset: Preset,
    /// Marker that introduces a keyword cell.
    pub marker: String,
    /// Per-workbook preset override — sheets has no workbook file name, so
    /// this is unused (it is only a validation error when a script is
    /// declared, as in `ExcelConvention::validate`); kept for the shared
    /// vocabulary.
    pub workbooks: IndexMap<String, Preset>,
    /// Script adapter path, as in `ExcelConvention.script`.
    pub script: Option<String>,
    /// Extra script argv.
    pub script_args: Vec<String>,
    /// Script stdout timeout in seconds.
    pub script_timeout_secs: u64,
    /// Any TOML table passed to the script in the request's `config` field.
    pub script_config: JsonValue,
}

impl Default for SheetsConvention {
    /// Same defaults as [`ExcelConvention`]: a sheet author who writes
    /// `preset = "marker_rows"` gets `##` as the marker and the same
    /// script timeout, not the type defaults.
    fn default() -> Self {
        Self {
            header_row: 0,
            preset: Preset::Data,
            marker: DEFAULT_MARKER.to_string(),
            workbooks: IndexMap::new(),
            script: None,
            script_args: Vec::new(),
            script_timeout_secs: DEFAULT_SCRIPT_TIMEOUT_SECS,
            script_config: serde_json::json!({}),
        }
    }
}

impl SheetsConvention {
    /// Whether the script adapter handles the tab instead of the built-in
    /// presets.
    pub fn uses_script(&self) -> bool {
        self.script.is_some()
    }

    /// Declaration-level coherence checks (E2201), as
    /// [`ExcelConvention::validate`].
    pub fn validate(&self) -> Result<(), String> {
        self.as_excel_convention().validate()
    }

    /// The shared convention view (the `[source.excel]`-equivalent), so the
    /// sheets adapter reuses the same decode path and E-code vocabulary.
    pub fn as_excel_convention(&self) -> ExcelConvention {
        ExcelConvention {
            preset: self.preset,
            marker: self.marker.clone(),
            workbooks: self.workbooks.clone(),
            script: self.script.clone(),
            script_args: self.script_args.clone(),
            script_timeout_secs: self.script_timeout_secs,
            script_config: self.script_config.clone(),
        }
    }
}

/// A1 column letters for a 0-based absolute column index (0 → "A", 25 →
/// "Z", 26 → "AA").
pub fn col_index_to_letter(mut col: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    letters.reverse();
    letters.into_iter().collect()
}

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

/// `true` / `false`, case-insensitive — nothing else (E2202).
fn parse_bool_text(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, got {value:?}")),
    }
}

/// A number, verbatim (E2202).
fn parse_number_text(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("{value:?} is not a number"))
}

/// A comma-separated value list, entries trimmed, empties dropped; an empty
/// result is an error (E2202).
fn parse_list_text(value: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if items.is_empty() {
        Err(format!("{value:?} is not a value list"))
    } else {
        Ok(items)
    }
}

/// `Table.Field[:cardinality]` — cardinality defaults to `one`, matching
/// `ReferenceSchema`'s own default (E2202).
fn parse_reference_text(value: &str) -> Result<ReferenceSchema, String> {
    let (target, cardinality) = match value.split_once(':') {
        Some((target, card)) => (target, card.trim().to_string()),
        None => (value, "one".to_string()),
    };
    if !matches!(cardinality.as_str(), "one" | "many" | "optional") {
        return Err(format!(
            "reference cardinality must be one | many | optional, got {cardinality:?}"
        ));
    }
    let Some((table, field)) = target.split_once('.') else {
        return Err(format!(
            "reference must be Table.field, got {value:?} (no '.')"
        ));
    };
    let (table, field) = (table.trim(), field.trim());
    if table.is_empty() || field.is_empty() {
        return Err(format!("reference must be Table.field, got {value:?}"));
    }
    Ok(ReferenceSchema {
        table: table.to_string(),
        field: field.to_string(),
        predicate: None,
        cardinality,
        compatible_with: None,
    })
}

/// The type cell accepts two spellings (design §51, cell text notation):
/// the Schema's own expression form — `{ kind: Int32 }`,
/// `{ kind: Array, value: { kind: Int32 } }` — parsed by the Schema's own
/// deserializer so there is no second vocabulary; and the cell shorthand —
/// `Int32`, `Array<Int32>`, `Enum<Rarity>`, `Map<string, Int32>` — which
/// renders into exactly that tagged form. An `Object` cannot be declared in
/// a cell (its value is a property map): it belongs in the Schema (E2202).
pub fn parse_type_text(text: &str) -> Result<FieldType, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("empty type cell".to_string());
    }
    if t.starts_with('{') {
        return serde_yaml::from_str::<FieldType>(t)
            .map_err(|e| format!("{t:?} is not a Schema type expression: {e}"));
    }
    if let Some(rest) = t.strip_prefix("Array<").and_then(|s| s.strip_suffix('>')) {
        return parse_type_text(rest)
            .map(|inner| FieldType::Array(Box::new(inner)))
            .map_err(|e| format!("{t:?}: {e}"));
    }
    if let Some(rest) = t.strip_prefix("Enum<").and_then(|s| s.strip_suffix('>')) {
        let name = rest.trim();
        if !is_identifier(name) {
            return Err(format!("{t:?} is not a valid Enum name"));
        }
        return Ok(FieldType::Enum(name.to_string()));
    }
    if let Some(rest) = t.strip_prefix("Map<").and_then(|s| s.strip_suffix('>')) {
        let Some((key, value)) = rest.split_once(',') else {
            return Err(format!(
                "{t:?}: a Map needs Map<key, value> with a comma between the types"
            ));
        };
        let key_type = match key.trim() {
            "string" => MapKeyType::String,
            "int" => MapKeyType::Int,
            other => {
                return Err(format!(
                    "{t:?}: map key type must be string or int, got {other:?}"
                ))
            }
        };
        let value_type = parse_type_text(value).map_err(|e| format!("{t:?}: {e}"))?;
        return Ok(FieldType::Map(MapField {
            key_type,
            value_type: Box::new(value_type),
        }));
    }
    if t.starts_with("Object") {
        return Err(format!(
            "{t:?}: an Object's property map cannot be declared in a cell — declare it in the \
             Schema"
        ));
    }
    scalar_kind(t).ok_or_else(|| format!("{t:?} is not a Schema type name"))
}

fn scalar_kind(t: &str) -> Option<FieldType> {
    Some(match t {
        "Null" => FieldType::Null,
        "Bool" => FieldType::Bool,
        "Int8" => FieldType::Int8,
        "Int16" => FieldType::Int16,
        "Int32" => FieldType::Int32,
        "Int64" => FieldType::Int64,
        "UInt8" => FieldType::UInt8,
        "UInt16" => FieldType::UInt16,
        "UInt32" => FieldType::UInt32,
        "UInt64" => FieldType::UInt64,
        "Float32" => FieldType::Float32,
        "Float64" => FieldType::Float64,
        "String" => FieldType::String,
        "Bytes" => FieldType::Bytes,
        "Any" => FieldType::Any,
        _ => return None,
    })
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Absolute A1 anchor for a 0-based absolute row/column (`"G27"`).
pub fn anchor(row: usize, col: usize) -> String {
    format!("{}{}", col_index_to_letter(col), row + 1)
}

/// Source location for a convention cell.
fn loc(file: &str, sheet: &str, row: usize, col: usize) -> SourceLocation {
    SourceLocation::new(file)
        .with_format("excel")
        .with_sheet(sheet.to_string())
        .with_row(row + 1)
        .with_column(col_index_to_letter(col))
}

fn error(code: &str, message: String) -> Diagnostic {
    Diagnostic::error(code, message)
}

/// Resolve a script path declared in cage.toml against the project root
/// (design §51 Q2: repo-relative explicit path, no `$PATH` lookup, no `..`
/// escape, symlinks may not leave the root).
pub fn resolve_script(root: &Path, rel: &str) -> Result<PathBuf, Diagnostics> {
    let fail = |message: String| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(E2203, message));
        Err(diags)
    };
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return fail(format!(
            "{E2203} script must be relative to the project root, got {rel:?}"
        ));
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return fail(format!(
            "{E2203} script path must not contain '.' or '..' segments, got {rel:?}"
        ));
    }
    let joined = root.join(rel_path);
    let (Ok(root_canon), Ok(joined_canon)) = (root.canonicalize(), joined.canonicalize()) else {
        return fail(format!(
            "{E2203} script not found: {} (relative to the project root)",
            joined.display()
        ));
    };
    if !joined_canon.starts_with(&root_canon) {
        return fail(format!(
            "{E2203} script escapes the project root through a symlink: {rel:?}"
        ));
    }
    if !joined_canon.is_file() {
        return fail(format!(
            "{E2203} script is not a file: {}",
            joined.display()
        ));
    }
    // Return the canonical path: `root` may itself be a relative path (e.g.
    // `.` from the CLI), so the raw `root.join(rel)` spelling would make the
    // same project hash differently depending on how the root was spelled.
    // The canonical path is the same bytes for every spelling of the same
    // project, preserving the "same input → same build_id" contract.
    Ok(joined_canon)
}

/// Run the script adapter once: the request JSON on stdin, the response on
/// stdout (design §51 Q2). E2203 will not start, E2204 timeout, E2205
/// non-zero exit, E2206 unusable output. The script runs with the author's
/// own privileges — cage sandboxes nothing and claims nothing.
pub fn run_script(
    root: &Path,
    rel: &str,
    args: &[String],
    timeout: Duration,
    request: &JsonValue,
) -> Result<JsonValue, Diagnostics> {
    let script = resolve_script(root, rel)?;
    let mut child = std::process::Command::new(&script)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                E2203,
                format!("failed to start script {}: {e}", script.display()),
            ));
            diags
        })?;

    // The request goes out before we wait: a script that exits early gets a
    // broken pipe, reported as E2205 (it chose to stop reading).
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(request.to_string().as_bytes()) {
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                E2205,
                format!(
                    "script {} closed stdin before the request was written: {e}",
                    script.display()
                ),
            ));
            let _ = child.kill();
            return Err(diags);
        }
    } // dropping stdin signals EOF

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    let (abort_tx, abort_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut child = child;
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    let _ = tx.send(child.wait_with_output());
                    return;
                }
                Ok(None) => {
                    if abort_rx.try_recv().is_ok() {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = tx.send(Err(std::io::Error::other("timed out")));
                        return;
                    }
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = tx.send(Err(std::io::Error::other("timed out")));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        }
    });

    let output = match rx.recv_timeout(Duration::from_secs(2).max(timeout)) {
        Ok(Ok(output)) => Some(output),
        Ok(Err(_timed_out)) => None,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = abort_tx.send(());
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    };
    let Some(output) = output else {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2204,
            format!(
                "script {} timed out after {}s without producing a response",
                script.display(),
                timeout.as_secs_f64()
            ),
        ));
        return Err(diags);
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let stderr = if stderr.len() > 400 {
            &stderr[..400]
        } else {
            stderr
        };
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2205,
            format!(
                "script {} exited with {}: {stderr}",
                script.display(),
                output.status
            ),
        ));
        return Err(diags);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let value: JsonValue = serde_json::from_str(text.trim()).map_err(|e| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2206,
            format!(
                "script {} did not produce valid JSON: {e}",
                script.display()
            ),
        ));
        diags
    })?;
    if value.get("contract").and_then(|c| c.as_str()) != Some(CONTRACT_VERSION) {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(
            E2206,
            format!(
                "script {} answered contract {:?}, cage speaks {CONTRACT_VERSION:?}",
                script.display(),
                value.get("contract").cloned().unwrap_or(JsonValue::Null)
            ),
        ));
        return Err(diags);
    }
    Ok(value)
}

/// Parse and gate the script's `columns` against the data header row. The
/// vocabulary is enforced by deserialization (`deny_unknown_fields` on
/// [`ColumnMeta`]); a `type` cage cannot parse is E2202, any other shape
/// violation is E2206, a column name outside the header row or a wrong
/// `table` is E2207 (design §51 Q3).
pub fn parse_response(
    value: &JsonValue,
    expected_table: &str,
    header_names: &[String],
    source_file: &str,
    sheet: &str,
) -> Result<TableMeta, Diagnostics> {
    let fail = |code: &str, message: String| {
        let mut diags = Diagnostics::new();
        diags.add(Diagnostic::error(code, message));
        Err(diags)
    };
    let Some(obj) = value.as_object() else {
        return fail(E2206, format!("{E2206} response is not an object"));
    };
    let table = match obj.get("table").and_then(serde_json::Value::as_str) {
        Some(t) => t.to_string(),
        None => return fail(E2206, format!("{E2206} response carries no table name")),
    };
    if table != expected_table {
        return fail(
            E2207,
            format!(
                "{E2207} script answered table {table:?}, this sheet resolves to \
                 {expected_table:?}"
            ),
        );
    }
    let header_row = match obj.get("header_row").and_then(serde_json::Value::as_u64) {
        Some(h) => h as usize,
        None => {
            return fail(
                E2206,
                format!("{E2206} response carries no usable header_row"),
            )
        }
    };
    let Some(columns_json) = obj.get("columns").and_then(serde_json::Value::as_object) else {
        return fail(E2206, format!("{E2206} response carries no columns object"));
    };
    if columns_json.is_empty() {
        return fail(E2206, format!("{E2206} response declares no columns"));
    }

    let mut columns = IndexMap::new();
    for (name, col_json) in columns_json {
        let meta: ColumnMeta = serde_json::from_value(col_json.clone()).map_err(|e| {
            let message = e.to_string();
            let is_vocabulary =
                message.contains("unknown variant") || message.contains("unknown field");
            let code = if is_vocabulary { E2202 } else { E2206 };
            let mut diags = Diagnostics::new();
            diags.add(Diagnostic::error(
                code,
                format!("{code} column {name:?}: {e} (source {source_file}, sheet {sheet})"),
            ));
            diags
        })?;
        if meta.cell.is_empty() || !is_a1(&meta.cell) {
            return fail(
                E2206,
                format!(
                    "{E2206} column {name:?} carries cell anchor {:?}, expected an A1 \
                     address like \"B2\" or \"G27\"",
                    meta.cell
                ),
            );
        }
        if has_sub_columns(&meta) {
            return fail(
                E2206,
                format!(
                    "{E2206} column {name:?} declares sub_columns; script-declared nesting \
                     is a later slice (design §51 Q6)"
                ),
            );
        }
        if meta.multi_row {
            return fail(
                E2206,
                format!(
                    "{E2206} column {name:?} declares multi_row; script-declared multi-row \
                     records are a later slice (design §51 Q6)"
                ),
            );
        }
        if !header_names.iter().any(|h| h == name) {
            return fail(
                E2207,
                format!("{E2207} script declares column {name:?}, which is not in the header row"),
            );
        }
        columns.insert(name.clone(), meta);
    }
    Ok(TableMeta {
        header_row,
        columns,
    })
}

/// Whether this column (or any descendant) carries `##+` sub-columns.
fn has_sub_columns(column: &ColumnMeta) -> bool {
    !column.sub_columns.is_empty() || column.sub_columns.values().any(has_sub_columns)
}

/// 0-based column index of an A1 cell (`"G27"` → 6). Public because the
/// Excel data plane maps a sub-column's cell anchor to its sheet column.
pub fn a1_column(cell: &str) -> Option<usize> {
    if !is_a1(cell) {
        return None;
    }
    let letters = cell.bytes().take_while(u8::is_ascii_uppercase);
    Some(letters.fold(0, |acc, b| acc * 26 + usize::from(b - b'A' + 1)) - 1)
}

/// 0-based row of an A1 cell (`"G27"` → 26), alongside [`a1_column`].
pub fn a1_row(cell: &str) -> Option<usize> {
    if !is_a1(cell) {
        return None;
    }
    let letters = cell.bytes().take_while(u8::is_ascii_uppercase).count();
    cell[letters..].parse::<usize>().ok().map(|r| r - 1)
}

/// Both coordinates of an A1 cell, 0-based.
fn a1_row_col(cell: &str) -> Option<(usize, usize)> {
    Some((a1_row(cell)?, a1_column(cell)?))
}

/// A1 address shape check: 1-3 capital letters, then a row number without a
/// leading zero.
fn is_a1(cell: &str) -> bool {
    let bytes = cell.as_bytes();
    let letters = bytes.iter().take_while(|b| b.is_ascii_uppercase()).count();
    let digits = bytes.len() - letters;
    (1..=3).contains(&letters)
        && digits >= 1
        && bytes[letters..].iter().all(u8::is_ascii_digit)
        && bytes[letters] != b'0'
}

#[cfg(test)]
mod tests;

/// X2: decoded `TableMeta` as a schema, and the drift check (E2208).
pub mod derive;
