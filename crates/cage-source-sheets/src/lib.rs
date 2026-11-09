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

use cage_core::convention::{Preset, SheetsConvention};
use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::E1901;
use cage_core::error::codes::remote::{E1902, E1903, E1904};
use cage_core::error::codes::source_convention::E2201;
use cage_core::manifest::{ProjectConfig, RemoteSourceConfig};
use cage_core::remote::{self, FetchFailure};
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
use rows::canonical_with_convention;
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
        FetchFailure::TooLarge(n) => format!("body exceeds the {n} byte cap"),
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
fn needs_meta_fetch(convention: Option<&SheetsConvention>) -> bool {
    convention.is_some_and(|c| !c.uses_script() && c.preset == Preset::MetaSheet)
}

mod rows;
#[cfg(test)]
mod tests;
