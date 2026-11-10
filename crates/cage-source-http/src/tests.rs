use super::meta::meta_path;
use super::paging::{fetch_pages_bound, next_page_link, param_is_rel_next};
use super::*;
use cage_core::remote::{FetchFailure, Validators};
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
                    // Windows accepts inherit the listener's non-blocking
                    // mode; restore blocking so the request read waits.
                    let _ = stream.set_nonblocking(false);
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
                    // Windows accepts inherit the listener's non-blocking
                    // mode; restore blocking so the request read waits.
                    let _ = stream.set_nonblocking(false);
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
                    // Windows accepts inherit the listener's non-blocking
                    // mode; restore blocking so the request read waits.
                    let _ = stream.set_nonblocking(false);
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
    let err = HttpSourceAdapter::load(tmp.path(), "ftp://example.com/x.json", false).unwrap_err();
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
                    // Windows accepts inherit the listener's non-blocking
                    // mode; restore blocking so the request read waits.
                    let _ = stream.set_nonblocking(false);
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
