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

use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::E1901;
use cage_core::error::codes::remote::{E1902, E1903, E1904};
use cage_core::manifest::{ProjectConfig, RemoteSourceConfig};
use cage_core::remote::{self, FetchFailure};
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
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

/// Validate the response shape and serialize it to canonical JSON
/// (`{tab: [row objects]}`): first row = header (blank header cells
/// fall back to `col<i>` like the Excel adapter), remaining rows =
/// records, ragged rows padded/truncated to the header width, fully
/// blank rows skipped, natural row order preserved. Everything that is
/// not a proper row set is E1903 — this is the shape gate E1903 exists
/// for.
pub fn sheets_json_to_canonical(table_name: &str, body: &[u8]) -> Result<Vec<u8>, String> {
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
    let header_row = values
        .first()
        .and_then(|row| row.as_array())
        .ok_or_else(|| reject("no values (empty tab or malformed response)"))?;
    if header_row.is_empty() {
        return Err(reject("blank header row"));
    }
    let headers: Vec<String> = header_row
        .iter()
        .enumerate()
        .map(|(idx, cell)| match cell.as_str() {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => format!("col{idx}"),
        })
        .collect();

    let mut rows = Vec::new();
    for row in values.iter().skip(1) {
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
                let bytes = sheets_json_to_canonical(tab, &body)?;
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
}
