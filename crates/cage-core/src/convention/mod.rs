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

use crate::diagnostics::Diagnostic;
use crate::error::codes::source_convention::E2201;
use crate::schema::{FieldType, ReferenceSchema};
use crate::value::SourceLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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

mod meta;
mod script;
mod types;

pub use meta::{convention_territory, decode};
pub use script::{parse_response, resolve_script, run_script, script_fingerprint};
pub use types::parse_type_text;

#[cfg(test)]
mod tests;

/// X2: decoded `TableMeta` as a schema, and the drift check (E2208).
pub mod derive;
