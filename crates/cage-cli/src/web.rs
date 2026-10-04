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
//! GET  /                → landing page (W3 ships the real editor)
//! ```
//!
//! The frontend never parses or renders YAML — `cage_core::edit` does the
//! YAML↔JSON mapping, the server only moves bytes.

use crate::load_project_config;
use crate::load_schema;
use cage_core::diagnostics::Diagnostics;
use cage_core::edit::{from_editor_json, to_canonical_yaml, to_editor_json};
use cage_core::manifest::ProjectConfig;
use cage_core::schema::Schema;
use std::path::{Path, PathBuf};

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
        (tiny_http::Method::Get, "") => html_response(200, landing_page()),
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

fn html_response(status: u16, html: String) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(html)
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                .expect("static header"),
        )
}

/// GET /api/schema — merged schema as the editor document plus the project
/// facts the frontend needs (profiles, write-back target).
fn api_schema(ctx: &Ctx) -> serde_json::Value {
    let schema = match &ctx.config.schema_path {
        Some(rel) => match load_schema(&ctx.root.join(rel)) {
            Ok(s) => s,
            Err(e) => {
                return serde_json::json!({"ok": false, "error": e});
            }
        },
        None => Schema::new(),
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
/// names. Multi-file schemas (a `schema_path` pointing at a directory) are
/// an author-side organization the editor does not rewrite — the client
/// gets a 409 with instructions. With no `schema_path` configured the
/// document is written to `schema.yaml` and the response reminds the user
/// to wire it into the config; the server never rewrites `cage.toml`.
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

/// Landing page — the W3 editor replaces this; the API contract is
/// already final.
fn landing_page() -> String {
    r#"<!DOCTYPE html>
<html lang="zh">
<head><meta charset="utf-8"><title>cage web</title></head>
<body>
<h1>cage web</h1>
<p>Schema 编辑器 API 服务已启动（W3 将在此提供编辑器界面）。</p>
<pre>GET  /api/schema    → 合并 Schema 的编辑器文档
POST /api/validate  → 编辑态校验（E1701 / E1004 诊断）
POST /api/schema    → 保存回环 canonical YAML</pre>
</body>
</html>"#
        .to_string()
}

fn json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).expect("api response serializes")
}
