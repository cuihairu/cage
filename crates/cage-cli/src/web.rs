//! `cage web` — local HTTP service for the Schema editor (third phase W2).
//!
//! A local tool: binds `127.0.0.1` only, no authentication. The API
//! exchanges the same documents as [`cage_core::edit`] — the frontend
//! edits the canonical editor JSON, the server validates it and writes
//! schemas back as canonical YAML:
//!
//! ```text
//! GET  /api/schema      → editor document of the merged schema (+ project)
//! POST /api/validate    → E1701 / E1004 diagnostics for an edited document
//! POST /api/schema      → save the canonical YAML (single-file schema only)
//! GET  /                → the Schema editor page (W3, embedded below)
//! GET  /app.js, /app.css→ editor assets
//! ```
//!
//! The frontend never parses or renders YAML — `cage_core::edit` does the
//! YAML↔JSON mapping, the server only moves bytes.
//!
//! The editor page is compiled into the binary from the `docs/public/editor/`
//! sources (the single authoring copy; the docs site serves them too), so
//! the shipped `cage` binary carries its editor — no Node toolchain.

use crate::load_project_config;
use crate::load_schema_for_config;
use cage_core::diagnostics::Diagnostics;
use cage_core::edit::{from_editor_json, to_canonical_yaml, to_editor_json};
use cage_core::manifest::ProjectConfig;
use std::path::{Path, PathBuf};

/// W3 editor page and assets, embedded at compile time from the docs-site
/// copy of the sources (include! resolves relative to this file).
const EDITOR_INDEX: &str = include_str!("../../../docs/public/editor/index.html");
const EDITOR_APP_JS: &str = include_str!("../../../docs/public/editor/app.js");
const EDITOR_APP_CSS: &str = include_str!("../../../docs/public/editor/app.css");

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

fn handle(mut request: tiny_http::Request, ctx: &Ctx) {
    let method = request.method().clone();
    let url = request.url().to_string();

    let mut body = String::new();
    if matches!(method, tiny_http::Method::Post) {
        if let Err(e) = request.as_reader().read_to_string(&mut body) {
            let _ = request.respond(json_response(
                400,
                &serde_json::json!({"ok": false, "error": format!("cannot read request body: {e}")}),
            ));
            return;
        }
    }

    let response = match (method, url.trim_end_matches('/')) {
        (tiny_http::Method::Get, "/api/schema") => json_response(200, &api_schema(ctx)),
        (tiny_http::Method::Post, "/api/validate") => json_response(200, &api_validate(&body)),
        (tiny_http::Method::Post, "/api/schema") => {
            let (status, value) = api_save(ctx, &body);
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
        (tiny_http::Method::Get, "/favicon.ico") => html_response(404, "not found".to_string()),
        _ => json_response(404, &serde_json::json!({"ok": false, "error": "not found"})),
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

/// GET /api/schema — merged schema as the editor document plus the project
/// facts the frontend needs (profiles, write-back target).
fn api_schema(ctx: &Ctx) -> serde_json::Value {
    let schema = match load_schema_for_config(&ctx.root, &ctx.config) {
        Ok(s) => s,
        Err(e) => {
            return serde_json::json!({"ok": false, "error": e});
        }
    };
    serde_json::json!({
        "ok": true,
        "project": ctx.config.project.name,
        "schema_path": ctx.config.schema_path,
        "profile_names": ctx.config.profiles.keys().collect::<Vec<_>>(),
        "warnings_as_errors": ctx.config.warnings_as_errors,
        "schema": to_editor_json(&schema),
    })
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

fn json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).expect("api response serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::schema::Schema;
    use std::fs;
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

        let doc = api_schema(&ctx);
        assert_eq!(doc["ok"], serde_json::json!(true));
        assert_eq!(doc["project"], serde_json::json!("demo"));
        assert_eq!(doc["schema_path"], serde_json::json!("schema.yaml"));
        assert!(doc["schema"].is_object(), "editor schema present");
        let profiles = doc["profile_names"].as_array().expect("profiles");
        assert!(profiles.iter().any(|p| p == "client"));

        // No schema_path configured → an empty editor document, still ok.
        let empty = ctx_in(&dir, None);
        let doc = api_schema(&empty);
        assert_eq!(doc["ok"], serde_json::json!(true));
        assert_eq!(doc["schema_path"], serde_json::json!(null));
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
}
