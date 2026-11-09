//! `cage web` — local HTTP service for the Schema editor (third phase W2)
//! and the project status face (U1).
//!
//! A local tool: binds `127.0.0.1` only, no authentication. The API
//! exchanges the same documents as [`cage_core::edit`] — the frontend
//! edits the canonical editor JSON, the server validates it and writes
//! schemas back as canonical YAML. U1 adds the status face: validation
//! through the same `validate_project` entry `check` and the LSP use,
//! per-profile build freshness (the layer-1 comparison `build_project`
//! makes, reported instead of acted on), and a build trigger:
//!
//! ```text
//! GET  /api/schema      → editor document of the merged schema (+ project);
//!                         ?profile= adds the projected (filtered) schema
//! POST /api/validate    → E1701 / E1004 diagnostics for an edited document
//! POST /api/schema      → save the canonical YAML (single-file schema only)
//! GET  /api/status      → validation panel + per-profile build facts
//! POST /api/build       → run the build pipeline for one profile
//! GET  /api/artifacts   → the build manifest as the artifact ledger
//! GET  /api/artifact    → serve one ledgered artifact (hash-verified)
//! GET  /api/snapshots   → packed snapshots, each freshly verified
//! POST /api/upload      → replace one table's registered source file
//!                         (validated post-swap; original restored on refusal)
//! POST /api/snapshot    → pack one profile (the `cage snapshot` pipeline)
//! GET  /api/diff        → layered diff between two build manifests
//! GET  /api/registry    → the local registry index ([registry] in config)
//! GET  /                → the Schema editor page (W3, embedded below)
//! GET  /app.js, /app.css→ editor assets
//! GET  /status/         → the project status page (U1, embedded below)
//! ```
//!
//! The frontend never parses or renders YAML — `cage_core::edit` does the
//! YAML↔JSON mapping, the server only moves bytes.
//!
//! Both pages are compiled into the binary from the `docs/public/` sources
//! (the single authoring copy; the docs site serves them too), so the
//! shipped `cage` binary carries its UI — no Node toolchain.

use crate::load_project_config;
use cage_core::manifest::ProjectConfig;
use std::path::{Path, PathBuf};

/// W3 editor page and assets, embedded at compile time from the docs-site
/// copy of the sources (include! resolves relative to this file).
const EDITOR_INDEX: &str = include_str!("../../../../docs/public/editor/index.html");
const EDITOR_APP_JS: &str = include_str!("../../../../docs/public/editor/app.js");
const EDITOR_APP_CSS: &str = include_str!("../../../../docs/public/editor/app.css");
// The editor entry (app.js) is an ES module that imports its split parts;
// each is embedded and served the same way (LAY-027).
const EDITOR_CONSTANTS_JS: &str = include_str!("../../../../docs/public/editor/constants.js");
const EDITOR_STATE_JS: &str = include_str!("../../../../docs/public/editor/state.js");
const EDITOR_DOM_JS: &str = include_str!("../../../../docs/public/editor/dom.js");
const EDITOR_API_JS: &str = include_str!("../../../../docs/public/editor/api.js");
const EDITOR_TYPES_JS: &str = include_str!("../../../../docs/public/editor/types.js");
const EDITOR_FORMS_JS: &str = include_str!("../../../../docs/public/editor/forms.js");

/// U1 status page and assets, embedded the same way.
const STATUS_INDEX: &str = include_str!("../../../../docs/public/status/index.html");
const STATUS_APP_JS: &str = include_str!("../../../../docs/public/status/app.js");
const STATUS_APP_CSS: &str = include_str!("../../../../docs/public/status/app.css");

/// Serve the editor API for a project root until killed (Ctrl+C).
pub fn run_web(root: &Path, port: u16) -> Result<(), String> {
    let config = load_project_config(root)?;
    let server = tiny_http::Server::http(("127.0.0.1", port))
        .map_err(|e| format!("cannot bind 127.0.0.1:{port}: {e}"))?;
    let addr = server
        .server_addr()
        .to_ip()
        .expect("bound to an ip address");
    println!("cage web: http://{addr} serving {}", root.display());
    println!("cage web: Ctrl+C to stop");

    let ctx = Ctx {
        root: root.to_path_buf(),
        config,
    };
    for request in server.incoming_requests() {
        handle(request, &ctx);
    }
    Ok(())
}

/// Project snapshot the API reads from (schema is reloaded per request, so
/// edits saved through the API are visible on the next GET).
pub(crate) struct Ctx {
    root: PathBuf,
    config: ProjectConfig,
}

/// Request-body cap: the editor API carries schema/row JSON whose real
/// sizes stay far below this. Bounds the memory one connection can force
/// onto the single-threaded serve loop (review 2026-10-18 表一#2).
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
/// Wall-clock budget for receiving a body: a client dripping bytes forever
/// must not pin the serve loop. Checked between reads; the `tiny_http`
/// connection-level idle timeout also bounds a single stalled read to 1 second.
const MAX_BODY_SECS: u64 = 30;

/// Loopback hostnames a legitimate local request may name (unbracketed
/// forms — `host_hostname` strips the brackets). Port is deliberately not
/// matched: the rebinding attack swaps the hostname, and the client picks
/// whichever port it knocks on anyway.
const LOCAL_HOSTNAMES: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

/// Refusal reason for a request that does not name a loopback host:
/// `Some(message)` → respond 403. GETs carry no `Origin`; writes must
/// either omit it (curl, same-origin HTML form legacy) or name a loopback
/// origin. A missing `Host` (HTTP/1.0) is refused outright.
fn local_boundary_violation(request: &tiny_http::Request) -> Option<&'static str> {
    let Some(host) = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str())
    else {
        return Some("request has no Host header");
    };
    if !LOCAL_HOSTNAMES.contains(&host_hostname(host)) {
        return Some("request does not name a loopback host");
    }
    if matches!(request.method(), tiny_http::Method::Post) {
        if let Some(origin) = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Origin"))
            .map(|h| h.value.as_str())
        {
            let local = origin
                .strip_prefix("http://")
                .or_else(|| origin.strip_prefix("https://"))
                .is_some_and(|rest| LOCAL_HOSTNAMES.contains(&host_hostname(rest)));
            if !local {
                return Some("cross-origin write request");
            }
        }
    }
    None
}

/// Hostname part of a `Host`/`Origin` authority: `[::1]:8765` → `[::1]`,
/// `example.com:80` → `example.com`, bare `localhost` → itself.
fn host_hostname(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(authority);
    }
    match authority.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|c| c.is_ascii_digit()) => h,
        _ => authority,
    }
}

/// How reading a request body failed; each maps to its own status code.
#[derive(Debug, PartialEq, Eq)]
enum BodyRead {
    TooLarge,
    TooSlow,
    Invalid,
}

/// Bounded body read (`MAX_BODY_BYTES` / `MAX_BODY_SECS`). Raw bytes:
/// `/api/upload` carries file content that need not be UTF-8; the
/// string-shaped routes decode in `handle` and keep refusing non-UTF-8.
fn read_body_limited(reader: &mut dyn std::io::Read) -> Result<Vec<u8>, BodyRead> {
    read_body_with(
        reader,
        MAX_BODY_BYTES,
        std::time::Instant::now() + std::time::Duration::from_secs(MAX_BODY_SECS),
    )
}

/// The bounded read itself, parameterized for tests: at most `max` bytes,
/// no later than `deadline` between successful reads.
fn read_body_with(
    reader: &mut dyn std::io::Read,
    max: u64,
    deadline: std::time::Instant,
) -> Result<Vec<u8>, BodyRead> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                BodyRead::TooSlow
            } else {
                BodyRead::Invalid
            }
        })?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() as u64 > max {
            return Err(BodyRead::TooLarge);
        }
        if std::time::Instant::now() > deadline {
            return Err(BodyRead::TooSlow);
        }
    }
    Ok(buf)
}

fn handle(mut request: tiny_http::Request, ctx: &Ctx) {
    let method = request.method().clone();
    let full_url = request.url().to_string();
    // Split off the query string: /api/status takes optional knobs
    // (?profile= / ?env= / ?level=); the static routes match on the path.
    let (url, query) = match full_url.split_once('?') {
        Some((path, q)) => (path.to_string(), q.to_string()),
        None => (full_url.clone(), String::new()),
    };

    // Loopback boundary (review 2026-10-18 表一#2): the socket binds
    // 127.0.0.1, but a page in the user's browser can still reach it —
    // a cross-site POST carries a foreign `Origin`, and DNS rebinding
    // swaps the `Host`. Both must name a loopback hostname.
    if let Some(reason) = local_boundary_violation(&request) {
        let _ = request.respond(json_response(
            403,
            &serde_json::json!({"ok": false, "error": reason}),
        ));
        return;
    }

    // Raw bytes for the upload route; the string-shaped routes get a
    // UTF-8 decode and keep the old "cannot read request body" refusal
    // for non-UTF-8 bodies.
    let mut raw_body: Vec<u8> = Vec::new();
    let mut body = String::new();
    let mut body_is_text = true;
    if matches!(method, tiny_http::Method::Post) {
        match read_body_limited(request.as_reader()) {
            Ok(b) => {
                raw_body = b;
                match String::from_utf8(raw_body.clone()) {
                    Ok(s) => body = s,
                    Err(_) => body_is_text = false,
                }
            }
            Err(e) => {
                let (status, msg) = match e {
                    BodyRead::TooLarge => (413, "request body too large"),
                    BodyRead::TooSlow => (408, "request body read timed out"),
                    BodyRead::Invalid => (400, "cannot read request body"),
                };
                let _ = request.respond(json_response(
                    status,
                    &serde_json::json!({"ok": false, "error": msg}),
                ));
                return;
            }
        }
        if !body_is_text && url.trim_end_matches('/') != "/api/upload" {
            let _ = request.respond(json_response(
                400,
                &serde_json::json!({"ok": false, "error": "cannot read request body"}),
            ));
            return;
        }
    }

    // /status (no trailing slash) redirects so the page's relative asset
    // references (./app.js) resolve under /status/ — from the bare path
    // they would collide with the editor's /app.js.
    let response = if method == tiny_http::Method::Get && url == "/status" {
        redirect_response("/status/")
    } else {
        match (method, url.trim_end_matches('/')) {
            (tiny_http::Method::Get, "/api/schema") => {
                let (status, value) = api_schema(ctx, &query);
                json_response(status, &value)
            }
            (tiny_http::Method::Post, "/api/validate") => json_response(200, &api_validate(&body)),
            (tiny_http::Method::Post, "/api/schema") => {
                let (status, value) = api_save(ctx, &body);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/status") => {
                let (status, value) = api_status(ctx, &query);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/tables") => {
                let (status, value) = api_tables(ctx, &query);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/table") => {
                let (status, value) = api_table(ctx, &query);
                json_response(status, &value)
            }
            (tiny_http::Method::Post, "/api/upload") => {
                let (status, value) = api_upload(ctx, &query, &raw_body);
                json_response(status, &value)
            }
            (tiny_http::Method::Post, "/api/build") => {
                let (status, value) = api_build(ctx, &body);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/artifacts") => {
                let (status, value) = api_artifacts(ctx, &query);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/artifact") => {
                let (status, mime, bytes) = api_artifact(ctx, &query);
                bytes_response(status, mime, bytes)
            }
            (tiny_http::Method::Get, "/api/snapshots") => {
                let (status, value) = api_snapshots(ctx);
                json_response(status, &value)
            }
            (tiny_http::Method::Post, "/api/snapshot") => {
                let (status, value) = api_snapshot(ctx, &body);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/diff") => {
                let (status, value) = api_diff(ctx, &query);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "/api/registry") => {
                let (status, value) = api_registry(ctx);
                json_response(status, &value)
            }
            (tiny_http::Method::Get, "" | "/index.html") => {
                text_response(200, "text/html; charset=utf-8", EDITOR_INDEX.to_string())
            }
            (tiny_http::Method::Get, "/app.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_APP_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/constants.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_CONSTANTS_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/state.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_STATE_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/dom.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_DOM_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/api.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_API_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/types.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_TYPES_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/forms.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                EDITOR_FORMS_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/app.css") => {
                text_response(200, "text/css; charset=utf-8", EDITOR_APP_CSS.to_string())
            }
            (tiny_http::Method::Get, "/status") => {
                text_response(200, "text/html; charset=utf-8", STATUS_INDEX.to_string())
            }
            (tiny_http::Method::Get, "/status/app.js") => text_response(
                200,
                "text/javascript; charset=utf-8",
                STATUS_APP_JS.to_string(),
            ),
            (tiny_http::Method::Get, "/status/app.css") => {
                text_response(200, "text/css; charset=utf-8", STATUS_APP_CSS.to_string())
            }
            (tiny_http::Method::Get, "/favicon.ico") => html_response(404, "not found".to_string()),
            _ => json_response(404, &serde_json::json!({"ok": false, "error": "not found"})),
        }
    };
    let _ = request.respond(response);
}

/// JSON response: every API endpoint speaks this shape
/// (`{"ok": ...}` plus endpoint-specific fields).
fn json_response(
    status: u16,
    value: &serde_json::Value,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(json(value))
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes(
                &b"Content-Type"[..],
                &b"application/json; charset=utf-8"[..],
            )
            .expect("static header"),
        )
}

/// Text response with an explicit content type (editor page and assets).
fn text_response(
    status: u16,
    mime: &str,
    text: String,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(text)
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], mime.as_bytes())
                .expect("static header"),
        )
}

fn html_response(status: u16, html: String) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    text_response(status, "text/html; charset=utf-8", html)
}

/// Binary response (artifact file serving): bytes + content type.
fn bytes_response(
    status: u16,
    mime: &str,
    bytes: Vec<u8>,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_data(bytes)
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], mime.as_bytes())
                .expect("static header"),
        )
}

/// Project-relative rendering of a save target (mirror of the index's
/// `file` names, so the echoed index can name it).
fn rel_of_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 301 to a canonical path — used to send bare `/status` to `/status/` so
/// the page's relative asset references resolve inside its own directory.
fn redirect_response(location: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(String::new())
        .with_status_code(301)
        .with_header(
            tiny_http::Header::from_bytes(&b"Location"[..], location.as_bytes())
                .expect("static header"),
        )
}

/// Minimal query-string reader for `/api/status`'s optional knobs
/// (`?profile=NAME&env=ENV&level=LEVEL`). Values stay raw (the knobs are
/// identifiers, not free text); unknown keys and bare flags are ignored.
fn query_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|kv| {
            let (key, value) = kv.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// GET /api/status[?profile=NAME&env=ENV&level=LEVEL] — the project status
/// face (U1). Three blocks in one document so the page renders from a
/// single fetch: project facts, the validation panel for one profile (the
/// same `validate_project` entry `cage check` and `cage lsp` consume —
/// page, CLI and editor diagnostics can never diverge), and per-profile
/// build facts (freshness = the layer-1 comparison `build_project` makes
/// before deciding to skip, reported instead of acted on). No state: every
/// request re-reads the project and the manifest from disk, like
/// `/api/schema`.
mod handlers;
#[cfg(test)]
mod tests;

pub(crate) use handlers::{
    api_artifact, api_artifacts, api_build, api_diff, api_registry, api_save, api_schema,
    api_snapshot, api_snapshots, api_status, api_table, api_tables, api_upload, api_validate, json,
};
