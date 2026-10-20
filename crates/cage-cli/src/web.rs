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
use crate::load_schema_for_config;
use cage_core::diagnostics::Diagnostics;
use cage_core::edit::{from_editor_json, to_canonical_yaml, to_editor_json};
use cage_core::error::codes::Severity;
use cage_core::manifest::ProjectConfig;
use cage_core::value::{Document, Value};
use std::path::{Path, PathBuf};

/// W3 editor page and assets, embedded at compile time from the docs-site
/// copy of the sources (include! resolves relative to this file).
const EDITOR_INDEX: &str = include_str!("../../../docs/public/editor/index.html");
const EDITOR_APP_JS: &str = include_str!("../../../docs/public/editor/app.js");
const EDITOR_APP_CSS: &str = include_str!("../../../docs/public/editor/app.css");

/// U1 status page and assets, embedded the same way.
const STATUS_INDEX: &str = include_str!("../../../docs/public/status/index.html");
const STATUS_APP_JS: &str = include_str!("../../../docs/public/status/app.js");
const STATUS_APP_CSS: &str = include_str!("../../../docs/public/status/app.css");

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
struct Ctx {
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

/// Bounded body read (`MAX_BODY_BYTES` / `MAX_BODY_SECS`).
fn read_body_limited(reader: &mut dyn std::io::Read) -> Result<String, BodyRead> {
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
) -> Result<String, BodyRead> {
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
    String::from_utf8(buf).map_err(|_| BodyRead::Invalid)
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

    let mut body = String::new();
    if matches!(method, tiny_http::Method::Post) {
        match read_body_limited(request.as_reader()) {
            Ok(b) => body = b,
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

/// GET /api/schema[?profile=NAME] — merged schema as the editor document
/// plus the project facts the frontend needs (profiles, write-back
/// target). With a `profile` the payload also carries the projected
/// schema: `filter_by_profile` — the exact gating the build pipeline
/// applies — run against the same merged schema. The frontend diffs
/// full-minus-projected to grey out what the profile would drop (hidden
/// things stay visibly hidden), so no visibility semantics live in JS.
fn api_schema(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let schema = match load_schema_for_config(&ctx.root, &ctx.config) {
        Ok(s) => s,
        Err(e) => {
            return (200, serde_json::json!({"ok": false, "error": e}));
        }
    };
    let profile = query_params(query)
        .into_iter()
        .find(|(k, _)| k == "profile")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty());
    let projection = match profile.as_deref() {
        None => None,
        Some(name) => {
            if !ctx.config.profiles.contains_key(name) {
                return (
                    400,
                    serde_json::json!({"ok": false, "error": format!("unknown profile '{name}'")}),
                );
            }
            // The projection only gates schema tables/fields; the empty
            // document just satisfies filter_by_profile's signature (its
            // row filtering is a no-op on no rows).
            let empty = Document {
                tables: indexmap::IndexMap::new(),
                source_files: Vec::new(),
                metadata: cage_core::DocumentMetadata::default(),
            };
            let (projected, _) = crate::filter_by_profile(&schema, &empty, name);
            Some(serde_json::json!({
                "profile": name,
                "schema": to_editor_json(&projected),
                "visible_tables": projected.tables.len(),
            }))
        }
    };
    let mut payload = serde_json::json!({
        "ok": true,
        "project": ctx.config.project.name,
        "schema_path": ctx.config.schema_path,
        "profile_names": ctx.config.profiles.keys().collect::<Vec<_>>(),
        "warnings_as_errors": ctx.config.warnings_as_errors,
        "schema": to_editor_json(&schema),
    });
    if let Some(p) = projection {
        payload["projection"] = p;
    }
    (200, payload)
}

/// POST /api/validate — run the editor-side checks on an edited document:
/// E1701 (does not parse into a Schema) then L1 schema consistency
/// checks (raises the existing `E1004` codes). Full L2-L7 validation needs
/// the sources and stays with `cage check` — the editor is an authoring
/// surface, not a replacement for the validation pipeline.
fn api_validate(body: &str) -> serde_json::Value {
    let (ok, diagnostics) = match from_editor_json(body) {
        Err(diags) => (false, diags),
        Ok(schema) => {
            let diags = schema.validate();
            (!diags.has_errors(), diags)
        }
    };
    serde_json::json!({"ok": ok, "diagnostics": diags_json(&diagnostics)})
}

/// The editor contract delivers diagnostics as a bare JSON list.
/// `Diagnostics` derives Serialize over its named `items` field and would
/// render as `{"items": [...]}` — the API flattens that wrapper.
fn diags_json(diags: &Diagnostics) -> serde_json::Value {
    serde_json::to_value(diags.iter().collect::<Vec<_>>()).expect("diagnostics serialize")
}

/// POST /api/schema — save the given editor document as canonical YAML.
///
/// The write-back target is the single schema file the project config
/// names. Two targets are refused with a 409: multi-file schemas (a
/// `schema_path` pointing at a directory) are an author-side organization
/// the editor does not rewrite, and a `registry:` `schema_path` (R2) is
/// owned by the published entry — edits belong in the publisher project.
/// With no `schema_path` configured the document is written to
/// `schema.yaml` and the response reminds the user to wire it into the
/// config; the server never rewrites `cage.toml`.
fn api_save(ctx: &Ctx, body: &str) -> (u16, serde_json::Value) {
    let schema = match from_editor_json(body) {
        Ok(schema) => schema,
        Err(diags) => {
            return (
                200,
                serde_json::json!({"ok": false, "diagnostics": diags_json(&diags)}),
            );
        }
    };

    if let Some(rel) = ctx.config.schema_path.as_deref() {
        if rel.starts_with("registry:") {
            return (
                409,
                serde_json::json!({
                    "ok": false,
                    "save_target": rel,
                    "error": format!(
                        "schema_path '{rel}' resolves into the Configuration Registry; the \
                         entry schema is published, not edited — change the schema in the \
                         publisher project and re-publish"
                    ),
                }),
            );
        }
    }

    let target: PathBuf = match &ctx.config.schema_path {
        Some(rel) => ctx.root.join(rel),
        None => ctx.root.join("schema.yaml"),
    };
    if target.is_dir() {
        return (
            409,
            serde_json::json!({
                "ok": false,
                "save_target": target.display().to_string(),
                "error": format!(
                    "schema_path '{}' is a directory; the editor writes the canonical \
                     single-file schema — point schema_path at one YAML file (or remove it \
                     to enable defaults to schema.yaml)",
                    ctx.config.schema_path.as_deref().unwrap_or("schema.yaml")
                ),
            }),
        );
    }

    let yaml = to_canonical_yaml(&schema);
    let bytes = yaml.len();
    if let Err(e) = std::fs::write(&target, &yaml) {
        return (
            500,
            serde_json::json!({"ok": false, "error": format!(
                "cannot write {}: {e}", target.display()
            )}),
        );
    }
    let mut payload = serde_json::json!({
        "ok": true,
        "save_target": target.display().to_string(),
        "bytes": bytes,
    });
    if ctx.config.schema_path.is_none() {
        payload["note"] = serde_json::json!(
            "schema_path is not set in the project config; add \
             'schema_path = \"schema.yaml\"' for builds to read this schema"
        );
    }
    (200, payload)
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
fn api_status(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let profile = match get("profile") {
        Some(p) => p,
        // No query → the first configured profile (config order, the same
        // order `cage build` lists when an unknown one is rejected).
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    if !ctx.config.profiles.contains_key(&profile) {
        return (
            400,
            serde_json::json!({"ok": false, "error": format!(
                "unknown profile '{profile}' (available: {})",
                ctx.config.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )}),
        );
    }
    let env = get("env");
    // Default matches `cage check --level` (semantic).
    let level = get("level").unwrap_or_else(|| "semantic".to_string());

    let validation =
        match crate::validate_project(&ctx.root, &level, env.as_deref(), &profile, false) {
            Ok(out) => {
                let (mut errors, mut warnings, mut infos) = (0usize, 0usize, 0usize);
                for d in &out.diagnostics {
                    match d.severity {
                        Severity::Error => errors += 1,
                        Severity::Warning => warnings += 1,
                        Severity::Info => infos += 1,
                    }
                }
                serde_json::json!({
                    "ok": !out.diagnostics.has_errors(),
                    "profile": profile,
                    "env": env,
                    "level": level,
                    "errors": errors,
                    "warnings": warnings,
                    "info": infos,
                    "visible_tables": out.visible_tables,
                    "diagnostics": diags_json(&out.diagnostics),
                })
            }
            // Project load failure (bad cage.toml, unresolvable schema or
            // registry reference): the page stays up and names the problem —
            // this is exactly the state a status face exists for.
            Err(e) => serde_json::json!({"ok": false, "error": e, "diagnostics": []}),
        };

    (
        200,
        serde_json::json!({
            "ok": true,
            "project": ctx.config.project.name,
            "schema_path": ctx.config.schema_path,
            "warnings_as_errors": ctx.config.warnings_as_errors,
            "output_dir": ctx.config.output_dir.as_deref().unwrap_or("build"),
            "profile_names": ctx.config.profiles.keys().collect::<Vec<_>>(),
            "profile": profile,
            "validation": validation,
            "builds": build_facts(ctx, env.as_deref()),
        }),
    )
}

/// Shared profile/env resolution for the data-panel endpoints: the same
/// contract as `/api/status` (query wins, else first configured profile;
/// unknown profile 400; no profile 500; bad env 400 through the same
/// load chain).
fn data_panel_project(
    ctx: &Ctx,
    profile: &str,
    env: Option<&str>,
) -> Result<(cage_core::schema::Schema, cage_core::value::Document), (u16, serde_json::Value)> {
    if !ctx.config.profiles.contains_key(profile) {
        return Err((
            400,
            serde_json::json!({"ok": false, "error": format!(
                "unknown profile '{profile}' (available: {})",
                ctx.config.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )}),
        ));
    }
    let project = crate::load_project(&ctx.root, false)
        .and_then(|p| crate::project_with_env(p, env))
        .map_err(|e| (400u16, serde_json::json!({"ok": false, "error": e})))?;
    let (schema, document) = crate::filter_by_profile(&project.schema, &project.document, profile);
    Ok((schema, document))
}

/// GET /api/tables[?profile=NAME&env=ENV] — the data panel's table
/// inventory (planner interface P0). One entry per profile-visible table:
/// name, row count, source facts, primary key — straight from the same
/// load chain `check` consumes, so the list can never show a table the
/// CLI does not see. No validation here: the panel pairs this list with
/// the `/api/status` diagnostics, and the anchoring data (table / row /
/// column) already rides each diagnostic.
fn api_tables(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    let env = get("env");
    let (schema, document) = match data_panel_project(ctx, &profile, env.as_deref()) {
        Ok(pair) => pair,
        Err((status, value)) => return (status, value),
    };

    // Primary key comes from the schema (authoritative and always
    // present), not the loaded table — some source readers (JSON) leave
    // the value-table's own field empty.
    let mut tables: Vec<serde_json::Value> = document
        .tables
        .values()
        .map(|t| {
            let pk = schema
                .tables
                .get(&t.name)
                .map(|def| def.primary_key.clone())
                .unwrap_or_default();
            serde_json::json!({
                "name": t.name,
                "rows": t.rows.len(),
                "source_file": t.source_file,
                "sheet": t.sheet,
                "primary_key": pk,
            })
        })
        .collect();
    // Name order, not load order: the panel is an inventory, and a
    // stable list beats "the order the source files happened to mention
    // tables in" (which changes with every source reorganization).
    tables.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    (
        200,
        serde_json::json!({"ok": true, "profile": profile, "tables": tables}),
    )
}

/// GET /api/table?name=NAME[&profile=&env=&offset=&limit=] — one table's
/// rows for the data panel (planner interface P0). Rows render with the
/// schema-declared column order (the same `ordered_columns` the migrate
/// write-back uses), paginated and truncated: default 200 rows, hard cap
/// 1000, so a huge table cannot blow up either the response or the
/// browser. Cell values go through a total JSON mapping (bytes, which
/// have no display form, become a marker object). 404 for a table the
/// profile projection hid — same visibility rule as the CLI.
// Total JSON mapping: bytes have no display form, so they become a
// marker object instead of failing the whole request.
fn data_cell(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::UInt(u) => serde_json::Value::from(*u),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::String(s) => serde_json::Value::from(s.clone()),
        Value::Bytes(b) => serde_json::json!({"bytes": b.len()}),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(data_cell).collect()),
        Value::Object(map) => {
            let mut m = serde_json::Map::new();
            for (k, v) in map {
                m.insert(k.clone(), data_cell(v));
            }
            serde_json::Value::Object(m)
        }
    }
}

fn api_table(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    let Some(name) = get("name") else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "missing required parameter 'name'"}),
        );
    };
    // Parse failures fall back to the defaults (the panel always sends
    // well-formed numbers; hand-typed URLs should not error out).
    let offset = get("offset")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = match get("limit").and_then(|v| v.parse::<usize>().ok()) {
        Some(n) if n > 0 => n.min(1000),
        _ => 200,
    };

    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    let env = get("env");
    let (schema, document) = match data_panel_project(ctx, &profile, env.as_deref()) {
        Ok(pair) => pair,
        Err((status, value)) => return (status, value),
    };
    let Some(table) = document.tables.values().find(|t| t.name == name) else {
        return (
            404,
            serde_json::json!({
                "ok": false,
                "error": format!("no table '{name}' in this profile's view")
            }),
        );
    };

    let columns = crate::ordered_columns(table, &schema);
    let total = table.rows.len();
    let start = offset.min(total);
    let end = (start + limit).min(total);
    let rows: Vec<serde_json::Value> = table.rows[start..end]
        .iter()
        .map(|row| {
            let mut cells = serde_json::Map::new();
            for col in &columns {
                match row.fields.get(col) {
                    Some(tv) => cells.insert(col.clone(), data_cell(&tv.value)),
                    None => cells.insert(col.clone(), serde_json::Value::Null),
                };
            }
            serde_json::json!({
                "index": row.index,
                "pk": row.primary_key.iter().map(data_cell).collect::<Vec<_>>(),
                "cells": cells,
            })
        })
        .collect();

    let pk = schema.tables.get(&name).map_or_else(
        || table.primary_key_fields.clone(),
        |def| def.primary_key.clone(),
    );
    (
        200,
        serde_json::json!({
            "ok": true,
            "name": name,
            "profile": profile,
            "primary_key": pk,
            "source_file": table.source_file,
            "sheet": table.sheet,
            "columns": columns,
            "total": total,
            "offset": start,
            "limit": limit,
            "rows": rows,
        }),
    )
}

/// Per-profile build facts for `/api/status`. The manifest is single-slot
/// (`<output_dir>/manifest.json`, one profile at a time on disk), so
/// "fresh" additionally implies the last build on disk was for this
/// profile: same profile, same environment, same schema/source/target
/// fingerprints, and every listed artifact still intact — the exact
/// comparison `build_project` layer 1 makes (kept in step by pointing at
/// the same manifest fields; drift here is a status-page bug, not a build
/// bug). `changed_tables` is the data-staleness detail: the tables whose
/// rows fingerprint moved since that build (the layer-2 detector).
fn build_facts(ctx: &Ctx, env: Option<&str>) -> serde_json::Value {
    use cage_core::manifest::ManifestGenerator;

    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let prev = crate::load_manifest(&ctx.root.join(output_dir)).ok();
    // Current inputs: one project load (one pass over the sources), then
    // each profile projects its own view and hashes.
    let project =
        match crate::load_project(&ctx.root, false).and_then(|p| crate::project_with_env(p, env)) {
            Ok(p) => p,
            Err(e) => return serde_json::json!({"ok": false, "error": e}),
        };

    let mut builds = serde_json::Map::new();
    for (name, build_profile) in &ctx.config.profiles {
        let target_records = ManifestGenerator::target_records(&build_profile.targets);
        let (schema, document) = crate::filter_by_profile(&project.schema, &project.document, name);
        let normalized = cage_core::normalize::normalize_document(&document);
        let (schema_hash, source_hash) = ManifestGenerator::input_hashes(&schema, &normalized);
        let cur_table_hashes = ManifestGenerator::table_hashes(&normalized);

        let mut entry = serde_json::json!({
            "target_count": build_profile.targets.len(),
            "targets": build_profile.targets.iter().map(|t| serde_json::json!(
                {"format": t.format, "output_dir": t.output_dir}
            )).collect::<Vec<_>>(),
            "manifest_present": prev.is_some(),
            "fresh": false,
            "changed_tables": [],
        });
        if let Some(m) = &prev {
            entry["last_build_profile"] = serde_json::json!(m.profile);
            if m.profile == *name {
                entry["build_id"] = serde_json::json!(m.build_id);
                entry["content_hash"] = serde_json::json!(m.content_hash);
                entry["artifact_count"] = serde_json::json!(m.artifacts.len());
                let same_inputs = m.environment.as_deref() == env
                    && m.schema_hash == schema_hash
                    && m.source_hash == source_hash
                    && m.targets == target_records
                    && m.artifacts.keys().all(|rel| ctx.root.join(rel).is_file());
                entry["fresh"] = serde_json::json!(same_inputs);
                if !same_inputs && !m.table_hashes.is_empty() {
                    let changed: Vec<&String> = cur_table_hashes
                        .keys()
                        .filter(|t| m.table_hashes.get(*t) != cur_table_hashes.get(*t))
                        .collect();
                    entry["changed_tables"] = serde_json::json!(changed);
                }
            }
        } else {
            entry["last_build_profile"] = serde_json::Value::Null;
        }
        builds.insert(name.clone(), entry);
    }
    serde_json::Value::Object(builds)
}

/// POST /api/build — run the build pipeline for one profile: the same
/// `build_project` entry `cage build` uses (full validation, three-layer
/// incremental, deterministic manifest). Body is `{"profile": "..."}` with
/// optional `"env"`, `"level"` and `"incremental": false`. Validation
/// failures answer `{"ok": false, "diagnostics": [...]}` with 200 (the
/// /api/validate shape — the page renders them in the panel), usage / I/O
/// errors answer 400 with `error`, and an up-to-date build answers
/// `up_to_date: true` without touching anything.
fn api_build(ctx: &Ctx, body: &str) -> (u16, serde_json::Value) {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("invalid JSON body: {e}")}),
            )
        }
    };
    let Some(profile) = parsed.get("profile").and_then(|p| p.as_str()) else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "body must carry {\"profile\": \"...\"}"}),
        );
    };
    let env = parsed.get("env").and_then(|e| e.as_str());
    let level = parsed
        .get("level")
        .and_then(|l| l.as_str())
        .unwrap_or("semantic");
    let incremental = parsed
        .get("incremental")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    match crate::build_project(&ctx.root, level, profile, env, incremental, false) {
        Ok(out) => (
            200,
            serde_json::json!({
                "ok": true,
                "profile": out.profile,
                "up_to_date": false,
                "layer2": out.layer2,
                "regenerated": out.artifacts.len() - out.carried_count,
                "carried": out.carried_count,
                "artifact_count": out.artifacts.len(),
                "manifest_path": out.manifest_path.display().to_string(),
                "build_id": out.manifest.build_id,
                "content_hash": out.manifest.content_hash,
            }),
        ),
        Err(crate::BuildFailure::UpToDate {
            artifacts,
            manifest,
        }) => (
            200,
            serde_json::json!({
                "ok": true,
                "profile": profile,
                "up_to_date": true,
                "artifact_count": artifacts,
                "manifest_path": manifest.display().to_string(),
            }),
        ),
        Err(crate::BuildFailure::Validation(diags)) => (
            200,
            serde_json::json!({"ok": false, "diagnostics": diags_json(&diags)}),
        ),
        Err(crate::BuildFailure::Io(msg)) => (400, serde_json::json!({"ok": false, "error": msg})),
    }
}

/// GET /api/artifacts[?profile=NAME&env=ENV] — the artifact tree of one
/// profile's build, straight from the build manifest (the ledger the build
/// wrote). The manifest is single-slot: when another profile owns
/// `<output_dir>/manifest.json`, this profile reports `manifest_profile`
/// and an empty list instead of reading unverifiable leftovers — build it
/// to browse it. `stale` repeats the status face's layer-1 verdict: a
/// manifest whose inputs moved describes a build that no longer matches
/// the sources.
fn api_artifacts(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let profile = match get("profile") {
        Some(p) => p,
        None => match ctx.config.profiles.keys().next() {
            Some(first) => first.clone(),
            None => {
                return (
                    500,
                    serde_json::json!({"ok": false, "error": "no build profile is configured"}),
                )
            }
        },
    };
    if !ctx.config.profiles.contains_key(&profile) {
        return (
            400,
            serde_json::json!({"ok": false, "error": format!(
                "unknown profile '{profile}' (available: {})",
                ctx.config.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            )}),
        );
    }
    let env = get("env");
    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let Ok(manifest) = crate::load_manifest(&ctx.root.join(output_dir)) else {
        return (
            200,
            serde_json::json!({
                "ok": true, "profile": profile, "manifest_profile": null,
                "fresh": false, "stale": false, "targets": [], "artifacts": [],
            }),
        );
    };
    if manifest.profile != profile {
        return (
            200,
            serde_json::json!({
                "ok": true, "profile": profile, "manifest_profile": manifest.profile,
                "fresh": false, "stale": false, "targets": [], "artifacts": [],
            }),
        );
    }

    let builds = build_facts(ctx, env.as_deref());
    let fresh = builds
        .get(&profile)
        .and_then(|b| b.get("fresh"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    (
        200,
        serde_json::json!({
            "ok": true,
            "profile": profile,
            "manifest_profile": manifest.profile,
            "fresh": fresh,
            "stale": !fresh,
            "build_id": manifest.build_id,
            "content_hash": manifest.content_hash,
            "targets": manifest.targets,
            "artifacts": manifest.artifacts.values().collect::<Vec<_>>(),
        }),
    )
}

/// GET /api/artifact?path=REL — one artifact's bytes for preview /
/// download. The path must be a key of the build manifest (a lookup,
/// never join-and-pray: nothing outside the ledger is servable), and the
/// bytes are re-hashed against the recorded blake3 before serving — a
/// file that no longer matches answers 409 instead of handing out
/// tampered content, the same verify-first stance the snapshot loader
/// takes.
fn api_artifact(ctx: &Ctx, query: &str) -> (u16, &'static str, Vec<u8>) {
    const JSON: &str = "application/json; charset=utf-8";
    let Some(rel) = query_params(query)
        .iter()
        .find(|(k, _)| k == "path")
        .map(|(_, v)| v.clone())
    else {
        return (
            400,
            JSON,
            br#"{"ok": false, "error": "missing path parameter"}"#.to_vec(),
        );
    };
    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let manifest = match crate::load_manifest(&ctx.root.join(output_dir)) {
        Ok(m) => m,
        Err(e) => {
            let mut err = serde_json::json!({"ok": false, "error": e});
            err["error"] = serde_json::json!(format!(
                "no build manifest under {output_dir} — build first",
            ));
            return (404, JSON, json(&err).into_bytes());
        }
    };
    let Some(info) = manifest.artifacts.get(&rel) else {
        let err = serde_json::json!({"ok": false, "error": format!(
            "'{rel}' is not in the build manifest — only ledgered artifacts are served"
        )});
        return (404, JSON, json(&err).into_bytes());
    };
    let Ok(bytes) = std::fs::read(ctx.root.join(&rel)) else {
        let err = serde_json::json!({"ok": false, "error": format!(
            "cannot read artifact '{rel}'"
        )});
        return (500, JSON, json(&err).into_bytes());
    };
    if blake3::hash(&bytes).to_hex().to_string() != info.hash {
        let err = serde_json::json!({"ok": false, "error": format!(
            "artifact '{rel}' no longer matches its manifest hash — rebuild (the file \
             changed or was tampered with after the build)"
        )});
        return (409, JSON, json(&err).into_bytes());
    }
    (200, artifact_mime(&rel), bytes)
}

/// Content type for an artifact path — preview-rendering hint for the
/// status page; anything unknown serves as plain text.
fn artifact_mime(rel: &str) -> &'static str {
    let ext = Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "json" => "application/json; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "yaml" | "yml" => "text/yaml; charset=utf-8",
        "msgpack" => "application/octet-stream",
        _ => "text/plain; charset=utf-8",
    }
}

/// GET /api/snapshots — the packed snapshots under
/// `<output_dir>/snapshot/` (U3 列/验): each pack with its ledger facts
/// (profile / environment / `build_id` / `content_hash` / artifact count)
/// and a fresh verification pass — the same `verify_snapshot` entry
/// `cage snapshot --verify` runs, so the page never claims a clean pack it
/// did not just check.
fn api_snapshots(ctx: &Ctx) -> (u16, serde_json::Value) {
    let output_dir = ctx.config.output_dir.as_deref().unwrap_or("build");
    let snap_root = ctx.root.join(output_dir).join("snapshot");
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(&snap_root) {
        Ok(rd) => rd
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    let snapshots: Vec<serde_json::Value> = dirs
        .iter()
        .map(|dir| {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            let (verified, files_checked, problems) =
                match cage_core::snapshot::verify_snapshot(dir) {
                    Ok(report) if report.ok => (true, report.files_checked, Vec::new()),
                    Ok(report) => (false, report.files_checked, report.mismatches),
                    Err(e) => (false, 0, vec![e]),
                };
            let manifest = crate::load_manifest(dir).ok();
            let name_ref = &name;
            serde_json::json!({
                "name": name_ref,
                "profile": manifest.as_ref().map(|m| m.profile.clone()),
                "environment": manifest.as_ref().and_then(|m| m.environment.clone()),
                "build_id": manifest.as_ref().map(|m| m.build_id.clone()),
                "content_hash": manifest.as_ref().map(|m| m.content_hash.clone()),
                "artifact_count": manifest.as_ref().map(|m| m.artifacts.len()),
                "verified": verified,
                "files_checked": files_checked,
                "problems": problems,
            })
        })
        .collect();
    (
        200,
        serde_json::json!({
            "ok": true,
            "output_dir": output_dir,
            "snapshots": snapshots,
        }),
    )
}

/// POST /api/snapshot — pack one profile into a self-verifying snapshot
/// (U3 打包): the same pipeline as `cage snapshot` — a fresh full build
/// (no incremental carry-over) through `build_project`, then
/// `pack_snapshot`, which refuses to hand out a pack that does not
/// self-verify. `{profile, env}`; env optional.
fn api_snapshot(ctx: &Ctx, body: &str) -> (u16, serde_json::Value) {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("invalid JSON body: {e}")}),
            );
        }
    };
    let profile = parsed
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| ctx.config.profiles.keys().next().cloned());
    let Some(profile) = profile else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "no profile in body and no profiles configured"}),
        );
    };
    if !ctx.config.profiles.contains_key(&profile) {
        return (
            400,
            serde_json::json!({"ok": false, "error": format!("unknown profile '{profile}'")}),
        );
    }
    let env = parsed
        .get("env")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty());
    match crate::build_project(&ctx.root, "gamerule", &profile, env, false, false) {
        Ok(out) => match crate::pack_snapshot(&ctx.root, &out) {
            Ok((snap_dir, files)) => {
                let rel = snap_dir.strip_prefix(&ctx.root).map_or_else(
                    |_| snap_dir.to_string_lossy().to_string(),
                    |p| p.to_string_lossy().to_string(),
                );
                (
                    200,
                    serde_json::json!({
                        "ok": true,
                        "snapshot_dir": rel,
                        "files": files,
                        "artifacts": out.artifacts.len(),
                        "build_id": out.manifest.build_id,
                    }),
                )
            }
            Err(e) => (500, serde_json::json!({"ok": false, "error": e})),
        },
        Err(crate::BuildFailure::Validation(diags)) => {
            (200, serde_json::json!({"ok": false, "diagnostics": diags}))
        }
        Err(crate::BuildFailure::Io(msg)) => (400, serde_json::json!({"ok": false, "error": msg})),
        Err(crate::BuildFailure::UpToDate { .. }) => (
            500,
            serde_json::json!({"ok": false, "error": "internal: snapshot build did not rebuild"}),
        ),
    }
}

/// GET /api/diff?from=REL&to=REL — the layered diff between two build
/// manifests (U3): paths are project-root-relative and name a directory
/// holding `manifest.json` (the build output dir or a snapshot dir) or a
/// manifest file. Shape per the UI ruling: `{changed_tables,
/// affected_targets, files:{added,removed,changed},
/// roots:{content_hash, build_id}}` — the page folds 表 → target → 文件
/// from it. `table`/`target` ride on each file entry so the fold is pure
/// grouping.
fn api_diff(ctx: &Ctx, query: &str) -> (u16, serde_json::Value) {
    let params = query_params(query);
    let get = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let (Some(from), Some(to)) = (get("from"), get("to")) else {
        return (
            400,
            serde_json::json!({"ok": false, "error": "missing from/to parameters (?from=REL&to=REL)"}),
        );
    };
    let safe_path = |rel: &str| -> Result<std::path::PathBuf, String> {
        let relative = std::path::Path::new(rel);
        if relative.is_absolute()
            || relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("path must be a normalized project-relative path".to_string());
        }
        let root = std::fs::canonicalize(&ctx.root).map_err(|e| format!("project root: {e}"))?;
        let path = std::fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
        if !path.starts_with(&root) {
            return Err("path resolves outside project root".to_string());
        }
        Ok(path)
    };
    let from_path = match safe_path(&from) {
        Ok(path) => path,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("baseline '{from}': {e}")}),
            )
        }
    };
    let to_path = match safe_path(&to) {
        Ok(path) => path,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("target '{to}': {e}")}),
            )
        }
    };
    let base = match crate::load_manifest(&from_path) {
        Ok(m) => m,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("baseline '{from}': {e}")}),
            );
        }
    };
    let new = match crate::load_manifest(&to_path) {
        Ok(m) => m,
        Err(e) => {
            return (
                400,
                serde_json::json!({"ok": false, "error": format!("target '{to}': {e}")}),
            );
        }
    };

    // Layer 1: tables whose per-table fingerprint moved (either side).
    let mut changed_tables: Vec<String> = base
        .table_hashes
        .keys()
        .chain(new.table_hashes.keys())
        .filter(|t| base.table_hashes.get(*t) != new.table_hashes.get(*t))
        .cloned()
        .collect();
    changed_tables.sort();
    changed_tables.dedup();

    // Layer 3: the files, each tagged with its table (the manifests'
    // ledger) and target (the output_dir the path lives under) so the UI
    // folds without re-deriving either.
    let mut target_dirs: Vec<&str> = base
        .targets
        .iter()
        .chain(new.targets.iter())
        .map(|t| t.output_dir.as_str())
        .collect();
    target_dirs.sort_unstable();
    target_dirs.dedup();
    let target_of = |file: &str| -> Option<String> {
        target_dirs
            .iter()
            .rfind(|dir| {
                let dir = dir.trim_end_matches('/');
                *file == *dir || file.starts_with(&format!("{dir}/"))
            })
            .map(std::string::ToString::to_string)
    };
    let entry = |path: &str,
                 from: Option<&cage_core::manifest::ArtifactInfo>,
                 to: Option<&cage_core::manifest::ArtifactInfo>| {
        let table = to
            .and_then(|i| i.table.clone())
            .or_else(|| from.and_then(|i| i.table.clone()));
        serde_json::json!({
            "path": path,
            "table": table,
            "target": target_of(path),
            "from_hash": from.map(|i| i.hash.clone()),
            "to_hash": to.map(|i| i.hash.clone()),
        })
    };
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (path, info) in &new.artifacts {
        match base.artifacts.get(path) {
            None => added.push(entry(path, None, Some(info))),
            Some(old) if old.hash != info.hash => changed.push(entry(path, Some(old), Some(info))),
            Some(_) => {}
        }
    }
    for (path, info) in &base.artifacts {
        if !new.artifacts.contains_key(path) {
            removed.push(entry(path, Some(info), None));
        }
    }

    // Layer 2: the targets whose outputs sit under any moved file.
    let touches = |file: &str, dir: &str| {
        let dir = dir.trim_end_matches('/');
        file == dir || file.starts_with(&format!("{dir}/"))
    };
    let moved: Vec<&str> = added
        .iter()
        .chain(removed.iter())
        .chain(changed.iter())
        .filter_map(|e| e["path"].as_str())
        .collect();
    let affected_targets: Vec<String> = target_dirs
        .iter()
        .filter(|dir| moved.iter().any(|f| touches(f, dir)))
        .map(std::string::ToString::to_string)
        .collect();

    (
        200,
        serde_json::json!({
            "ok": true,
            "from": from,
            "to": to,
            "changed_tables": changed_tables,
            "affected_targets": affected_targets,
            "files": {"added": added, "removed": removed, "changed": changed},
            "roots": {
                "content_hash": {"from": base.content_hash, "to": new.content_hash},
                "build_id": {"from": base.build_id, "to": new.build_id},
            },
        }),
    )
}

/// GET /api/registry — the local registry this project would publish into
/// (`[registry] path` in cage.toml), browsed (U3): packages with their
/// published versions. Unconfigured → ok with an empty list and a note;
/// a corrupt index is the registry's own error string.
fn api_registry(ctx: &Ctx) -> (u16, serde_json::Value) {
    let Some(reg) = &ctx.config.registry else {
        return (
            200,
            serde_json::json!({
                "ok": true,
                "packages": [],
                "note": "no [registry] configured in cage.toml",
            }),
        );
    };
    let reg_root = ctx.root.join(&reg.path);
    match cage_core::registry::packages(&reg_root) {
        Ok(indexes) => (
            200,
            serde_json::json!({
                "ok": true,
                "registry": reg.path,
                "packages": indexes,
            }),
        ),
        Err(e) => (
            200,
            serde_json::json!({"ok": false, "error": e, "registry": reg.path}),
        ),
    }
}

fn json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).expect("api response serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::schema::Schema;
    use std::fs;
    use std::io::{Read as _, Write as _};
    use tempfile::TempDir;

    /// Editor document for a schema with one table whose primary key points
    /// at a declared field — the smallest document the API accepts.
    pub(crate) fn good_doc() -> String {
        let schema = Schema::new();
        to_editor_json(&schema).to_string()
    }

    /// Editor document whose table primary key references an undeclared
    /// field — parses as a Schema but fails the L1 consistency checks
    /// (E1004) that `POST /api/validate` runs.
    fn dangling_pk_doc() -> String {
        use cage_core::schema::TableSchema;
        let mut schema = Schema::new();
        schema.add_table(TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["ghost".to_string()],
            fields: indexmap::IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: indexmap::IndexMap::new(),
        });
        to_editor_json(&schema).to_string()
    }

    pub(crate) fn ctx_in(dir: &TempDir, schema_path: Option<&str>) -> Ctx {
        let mut config = ProjectConfig::default();
        config.project.name = "demo".to_string();
        config.schema_path = schema_path.map(str::to_string);
        Ctx {
            root: dir.path().to_path_buf(),
            config,
        }
    }

    /// A full project for the status/build endpoints (U1): two profiles
    /// (client → json, server → csv), one table, one JSON source. The
    /// status/build handlers re-read the project from disk (they reuse the
    /// `check` / `build` entries), so the on-disk cage.toml matters, not
    /// just the Ctx config object.
    pub(crate) fn demo_ctx(dir: &TempDir) -> Ctx {
        use cage_core::manifest::{BuildProfile, TargetConfig};

        fs::write(
            dir.path().join("cage.toml"),
            r#"output_dir = "build"
schema_path = "schema.yaml"

[source_roots]
main = "config"

[project]
name = "demo"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"

[profiles.server]
name = "server"

[[profiles.server.targets]]
format = "csv"
output_dir = "build/server/csv"
"#,
        )
        .expect("write config");
        fs::write(
            dir.path().join("schema.yaml"),
            "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\n      \
             name: { name: name, type: { kind: String } }\nenums: {}\n",
        )
        .expect("write schema");
        fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
        fs::write(
            dir.path().join("config/item.json"),
            r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
        )
        .expect("write source");

        let target = |format: &str, output_dir: &str| TargetConfig {
            format: format.to_string(),
            output_dir: output_dir.to_string(),
            file_template: None,
            options: None,
        };
        let profile = |name: &str, targets: Vec<TargetConfig>| BuildProfile {
            name: name.to_string(),
            targets,
            description: None,
            variables: indexmap::IndexMap::new(),
        };
        let mut config = ProjectConfig::default();
        config.project.name = "demo".to_string();
        config.schema_path = Some("schema.yaml".to_string());
        config
            .source_roots
            .insert("main".to_string(), "config".to_string());
        config.profiles.insert(
            "client".to_string(),
            profile("client", vec![target("json", "build/client/json")]),
        );
        config.profiles.insert(
            "server".to_string(),
            profile("server", vec![target("csv", "build/server/csv")]),
        );
        Ctx {
            root: dir.path().to_path_buf(),
            config,
        }
    }

    #[test]
    fn api_validate_accepts_clean_and_rejects_broken_documents() {
        let ok = api_validate(&good_doc());
        assert_eq!(ok["ok"], serde_json::json!(true));
        assert_eq!(ok["diagnostics"], serde_json::json!([]));

        // Not JSON at all → E1701 from the editor-door parse.
        let bad = api_validate("{not json");
        assert_eq!(bad["ok"], serde_json::json!(false));
        assert_eq!(bad["diagnostics"].as_array().map(Vec::len), Some(1));
        assert_eq!(bad["diagnostics"][0]["code"], serde_json::json!("E1701"));

        // Parses as a Schema but the primary key dangles → E1004.
        let dangling = api_validate(&dangling_pk_doc());
        assert_eq!(dangling["ok"], serde_json::json!(false));
        let codes: Vec<&str> = dangling["diagnostics"]
            .as_array()
            .expect("diagnostics list")
            .iter()
            .filter_map(|d| d["code"].as_str())
            .collect();
        assert!(codes.contains(&"E1004"), "expected E1004, got {codes:?}");
    }

    #[test]
    fn api_schema_serves_the_editor_document_and_project_facts() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("write schema");
        let ctx = ctx_in(&dir, Some("schema.yaml"));

        let (status, doc) = api_schema(&ctx, "");
        assert_eq!(status, 200);
        assert_eq!(doc["ok"], serde_json::json!(true));
        assert_eq!(doc["project"], serde_json::json!("demo"));
        assert_eq!(doc["schema_path"], serde_json::json!("schema.yaml"));
        assert!(doc["schema"].is_object(), "editor schema present");
        let profiles = doc["profile_names"].as_array().expect("profiles");
        assert!(profiles.iter().any(|p| p == "client"));
        assert!(
            doc.get("projection").is_none(),
            "no profile → no projection"
        );

        // No schema_path configured → an empty editor document, still ok.
        let empty = ctx_in(&dir, None);
        let (status, doc) = api_schema(&empty, "");
        assert_eq!(status, 200);
        assert_eq!(doc["ok"], serde_json::json!(true));
        assert_eq!(doc["schema_path"], serde_json::json!(null));
    }

    #[test]
    fn api_schema_projection_gates_tables_and_fields() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        // Rewrite the demo schema with a server-only field and a
        // server-only table — filter_by_profile's exact input shape.
        fs::write(
            dir.path().join("schema.yaml"),
            "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\n      \
             name: { name: name, type: { kind: String } }\n      \
             internal_note: { name: internal_note, type: { kind: String }, targets: [server] }\n  \
             ServerOnly:\n    name: ServerOnly\n    primary_key: [id]\n    targets: [server]\n    fields:\n      \
             id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
        )
        .expect("rewrite schema");

        // client: both the server-only field and table drop out of the
        // projection while the full document keeps them.
        let (status, doc) = api_schema(&ctx, "profile=client");
        assert_eq!(status, 200);
        assert_eq!(doc["ok"], serde_json::json!(true));
        assert_eq!(doc["projection"]["profile"], serde_json::json!("client"));
        let full = &doc["schema"]["tables"];
        let proj = &doc["projection"]["schema"]["tables"];
        assert!(full.get("ServerOnly").is_some(), "full keeps the table");
        assert!(
            proj.get("ServerOnly").is_none(),
            "client projection drops it"
        );
        assert!(
            full["Item"]["fields"].get("internal_note").is_some(),
            "full keeps the field"
        );
        assert!(
            proj["Item"]["fields"].get("internal_note").is_none(),
            "client projection drops it"
        );
        assert_eq!(doc["projection"]["visible_tables"], serde_json::json!(1));

        // server: the projected view matches the full document.
        let (_, doc) = api_schema(&ctx, "profile=server");
        let proj = &doc["projection"]["schema"]["tables"];
        assert!(proj.get("ServerOnly").is_some(), "server sees the table");
        assert!(
            proj["Item"]["fields"].get("internal_note").is_some(),
            "server sees the field"
        );
    }

    #[test]
    fn api_schema_rejects_an_unknown_projection_profile() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        let (status, doc) = api_schema(&ctx, "profile=ghost");
        assert_eq!(status, 400);
        assert_eq!(doc["ok"], serde_json::json!(false));
        let err = doc["error"].as_str().expect("error message");
        assert!(err.contains("ghost"), "error names the profile: {err}");
    }

    #[test]
    fn api_save_writes_canonical_yaml_and_reports_the_target() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, Some("schema.yaml"));
        let doc = good_doc();

        let (status, payload) = api_save(&ctx, &doc);
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(true));
        // bytes counts the canonical YAML payload, not the request
        let schema = from_editor_json(&doc).expect("doc parses");
        let yaml = to_canonical_yaml(&schema);
        assert_eq!(payload["bytes"], serde_json::json!(yaml.len()));
        let written = fs::read_to_string(dir.path().join("schema.yaml")).expect("saved file");
        assert_eq!(written, yaml);
        assert!(
            payload.get("note").is_none(),
            "no note when schema_path is set"
        );
    }

    #[test]
    fn api_save_without_schema_path_defaults_and_notes_the_gap() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, None);

        let (status, payload) = api_save(&ctx, &good_doc());
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(true));
        assert!(
            fs::exists(dir.path().join("schema.yaml")).expect("stat"),
            "default target written"
        );
        assert!(payload["note"]
            .as_str()
            .is_some_and(|n| n.contains("schema_path = \"schema.yaml\"")));
    }

    #[test]
    fn api_save_refuses_directory_and_registry_targets_without_writing() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir(dir.path().join("schemas")).expect("mkdir");

        let (status, payload) = api_save(&ctx_in(&dir, Some("schemas")), &good_doc());
        assert_eq!(status, 409);
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["error"]
            .as_str()
            .is_some_and(|e| e.contains("is a directory")));
        assert!(!dir.path().join("schemas").is_file(), "nothing written");

        let (status, payload) = api_save(&ctx_in(&dir, Some("registry:pkg")), &good_doc());
        assert_eq!(status, 409);
        assert!(payload["error"]
            .as_str()
            .is_some_and(|e| e.contains("Configuration Registry")));
    }

    #[test]
    fn api_save_reports_broken_documents_as_diagnostics_without_writing() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, Some("schema.yaml"));

        let (status, payload) = api_save(&ctx, "{not json");
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert_eq!(
            payload["diagnostics"][0]["code"],
            serde_json::json!("E1701")
        );
        assert!(
            !fs::exists(dir.path().join("schema.yaml")).expect("stat"),
            "nothing written for a broken document"
        );
    }

    #[test]
    fn api_status_reports_validation_and_untouched_build_facts() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        let (status, payload) = api_status(&ctx, "");
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(true));
        assert_eq!(payload["project"], serde_json::json!("demo"));
        assert_eq!(payload["profile"], serde_json::json!("client"));
        assert_eq!(
            payload["profile_names"],
            serde_json::json!(["client", "server"])
        );
        // Validation panel: the on-disk project validates clean.
        assert_eq!(payload["validation"]["ok"], serde_json::json!(true));
        assert_eq!(payload["validation"]["errors"], serde_json::json!(0));
        assert_eq!(
            payload["validation"]["visible_tables"],
            serde_json::json!(1)
        );
        assert_eq!(payload["validation"]["diagnostics"], serde_json::json!([]));
        // No manifest yet: nothing built, nothing fresh.
        let client = &payload["builds"]["client"];
        assert_eq!(client["fresh"], serde_json::json!(false));
        assert_eq!(client["last_build_profile"], serde_json::json!(null));
        assert_eq!(client["target_count"], serde_json::json!(1));
        assert_eq!(client["changed_tables"], serde_json::json!([]));
        assert!(client.get("build_id").is_none(), "no build to report");
        assert_eq!(
            payload["builds"]["server"]["target_count"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn api_status_rejects_an_unknown_profile() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        let (status, payload) = api_status(&ctx, "profile=ghost");
        assert_eq!(status, 400);
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["error"]
            .as_str()
            .is_some_and(|e| e.contains("unknown profile 'ghost'")));
    }

    #[test]
    fn api_tables_lists_visible_tables_with_facts() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        let (status, payload) = api_tables(&ctx, "");
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(true));
        assert_eq!(payload["profile"], serde_json::json!("client"));
        let tables = payload["tables"].as_array().expect("tables array");
        assert_eq!(tables.len(), 1, "the demo project has one table");
        let item = &tables[0];
        assert_eq!(item["name"], serde_json::json!("Item"));
        assert_eq!(item["rows"], serde_json::json!(1));
        assert_eq!(item["primary_key"], serde_json::json!(["id"]));
        assert!(item["source_file"]
            .as_str()
            .is_some_and(|s| s.ends_with("item.json")));

        // Same visibility contract as the CLI: an unknown profile is
        // rejected, not silently defaulted.
        let (status, payload) = api_tables(&ctx, "profile=ghost");
        assert_eq!(status, 400);
        assert_eq!(payload["ok"], serde_json::json!(false));
    }

    #[test]
    fn api_table_paginates_rows_and_hides_hidden_tables() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        // Five rows so pagination has something to paginate — the panel
        // handlers re-read the project from disk, so the source file is
        // what matters.
        let data: Vec<String> = (0..5)
            .map(|i| format!(r#"{{ "id": {i}, "name": "row{i}" }}"#))
            .collect();
        fs::write(
            dir.path().join("config/item.json"),
            format!("{{\"Item\": [{}]}}", data.join(", ")),
        )
        .expect("write source");

        // Missing name is a 400, not a silent empty list.
        let (status, payload) = api_table(&ctx, "");
        assert_eq!(status, 400);
        assert_eq!(payload["ok"], serde_json::json!(false));

        // Ghost table is a 404: hidden by the profile projection or
        // misspelled — either way the panel must not pretend it exists.
        let (status, payload) = api_table(&ctx, "name=Ghost");
        assert_eq!(status, 404);
        assert!(payload["error"]
            .as_str()
            .is_some_and(|e| e.contains("Ghost")));

        // Default page: all five rows, schema column order.
        let (status, payload) = api_table(&ctx, "name=Item");
        assert_eq!(status, 200);
        assert_eq!(payload["total"], serde_json::json!(5));
        assert_eq!(payload["offset"], serde_json::json!(0));
        assert_eq!(payload["columns"], serde_json::json!(["id", "name"]));
        assert_eq!(payload["rows"].as_array().expect("rows").len(), 5);

        // A window in the middle keeps its absolute indexes.
        let (status, payload) = api_table(&ctx, "name=Item&offset=2&limit=2");
        assert_eq!(status, 200);
        assert_eq!(payload["offset"], serde_json::json!(2));
        let rows = payload["rows"].as_array().expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["index"], serde_json::json!(2));
        assert_eq!(rows[0]["cells"]["name"], serde_json::json!("row2"));

        // Offset past the end stays 200 with zero rows (the panel just
        // shows an empty page; it does not error the user out).
        let (status, payload) = api_table(&ctx, "name=Item&offset=50");
        assert_eq!(status, 200);
        assert_eq!(payload["rows"].as_array().expect("rows").len(), 0);
    }

    #[test]
    fn api_build_then_status_reports_fresh_and_up_to_date() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        let (status, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(true));
        assert_eq!(payload["up_to_date"], serde_json::json!(false));
        assert!(payload["artifact_count"].as_u64().unwrap() >= 1);
        assert!(payload["build_id"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(
            fs::exists(dir.path().join("build/client/json/Item.json")).expect("stat"),
            "the json artifact landed"
        );

        // Status now: client fresh with the manifest facts, server stale
        // (the single-slot manifest belongs to client).
        let (_, payload) = api_status(&ctx, "profile=client");
        let client = &payload["builds"]["client"];
        assert_eq!(client["fresh"], serde_json::json!(true));
        assert_eq!(client["last_build_profile"], serde_json::json!("client"));
        assert_eq!(client["changed_tables"], serde_json::json!([]));
        assert_eq!(client["build_id"], payload["builds"]["client"]["build_id"]);
        assert_eq!(
            payload["builds"]["server"]["fresh"],
            serde_json::json!(false)
        );
        assert_eq!(
            payload["builds"]["server"]["last_build_profile"],
            serde_json::json!("client")
        );

        // A second build is a no-op, not a rebuild.
        let (_, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
        assert_eq!(payload["up_to_date"], serde_json::json!(true));
    }

    #[test]
    fn api_status_names_stale_tables_after_a_source_edit() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        api_build(&ctx, r#"{"profile": "client"}"#);

        fs::write(
            dir.path().join("config/item.json"),
            r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
        )
        .expect("edit source");

        let (_, payload) = api_status(&ctx, "profile=client");
        let client = &payload["builds"]["client"];
        assert_eq!(client["fresh"], serde_json::json!(false));
        assert_eq!(client["changed_tables"], serde_json::json!(["Item"]));
    }

    #[test]
    fn api_build_reports_broken_data_as_diagnostics() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        // Row without the required id — L1 validation must refuse the build.
        fs::write(
            dir.path().join("config/item.json"),
            r#"{"Item": [{"name": "Sword"}]}"#,
        )
        .expect("write broken source");

        let (status, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["diagnostics"]
            .as_array()
            .is_some_and(|d| !d.is_empty()));
        assert!(
            !fs::exists(dir.path().join("build/client/json/Item.json")).expect("stat"),
            "nothing built on validation failure"
        );
    }

    #[test]
    fn api_build_rejects_an_unknown_profile() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        let (status, payload) = api_build(&ctx, r#"{"profile": "ghost"}"#);
        assert_eq!(status, 400);
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["error"]
            .as_str()
            .is_some_and(|e| e.contains("unknown profile 'ghost'")));
    }

    #[test]
    fn api_artifacts_lists_manifest_files_for_the_built_profile() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        // Nothing built: an empty list, no manifest_profile.
        let (status, payload) = api_artifacts(&ctx, "profile=client");
        assert_eq!(status, 200);
        assert_eq!(payload["manifest_profile"], serde_json::json!(null));
        assert_eq!(payload["artifacts"], serde_json::json!([]));

        api_build(&ctx, r#"{"profile": "client"}"#);
        let (_, payload) = api_artifacts(&ctx, "profile=client");
        assert_eq!(payload["manifest_profile"], serde_json::json!("client"));
        assert_eq!(payload["fresh"], serde_json::json!(true));
        assert_eq!(payload["stale"], serde_json::json!(false));
        let artifacts = payload["artifacts"].as_array().expect("artifact list");
        assert_ne!(artifacts.len(), 0);
        assert!(artifacts[0]["path"].as_str().is_some());
        assert!(artifacts[0]["hash"].as_str().is_some());
        assert_ne!(artifacts.len(), 0);
        let targets = payload["targets"].as_array().expect("target records");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0]["format"], serde_json::json!("json"));

        // The single-slot manifest belongs to client: server reports the
        // owner and an empty list instead of unverifiable leftovers.
        let (_, payload) = api_artifacts(&ctx, "profile=server");
        assert_eq!(payload["manifest_profile"], serde_json::json!("client"));
        assert_eq!(payload["artifacts"], serde_json::json!([]));
    }

    #[test]
    fn api_artifact_serves_ledgered_bytes_and_refuses_the_rest() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        let (_, payload) = api_build(&ctx, r#"{"profile": "client"}"#);
        drop(payload);

        // A ledgered file serves byte-identical content with a JSON type.
        let (status, mime, bytes) = api_artifact(&ctx, "path=build/client/json/Item.json");
        assert_eq!(status, 200);
        assert_eq!(mime, "application/json; charset=utf-8");
        assert_eq!(
            bytes,
            fs::read(dir.path().join("build/client/json/Item.json")).expect("read artifact")
        );

        // A path outside the manifest is refused outright — lookup, never
        // join-and-pray (path traversal included).
        let (status, _, bytes) = api_artifact(&ctx, "path=schema.yaml");
        assert_eq!(status, 404);
        assert!(String::from_utf8_lossy(&bytes).contains("not in the build manifest"));
        let (status, _, bytes) = api_artifact(&ctx, "path=../cage.toml");
        assert_eq!(status, 404);
        assert!(String::from_utf8_lossy(&bytes).contains("not in the build manifest"));
        let (status, _, _bytes) = api_artifact(&ctx, "");
        assert_eq!(status, 400);

        // A file edited after the build no longer matches its hash: 409,
        // not tampered content.
        fs::write(
            dir.path().join("build/client/json/Item.json"),
            r#"{"hacked":1}"#,
        )
        .expect("tamper");
        let (status, _, bytes) = api_artifact(&ctx, "path=build/client/json/Item.json");
        assert_eq!(status, 409);
        assert!(
            String::from_utf8_lossy(&bytes).contains("no longer matches"),
            "{}",
            String::from_utf8_lossy(&bytes)
        );
    }

    #[test]
    fn api_artifacts_names_stale_manifests() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        api_build(&ctx, r#"{"profile": "client"}"#);

        fs::write(
            dir.path().join("config/item.json"),
            r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
        )
        .expect("edit source");

        let (_, payload) = api_artifacts(&ctx, "profile=client");
        assert_eq!(payload["stale"], serde_json::json!(true));
        assert_eq!(payload["fresh"], serde_json::json!(false));
        // The manifest still lists what the build produced — with the
        // stale badge the page can say "看可以，别当新鲜".
        assert_ne!(payload["artifacts"].as_array().expect("list").len(), 0);
    }

    #[test]
    fn api_snapshot_packs_and_api_snapshots_verifies() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);

        // No snapshots yet — ok with an empty list.
        let (status, payload) = api_snapshots(&ctx);
        assert_eq!(status, 200);
        assert_eq!(payload["ok"], serde_json::json!(true));
        assert_eq!(payload["snapshots"], serde_json::json!([]));

        // Pack one profile: the `cage snapshot` pipeline, self-verified.
        let (status, pack) = api_snapshot(&ctx, r#"{"profile": "client"}"#);
        assert_eq!(status, 200, "{pack}");
        assert_eq!(pack["ok"], serde_json::json!(true));
        let rel = pack["snapshot_dir"].as_str().expect("snapshot dir");
        assert!(
            rel.starts_with("build/snapshot/client-"),
            "pack lands under output_dir/snapshot: {rel}"
        );
        assert!(pack["files"].as_u64().expect("files") >= 3, "{pack}");

        // The listing now shows it, freshly verified.
        let (_, list) = api_snapshots(&ctx);
        let snaps = list["snapshots"].as_array().expect("list");
        assert_eq!(snaps.len(), 1, "{list}");
        assert_eq!(
            snaps[0]["name"],
            serde_json::json!(rel.rsplit('/').next().unwrap())
        );
        assert_eq!(snaps[0]["verified"], serde_json::json!(true));
        assert_eq!(snaps[0]["profile"], serde_json::json!("client"));
        assert_eq!(snaps[0]["build_id"], pack["build_id"]);

        // Unknown profile and broken body refuse.
        let (status, err) = api_snapshot(&ctx, r#"{"profile": "ghost"}"#);
        assert_eq!(status, 400);
        assert_eq!(err["ok"], serde_json::json!(false));
        let (status, err) = api_snapshot(&ctx, "not json");
        assert_eq!(status, 400);
        assert_eq!(err["ok"], serde_json::json!(false));
    }

    #[test]
    fn api_diff_layers_tables_targets_and_files() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        api_build(&ctx, r#"{"profile": "client"}"#);

        // Identical manifests: nothing moved, roots agree.
        let (status, d) = api_diff(&ctx, "from=build&to=build");
        assert_eq!(status, 200);
        assert_eq!(d["ok"], serde_json::json!(true));
        assert_eq!(d["changed_tables"], serde_json::json!([]));
        assert_eq!(d["affected_targets"], serde_json::json!([]));
        assert_eq!(d["files"]["changed"], serde_json::json!([]));
        assert_eq!(
            d["roots"]["content_hash"]["from"],
            d["roots"]["content_hash"]["to"]
        );

        // Baseline manifest aside, then edit the source and rebuild.
        fs::copy(
            dir.path().join("build/manifest.json"),
            dir.path().join("baseline.json"),
        )
        .expect("keep baseline");
        fs::write(
            dir.path().join("config/item.json"),
            r#"{"Item": [{"id": 1, "name": "Axe"}]}"#,
        )
        .expect("edit source");
        api_build(&ctx, r#"{"profile": "client"}"#);

        let (status, d) = api_diff(&ctx, "from=baseline.json&to=build");
        assert_eq!(status, 200, "{d}");
        assert_eq!(
            d["changed_tables"],
            serde_json::json!(["Item"]),
            "the table fingerprint moved"
        );
        assert_eq!(
            d["affected_targets"],
            serde_json::json!(["build/client/json"]),
            "the target under the changed file"
        );
        let changed = d["files"]["changed"].as_array().expect("changed");
        assert_eq!(changed.len(), 1, "{d}");
        assert_eq!(
            changed[0]["path"],
            serde_json::json!("build/client/json/Item.json")
        );
        assert_eq!(changed[0]["table"], serde_json::json!("Item"));
        assert_eq!(changed[0]["target"], serde_json::json!("build/client/json"));
        assert_ne!(
            d["roots"]["content_hash"]["from"],
            d["roots"]["content_hash"]["to"]
        );
        assert_ne!(d["roots"]["build_id"]["from"], d["roots"]["build_id"]["to"]);

        // Missing parameters and missing manifests refuse with 400.
        let (status, err) = api_diff(&ctx, "from=build");
        assert_eq!(status, 400);
        assert_eq!(err["ok"], serde_json::json!(false));
        let (status, err) = api_diff(&ctx, "from=ghost/manifest.json&to=build");
        assert_eq!(status, 400);
        assert!(err["error"].as_str().expect("msg").contains("baseline"));
    }

    #[test]
    fn api_diff_rejects_absolute_paths_and_parent_traversal() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = demo_ctx(&dir);
        api_build(&ctx, r#"{"profile": "client"}"#);

        let absolute = format!("from={}&to=build", dir.path().join("build").display());
        let (status, err) = api_diff(&ctx, &absolute);
        assert_eq!(status, 400);
        assert!(err["error"]
            .as_str()
            .unwrap()
            .contains("normalized project-relative path"));

        let (status, err) = api_diff(&ctx, "from=../build&to=build");
        assert_eq!(status, 400);
        assert!(err["error"]
            .as_str()
            .unwrap()
            .contains("normalized project-relative path"));
    }

    #[test]
    fn api_registry_browses_the_configured_registry() {
        let dir = TempDir::new().expect("tempdir");
        let mut ctx = demo_ctx(&dir);

        // Unconfigured: ok with an empty list and a note.
        let (status, reg) = api_registry(&ctx);
        assert_eq!(status, 200);
        assert_eq!(reg["ok"], serde_json::json!(true));
        assert_eq!(reg["packages"], serde_json::json!([]));
        assert!(reg["note"].as_str().expect("note").contains("registry"));

        // Configure [registry], pack a snapshot, publish it for real, and
        // the browse endpoint lists the package.
        ctx.config.registry = Some(cage_core::manifest::RegistryConfig {
            path: "reg".to_string(),
            auth_env: None,
        });
        let (_, pack) = api_snapshot(&ctx, r#"{"profile": "client"}"#);
        let snap_rel = pack["snapshot_dir"].as_str().expect("snapshot dir");
        cage_core::registry::publish(
            &dir.path().join("reg"),
            "demo",
            "0.1.0",
            &dir.path().join(snap_rel),
        )
        .expect("publish");

        let (status, reg) = api_registry(&ctx);
        assert_eq!(status, 200);
        assert_eq!(reg["ok"], serde_json::json!(true));
        assert_eq!(reg["registry"], serde_json::json!("reg"));
        let packages = reg["packages"].as_array().expect("packages");
        assert_eq!(packages.len(), 1, "{reg}");
        assert_eq!(packages[0]["package"], serde_json::json!("demo"));
        assert_eq!(
            packages[0]["entries"][0]["version"],
            serde_json::json!("0.1.0")
        );
    }
    /// Reader that returns one byte per call, sleeping in between — the
    /// drip-feed a cross-origin page would use to pin the serve loop.
    struct DripReader {
        left: u32,
        delay: std::time::Duration,
    }
    impl std::io::Read for DripReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Ok(0);
            }
            std::thread::sleep(self.delay);
            self.left -= 1;
            buf[0] = b'x';
            Ok(1)
        }
    }
    struct TimedOutReader;
    impl std::io::Read for TimedOutReader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "read timed out",
            ))
        }
    }

    /// The body cap and the read budget are both enforced
    /// (review 2026-10-18 表一#2).
    #[test]
    fn body_reader_limits_size_and_read_budget() {
        // Under both limits → fine
        let mut r = std::io::Cursor::new(b"{}".to_vec());
        let got = read_body_with(
            &mut r,
            16,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        );
        assert_eq!(got, Ok("{}".to_string()));

        // Over the byte cap → TooLarge (checked as soon as it overflows)
        let mut r = std::io::Cursor::new(vec![b'x'; 33]);
        assert_eq!(
            read_body_with(
                &mut r,
                16,
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            ),
            Err(BodyRead::TooLarge)
        );

        // Drip-feed past the wall-clock budget → TooSlow even under the cap
        let mut r = DripReader {
            left: 64,
            delay: std::time::Duration::from_millis(10),
        };
        assert_eq!(
            read_body_with(
                &mut r,
                1024,
                std::time::Instant::now() + std::time::Duration::from_millis(50)
            ),
            Err(BodyRead::TooSlow)
        );

        // The socket-level tiny_http read timeout also surfaces as the
        // bounded-time response rather than a generic malformed-body error.
        assert_eq!(
            read_body_with(
                &mut TimedOutReader,
                1024,
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            ),
            Err(BodyRead::TooSlow)
        );
    }

    /// The connection-level timeout reaches the real request body reader:
    /// an idle client receives 408 instead of pinning the web handler.
    #[test]
    fn stalled_body_socket_timeout_returns_408() {
        let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
        let port = server.server_addr().to_ip().expect("addr").port();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("client read timeout");
        write!(
            client,
            "POST /api/validate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1\r\nConnection: close\r\n\r\n"
        )
        .expect("send headers without body");

        // tiny_http reads small request bodies while parsing headers, so its
        // parser itself returns the timeout response before a Request reaches
        // the application handler.
        let mut status = [0u8; 12];
        client
            .read_exact(&mut status)
            .expect("read timeout response");
        assert_eq!(&status, b"HTTP/1.1 408");
    }

    /// The loopback boundary: foreign Host, missing Host, and cross-origin
    /// writes are refused; local Host with or without Origin passes
    /// (review 2026-10-18 表一#2). Drives real requests through `handle`
    /// over a socket, the same way the unreadable-body test does.
    #[test]
    fn local_boundary_checks_host_and_origin() {
        fn round_trip(
            server: &tiny_http::Server,
            ctx: &Ctx,
            port: u16,
            method: &str,
            path: &str,
            host: Option<&str>,
            origin: Option<&str>,
        ) -> String {
            let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
            client
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("read timeout");
            let mut head = format!("{method} {path} HTTP/1.1\r\n");
            if let Some(h) = host {
                head.push_str("Host: ");
                head.push_str(h);
                head.push_str("\r\n");
            }
            if let Some(o) = origin {
                head.push_str("Origin: ");
                head.push_str(o);
                head.push_str("\r\n");
            }
            head.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
            client.write_all(head.as_bytes()).expect("send head");
            client
                .shutdown(std::net::Shutdown::Write)
                .expect("shutdown");

            let request = server.incoming_requests().next().expect("request");
            handle(request, ctx);

            let mut reply = Vec::new();
            client.read_to_end(&mut reply).expect("read reply");
            String::from_utf8(reply).expect("utf8")
        }

        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, None);
        let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
        let port = server.server_addr().to_ip().expect("addr").port();

        // Local host, any loopback spelling, GET → past the boundary
        // (the schema endpoint answers 500 with no schema configured —
        // anything but 403 means the boundary let it through)
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            let out = round_trip(
                &server,
                &ctx,
                port,
                "GET",
                "/api/schema",
                Some(&format!("{host}:{port}")),
                None,
            );
            assert!(!out.contains("403"), "host {host} refused: {out}");
        }

        // Foreign host (DNS rebinding) and missing host (HTTP/1.0) → 403
        assert!(round_trip(
            &server,
            &ctx,
            port,
            "GET",
            "/api/schema",
            Some("evil.example"),
            None
        )
        .contains("403"));
        assert!(round_trip(
            &server,
            &ctx,
            port,
            "GET",
            "/api/schema",
            Some(&format!("evil.example:{port}")),
            None
        )
        .contains("403"));
        assert!(round_trip(&server, &ctx, port, "GET", "/api/schema", None, None).contains("403"));

        // Writes: no Origin (curl) or loopback Origin → past the boundary
        // (empty body fails validation with ok:false, status 200)
        let out = round_trip(
            &server,
            &ctx,
            port,
            "POST",
            "/api/validate",
            Some(&format!("127.0.0.1:{port}")),
            None,
        );
        assert!(!out.contains("403"), "originless POST refused: {out}");
        for host in ["127.0.0.1", "localhost"] {
            let out = round_trip(
                &server,
                &ctx,
                port,
                "POST",
                "/api/validate",
                Some(&format!("{host}:{port}")),
                Some(&format!("http://{host}:{port}")),
            );
            assert!(!out.contains("403"), "local origin {host} refused: {out}");
        }

        // Cross-origin write → 403 (this is the CSRF surface); the
        // sandboxed/null origin is not a loopback origin either
        assert!(round_trip(
            &server,
            &ctx,
            port,
            "POST",
            "/api/validate",
            Some(&format!("127.0.0.1:{port}")),
            Some("https://evil.example")
        )
        .contains("403"));
        assert!(round_trip(
            &server,
            &ctx,
            port,
            "POST",
            "/api/validate",
            Some(&format!("127.0.0.1:{port}")),
            Some("null")
        )
        .contains("403"));
    }
}

#[cfg(test)]
impl Ctx {
    /// Drive [`handle`] over a real bound socket: the test acts as the HTTP
    /// client, so the routing, response helpers, and asset endpoints get the
    /// same coverage the smoke script gives `run_web` — inside `cargo test`.
    fn served(self, count: usize) -> (std::thread::JoinHandle<()>, u16) {
        let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("bind ephemeral port");
        let port = server.server_addr().to_ip().expect("bound").port();
        let driver = std::thread::spawn(move || {
            for request in server.incoming_requests().take(count) {
                handle(request, &self);
            }
        });
        (driver, port)
    }
}

#[cfg(test)]
mod handle_tests {
    use super::tests::{ctx_in, good_doc};
    use super::{api_save, api_schema, handle, run_web};
    use std::fs;
    use std::io::{Read, Write};
    use tempfile::TempDir;

    fn send(port: u16, request: &str) -> String {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.write_all(request.as_bytes()).expect("write request");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read response");
        String::from_utf8_lossy(&raw).into_owned()
    }

    fn get(port: u16, path: &str) -> String {
        send(
            port,
            &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
        )
    }

    fn post(port: u16, path: &str, body: &str) -> String {
        send(
            port,
            &format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    #[test]
    fn handle_routes_the_api_pages_and_assets() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("write schema");
        let (driver, port) = ctx_in(&dir, Some("schema.yaml")).served(8);

        // API: schema document, validate, and save over the wire.
        let raw = get(port, "/api/schema");
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("\"project\":\"demo\""), "{raw}");
        assert!(raw.contains("application/json"), "{raw}");

        let raw = post(port, "/api/validate", &good_doc());
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("\"ok\":true"), "{raw}");
        assert!(raw.contains("\"diagnostics\":[]"), "{raw}");

        let raw = post(port, "/api/schema", &good_doc());
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("\"ok\":true"), "{raw}");
        assert!(
            fs::read_to_string(dir.path().join("schema.yaml"))
                .expect("saved file")
                .contains("tables:"),
            "save endpoint wrote the canonical YAML"
        );

        // Editor page and assets, then the two refusal routes.
        assert!(get(port, "/").contains("text/html"), "index page");
        assert!(
            get(port, "/app.js").contains("text/javascript"),
            "app.js asset"
        );
        assert!(get(port, "/app.css").contains("text/css"), "app.css asset");
        assert!(get(port, "/favicon.ico").starts_with("HTTP/1.1 404"));
        let raw = get(port, "/nope");
        assert!(raw.starts_with("HTTP/1.1 404"), "{raw}");
        assert!(raw.contains("not found"), "{raw}");

        driver.join().expect("driver thread");
    }

    #[test]
    fn run_web_rejects_without_a_project_config() {
        let dir = TempDir::new().expect("tempdir");
        let err = run_web(dir.path(), 1).unwrap_err();
        assert!(!err.contains("cannot bind"), "{err}");
    }

    #[test]
    fn run_web_rejects_when_the_port_is_taken() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("cage.toml"),
            r#"[project]
name = "demo"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#,
        )
        .expect("write config");
        // Hold the port: the bind must fail before the serve loop starts.
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = held.local_addr().expect("addr").port();
        let err = run_web(dir.path(), port).unwrap_err();
        assert!(
            err.contains(&format!("cannot bind 127.0.0.1:{port}:")),
            "{err}"
        );
    }

    #[test]
    fn api_schema_reports_a_missing_schema_file() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, Some("absent.yaml"));
        let (status, doc) = api_schema(&ctx, "");
        assert_eq!(status, 200);
        assert_eq!(doc["ok"], serde_json::json!(false));
        assert!(doc["schema"].is_null(), "{doc}");
    }

    #[test]
    #[cfg(unix)]
    fn api_save_reports_an_unwritable_target() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("schema.yaml"), "tables: {}\nenums: {}\n").expect("write schema");
        let file = dir.path().join("schema.yaml");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).expect("chmod");
        let ctx = ctx_in(&dir, Some("schema.yaml"));

        let (status, payload) = api_save(&ctx, &good_doc());
        assert_eq!(status, 500, "{payload}");
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["error"].as_str().unwrap().contains("cannot write"));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("chmod back");
    }

    /// POST body that is not valid UTF-8: the body read fails and the
    /// handler must answer 400 instead of wedging or panicking.
    #[test]
    fn handle_reports_an_unreadable_post_body() {
        let dir = TempDir::new().expect("tempdir");
        let ctx = ctx_in(&dir, None);
        let server = tiny_http::Server::http(("127.0.0.1", 0)).expect("server");
        let port = server.server_addr().to_ip().expect("addr").port();

        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("read timeout");
        // Invalid UTF-8 bytes: the body read rejects them mid-body.
        let body = [0xFFu8, 0xFE, 0x80, 0x81, 0x82];
        let head = format!(
            "POST /api/validate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        client.write_all(head.as_bytes()).expect("send head");
        client.write_all(&body).expect("send body");
        client
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");

        let request = server.incoming_requests().next().expect("request");
        handle(request, &ctx);

        let mut reply = Vec::new();
        client.read_to_end(&mut reply).expect("read reply");
        let out = String::from_utf8(reply).expect("utf8");
        assert!(out.contains("400"), "{out}");
        assert!(out.contains("cannot read request body"), "{out}");
    }
}
