//! Versioned, language-agnostic JSON IPC protocol for Cage validation plugins.
//!
//! This crate is the *wire contract* for out-of-process validation plugins
//! (design §17 / review `2026-10-22-schema-workbench-python-plugins.md` §四).
//! It deliberately carries only plain data (`serde_json::Value` for the
//! schema/options payload) so that any host language — Rust, Python, JS, Go —
//! can speak the same protocol without sharing types.
//!
//! ## Framing (MVP)
//!
//! A plugin run is a single request/response exchange over the worker's
//! stdin/stdout:
//!
//! * The host writes exactly one compact UTF-8 JSON object followed by `\n`
//!   to the worker's stdin, then closes it.
//! * The worker reads all of stdin (one request), runs the plugin, and writes
//!   exactly one compact UTF-8 JSON object followed by `\n` to stdout, then
//!   exits.
//!
//! The MVP is deliberately single-shot: no keep-alive, no streaming, no
//! batching. A persistent worker would layer its own framing on top of the
//! same message types. Host-side process lifecycle (spawn, timeout, crash
//! recovery) is out of scope here — see the P2-2 failure-behaviour table.
//!
//! ## Versioning
//!
//! Every message carries a `protocol` integer. A worker that receives a
//! request whose `protocol` differs from [`PROTOCOL_VERSION`] MUST respond
//! with an error whose `code` is [`ERR_PROTOCOL_VERSION_MISMATCH`] and exit
//! non-zero; it MUST NOT attempt to interpret the message.
//!
//! ## Methods
//!
//! The MVP exposes exactly one method, [`METHOD_RUN`] (`"run"`). It carries
//! the whole validated input — schema, normalized tables, and plugin options
//! — so the plugin is a pure function
//!
//! ```text
//! Diagnostics = f(Schema, Data, Options)
//! ```
//!
//! and never parses source files or re-implements type coercion itself
//! (review §二: "Python 插件不应该自行打开 Monster.xlsx").

use serde::{Deserialize, Serialize};

/// Wire protocol version. Bump on any breaking change to the message shapes.
pub const PROTOCOL_VERSION: u32 = 1;

/// The single method the MVP protocol supports.
pub const METHOD_RUN: &str = "run";

/// Default diagnostic code when a plugin emits a diagnostic without one.
/// `E1601` is the gamerule "custom validator plugin returned error" code.
pub const DEFAULT_DIAGNOSTIC_CODE: &str = "E1601";

/// Request carried a `protocol` that this implementation does not support.
pub const ERR_PROTOCOL_VERSION_MISMATCH: &str = "PROTOCOL_VERSION_MISMATCH";
/// Request was not parseable as a [`Request`] (bad JSON, missing fields).
pub const ERR_PROTOCOL_MALFORMED: &str = "PROTOCOL_MALFORMED";
/// Request named a method the protocol does not implement.
pub const ERR_PROTOCOL_UNKNOWN_METHOD: &str = "PROTOCOL_UNKNOWN_METHOD";
/// The plugin raised an exception (uncaught in its `validate`).
pub const ERR_PLUGIN_ERROR: &str = "PLUGIN_ERROR";
/// The plugin produced output that violates the protocol shape.
pub const ERR_PLUGIN_INVALID_RESULT: &str = "PLUGIN_INVALID_RESULT";

/// Identity of the plugin being run, taken from the project configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Stable plugin id (e.g. `monster_balance`).
    pub id: String,
    /// Plugin version declared by its author (e.g. `1.2.0`).
    pub version: String,
}

/// A normalized table handed to the plugin. Values are already coerced to
/// the schema's declared types by Cage Core; the plugin never reads sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TablePayload {
    /// Table name.
    pub name: String,
    /// The schema's declared primary-key field names (empty if none).
    #[serde(default)]
    pub primary_key: Vec<String>,
    /// Rows in document order.
    #[serde(default)]
    pub rows: Vec<RowPayload>,
}

/// One row of a [`TablePayload`], with enough position metadata for the host
/// to map plugin diagnostics back to a source location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowPayload {
    /// 0-based row index within the table.
    pub index: u64,
    /// Human-readable source location of the row (e.g.
    /// `Monster.xlsx#Sheet1!A2`) when the source recorded one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Field name → value, already normalized to JSON.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// Parameters of the `run` method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunParams {
    /// The plugin identity the host is asking to run.
    pub plugin: PluginInfo,
    /// The canonical Schema as JSON (the same shape the editor stores).
    pub schema: serde_json::Value,
    /// Normalized tables the plugin may read.
    #[serde(default)]
    pub tables: Vec<TablePayload>,
    /// Project-configuration options for this plugin (read-only for the
    /// plugin; the host never lets it write back).
    #[serde(default)]
    pub options: serde_json::Value,
}

/// Severity of a plugin diagnostic. Serialized lowercase (`error`,
/// `warning`, `info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A hard violation that fails validation.
    Error,
    /// A soft finding reported but not fatal.
    Warning,
    /// An informational notice.
    Info,
}

/// A diagnostic produced by a plugin. The host maps `table`/`row`/`field`
/// back to a concrete source location in Cage's own diagnostic model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireDiagnostic {
    /// Severity of the finding.
    pub severity: Severity,
    /// Machine code; defaults to [`DEFAULT_DIAGNOSTIC_CODE`] when omitted.
    #[serde(default = "default_diagnostic_code")]
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Table the finding applies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// 0-based row index within the table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<u64>,
    /// Field name within the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Optional actionable hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// A protocol-level failure (as opposed to a plugin *finding*). The `code`
/// is one of the stable `ERR_*` constant family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Stable machine code from the `ERR_*` constant family.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Host → worker request. One per process in the MVP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Must equal [`PROTOCOL_VERSION`].
    pub protocol: u32,
    /// Opaque correlation id, echoed verbatim on the response.
    pub id: String,
    /// Must equal [`METHOD_RUN`].
    pub method: String,
    /// Method parameters.
    pub params: RunParams,
}

/// Worker → host response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Echoes the request's `protocol`.
    pub protocol: u32,
    /// Echoes the request's `id`.
    pub id: String,
    /// `true` when the plugin ran to completion; `false` on protocol or
    /// plugin failure (carried in [`Response::error`]).
    pub ok: bool,
    /// Plugin findings, present when `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<WireDiagnostic>>,
    /// Failure detail, present when `!ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

/// Why a request could not be accepted. The worker maps each variant to a
/// [`Response`] with `ok: false` (see [`ProtocolError`] constructors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// The input was not valid UTF-8 JSON or did not deserialize into a
    /// [`Request`] shape.
    Malformed(String),
    /// The request's `protocol` is not [`PROTOCOL_VERSION`].
    VersionMismatch {
        /// The protocol the request declared.
        request_protocol: u32,
        /// The protocol this implementation speaks.
        supported: u32,
    },
    /// The request named a method other than [`METHOD_RUN`].
    UnknownMethod(String),
}

impl Request {
    /// Parse a single request from raw bytes.
    ///
    /// The input is first parsed as a [`serde_json::Value`] so the `protocol`
    /// field can be checked before the rest of the shape is validated — a
    /// version mismatch is reported as such even if the message is otherwise
    /// well-formed. `serde_json`'s parser caps nesting at 128 levels, so a
    /// hostile deep payload is rejected here rather than recursing freely.
    pub fn parse(input: &[u8]) -> Result<Self, RequestError> {
        let text =
            std::str::from_utf8(input).map_err(|e| RequestError::Malformed(e.to_string()))?;
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| RequestError::Malformed(e.to_string()))?;
        let protocol = value
            .get("protocol")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                RequestError::Malformed("missing or non-integer 'protocol' field".to_string())
            })?;
        if protocol != u64::from(PROTOCOL_VERSION) {
            return Err(RequestError::VersionMismatch {
                request_protocol: protocol as u32,
                supported: PROTOCOL_VERSION,
            });
        }
        let request: Self = serde_json::from_value(value)
            .map_err(|e| RequestError::Malformed(format!("invalid request: {e}")))?;
        if request.method != METHOD_RUN {
            return Err(RequestError::UnknownMethod(request.method.clone()));
        }
        Ok(request)
    }

    /// Convenience: a `run` request with `id` and `params`.
    pub fn run(id: impl Into<String>, params: RunParams) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            id: id.into(),
            method: METHOD_RUN.to_string(),
            params,
        }
    }

    /// Serialize to the wire form: compact JSON + trailing `\n`.
    ///
    /// The trailing newline is part of the framing (one request per line), so a
    /// request is a complete message with no length header.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes =
            serde_json::to_vec(self).expect("protocol requests are always serializable");
        bytes.push(b'\n');
        bytes
    }
}

impl Response {
    /// A successful response carrying plugin findings.
    pub fn ok(id: impl Into<String>, diagnostics: Vec<WireDiagnostic>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            id: id.into(),
            ok: true,
            diagnostics: Some(diagnostics),
            error: None,
        }
    }

    /// A failed response carrying a protocol/plugin error.
    pub fn err(id: impl Into<String>, error: ProtocolError) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            id: id.into(),
            ok: false,
            diagnostics: None,
            error: Some(error),
        }
    }

    /// Serialize to the wire form: compact JSON + trailing `\n`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes =
            serde_json::to_vec(self).expect("protocol responses are always serializable");
        bytes.push(b'\n');
        bytes
    }
}

impl ProtocolError {
    /// The request's `protocol` was not supported.
    pub fn version_mismatch(request_protocol: u32, supported: u32) -> Self {
        Self {
            code: ERR_PROTOCOL_VERSION_MISMATCH.to_string(),
            message: format!(
                "plugin protocol version {request_protocol} is not supported (host speaks {supported})"
            ),
        }
    }

    /// The request could not be parsed.
    pub fn malformed(detail: &str) -> Self {
        Self {
            code: ERR_PROTOCOL_MALFORMED.to_string(),
            message: format!("malformed plugin request: {detail}"),
        }
    }

    /// The request named an unknown method.
    pub fn unknown_method(method: &str) -> Self {
        Self {
            code: ERR_PROTOCOL_UNKNOWN_METHOD.to_string(),
            message: format!("unknown plugin method '{method}'"),
        }
    }

    /// The plugin raised an exception.
    pub fn plugin_error(message: &str) -> Self {
        Self {
            code: ERR_PLUGIN_ERROR.to_string(),
            message: message.to_string(),
        }
    }

    /// The plugin produced output outside the protocol shape.
    pub fn invalid_result(message: &str) -> Self {
        Self {
            code: ERR_PLUGIN_INVALID_RESULT.to_string(),
            message: message.to_string(),
        }
    }
}

impl WireDiagnostic {
    /// An error-severity finding. Convenience for tests and host conversion.
    pub fn error(
        message: impl Into<String>,
        table: Option<&str>,
        row: Option<u64>,
        field: Option<&str>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            code: default_diagnostic_code(),
            message: message.into(),
            table: table.map(str::to_string),
            row,
            field: field.map(str::to_string),
            hint: None,
        }
    }
}

fn default_diagnostic_code() -> String {
    DEFAULT_DIAGNOSTIC_CODE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_request() -> Request {
        Request::run(
            "run-0001",
            RunParams {
                plugin: PluginInfo {
                    id: "monster_balance".to_string(),
                    version: "1.2.0".to_string(),
                },
                schema: json!({"name": "Monster", "tables": []}),
                tables: vec![TablePayload {
                    name: "Monster".to_string(),
                    primary_key: vec!["id".to_string()],
                    rows: vec![RowPayload {
                        index: 0,
                        location: Some("Monster.xlsx#Sheet1!A2".to_string()),
                        fields: {
                            let mut m = serde_json::Map::new();
                            m.insert("id".to_string(), json!(1));
                            m.insert("level".to_string(), json!(3));
                            m
                        },
                    }],
                }],
                options: json!({"base_cost": 100, "growth": 1.2}),
            },
        )
    }

    #[test]
    fn request_round_trips_through_the_wire() {
        let original = sample_request();
        let bytes = serde_json::to_vec(&original).unwrap();
        let parsed = Request::parse(&bytes).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn response_round_trips_through_the_wire() {
        let ok = Response::ok(
            "run-0001",
            vec![WireDiagnostic::error(
                "level exceeds curve",
                Some("Monster"),
                Some(1),
                Some("level"),
            )],
        );
        let bytes = ok.to_bytes();
        assert!(bytes.ends_with(b"\n"));
        let parsed: Response = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(parsed, ok);
        assert!(parsed.ok);
        assert_eq!(parsed.diagnostics.unwrap().len(), 1);

        let err = Response::err("run-0001", ProtocolError::plugin_error("boom"));
        let parsed: Response = serde_json::from_slice(&err.to_bytes()[..]).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.error.unwrap().code, ERR_PLUGIN_ERROR);
    }

    #[test]
    fn request_parse_rejects_a_version_mismatch_before_shape() {
        // Well-formed otherwise, but declares protocol 2.
        let mut req = sample_request();
        req.protocol = 2;
        let bytes = serde_json::to_vec(&req).unwrap();
        match Request::parse(&bytes).unwrap_err() {
            RequestError::VersionMismatch {
                request_protocol,
                supported,
            } => {
                assert_eq!(request_protocol, 2);
                assert_eq!(supported, PROTOCOL_VERSION);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn request_parse_rejects_malformed_json() {
        match Request::parse(b"{not json").unwrap_err() {
            RequestError::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn request_parse_rejects_missing_protocol() {
        let bytes = br#"{"id":"x","method":"run","params":{}}"#;
        match Request::parse(bytes).unwrap_err() {
            RequestError::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn request_parse_rejects_an_unknown_method() {
        let mut req = sample_request();
        req.method = "compile".to_string();
        let bytes = serde_json::to_vec(&req).unwrap();
        match Request::parse(&bytes).unwrap_err() {
            RequestError::UnknownMethod(m) => assert_eq!(m, "compile"),
            other => panic!("expected UnknownMethod, got {other:?}"),
        }
    }

    #[test]
    fn request_parse_rejects_a_deep_payload() {
        // serde_json caps nesting at 128; a hostile deep schema must be
        // rejected as Malformed rather than recursing freely on the host.
        let mut deep = serde_json::Value::Null;
        for _ in 0..200 {
            deep = json!([deep]);
        }
        let body = json!({"protocol": 1, "id": "x", "method": "run", "params": {
            "plugin": {"id": "p", "version": "1"},
            "schema": deep,
        }});
        assert!(Request::parse(body.to_string().as_bytes()).is_err());
    }

    #[test]
    fn diagnostic_code_defaults_to_e1601() {
        let d: WireDiagnostic = serde_json::from_value(json!({
            "severity": "error", "message": "boom"
        }))
        .unwrap();
        assert_eq!(d.code, DEFAULT_DIAGNOSTIC_CODE);
        assert_eq!(d.code, "E1601");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.table, None);
    }

    #[test]
    fn severity_serializes_lowercase_and_parses_back() {
        for (sev, s) in [
            (Severity::Error, "error"),
            (Severity::Warning, "warning"),
            (Severity::Info, "info"),
        ] {
            let v = serde_json::to_value(sev).unwrap();
            assert_eq!(v.as_str().unwrap(), s);
            assert_eq!(serde_json::from_value::<Severity>(v).unwrap(), sev);
        }
    }

    #[test]
    fn protocol_error_constructors_carry_stable_codes() {
        assert_eq!(
            ProtocolError::version_mismatch(2, 1).code,
            ERR_PROTOCOL_VERSION_MISMATCH
        );
        assert_eq!(ProtocolError::malformed("x").code, ERR_PROTOCOL_MALFORMED);
        assert_eq!(
            ProtocolError::unknown_method("x").code,
            ERR_PROTOCOL_UNKNOWN_METHOD
        );
        assert_eq!(ProtocolError::plugin_error("x").code, ERR_PLUGIN_ERROR);
        assert_eq!(
            ProtocolError::invalid_result("x").code,
            ERR_PLUGIN_INVALID_RESULT
        );
        let msg = ProtocolError::version_mismatch(2, 1).message;
        assert!(msg.contains("2") && msg.contains("1"));
    }

    #[test]
    fn request_run_constructor_is_stable() {
        let req = Request::run(
            "a",
            RunParams {
                plugin: PluginInfo {
                    id: "p".into(),
                    version: "1".into(),
                },
                schema: json!({}),
                tables: vec![],
                options: json!({}),
            },
        );
        assert_eq!(req.protocol, PROTOCOL_VERSION);
        assert_eq!(req.method, METHOD_RUN);
        assert_eq!(req.id, "a");
    }
}
