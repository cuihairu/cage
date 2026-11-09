use super::rows::{cell_to_cellvalue, render_cell, resolve_meta, value_nonempty};
use super::*;
use cage_core::convention::{CellValue, Grid};
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
    let err =
        SheetsSourceAdapter::load(tmp.path(), &config, "gsheet:../etc/passwd", false).unwrap_err();
    assert!(err.contains("E1901"), "{err}");

    // Valid spec, no credential section → E1904 before any fetch
    let err = SheetsSourceAdapter::load(tmp.path(), &config, "gsheet:abc/Tab", false).unwrap_err();
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
    let err = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
        .unwrap_err();
    assert!(err.contains("E1902"), "{err}");
    drop(server);

    // 404 (no such spreadsheet) → E1901
    let server = start_server(404, r#"{"error": "not found"}"#);
    let err = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
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
    let err = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
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

    let err =
        SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &base, true).unwrap_err();
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
    let body = br#"{"majorDimension":"ROWS","values":[["id","name"],[1,"sword"],[2,"shield"]]}"#;
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
        remote::source_cache_dir(tmp.path(), &format!("{CONFIG_KEY}\n1AbC\nLevels")).join(format!(
            "{}.json",
            remote::cache_key(&format!("{CONFIG_KEY}\n1AbC\nLevels"))
        )),
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
    let main =
        r#"{"range":"Levels!A1:B2","majorDimension":"ROWS","values":[["id","name"],[1,"sword"]]}"#;
    let server = start_server_routes(vec![("/values/Levels?", 200, main)]);
    let err = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
        .unwrap_err();
    assert!(err.contains("E2201"), "{err}");
    drop(server);

    // The sibling names a column the data header does not → E2207.
    let meta = r###"{"range":"Levels.meta!A1:B2","majorDimension":"ROWS","values":[["id","nope"],["##type Int32","String"]]}"###;
    let server = start_server_routes(vec![
        ("/values/Levels.meta?", 200, meta),
        ("/values/Levels?", 200, main),
    ]);
    let err = SheetsSourceAdapter::load_with_base(tmp.path(), &config, spec, &server.base, false)
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
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
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
    let seen: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(seen[1][0], serde_json::json!(7), "Int stays a number");
    assert_eq!(seen[1][1], serde_json::json!(true), "Bool stays a boolean");
    assert_eq!(
        seen[2][0],
        serde_json::Value::Null,
        "blank cell is null, not an empty string"
    );
    let _ = bytes;
}

/// Every `FetchFailure` kind renders URL-free: the API key rides the
/// query string, so no failure text may echo the URL.
#[test]
fn failure_brief_names_every_kind_without_the_url() {
    let url = "http://127.0.0.1:1/?key=secret";
    assert_eq!(
        failure_brief(&FetchFailure::Transport(format!("dial {url}"))),
        "transport failure"
    );
    assert_eq!(
        failure_brief(&FetchFailure::RateLimited(Duration::from_secs(9))),
        "rate limited"
    );
    assert_eq!(
        failure_brief(&FetchFailure::NotFound(url.to_string())),
        "not found"
    );
    assert_eq!(failure_brief(&FetchFailure::Status(503)), "http 503");
    assert_eq!(
        failure_brief(&FetchFailure::TooLarge(1024)),
        "body exceeds the 1024 byte cap"
    );
}

/// A payload without a usable header row is not a row set: the reject
/// names the missing index, and an empty header row is blank.
#[test]
fn plain_rows_without_a_header_row_is_e1903() {
    let err = plain_rows_canonical("Levels", &[], 0).expect_err("no rows at all");
    assert!(err.contains("E1903"), "{err}");
    assert!(err.contains("no header row at index 0"), "{err}");

    let err =
        plain_rows_canonical("Levels", &[serde_json::json!([])], 0).expect_err("empty header row");
    assert!(err.contains("E1903"), "{err}");
    assert!(err.contains("blank header row"), "{err}");
}

/// The per-cell and per-value conversion helpers cover every JSON
/// variant — null, float (non-i64 number), arrays/objects falling to
/// the text form, and the non-empty probe's false/true split.
#[test]
fn cell_and_value_helpers_cover_every_variant() {
    assert_eq!(cell_to_cellvalue(&serde_json::Value::Null), CellValue::Null);
    assert_eq!(
        cell_to_cellvalue(&serde_json::json!(1.5)),
        CellValue::Float(1.5)
    );
    assert_eq!(
        cell_to_cellvalue(&serde_json::json!([1])),
        CellValue::Str("[1]".to_string())
    );
    assert_eq!(
        cell_to_cellvalue(&serde_json::json!({"a": 1})),
        CellValue::Str("{\"a\":1}".to_string())
    );
    assert_eq!(
        render_cell(&serde_json::Value::Null),
        serde_json::Value::Null
    );

    assert!(!value_nonempty(&serde_json::Value::Null));
    assert!(!value_nonempty(&serde_json::json!("")));
    assert!(!value_nonempty(&serde_json::json!([])));
    assert!(!value_nonempty(&serde_json::json!({})));
    assert!(value_nonempty(&serde_json::json!(0)));
    assert!(value_nonempty(&serde_json::json!("x")));
    assert!(value_nonempty(&serde_json::json!([1])));
    assert!(value_nonempty(&serde_json::json!({"a": 1})));
}

/// The convention meta resolver answers the data preset with the
/// default metadata and refuses the meta_sheet preset when the
/// companion meta grid was not fetched.
#[test]
fn resolve_meta_handles_data_preset_and_missing_meta_grid() {
    let grid: Grid = vec![vec![
        CellValue::Str("id".to_string()),
        CellValue::Str("name".to_string()),
    ]];
    let root = std::path::Path::new(".");
    let (meta, start) = resolve_meta(&grid, 0, &conv(Preset::Data, 0), root, "T", "S", "W", None)
        .expect("data preset resolves");
    assert_eq!(start, 1);
    assert_eq!(meta.header_row, 0);

    let err = resolve_meta(
        &grid,
        0,
        &conv(Preset::MetaSheet, 0),
        root,
        "T",
        "S",
        "W",
        None,
    )
    .expect_err("meta grid required");
    assert!(err.contains("E2201"), "{err}");
}
