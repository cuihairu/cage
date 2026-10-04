//! HTTP API source adapter (S1, design §45): an `http(s)` URL in
//! `[source_roots]` is fetched once (bounded retries on transport flake),
//! the response bytes are materialized in the project-local cache
//! (`.cage-cache/source/<cache_key>/<cache_key>.json`), and the cached
//! file is parsed by the standard JSON adapter. The remote body follows
//! the exact same shape rules as a local `.json` source —
//! `{表名: 行数组}` / single object → `Root` / bare array → `Data` —
//! and enters the same L0-L7 pipeline. The remote is a data source, not
//! a trusted one: nothing skips validation.

use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::{E1901, E1902};
use cage_core::remote::{self, FetchFailure};
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
use std::path::{Path, PathBuf};

/// Whether a `[source_roots]` value names a remote source: an http(s)
/// URL — the same predicate the registry root uses, so a URL can never
/// mean both.
fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Fetch a remote JSON source into the project-local cache and parse it
/// into a `Document`.
pub struct HttpSourceAdapter;

impl HttpSourceAdapter {
    /// GET `url` and land the response bytes at
    /// `.cage-cache/source/<cache_key(url)>/<cache_key(url)>.json`,
    /// returning the cached file path. Errors: E1901 when the fetch
    /// fails (transport after bounded retries, 404, or any other
    /// non-auth status), E1902 on 401/403, E9902 when the cache cannot
    /// be written.
    pub fn fetch(project_root: &Path, url: &str) -> Result<PathBuf, String> {
        if !is_http_url(url) {
            return Err(format!("{E1901} not an http(s) remote source URL: {url}"));
        }
        let bytes = remote::http_get(url).map_err(|failure| match &failure {
            FetchFailure::Status(code @ (401 | 403)) => {
                format!("{E1902} remote source rejected access (HTTP {code}): {url}")
            }
            FetchFailure::NotFound(_) => {
                format!("{E1901} remote source not found (HTTP 404): {url}")
            }
            other => format!("{E1901} remote source fetch failed: {other} ({url})"),
        })?;

        // The byte anchor: whatever the server said is written verbatim
        // before anything parses it — later stages always see the same
        // file a re-run would.
        let cache_dir = remote::source_cache_dir(project_root, url);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(url)));
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
        Ok(cache_file)
    }

    /// `fetch` + parse the cached bytes with the standard JSON adapter —
    /// identical shapes, identical diagnostics (E0001 with line/column
    /// on a malformed body). Parse diagnostics render to stderr and the
    /// error carries the URL, mirroring how local source files report.
    pub fn load(project_root: &Path, url: &str) -> Result<Document, String> {
        let cache_file = Self::fetch(project_root, url)?;
        JsonSourceAdapter::parse_file(&cache_file).map_err(|diags| {
            eprintln!("{}", diags.render(false));
            format!("failed to parse remote source {url}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// Serve one canned response to every request until dropped.
    struct Server {
        port: u16,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    fn start_server(status: u16, reason: &str, body: &'static str) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut head_bytes = Vec::new();
                        let mut buf = [0u8; 1024];
                        loop {
                            match stream.read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    head_bytes.extend_from_slice(&buf[..n]);
                                    if head_bytes.windows(4).any(|w| w == b"\r\n\r\n")
                                        || head_bytes.len() > 64 * 1024
                                    {
                                        break;
                                    }
                                }
                            }
                        }
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
        Server {
            port,
            stop,
            handle: Some(handle),
        }
    }

    impl Drop for Server {
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

    const BODY: &str = r#"{
  "Item": [
    { "id": 1, "name": "Sword" },
    { "id": 2, "name": "Shield" }
  ],
  "Monster": [
    { "id": 1, "name": "Slime", "drop": 1 }
  ]
}"#;

    #[test]
    fn fetches_parses_and_caches_bytes() {
        let server = start_server(200, "OK", BODY);
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://127.0.0.1:{}/items.json", server.port);

        let doc = HttpSourceAdapter::load(tmp.path(), &url).unwrap();
        assert_eq!(doc.tables.len(), 2, "both tables load: {doc:?}");
        let item = doc.tables.get("Item").expect("Item table");
        assert_eq!(item.rows.len(), 2);
        assert!(doc.tables.contains_key("Monster"));

        // The response bytes are materialized project-locally, keyed by
        // the URL, and the parsed document records that file as source.
        let cached = remote::source_cache_dir(tmp.path(), &url)
            .join(format!("{}.json", remote::cache_key(&url)));
        assert!(cached.is_file(), "cache file must exist");
        assert_eq!(
            std::fs::read_to_string(&cached).unwrap(),
            BODY,
            "cache holds the response bytes verbatim"
        );
        assert!(doc
            .source_files
            .iter()
            .any(|f| f == &cached.display().to_string()));
    }

    #[test]
    fn http_401_maps_to_e1902() {
        let server = start_server(401, "Unauthorized", r#"{"error":"no key"}"#);
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/items.json", server.port),
        )
        .unwrap_err();
        assert!(err.contains("E1902"), "{err}");
    }

    #[test]
    fn http_404_maps_to_e1901() {
        let server = start_server(404, "Not Found", "nope");
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/items.json", server.port),
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("404"), "{err}");
    }

    #[test]
    fn dead_port_maps_to_e1901_after_bounded_retries() {
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/items.json", dead_port()),
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
    }

    #[test]
    fn malformed_body_reports_parse_error() {
        let server = start_server(200, "OK", "{ not json");
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/items.json", server.port),
        )
        .unwrap_err();
        assert!(err.contains("failed to parse"), "{err}");
    }

    #[test]
    fn non_http_url_is_rejected_without_fetching() {
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(tmp.path(), "ftp://example.com/x.json").unwrap_err();
        assert!(err.contains("E1901"), "{err}");
    }
}
