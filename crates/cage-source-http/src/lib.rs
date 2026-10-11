//! HTTP API source adapter (S1, design §45): an `http(s)` URL in
//! `[source_roots]` is fetched once (bounded retries on transport flake,
//! rate-limit negotiation on a 429 with an honorable `Retry-After`),
//! the response bytes are materialized in the project-local cache
//! (`.cage-cache/source/<cache_key>/<cache_key>.json`), and the cached
//! file is parsed by the standard JSON adapter. The remote body follows
//! the exact same shape rules as a local `.json` source —
//! `{表名: 行数组}` / single object → `Root` / bare array → `Data` —
//! and enters the same L0-L7 pipeline. The remote is a data source, not
//! a trusted one: nothing skips validation.
//!
//! Conditional revalidation (design §45 — the document-level core of
//! 增量拉取): the `ETag` / `Last-Modified` validators the server handed
//! out with the last 200 are kept beside the cached bytes
//! (`<cache_key>.meta.json`) and sent back as `If-None-Match` /
//! `If-Modified-Since`; a 304 answer reuses the cached bytes and touches
//! nothing. Sources the server cannot revalidate just fetch plainly.
//!
//! Pagination (design §45): when the response carries an RFC 8288
//! `Link: <...>; rel="next"` header the walk follows it — every page
//! must be a JSON array of rows, and pages concatenate in link order
//! into one merged document (cached bytes = merged array; a bare array
//! parses as the `Data` table). A source without a next link keeps the
//! single-GET contract byte-for-byte. Only the first GET revalidates —
//! its validators describe the collection state; follow-up pages are
//! plain GETs, and a 304 on the first page skips the walk entirely.

use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::{E1901, E1902};
use cage_core::remote::{self, FetchFailure, Validators};
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
use std::collections::HashSet;
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
    /// GET `url` (revalidating against the cached validators first, and
    /// following `Link: rel="next"` pagination when the server
    /// advertises it) and land the merged document bytes at
    /// `.cage-cache/source/<cache_key(url)>/<cache_key(url)>.json`,
    /// returning the cached file path. A 304 revalidation answer keeps
    /// the cached bytes and meta untouched. Errors: E1901 when the fetch
    /// fails (transport after bounded retries, a rate limit past its
    /// budget, 404, any other non-auth status, or a pagination-contract
    /// violation), E1902 on 401/403, E9902 when the cache cannot be
    /// written. A transport failure with a previous copy on disk falls
    /// back to it instead (E1906 WARNING) unless `strict` (`--no-cache`)
    /// is set; everything else never falls back — a server that answered
    /// has spoken, and stale bytes must not mask that.
    pub fn fetch(project_root: &Path, url: &str, strict: bool) -> Result<PathBuf, String> {
        if !is_http_url(url) {
            return Err(format!("{E1901} not an http(s) remote source URL: {url}"));
        }
        // The cache slot is keyed by the URL alone, so it resolves
        // before any bytes move — the offline fallback needs it too.
        let cache_dir = remote::source_cache_dir(project_root, url);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(url)));
        // Revalidation needs the validators AND the bytes they describe:
        // a meta without its payload is just stale bookkeeping — fetch
        // plainly and rewrite both.
        let validators = if cache_file.is_file() {
            read_meta(&cache_dir, url)?
        } else {
            None
        };
        let (bytes, fresh) = match fetch_pages(url, validators.as_ref()) {
            Ok(PageFetch::NotModified) => {
                // The server confirmed the cached bytes are current.
                return Ok(cache_file);
            }
            Ok(PageFetch::Fresh { bytes, validators }) => (bytes, validators),
            Err(PageFailure::Http(FetchFailure::Transport(e))) => {
                if let Some(path) = remote::source_cache_fallback(&cache_file, url, strict) {
                    return Ok(path);
                }
                return Err(format!(
                    "{E1901} remote source fetch failed: transport failure: {e} ({url})"
                ));
            }
            Err(PageFailure::Http(FetchFailure::Status(code @ (401 | 403)))) => {
                return Err(format!(
                    "{E1902} remote source rejected access (HTTP {code}): {url}"
                ));
            }
            Err(PageFailure::Http(FetchFailure::NotFound(inner))) => {
                return Err(format!(
                    "{E1901} remote source not found (HTTP 404): {inner}"
                ));
            }
            Err(PageFailure::Http(e)) => {
                return Err(format!("{E1901} remote source fetch failed: {e} ({url})"));
            }
            Err(e) => {
                return Err(format!("{E1901} remote source pagination failed: {e}"));
            }
        };

        // The byte anchor: whatever the server said is written verbatim
        // before anything parses it — later stages always see the same
        // file a re-run would. (Paginated sources materialize the merged
        // document, itself deterministic for a given server state.)
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
        write_meta(&cache_dir, url, &fresh)?;
        Ok(cache_file)
    }

    /// `fetch` + parse the cached bytes with the standard JSON adapter —
    /// identical shapes, identical diagnostics (E0001 with line/column
    /// on a malformed body). Parse diagnostics render to stderr and the
    /// error carries the URL, mirroring how local source files report.
    pub fn load(project_root: &Path, url: &str, strict: bool) -> Result<Document, String> {
        let cache_file = Self::fetch(project_root, url, strict)?;
        JsonSourceAdapter::parse_file(&cache_file).map_err(|diags| {
            eprintln!("{}", diags.render(false));
            format!("failed to parse remote source {url}")
        })
    }
}

/// Hard bound on the pagination walk: a server must not turn one source
/// into an unbounded request loop. 1000 pages is far past any real
/// config row set and still bounded work for a build.
const MAX_PAGES: usize = 1000;

/// One failure of the paginated fetch. `Http` wraps the shared failure
/// kinds verbatim; the rest are pagination-contract violations — the
/// server spoke, so they are definitive and never offline-fallback
/// material.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PageFailure {
    Http(FetchFailure),
    /// A page body is not valid JSON or not a JSON array
    NotArray(String),
    /// A next link that cannot resolve against its page URL
    BadLink(String),
    /// A resolved page URL the walk already fetched
    Cycle(String),
    /// More than the walked bound: the bound, then the page URL
    TooManyPages(usize, String),
}

impl std::fmt::Display for PageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PageFailure::Http(e) => write!(f, "{e}"),
            PageFailure::NotArray(url) => {
                write!(f, "pagination page is not a JSON array: {url}")
            }
            PageFailure::BadLink(url) => {
                write!(f, "pagination next link is not a usable URL: {url}")
            }
            PageFailure::Cycle(url) => write!(f, "pagination cycle at: {url}"),
            PageFailure::TooManyPages(bound, url) => {
                write!(f, "pagination exceeded {bound} pages at: {url}")
            }
        }
    }
}

/// One paginated fetch outcome. `NotModified` only ever comes from the
/// first GET (the revalidation round trip); `Fresh` carries the merged
/// bytes and the first response's validators for the cache meta.
#[derive(Debug, PartialEq, Eq)]
enum PageFetch {
    NotModified,
    Fresh {
        bytes: Vec<u8>,
        validators: Validators,
    },
}

/// Fetch `first` — revalidating against `validators` when the cache has
/// them — and, when the server advertises an RFC 8288 `Link: rel="next"`,
/// walk the pages: every page is a JSON array of rows, relative next
/// targets resolve against their page URL (RFC 3986), pages concatenate
/// in link order, and the merged array is the returned document. A 304
/// on the first GET is [`PageFetch::NotModified`] — the walk is skipped
/// entirely, the cached merged document stays authoritative. Follow-up
/// pages are plain GETs: the first page's validators describe the
/// collection state, so a fresh first page already means new work. No
/// next link on the first response returns the body bytes verbatim — the
/// single-GET contract is untouched.
fn fetch_pages(first: &str, validators: Option<&Validators>) -> Result<PageFetch, PageFailure> {
    fetch_pages_bound(first, validators, MAX_PAGES)
}

fn fetch_pages_bound(
    first: &str,
    validators: Option<&Validators>,
    max_pages: usize,
) -> Result<PageFetch, PageFailure> {
    let first_url = url::Url::parse(first).map_err(|_| PageFailure::BadLink(first.to_string()))?;
    let response = match validators {
        Some(v) => match remote::http_get_conditional(first, v).map_err(PageFailure::Http)? {
            remote::ConditionalGet::NotModified => return Ok(PageFetch::NotModified),
            remote::ConditionalGet::Fresh(response) => response,
        },
        None => remote::http_get_full(first).map_err(PageFailure::Http)?,
    };
    let fresh_validators = Validators {
        etag: response.etag.clone(),
        last_modified: response.last_modified.clone(),
    };
    let Some(next_raw) = response.link.as_deref().and_then(next_page_link) else {
        return Ok(PageFetch::Fresh {
            bytes: response.body,
            validators: fresh_validators,
        });
    };

    let mut merged = page_array(&response.body, first)?;
    let mut visited: HashSet<String> = HashSet::from([first_url.to_string()]);
    let mut current = first_url;
    let mut next = Some(next_raw);
    while let Some(raw) = next {
        current = current
            .join(&raw)
            .map_err(|_| PageFailure::BadLink(raw.clone()))?;
        let key = current.to_string();
        if !visited.insert(key.clone()) {
            return Err(PageFailure::Cycle(key));
        }
        if visited.len() > max_pages {
            return Err(PageFailure::TooManyPages(max_pages, key));
        }
        let response = remote::http_get_full(current.as_str()).map_err(PageFailure::Http)?;
        merged.extend(page_array(&response.body, current.as_str())?);
        next = response.link.as_deref().and_then(next_page_link);
    }
    Ok(PageFetch::Fresh {
        bytes: serde_json::to_vec(&merged).expect("serializing merged row arrays cannot fail"),
        validators: fresh_validators,
    })
}

/// The validators sidecar for one cache slot: `<key>.meta.json` beside
/// the payload bytes. `None` when absent — revalidation is best-effort,
/// a missing meta just means a plain fetch.
fn read_meta(cache_dir: &Path, url: &str) -> Result<Option<Validators>, String> {
    let meta_file = meta_path(cache_dir, url);
    let raw = match std::fs::read(&meta_file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "{E9902} cannot read source cache meta {}: {e}",
                meta_file.display()
            ))
        }
    };
    let meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| format!("{E9902} cannot parse {}: {e}", meta_file.display()))?;
    let etag = meta
        .get("etag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let last_modified = meta
        .get("last_modified")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Ok(Some(Validators {
        etag,
        last_modified,
    }))
}

/// Store the validators a 200 response handed out — written even when
/// the server sent neither header (an empty meta keeps the sidecar
/// honest: the next fetch sees "no validators" instead of a stale pair).
fn write_meta(cache_dir: &Path, url: &str, validators: &Validators) -> Result<(), String> {
    let meta_file = meta_path(cache_dir, url);
    let raw = serde_json::to_vec(&serde_json::json!({
        "etag": validators.etag,
        "last_modified": validators.last_modified,
    }))
    .expect("serializing the validators meta cannot fail");
    std::fs::write(&meta_file, raw).map_err(|e| {
        format!(
            "{E9902} cannot write source cache meta {}: {e}",
            meta_file.display()
        )
    })
}

fn meta_path(cache_dir: &Path, url: &str) -> PathBuf {
    cache_dir.join(format!("{}.meta.json", remote::cache_key(url)))
}

/// A page body must be a JSON array of rows — the only shape that
/// merges across pages. Anything else (object, scalar, garbage) is a
/// contract violation naming the page.
fn page_array(body: &[u8], page_url: &str) -> Result<Vec<serde_json::Value>, PageFailure> {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Array(rows)) => Ok(rows),
        _ => Err(PageFailure::NotArray(page_url.to_string())),
    }
}

/// The `rel="next"` target of one `Link` header value (RFC 8288):
/// members are `<uri-reference>; key=value` separated by top-level
/// commas (commas inside the angle brackets never split), and `rel` is
/// a case-insensitive space-separated token list. Only the target of
/// the first `next` member wins.
fn next_page_link(header: &str) -> Option<String> {
    for member in split_link_members(header) {
        let open = member.find('<')?;
        let close = open + member[open..].find('>')?;
        let target = member[open + 1..close].trim();
        if member[close + 1..].split(';').any(param_is_rel_next) {
            return Some(target.to_string());
        }
    }
    None
}

/// `true` when one `;`-parameter is a `rel` whose token list contains
/// `next` (case-insensitive, quotes stripped).
fn param_is_rel_next(param: &str) -> bool {
    let Some(rest) = param.trim().strip_prefix("rel") else {
        return false;
    };
    let Some(value) = rest.trim().strip_prefix('=') else {
        return false;
    };
    value
        .trim()
        .trim_matches('"')
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("next"))
}

/// Split a `Link` header into members at top-level commas — commas
/// inside `<...>` targets never split.
fn split_link_members(header: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, c) in header.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                members.push(&header[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    members.push(&header[start..]);
    members
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// Drain one request head: stop at the `\r\n\r\n` terminator, at
    /// EOF/error, past a 64 KiB head, or on the 2 s read timeout — raw
    /// misbehaving clients must not wedge the server thread.
    fn read_request_head(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    head.extend_from_slice(&buf[..n]);
                    if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 64 * 1024 {
                        break;
                    }
                }
            }
        }
        head
    }

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
                        let _ = read_request_head(&mut stream);
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

    /// Serve per-path canned responses (the pagination stand-in): each
    /// route is (path with query, status, `Link` header, body); a miss
    /// answers 404.
    struct RouteServer {
        port: u16,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    fn start_route_server(
        routes: Vec<(&'static str, u16, Option<&'static str>, &'static str)>,
    ) -> RouteServer {
        let table: std::collections::HashMap<String, (u16, Option<&'static str>, &'static str)> =
            routes
                .into_iter()
                .map(|(path, status, link, body)| (path.to_string(), (status, link, body)))
                .collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let head = read_request_head(&mut stream);
                        let request = String::from_utf8_lossy(&head);
                        let path = request.split_whitespace().nth(1).unwrap_or("/");
                        let (status, link, body) =
                            table.get(path).copied().unwrap_or((404, None, "no route"));
                        let mut head_out =
                            format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n");
                        if let Some(link) = link {
                            head_out.push_str(&format!("Link: {link}\r\n"));
                        }
                        head_out.push_str(&format!(
                            "Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        ));
                        let _ = stream.write_all(head_out.as_bytes());
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
        RouteServer {
            port,
            stop,
            handle: Some(handle),
        }
    }

    impl Drop for RouteServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    /// The first `throttled` requests answer 429 with the given
    /// `Retry-After`, everything after answers 200 with `body`. The
    /// request counter is exposed for asserting the retries happened.
    struct ThrottleServer {
        port: u16,
        hits: Arc<AtomicU32>,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    fn start_throttle_server(
        throttled: u32,
        retry_after: Option<&'static str>,
        body: &'static str,
    ) -> ThrottleServer {
        let hits = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let hits2 = hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = read_request_head(&mut stream);
                        let n = hits2.fetch_add(1, Ordering::SeqCst) + 1;
                        let (status, extra) = if n <= throttled {
                            (
                                429,
                                retry_after
                                    .map(|v| format!("Retry-After: {v}\r\n"))
                                    .unwrap_or_default(),
                            )
                        } else {
                            (200, String::new())
                        };
                        let head_out = format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                             {extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head_out.as_bytes());
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
        ThrottleServer {
            port,
            hits,
            stop,
            handle: Some(handle),
        }
    }

    impl Drop for ThrottleServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
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

    /// A rotated representation: same shape, one extra row — the bytes differ
    /// from [`BODY`] so a 200 refresh is observable on disk.
    const BODY_V2: &str = r#"{
  "Item": [
    { "id": 1, "name": "Sword" },
    { "id": 2, "name": "Shield" },
    { "id": 3, "name": "Potion" }
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

        let doc = HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
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
            false,
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
            false,
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
            false,
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
            false,
        )
        .unwrap_err();
        assert!(err.contains("failed to parse"), "{err}");
    }

    #[test]
    fn non_http_url_is_rejected_without_fetching() {
        let tmp = tempfile::tempdir().unwrap();
        let err =
            HttpSourceAdapter::load(tmp.path(), "ftp://example.com/x.json", false).unwrap_err();
        assert!(err.contains("E1901"), "{err}");
    }

    /// S4 offline semantics: a transport failure with a previous copy in
    /// the cache slot serves that copy (the WARNING line goes to stderr,
    /// unasserted here), and `strict` turns the same scenario back into
    /// the plain transport error.
    #[test]
    fn offline_build_falls_back_to_the_cached_copy_strict_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let server = start_server(200, "OK", BODY);
        let url = format!("http://127.0.0.1:{}/items.json", server.port);
        let first = HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        drop(server);

        let offline = HttpSourceAdapter::load(tmp.path(), &url, false)
            .expect("offline fallback must serve the cached copy");
        assert_eq!(offline.tables.len(), first.tables.len());
        assert_eq!(
            offline.tables.get("Item").unwrap().rows.len(),
            first.tables.get("Item").unwrap().rows.len(),
            "cached bytes produce the same document"
        );

        let err = HttpSourceAdapter::load(tmp.path(), &url, true).unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("transport failure"), "{err}");
    }

    /// A definitive server answer — deleted source (404) or revoked
    /// access (401) — never falls back, cache present or not: stale
    /// bytes must not mask those.
    #[test]
    fn deleted_source_and_revoked_access_never_fall_back() {
        let tmp = tempfile::tempdir().unwrap();
        let server = start_server(200, "OK", BODY);
        let url = format!("http://127.0.0.1:{}/items.json", server.port);
        HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        drop(server);

        let gone = start_server(404, "Not Found", "nope");
        let gone_url = format!("http://127.0.0.1:{}/items.json", gone.port);
        let cache_file = remote::source_cache_dir(tmp.path(), &gone_url)
            .join(format!("{}.json", remote::cache_key(&gone_url)));
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, BODY).unwrap();

        let err = HttpSourceAdapter::load(tmp.path(), &gone_url, false).unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("404"), "{err}");
        drop(gone);

        let forbidden = start_server(401, "Unauthorized", r#"{"error":"no key"}"#);
        let forbidden_url = format!("http://127.0.0.1:{}/items.json", forbidden.port);
        let cache_file = remote::source_cache_dir(tmp.path(), &forbidden_url)
            .join(format!("{}.json", remote::cache_key(&forbidden_url)));
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, BODY).unwrap();

        let err = HttpSourceAdapter::load(tmp.path(), &forbidden_url, false).unwrap_err();
        assert!(err.contains("E1902"), "{err}");
    }

    /// RFC 8288 pagination: a `Link: rel="next"` header walks the pages,
    /// relative targets resolve against their page URL, and pages
    /// concatenate in link order — the cache holds the merged array.
    #[test]
    fn paginated_pages_merge_in_link_order() {
        let server = start_route_server(vec![
            (
                "/items.json",
                200,
                Some("</items.json?page=2>; rel=\"next\""),
                r#"[{ "id": 1, "name": "Sword" }]"#,
            ),
            (
                "/items.json?page=2",
                200,
                None,
                r#"[{ "id": 2, "name": "Shield" }]"#,
            ),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://127.0.0.1:{}/items.json", server.port);

        let doc = HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        let data = doc.tables.get("Data").expect("bare array parses as Data");
        assert_eq!(data.rows.len(), 2, "both pages merged: {doc:?}");

        let cached = std::fs::read_to_string(
            remote::source_cache_dir(tmp.path(), &url)
                .join(format!("{}.json", remote::cache_key(&url))),
        )
        .unwrap();
        let merged: serde_json::Value = serde_json::from_str(&cached).unwrap();
        assert_eq!(
            merged,
            serde_json::json!([
                { "id": 1, "name": "Sword" },
                { "id": 2, "name": "Shield" }
            ]),
            "cache holds the pages merged in link order"
        );
    }

    /// The pagination contract is arrays only — an object (or any
    /// non-array) page anywhere in the walk is a definitive E1901.
    #[test]
    fn paginated_page_must_be_an_array() {
        let server = start_route_server(vec![
            (
                "/p.json",
                200,
                Some("</p.json?page=2>; rel=\"next\""),
                r#"[{ "id": 1 }]"#,
            ),
            ("/p.json?page=2", 200, None, r#"{ "rows": [] }"#),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/p.json", server.port),
            false,
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("not a JSON array"), "{err}");
        assert!(err.contains("page=2"), "the offending page is named: {err}");
    }

    /// A next link pointing back at a fetched page is a broken server,
    /// not a request loop: one warning-free definitive error.
    #[test]
    fn pagination_cycle_is_definitive() {
        let server = start_route_server(vec![(
            "/c.json",
            200,
            Some("</c.json>; rel=\"next\""),
            "[1]",
        )]);
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/c.json", server.port),
            false,
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("cycle"), "{err}");
    }

    /// The page bound is a hard loop guard: a longer chain stops with a
    /// definitive error instead of walking forever.
    #[test]
    fn pagination_page_bound_is_enforced() {
        let server = start_route_server(vec![
            (
                "/b.json",
                200,
                Some("</b.json?page=2>; rel=\"next\""),
                "[1]",
            ),
            (
                "/b.json?page=2",
                200,
                Some("</b.json?page=3>; rel=\"next\""),
                "[2]",
            ),
            ("/b.json?page=3", 200, None, "[3]"),
        ]);
        let first = format!("http://127.0.0.1:{}/b.json", server.port);
        let err = fetch_pages_bound(&first, None, 2).unwrap_err();
        assert!(
            matches!(err, PageFailure::TooManyPages(2, _)),
            "bound names itself: {err}"
        );

        // The same chain inside the bound merges fine.
        let fresh = fetch_pages_bound(&first, None, 3).unwrap();
        let PageFetch::Fresh { bytes, .. } = fresh else {
            panic!("a plain walk without validators returns Fresh")
        };
        assert_eq!(bytes, b"[1,2,3]");
    }

    /// A 429 with an honorable `Retry-After` is retried and the build
    /// succeeds once the server unthrottles.
    #[test]
    fn rate_limit_with_retry_after_is_retried() {
        let server = start_throttle_server(2, Some("0"), "[{ \"id\": 1 }]");
        let tmp = tempfile::tempdir().unwrap();
        let doc = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/i.json", server.port),
            false,
        )
        .expect("the rate limit clears within the retry budget");
        assert_eq!(doc.tables.get("Data").unwrap().rows.len(), 1);
        assert!(
            server.hits.load(Ordering::SeqCst) >= 3,
            "two throttled answers then the success"
        );
    }

    /// `Retry-After` past the honored cap is a definitive 429: no retry,
    /// no stall — a build must not wait out an hour-long throttle.
    #[test]
    fn rate_limit_beyond_cap_is_definitive_and_fast() {
        let started = std::time::Instant::now();
        let server = start_throttle_server(10, Some("3600"), "[]");
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/i.json", server.port),
            false,
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("429"), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the long wait is refused, not slept out"
        );
        assert_eq!(
            server.hits.load(Ordering::SeqCst),
            1,
            "a refused wait is definitive — no retry"
        );
    }

    /// A 429 without `Retry-After` is never guessed at: definitive.
    #[test]
    fn rate_limit_without_header_is_definitive() {
        let started = std::time::Instant::now();
        let server = start_throttle_server(10, None, "[]");
        let tmp = tempfile::tempdir().unwrap();
        let err = HttpSourceAdapter::load(
            tmp.path(),
            &format!("http://127.0.0.1:{}/i.json", server.port),
            false,
        )
        .unwrap_err();
        assert!(err.contains("E1901"), "{err}");
        assert!(err.contains("429"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn next_page_link_parses_rfc8288_forms() {
        let next = next_page_link("<https://a/2>; rel=\"next\"").unwrap();
        assert_eq!(next, "https://a/2");
        assert_eq!(
            next_page_link("</2>; rel=next").unwrap(),
            "/2",
            "unquoted rel"
        );
        assert_eq!(
            next_page_link("<https://a/2>; rel=\"prev\", </3>; rel=\"next\"").unwrap(),
            "/3",
            "the next member among several"
        );
        assert_eq!(
            next_page_link("</2>; rel=\"prev next\"").unwrap(),
            "/2",
            "rel is a token list"
        );
        assert_eq!(
            next_page_link("</2>; rel=\"NEXT\"").unwrap(),
            "/2",
            "rel tokens are case-insensitive"
        );
        assert_eq!(
            next_page_link("<https://a/2,3>; rel=\"next\"").unwrap(),
            "https://a/2,3",
            "commas inside the target never split members"
        );
        assert_eq!(next_page_link("</2>; rel=\"prev\""), None);
        assert_eq!(next_page_link("no links here"), None);
    }
    /// Serve one mutable `(etag, body)` representation: requests carrying
    /// a matching `If-None-Match` are answered 304 (no body), everything
    /// else 200 with an `ETag` header. Every request's conditional header
    /// is recorded for asserting the revalidation round trip.
    struct RevalidateServer {
        port: u16,
        hits: Arc<AtomicU32>,
        seen_inm: Arc<std::sync::Mutex<Vec<Option<String>>>>,
        current: Arc<std::sync::Mutex<(String, String)>>,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    fn start_revalidate_server(etag: &str, body: &str) -> RevalidateServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicU32::new(0));
        let seen_inm = Arc::new(std::sync::Mutex::new(Vec::new()));
        let current = Arc::new(std::sync::Mutex::new((etag.to_string(), body.to_string())));
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let hits2 = hits.clone();
        let seen2 = seen_inm.clone();
        let current2 = current.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let head = read_request_head(&mut stream);
                        let request = String::from_utf8_lossy(&head);
                        let inm = request
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("If-None-Match")
                                    .then(|| v.trim().to_string())
                            })
                            .filter(|v| !v.is_empty());
                        let carried = inm.as_deref().map(str::to_string);
                        seen2.lock().unwrap().push(carried);
                        hits2.fetch_add(1, Ordering::SeqCst);
                        let guard = current2.lock().unwrap();
                        let (etag, body) = (guard.0.as_str(), guard.1.as_str());
                        if inm.as_deref() == Some(etag) {
                            let head_out = format!(
                                "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\n\
                                 Connection: close\r\n\r\n"
                            );
                            let _ = stream.write_all(head_out.as_bytes());
                        } else {
                            let head_out = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                                 ETag: {etag}\r\nContent-Length: {}\r\n\
                                 Connection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(head_out.as_bytes());
                            let _ = stream.write_all(body.as_bytes());
                        }
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        RevalidateServer {
            port,
            hits,
            seen_inm,
            current,
            stop,
            handle: Some(handle),
        }
    }

    impl RevalidateServer {
        fn rotate(&self, etag: &str, body: &str) {
            *self.current.lock().unwrap() = (etag.to_string(), body.to_string());
        }
        fn inm_history(&self) -> Vec<Option<String>> {
            self.seen_inm.lock().unwrap().clone()
        }
    }

    impl Drop for RevalidateServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    /// RFC 7232 revalidation: the first fetch is plain and stores the
    /// server's validators; the second sends `If-None-Match` and the 304
    /// reuses the cached bytes untouched; a rotated representation comes
    /// back 200 with new validators; `strict` (`--no-cache`) still
    /// revalidates — a 304 is the server confirming the cache, not the
    /// cache answering for itself.
    #[test]
    fn conditional_get_revalidates_and_refreshes() {
        let server = start_revalidate_server("v1", BODY);
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://127.0.0.1:{}/items.json", server.port);
        let cache_dir = remote::source_cache_dir(tmp.path(), &url);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(&url)));
        let meta_file = cache_dir.join(format!("{}.meta.json", remote::cache_key(&url)));

        let first = HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        assert_eq!(
            server.inm_history()[0],
            None,
            "the first fetch has nothing to revalidate"
        );
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_file).unwrap()).unwrap();
        assert_eq!(meta["etag"], "v1", "validators stored with the bytes");

        let cached_before = std::fs::read(&cache_file).unwrap();
        let second = HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        assert_eq!(
            server.inm_history()[1].as_deref(),
            Some("v1"),
            "the second fetch revalidates with the stored ETag"
        );
        assert_eq!(server.hits.load(Ordering::SeqCst), 2);
        assert_eq!(
            std::fs::read(&cache_file).unwrap(),
            cached_before,
            "a 304 touches nothing"
        );
        assert_eq!(
            second.tables.get("Item").unwrap().rows.len(),
            first.tables.get("Item").unwrap().rows.len(),
            "the cached bytes still parse to the same document"
        );

        server.rotate("v2", BODY_V2);
        HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        assert_eq!(
            server.inm_history()[2].as_deref(),
            Some("v1"),
            "the rotation is still asked with the old validator"
        );
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_file).unwrap()).unwrap();
        assert_eq!(meta["etag"], "v2", "fresh validators replace the old");
        assert_ne!(
            std::fs::read(&cache_file).unwrap(),
            cached_before,
            "a 200 refresh lands the new bytes"
        );

        let third = HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        assert_eq!(
            server.inm_history()[3].as_deref(),
            Some("v2"),
            "the next fetch revalidates with the fresh ETag"
        );

        let strict = HttpSourceAdapter::load(tmp.path(), &url, true).unwrap();
        assert_eq!(
            strict.tables.get("Item").unwrap().rows.len(),
            third.tables.get("Item").unwrap().rows.len(),
            "--no-cache still revalidates: the 304 is a server answer"
        );
        assert_eq!(server.hits.load(Ordering::SeqCst), 5);
    }

    /// Revalidation needs the validators AND the bytes they describe:
    /// a meta whose payload is gone is stale bookkeeping — the next
    /// fetch goes out plain and rebuilds both.
    #[test]
    fn meta_without_payload_is_ignored_and_refetched() {
        let server = start_revalidate_server("v1", BODY);
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://127.0.0.1:{}/items.json", server.port);
        let cache_dir = remote::source_cache_dir(tmp.path(), &url);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(&url)));

        HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        std::fs::remove_file(&cache_file).unwrap();
        HttpSourceAdapter::load(tmp.path(), &url, false).unwrap();
        assert_eq!(
            server.inm_history()[1],
            None,
            "no payload behind the meta — plain fetch, no validator sent"
        );
        assert!(cache_file.is_file(), "the payload is rebuilt");
        assert_eq!(server.hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn page_failure_display_texts() {
        // The HTTP arm delegates to the inner fetch failure; the
        // pagination-contract arms carry their URLs (and the bound)
        // verbatim.
        let e = PageFailure::Http(FetchFailure::NotFound("http://x/y".to_string()));
        assert_eq!(e.to_string(), "not found: http://x/y");
        assert_eq!(
            PageFailure::BadLink("nope".to_string()).to_string(),
            "pagination next link is not a usable URL: nope"
        );
        assert_eq!(
            PageFailure::TooManyPages(8, "http://x/p9".to_string()).to_string(),
            "pagination exceeded 8 pages at: http://x/p9"
        );
    }

    #[test]
    fn rel_param_only_matches_rel_keys() {
        // A parameter keyed other than `rel` never carries next.
        assert!(!param_is_rel_next("href=<http://x/2>"));
    }

    #[test]
    fn read_meta_unreadable_meta_reports_e9902() {
        // A meta path occupied by a directory reads as a hard error, not
        // the benign NotFound miss.
        let tmp = tempfile::tempdir().unwrap();
        let meta = meta_path(tmp.path(), "http://x/a.json");
        std::fs::create_dir_all(&meta).unwrap();
        let err = read_meta(tmp.path(), "http://x/a.json").unwrap_err();
        assert!(
            err.contains("E9902") && err.contains("cannot read source cache meta"),
            "{err}"
        );
    }

    #[test]
    fn write_meta_occupied_meta_reports_e9902() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = meta_path(tmp.path(), "http://x/a.json");
        std::fs::create_dir_all(&meta).unwrap();
        let err = write_meta(
            tmp.path(),
            "http://x/a.json",
            &Validators {
                etag: None,
                last_modified: None,
            },
        )
        .unwrap_err();
        assert!(
            err.contains("E9902") && err.contains("cannot write source cache meta"),
            "{err}"
        );
    }

    #[test]
    fn cache_dir_occupied_by_file_reports_e9902() {
        // The fetch answers fine, but the cache slot path is a file —
        // create_dir_all fails after the server has spoken.
        let server = start_server(200, "OK", "[]");
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://127.0.0.1:{}/items.json", server.port);
        let cache_dir = remote::source_cache_dir(tmp.path(), &url);
        std::fs::create_dir_all(cache_dir.parent().unwrap()).unwrap();
        std::fs::write(&cache_dir, b"not a dir").unwrap();
        let err = HttpSourceAdapter::fetch(tmp.path(), &url, false).unwrap_err();
        assert!(
            err.contains("E9902") && err.contains("cannot create source cache"),
            "{err}"
        );
    }

    #[test]
    fn cache_file_occupied_by_dir_reports_e9902() {
        let server = start_server(200, "OK", "[]");
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("http://127.0.0.1:{}/items.json", server.port);
        let cache_dir = remote::source_cache_dir(tmp.path(), &url);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(&url)));
        std::fs::create_dir_all(&cache_file).unwrap();
        let err = HttpSourceAdapter::fetch(tmp.path(), &url, false).unwrap_err();
        assert!(
            err.contains("E9902") && err.contains("cannot write source cache"),
            "{err}"
        );
    }

    #[test]
    fn server_tolerates_misbehaving_clients() {
        // A truncated head ends the read loop at EOF; a >64 KiB head
        // without terminator trips the size guard — the canned reply
        // still lands and the server thread survives both.
        for junk in [
            b"GET /half HTTP/1.1\r\n".to_vec(),
            vec![b'a'; 64 * 1024 + 1],
        ] {
            let server = start_server(200, "OK", "[]");
            let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
            c.write_all(&junk).unwrap();
            let _ = c.shutdown(std::net::Shutdown::Write);
            let mut reply = Vec::new();
            let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = c.read_to_end(&mut reply);
            assert!(
                reply.starts_with(b"HTTP/1.1 200"),
                "misbehaving client still gets the canned reply: {reply:?}"
            );
            drop(c);
            drop(server);
        }
    }
}
