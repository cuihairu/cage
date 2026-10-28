//! Google Sheets source adapter (S3, design §45): `gsheet:<id>/<tab>`
//! fetches one tab through Sheets API v4 `values` with
//! `valueRenderOption=UNFORMATTED_VALUE` (formula cache values, no
//! recalculation — the Excel adapter's discipline), first row = header
//! (the Excel convention), remaining rows = records. The credential is
//! an API key read from the env var named in
//! `[remote.gsheets].credential_env` (E1904 when missing; the key never
//! lives in cage.toml nor in any error message — sources are referred to
//! by their `gsheet:<id>/<tab>` spec only). Service-account credentials
//! need an OAuth JWT exchange and are deliberately not in the first cut.
//!
//! Cells render as their initial string form — numbers and booleans keep
//! their JSON text, NULL stays null; typed interpretation is the
//! Schema's job (L2/L3), not the adapter's. The tab's natural row order
//! is the author's promised order, so it is preserved verbatim. The
//! response shape is validated (E1903 — the row-set shape gate), then
//! serialized to canonical JSON and parsed by the standard JSON adapter:
//! the same cache anchor and the same parsing chain as every other
//! source, L0-L7 with no bypass.

use cage_core::convention::{
    self, a1_column, convention_territory, decode, parse_response, run_script, CellValue,
    ColumnMeta, Grid, Preset, SheetsConvention, TableMeta, CONTRACT_VERSION,
};
use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::E1901;
use cage_core::error::codes::remote::{E1902, E1903, E1904};
use cage_core::error::codes::source_convention::{E2201, E2207};
use cage_core::manifest::{ProjectConfig, RemoteSourceConfig};
use cage_core::remote::{self, FetchFailure};
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
use std::collections::HashSet;
use std::path::Path;

/// Config section name for the Sheets scheme: the spec prefix is
/// `gsheet:` (per design §45) while the settings live under
/// `[remote.gsheets]`.
const CONFIG_KEY: &str = "gsheets";

/// Production API root. Tests inject a local stand-in through
/// `SheetsSourceAdapter::load_with_base` — the CLI path always uses the
/// real endpoint.
const API_BASE: &str = "https://sheets.googleapis.com/v4";

/// Split a `[source_roots]` value into (spreadsheet_id, tab):
/// `gsheet:1AbC...xz/Levels`. Anything else is not a Sheets source.
pub fn parse_spec(spec: &str) -> Option<(&str, &str)> {
    let rest = spec.strip_prefix("gsheet:")?;
    let (id, tab) = rest.split_once('/')?;
    (!id.is_empty() && !tab.is_empty()).then_some((id, tab))
}

/// A spreadsheet id is an opaque URL path segment: letters, digits,
/// dash and underscore only. Anything else must not reach a URL.
fn valid_spreadsheet_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Percent-encode one URL path segment (RFC 3986 unreserved set kept
/// verbatim, everything else — spaces, `/`, non-ASCII — as %XX UTF-8).
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Resolve the API key from the env var named in
/// `[remote.gsheets].credential_env` — undeclared, unset, or empty is
/// E1904; the key itself is never read from the config.
pub fn resolve_credential(config: Option<&RemoteSourceConfig>) -> Result<String, String> {
    let env_name = config
        .and_then(|c| c.credential_env.as_deref())
        .ok_or_else(|| {
            format!(
                "{E1904} [remote.{CONFIG_KEY}] has no credential_env — declare the env var NAME \
                 in cage.toml; the key itself comes from the environment"
            )
        })?;
    match std::env::var(env_name) {
        Ok(key) if !key.trim().is_empty() => Ok(key),
        _ => Err(format!(
            "{E1904} env {env_name} is not set ([remote.{CONFIG_KEY}].credential_env)"
        )),
    }
}

/// The values endpoint URL: one tab, formula cache values, rows as the
/// major dimension. The API key rides the query string and must never
/// leak into logs or diagnostics.
pub fn build_url(api_base: &str, spreadsheet_id: &str, tab: &str, api_key: &str) -> String {
    format!(
        "{api_base}/spreadsheets/{spreadsheet_id}/values/{}?\
         valueRenderOption=UNFORMATTED_VALUE&majorDimension=ROWS&key={api_key}",
        percent_encode(tab)
    )
}

/// Render one cell into its initial string form: numbers and booleans
/// keep their JSON text (UNFORMATTED_VALUE's cache value — no
/// recalculation, no type guessing), NULL stays null.
fn cell_to_json(cell: &serde_json::Value) -> serde_json::Value {
    match cell {
        serde_json::Value::Null => serde_json::Value::Null,
        serde_json::Value::String(s) => serde_json::Value::String(s.clone()),
        other => serde_json::Value::String(other.to_string()),
    }
}

/// Validate the response shape and return its `values` array (the tab's
/// rows, each an array of cells). Everything that is not a proper row set
/// is E1903 — this is the shape gate E1903 exists for.
fn values_from_body(table_name: &str, body: &[u8]) -> Result<Vec<serde_json::Value>, String> {
    let reject =
        |why: &str| format!("{E1903} sheets response is not a row set, {why}: {table_name}");
    let response: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| reject(&format!("invalid JSON ({e})")))?;
    let obj = response
        .as_object()
        .ok_or_else(|| reject("not an object"))?;
    if obj
        .get("majorDimension")
        .and_then(|d| d.as_str())
        .map(|d| d != "ROWS")
        .unwrap_or(false)
    {
        return Err(reject("majorDimension is not ROWS"));
    }
    let values = obj
        .get("values")
        .and_then(|v| v.as_array())
        .ok_or_else(|| reject("no values (empty tab or malformed response)"))?;
    if values
        .first()
        .and_then(|row| row.as_array())
        .is_none_or(|row| row.is_empty())
    {
        return Err(reject("no values (empty tab or malformed response)"));
    }
    Ok(values.clone())
}

/// Serialize rows to canonical JSON (`{tab: [row objects]}`): the header is
/// `values[header_row]` (blank header cells fall back to `col<i>` like the
/// Excel adapter), the rows below are records, ragged rows padded/truncated
/// to the header width, fully blank rows skipped, natural row order
/// preserved. This is the plain (no-convention) reading of a tab.
fn plain_rows_canonical(
    table_name: &str,
    values: &[serde_json::Value],
    header_row: usize,
) -> Result<Vec<u8>, String> {
    let reject =
        |why: &str| format!("{E1903} sheets response is not a row set, {why}: {table_name}");
    let header_cells = values
        .get(header_row)
        .and_then(|row| row.as_array())
        .ok_or_else(|| reject(&format!("no header row at index {header_row}")))?;
    if header_cells.is_empty() {
        return Err(reject("blank header row"));
    }
    let headers: Vec<String> = header_cells
        .iter()
        .enumerate()
        .map(|(idx, cell)| match cell.as_str() {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => format!("col{idx}"),
        })
        .collect();

    let mut rows = Vec::new();
    for row in values.iter().skip(header_row + 1) {
        let mut obj = serde_json::Map::new();
        let mut any_value = false;
        for (idx, header) in headers.iter().enumerate() {
            let cell = row
                .get(idx)
                .map(cell_to_json)
                .unwrap_or(serde_json::Value::Null);
            if !cell.is_null() && cell.as_str() != Some("") {
                any_value = true;
            }
            obj.insert(header.clone(), cell);
        }
        // A fully blank row carries no record — same skip_empty_rows
        // default as the Excel adapter.
        if any_value {
            rows.push(serde_json::Value::Object(obj));
        }
    }

    let mut doc = serde_json::Map::new();
    doc.insert(table_name.to_string(), serde_json::Value::Array(rows));
    serde_json::to_vec_pretty(&serde_json::Value::Object(doc))
        .map_err(|e| reject(&format!("cannot serialize ({e})")))
}

/// Validate the response shape and serialize it to canonical JSON with the
/// plain reading (header at row 0): the no-convention baseline.
pub fn sheets_json_to_canonical(table_name: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let values = values_from_body(table_name, body)?;
    plain_rows_canonical(table_name, &values, 0)
}

/// The canonical bytes for one fetched tab under the project's
/// `[source.sheets]` declaration (design §51 Q7): no declaration reads as
/// the plain baseline; a `data` preset with a custom `header_row` just
/// moves the header; the other presets and the script adapter decode the
/// meta-row convention and apply the decoded structure (nested beans,
/// multi-row folds) before serializing.
fn conventioned_bytes(
    project_root: &Path,
    config: &ProjectConfig,
    id: &str,
    tab: &str,
    body: &[u8],
    api_base: &str,
    api_key: &str,
) -> Result<Vec<u8>, String> {
    let Some(convention) = config.source_sheets() else {
        return sheets_json_to_canonical(tab, body);
    };
    // Declaration-level coherence, once per source (E2201), as Excel does.
    convention
        .validate()
        .map_err(|message| format!("{message} ({tab})"))?;
    let values = values_from_body(tab, body)?;
    if convention.preset == Preset::Data && !convention.uses_script() {
        return plain_rows_canonical(tab, &values, convention.header_row);
    }
    // `meta_sheet` reads the sibling `<tab>.meta` tab. The fetch happens
    // online alongside the main tab — the conventioned output is what gets
    // cached, so the offline fallback replays it without re-fetching.
    let meta_values = if needs_meta_fetch(Some(convention)) {
        let meta_tab = format!("{tab}.meta");
        let meta_url = build_url(api_base, id, &meta_tab, api_key);
        // A missing sibling is a convention error (E2201), as in the Excel
        // adapter — never a fetch error, and never the keyed URL.
        let meta_body = remote::http_get(&meta_url).map_err(|failure| {
            format!(
                "{E2201} the meta sheet {meta_tab:?} for {tab:?} could not be read: {}",
                failure_brief(&failure)
            )
        })?;
        Some(values_from_body(&meta_tab, &meta_body)?)
    } else {
        None
    };
    canonical_with_convention(
        tab,
        &values,
        convention.header_row,
        convention,
        project_root,
        id,
        tab,
        meta_values.as_deref(),
    )
}

/// A URL-free rendering of a fetch failure. The API key rides the query
/// string, so no failure text may echo the URL; `FetchFailure`'s `Display`
/// does (it embeds the resource URL), so convention errors that carry a
/// `FetchFailure` use this instead.
fn failure_brief(failure: &FetchFailure) -> String {
    match failure {
        FetchFailure::Transport(_) => "transport failure".to_string(),
        FetchFailure::RateLimited(_) => "rate limited".to_string(),
        FetchFailure::NotFound(_) => "not found".to_string(),
        FetchFailure::Status(code) => format!("http {code}"),
    }
}

/// Map one fetch failure to its error code: auth rejection (401/403) is
/// E1902, everything else at the fetch stage is E1901. Diagnostics refer
/// to the spec — never to the URL, which carries the API key.
fn fetch_error(failure: FetchFailure, spec: &str) -> String {
    match failure {
        FetchFailure::Status(code @ (401 | 403)) => {
            format!("{E1902} sheets auth rejected for {spec} (http {code})")
        }
        other => format!("{E1901} cannot fetch {spec}: {other}"),
    }
}

/// Google Sheets source adapter.
pub struct SheetsSourceAdapter;

impl SheetsSourceAdapter {
    /// Load `gsheet:<id>/<tab>` against `config`'s `[remote.gsheets]`
    /// settings: validate spec → resolve credential (E1904) → fetch
    /// (E1901 / E1902, with the S4 offline fallback to the previous
    /// cache copy on transport failures unless `strict`) → shape gate
    /// (E1903) → canonical JSON → cache file → standard JSON parse.
    pub fn load(
        project_root: &Path,
        config: &ProjectConfig,
        spec: &str,
        strict: bool,
    ) -> Result<Document, String> {
        Self::load_with_base(project_root, config, spec, API_BASE, strict)
    }

    /// Same pipeline against an injected API root — the seam tests use
    /// to stand in for sheets.googleapis.com; the CLI always goes
    /// through `load`.
    fn load_with_base(
        project_root: &Path,
        config: &ProjectConfig,
        spec: &str,
        api_base: &str,
        strict: bool,
    ) -> Result<Document, String> {
        let (id, tab) =
            parse_spec(spec).ok_or_else(|| format!("{E1901} not a gsheet: source spec: {spec}"))?;
        if !valid_spreadsheet_id(id) {
            return Err(format!(
                "{E1901} invalid spreadsheet id in {spec} (letters, digits, '-' and '_' only)"
            ));
        }
        let key = resolve_credential(config.remote.get(CONFIG_KEY))?;
        let url = build_url(api_base, id, tab, &key);

        // The cache key covers what identifies the bytes (spreadsheet +
        // tab); the API key does not change the payload and never lands
        // in a path. Resolved before any bytes move — the offline
        // fallback needs it too.
        let key_input = format!("{CONFIG_KEY}\n{id}\n{tab}");
        let cache_dir = remote::source_cache_dir(project_root, &key_input);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(&key_input)));

        let cache = match remote::http_get(&url) {
            Ok(body) => {
                let bytes =
                    conventioned_bytes(project_root, config, id, tab, &body, api_base, &key)?;
                std::fs::create_dir_all(&cache_dir).map_err(|e| {
                    format!(
                        "{E9902} cannot create source cache {}: {e}",
                        cache_dir.display()
                    )
                })?;
                std::fs::write(&cache_file, &bytes).map_err(|e| {
                    format!(
                        "{E9902} cannot write source cache {}: {e}",
                        cache_file.display()
                    )
                })?;
                cache_file
            }
            Err(FetchFailure::Transport(e)) => {
                // Offline with a previous copy → serve it (E1906
                // WARNING). The warning carries the spec only — never
                // the URL, which carries the API key.
                remote::source_cache_fallback(&cache_file, spec, strict)
                    .ok_or_else(|| fetch_error(FetchFailure::Transport(e), spec))?
            }
            Err(failure) => return Err(fetch_error(failure, spec)),
        };
        JsonSourceAdapter::parse_file(&cache).map_err(|diags| {
            eprintln!("{}", diags.render(false));
            format!("failed to parse row set for {spec}")
        })
    }
}

// ---- design §51 Q7: the sheets adapter carries the same convention the
// Excel adapter does. The pipeline is fetch → canonical JSON → JSON adapter,
// so the convention decode and the structure application (sub_columns → nested
// objects, multi_row → folded arrays) happen at the JSON level, reusing the
// shared cage-core::convention machinery (decode / run_script /
// parse_response) but not the Excel `Range<Data>` code.

/// One fetched cell into the convention grid's cell form.
fn cell_to_cellvalue(cell: &serde_json::Value) -> CellValue {
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
fn row_to_grid_row(row: &serde_json::Value) -> Vec<CellValue> {
    row.as_array()
        .map(|cells| cells.iter().map(cell_to_cellvalue).collect())
        .unwrap_or_default()
}

/// The fetched rows as a convention grid (the `values` array is already the
/// tab's used range from its top-left, so row 0 is the grid's row 0).
fn values_grid(values: &[serde_json::Value]) -> Grid {
    values.iter().map(row_to_grid_row).collect()
}

/// A data cell's canonical form: numbers and booleans keep their JSON text
/// (UNFORMATTED_VALUE's cache value), NULL stays null, strings stay strings —
/// the same rendering the baseline `sheets_json_to_canonical` uses.
fn render_cell(cell: &serde_json::Value) -> serde_json::Value {
    match cell {
        serde_json::Value::Null => serde_json::Value::Null,
        serde_json::Value::String(s) => serde_json::Value::String(s.clone()),
        other => serde_json::Value::String(other.to_string()),
    }
}

/// The cell at absolute column `abs_col` in one fetched row (ragged rows read
/// as null past their width, like the baseline).
fn cell_at(row: &serde_json::Value, abs_col: usize) -> serde_json::Value {
    row.as_array()
        .and_then(|cells| cells.get(abs_col))
        .map(render_cell)
        .unwrap_or(serde_json::Value::Null)
}

/// One top-level column's value for a data row: a column with sub-columns is
/// a nested bean (object of its children), a childless column is its cell.
fn column_value(meta: &ColumnMeta, row: &serde_json::Value) -> serde_json::Value {
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
fn value_nonempty(value: &serde_json::Value) -> bool {
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
fn row_object(meta: &TableMeta, row: &serde_json::Value) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (name, col) in &meta.columns {
        obj.insert(name.clone(), column_value(col, row));
    }
    serde_json::Value::Object(obj)
}

/// A fully blank row (all cells null or empty string) is a separator, not an
/// element — the sheets baseline's `any_value` rule.
fn row_blank(row: &serde_json::Value) -> bool {
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
fn structured_rows(
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
fn resolve_meta(
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
fn canonical_with_convention(
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

/// Whether a declared convention needs a `<tab>.meta` fetch (the `meta_sheet`
/// preset). The caller fetches it before calling `canonical_with_convention`.
fn needs_meta_fetch(convention: Option<&SheetsConvention>) -> bool {
    convention.is_some_and(|c| !c.uses_script() && c.preset == Preset::MetaSheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    fn config_with(credential_env: Option<&str>) -> RemoteSourceConfig {
        RemoteSourceConfig {
            dsn_env: None,
            credential_env: credential_env.map(String::from),
            queries: Default::default(),
        }
    }

    #[test]
    fn parse_spec_splits_id_and_tab() {
        assert_eq!(
            parse_spec("gsheet:1AbC-x_y/Levels"),
            Some(("1AbC-x_y", "Levels"))
        );
        // Tab names may carry spaces and slashes in Sheets; everything
        // after the first '/' is the tab.
        assert_eq!(
            parse_spec("gsheet:abc/Balance Sheets/Q3"),
            Some(("abc", "Balance Sheets/Q3"))
        );
        assert_eq!(parse_spec("mysql:Monsters"), None, "S2 scheme, not ours");
        assert_eq!(parse_spec("gsheet:noslash"), None);
        assert_eq!(parse_spec("gsheet:/tab"), None, "empty id");
        assert_eq!(parse_spec("gsheet:id/"), None, "empty tab");
    }

    #[test]
    fn valid_spreadsheet_id_is_strict() {
        assert!(valid_spreadsheet_id("1AbC-x_y"));
        assert!(!valid_spreadsheet_id(""));
        assert!(!valid_spreadsheet_id("../etc"), "no traversal");
        assert!(!valid_spreadsheet_id("a/b"), "no path separators");
        assert!(!valid_spreadsheet_id("a?x=1"), "no query injection");
        assert!(!valid_spreadsheet_id("a b"), "no spaces in the id");
    }

    /// Every env-touching test holds this: `std::env::set_var` must not
    /// race a concurrent `env::var` from another test thread.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolve_credential_needs_declared_and_nonempty_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let err = resolve_credential(None).unwrap_err();
        assert!(err.contains("E1904"), "{err}");
        assert!(err.contains("credential_env"), "{err}");

        let cfg = config_with(Some("CAGE_TEST_SHEETS_KEY_UNSET"));
        let err = resolve_credential(Some(&cfg)).unwrap_err();
        assert!(err.contains("E1904"), "{err}");
        assert!(err.contains("CAGE_TEST_SHEETS_KEY_UNSET"), "{err}");

        let err = resolve_credential(Some(&config_with(None))).unwrap_err();
        assert!(err.contains("E1904"), "{err}");
    }

    #[test]
    fn build_url_encodes_the_tab_and_hides_the_key_nowhere() {
        let url = build_url(API_BASE, "1AbC", "Balance Sheets", "SECRET");
        assert!(url.starts_with("https://sheets.googleapis.com/v4/spreadsheets/1AbC/values/"));
        assert!(url.contains("Balance%20Sheets"), "{url}");
        assert!(url.contains("valueRenderOption=UNFORMATTED_VALUE"), "{url}");
        assert!(url.contains("majorDimension=ROWS"), "{url}");
        // The key rides the query string by protocol necessity — but the
        // adapter's error messages never carry it (see fetch_error).
        assert!(url.ends_with("&key=SECRET"), "{url}");
    }

    #[test]
    fn fetch_error_maps_auth_and_never_leaks_the_key() {
        let msg = fetch_error(FetchFailure::Status(403), "gsheet:abc/Tab");
        assert!(msg.contains("E1902"), "{msg}");
        let msg = fetch_error(FetchFailure::Status(404), "gsheet:abc/Tab");
        assert!(msg.contains("E1901"), "{msg}");
        let msg = fetch_error(
            FetchFailure::Transport("connection refused".to_string()),
            "gsheet:abc/Tab",
        );
        assert!(msg.contains("E1901"), "{msg}");
        assert!(
            !fetch_error(FetchFailure::Status(401), "gsheet:abc/Tab").contains("SECRET_KEY"),
            "error messages refer to the spec, not the URL"
        );
    }

    fn sheet_body(values: &str) -> Vec<u8> {
        format!(r#"{{ "range": "Tab!A1:Z99", "majorDimension": "ROWS", "values": {values} }}"#)
            .into_bytes()
    }

    #[test]
    fn canonical_mapping_follows_the_excel_conventions() {
        let body = sheet_body(
            r#"[
            ["id", "name", ""],
            [1, "Sword", "x"],
            ["", "", ""],
            [2, "Shield"],
            [true, 1.5, null]
        ]"#,
        );
        let bytes = sheets_json_to_canonical("Tab", &body).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        // Blank header cell → col2 (Excel convention); cells render in
        // their initial string form; blank row skipped; ragged row
        // padded with null; natural row order preserved.
        assert_eq!(
            text,
            concat!(
                "{\n  \"Tab\": [\n    {\n      \"id\": \"1\",\n      \"name\": \"Sword\",\n",
                "      \"col2\": \"x\"\n    },\n    {\n      \"id\": \"2\",\n",
                "      \"name\": \"Shield\",\n      \"col2\": null\n    },\n",
                "    {\n      \"id\": \"true\",\n      \"name\": \"1.5\",\n",
                "      \"col2\": null\n    }\n  ]\n}"
            ),
            "numbers/bools keep their JSON text, NULL stays null, row order is the tab's"
        );
    }

    #[test]
    fn shape_gate_rejects_everything_that_is_not_a_row_set() {
        let reject = |body: &[u8]| {
            let err = sheets_json_to_canonical("Tab", body).unwrap_err();
            assert!(err.contains("E1903"), "{err}");
        };
        reject(b"{ not json");
        reject(b"[1, 2, 3]");
        reject(br#"{"majorDimension": "COLUMNS", "values": [["a"], ["1"]]}"#);
        reject(br#"{"range": "Tab!A1"}"#);

        // An empty tab carries no header — refused, not silently skipped
        reject(br#"{"values": []}"#);
        reject(br#"{"values": [[]]}"#);
    }

    #[test]
    fn load_rejects_spec_and_credential_before_any_network() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let config = ProjectConfig::default();

        // Not a Sheets spec at all
        let err = SheetsSourceAdapter::load(tmp.path(), &config, "https://x/y", false).unwrap_err();
        assert!(err.contains("E1901"), "{err}");

        // A traversal-shaped id never reaches a URL
        let err = SheetsSourceAdapter::load(tmp.path(), &config, "gsheet:../etc/passwd", false)
            .unwrap_err();
        assert!(err.contains("E1901"), "{err}");

        // Valid spec, no credential section → E1904 before any fetch
        let err =
            SheetsSourceAdapter::load(tmp.path(), &config, "gsheet:abc/Tab", false).unwrap_err();
        assert!(err.contains("E1904"), "{err}");
    }

    /// A canned-response HTTP server over std::net — the Sheets API
    /// stand-in. Answers every GET with one (status, body); stops on Drop.
    struct TestServer {
        base: String,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    fn start_server(status: u16, body: &'static str) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let head = format!(
                            "HTTP/1.1 {status} REASON\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        use std::io::Write as _;
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        TestServer {
            base: format!("http://127.0.0.1:{port}"),
            stop,
            handle: Some(handle),
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    /// A canned-response server that routes by substring of the request
    /// line (the tab path), answering the first matching route — used when
    /// one fetch pulls the main tab and the `<tab>.meta` sibling.
    fn start_server_routes(routes: Vec<(&'static str, u16, &'static str)>) -> TestServer {
        use std::io::Read as _;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 2048];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let head = String::from_utf8_lossy(&buf[..n]).to_string();
                        let (status, body) = routes
                            .iter()
                            .find(|(pattern, _, _)| head.contains(pattern))
                            .map(|(_, s, b)| (*s, *b))
                            .unwrap_or((404, r#"{"error": "no route"}"#));
                        let resp = format!(
                            "HTTP/1.1 {status} REASON\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        use std::io::Write as _;
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.write_all(body.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        TestServer {
            base: format!("http://127.0.0.1:{port}"),
            stop,
            handle: Some(handle),
        }
    }

    /// A port with nothing listening on it (bound, then released).
    fn dead_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    const VALUES_OK: &str = r#"{
        "range": "Levels!A1:C4",
        "majorDimension": "ROWS",
        "values": [
            ["id", "name", ""],
            [1, "Sword", "x"],
            [2, "Shield"]
        ]
    }"#;

    #[test]
    fn load_fetches_maps_and_caches_end_to_end() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let server = start_server(200, VALUES_OK);
        let var = "CAGE_TEST_SHEETS_KEY_E2E";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));

        let doc = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        let table = doc.tables.get("Levels").expect("Levels table");
        assert_eq!(
            table.rows.len(),
            2,
            "blank header col becomes col2, no rows dropped"
        );

        // The canonical bytes are the cache anchor — same key, same slot.
        let key_input = format!("{CONFIG_KEY}\n1AbC\nLevels");
        let cache_dir = remote::source_cache_dir(tmp.path(), &key_input);
        let cached = cache_dir.join(format!("{}.json", remote::cache_key(&key_input)));
        assert!(cached.is_file(), "cache file must exist");
        let cached_text = std::fs::read_to_string(&cached).unwrap();
        assert!(cached_text.contains("\"id\": \"1\""), "{cached_text}");
        assert!(
            cached_text.contains("\"col2\": \"x\""),
            "blank header cell fell back to col2: {cached_text}"
        );

        // Same spec again → same cache slot, still loads.
        let doc2 = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        assert_eq!(doc.tables.len(), doc2.tables.len());

        std::env::remove_var(var);
    }

    #[test]
    fn load_maps_http_and_shape_failures_to_their_codes() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let var = "CAGE_TEST_SHEETS_KEY_ERR";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));

        let spec = "gsheet:1AbC/Levels";

        // 403 (bad key / no access) → E1902
        let server = start_server(403, r#"{"error": {"message": "bad key"}}"#);
        let err =
            SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
                .unwrap_err();
        assert!(err.contains("E1902"), "{err}");
        drop(server);

        // 404 (no such spreadsheet) → E1901
        let server = start_server(404, r#"{"error": "not found"}"#);
        let err =
            SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
                .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        drop(server);

        // Unreachable endpoint → E1901 after bounded retries
        let err = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            spec,
            &format!("http://127.0.0.1:{}", dead_port()),
            false,
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");

        // A 200 body that is not a row set → E1903 (the shape gate)
        let server = start_server(200, r#"{"error": {"message": "API key not valid"}}"#);
        let err =
            SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
                .unwrap_err();
        assert!(err.contains("E1903"), "{err}");
        drop(server);

        std::env::remove_var(var);
    }

    /// S4 offline semantics: a transport failure with a previous copy in
    /// the cache slot serves that copy (the WARNING line carries the
    /// spec, never the keyed URL); `strict` refuses and keeps E1901.
    #[test]
    fn offline_transport_failure_falls_back_to_the_cached_copy() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let var = "CAGE_TEST_SHEETS_KEY_OFFLINE";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));

        let spec = "gsheet:1AbC/Levels";
        let server = start_server(200, VALUES_OK);
        let base = server.base.clone();
        let first =
            SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &base, false).unwrap();
        drop(server);

        let offline = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &base, false)
            .expect("offline fallback must serve the cached copy");
        assert_eq!(offline.tables.get("Levels").unwrap().rows.len(), 2);
        assert_eq!(
            offline.tables.get("Levels").unwrap().rows.len(),
            first.tables.get("Levels").unwrap().rows.len(),
        );

        let err = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &base, true)
            .unwrap_err();
        assert!(err.contains("E1901"), "{err}");

        std::env::remove_var(var);
    }

    // ---- design §51 Q7: the `[source.sheets]` declaration carries the same
    // convention as `[source.excel]`.

    fn conv(preset: Preset, header_row: usize) -> SheetsConvention {
        SheetsConvention {
            header_row,
            preset,
            marker: "##".to_string(),
            ..Default::default()
        }
    }

    fn set_sheets(config: &mut ProjectConfig, convention: SheetsConvention) {
        config.source = Some(cage_core::manifest::SourceConfig {
            sheets: Some(convention),
            ..Default::default()
        });
    }

    #[test]
    fn no_declaration_reads_byte_identical_to_the_baseline() {
        let body =
            br#"{"majorDimension":"ROWS","values":[["id","name"],[1,"sword"],[2,"shield"]]}"#;
        let baseline = sheets_json_to_canonical("Levels", body).unwrap();
        let config = ProjectConfig {
            source: Some(cage_core::manifest::SourceConfig {
                sheets: Some(SheetsConvention::default()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let via_adapter = conventioned_bytes(
            Path::new("/tmp"),
            &config,
            "1AbC",
            "Levels",
            body,
            "http://127.0.0.1:1",
            "k",
        )
        .unwrap();
        assert_eq!(
            baseline, via_adapter,
            "data preset with header_row 0 must match the baseline"
        );
    }

    #[test]
    fn marker_rows_decodes_declarations_above_the_header() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body = r###"{
            "range": "Levels!A1:B5",
            "majorDimension": "ROWS",
            "values": [
                ["##type Int32", "String"],
                ["##description 描述词", "武器名"],
                ["id", "name"],
                [1, "sword"],
                [2, "shield"]
            ]
        }"###;
        let server = start_server(200, body);
        let var = "CAGE_TEST_SHEETS_KEY_CONV1";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));
        set_sheets(&mut config, conv(Preset::MarkerRows, 2));

        let doc = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        let table = doc.tables.get("Levels").expect("Levels table");
        assert_eq!(table.rows.len(), 2);

        // The cache anchor holds the structured rows — the marker rows are
        // decoded, not emitted.
        let key_input = format!("{CONFIG_KEY}\n1AbC\nLevels");
        let cache_file = remote::source_cache_dir(tmp.path(), &key_input)
            .join(format!("{}.json", remote::cache_key(&key_input)));
        let text = std::fs::read_to_string(&cache_file).unwrap();
        assert!(text.contains("\"id\": \"1\""), "{text}");
        assert!(
            !text.contains("##type"),
            "marker rows must not leak into the canonical rows: {text}"
        );

        std::env::remove_var(var);
    }

    #[test]
    fn data_preset_moves_the_header_row() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body = r#"{
            "range": "Levels!A1:B4",
            "majorDimension": "ROWS",
            "values": [
                ["meta junk", ""],
                ["more junk", ""],
                ["id", "name"],
                [1, "sword"]
            ]
        }"#;
        let server = start_server(200, body);
        let var = "CAGE_TEST_SHEETS_KEY_CONV2";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));
        set_sheets(&mut config, conv(Preset::Data, 2));

        let doc = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        let table = doc.tables.get("Levels").expect("Levels table");
        assert_eq!(table.rows.len(), 1);
        let text = std::fs::read_to_string(
            remote::source_cache_dir(tmp.path(), &format!("{CONFIG_KEY}\n1AbC\nLevels")).join(
                format!(
                    "{}.json",
                    remote::cache_key(&format!("{CONFIG_KEY}\n1AbC\nLevels"))
                ),
            ),
        )
        .unwrap();
        assert!(
            !text.contains("meta junk"),
            "pre-header rows must be skipped: {text}"
        );

        std::env::remove_var(var);
    }

    #[test]
    fn var_row_multi_row_folds_rows_into_arrays() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body = r###"{
            "range": "Levels!A1:D6",
            "majorDimension": "ROWS",
            "values": [
                ["##var", "id", "*waves", "*waves"],
                ["##+", "", "atk", "def"],
                ["##type", "Int32", "Int32", "Int32"],
                ["", 1, 10, 20],
                ["", "", 11, 21],
                ["", 2, 30, 40]
            ]
        }"###;
        let server = start_server(200, body);
        let var = "CAGE_TEST_SHEETS_KEY_CONV3";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));
        set_sheets(&mut config, conv(Preset::MarkerRows, 0));

        let doc = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        let table = doc.tables.get("Levels").expect("Levels table");
        assert_eq!(table.rows.len(), 2);
        let key_input = format!("{CONFIG_KEY}\n1AbC\nLevels");
        let text = std::fs::read_to_string(
            remote::source_cache_dir(tmp.path(), &key_input)
                .join(format!("{}.json", remote::cache_key(&key_input))),
        )
        .unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let rows = doc["Levels"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{text}");
        assert_eq!(rows[0]["id"], "1");
        let waves0 = rows[0]["waves"].as_array().unwrap();
        assert_eq!(
            waves0.len(),
            2,
            "contiguous rows fold into the multi-row array: {text}"
        );
        assert_eq!(waves0[0]["atk"], "10");
        assert_eq!(waves0[0]["def"], "20");
        assert_eq!(waves0[1]["atk"], "11");
        assert_eq!(waves0[1]["def"], "21");
        let waves1 = rows[1]["waves"].as_array().unwrap();
        assert_eq!(waves1.len(), 1);
        assert_eq!(waves1[0]["atk"], "30");
        assert_eq!(rows[1]["id"], "2");

        std::env::remove_var(var);
    }

    #[test]
    fn sub_columns_assemble_nested_objects() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body = r###"{
            "range": "Levels!A1:D4",
            "majorDimension": "ROWS",
            "values": [
                ["##var", "id", "weapon", "weapon"],
                ["##+", "", "atk", "def"],
                ["##type", "Int32", "Int32", "Int32"],
                ["", 1, 10, 20]
            ]
        }"###;
        let server = start_server(200, body);
        let var = "CAGE_TEST_SHEETS_KEY_CONV4";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));
        set_sheets(&mut config, conv(Preset::MarkerRows, 0));

        let doc = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        let table = doc.tables.get("Levels").expect("Levels table");
        assert_eq!(table.rows.len(), 1);
        let key_input = format!("{CONFIG_KEY}\n1AbC\nLevels");
        let text = std::fs::read_to_string(
            remote::source_cache_dir(tmp.path(), &key_input)
                .join(format!("{}.json", remote::cache_key(&key_input))),
        )
        .unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let rows = doc["Levels"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{text}");
        assert_eq!(rows[0]["id"], "1");
        assert_eq!(
            rows[0]["weapon"]["atk"], "10",
            "sub-columns group into a nested bean: {text}"
        );
        assert_eq!(rows[0]["weapon"]["def"], "20");

        std::env::remove_var(var);
    }

    #[test]
    fn meta_sheet_fetches_the_sibling_tab() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let main = r#"{
            "range": "Levels!A1:B2",
            "majorDimension": "ROWS",
            "values": [["id", "name"], [1, "sword"]]
        }"#;
        let meta = r###"{
            "range": "Levels.meta!A1:B2",
            "majorDimension": "ROWS",
            "values": [["id", "name"], ["##type Int32", "String"]]
        }"###;
        let server = start_server_routes(vec![
            ("/values/Levels.meta?", 200, meta),
            ("/values/Levels?", 200, main),
        ]);
        let var = "CAGE_TEST_SHEETS_KEY_CONV5";
        std::env::set_var(var, "test-key");

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));
        set_sheets(&mut config, conv(Preset::MetaSheet, 0));

        let doc = SheetsSourceAdapter::load_with_base(
            tmp.path(),
            &config,
            "gsheet:1AbC/Levels",
            &server.base,
            false,
        )
        .unwrap();
        let table = doc.tables.get("Levels").expect("Levels table");
        assert_eq!(table.rows.len(), 1);
        let key_input = format!("{CONFIG_KEY}\n1AbC\nLevels");
        let text = std::fs::read_to_string(
            remote::source_cache_dir(tmp.path(), &key_input)
                .join(format!("{}.json", remote::cache_key(&key_input))),
        )
        .unwrap();
        assert!(text.contains("\"id\": \"1\""), "{text}");
        assert!(
            !text.contains("##type"),
            "the meta sheet is decoded, not emitted: {text}"
        );

        std::env::remove_var(var);
    }

    #[test]
    fn meta_sheet_missing_or_mismatched_is_e2207_or_e2201() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let var = "CAGE_TEST_SHEETS_KEY_CONV6";
        std::env::set_var(var, "test-key");
        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config
            .remote
            .insert(CONFIG_KEY.to_string(), config_with(Some(var)));
        set_sheets(&mut config, conv(Preset::MetaSheet, 0));
        let spec = "gsheet:1AbC/Levels";

        // The sibling tab is absent → E2201 (the fetch is online and fails).
        let main = r#"{"range":"Levels!A1:B2","majorDimension":"ROWS","values":[["id","name"],[1,"sword"]]}"#;
        let server = start_server_routes(vec![("/values/Levels?", 200, main)]);
        let err =
            SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
                .unwrap_err();
        assert!(err.contains("E2201"), "{err}");
        drop(server);

        // The sibling names a column the data header does not → E2207.
        let meta = r###"{"range":"Levels.meta!A1:B2","majorDimension":"ROWS","values":[["id","nope"],["##type Int32","String"]]}"###;
        let server = start_server_routes(vec![
            ("/values/Levels.meta?", 200, meta),
            ("/values/Levels?", 200, main),
        ]);
        let err =
            SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
                .unwrap_err();
        assert!(err.contains("E2207"), "{err}");

        std::env::remove_var(var);
    }

    /// The script adapter's `request.grid` carries the contract's typed
    /// cells — number / string / boolean / null — not `.text()`-stringified
    /// cells (review finding: sheets was flattening every cell to a string,
    /// turning blank cells into `""` instead of `null`).
    #[test]
    fn script_grid_keeps_typed_cells_not_strings() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        // The script echoes the grid it saw into a side file, then answers
        // the contract with two columns typed from the header.
        let script = tmp.path().join("echo_grid.py");
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import json,sys
req = json.load(sys.stdin)
with open(sys.argv[1], "w") as f:
    json.dump(req["grid"], f)
print(json.dumps({
  "contract": req["contract"],
  "table": req["sheet"],
  "header_row": 0,
  "columns": {
    "id": {"cell": "A1", "type": {"kind": "Int32"}},
    "flag": {"cell": "B1", "type": {"kind": "Bool"}},
  },
}))"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let out = tmp.path().join("grid_seen.json");

        let body = r#"{"majorDimension":"ROWS","values":[["id","flag"],[7,true],["",""]]}"#;
        let config = ProjectConfig {
            source: Some(cage_core::manifest::SourceConfig {
                sheets: Some(SheetsConvention {
                    script: Some("echo_grid.py".to_string()),
                    script_args: vec![out.display().to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let bytes = conventioned_bytes(
            tmp.path(),
            &config,
            "1AbC",
            "Levels",
            body.as_bytes(),
            "http://127.0.0.1:1",
            "k",
        )
        .unwrap();
        let seen: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(seen[1][0], serde_json::json!(7), "Int stays a number");
        assert_eq!(seen[1][1], serde_json::json!(true), "Bool stays a boolean");
        assert_eq!(
            seen[2][0],
            serde_json::Value::Null,
            "blank cell is null, not an empty string"
        );
        let _ = bytes;
    }
}
