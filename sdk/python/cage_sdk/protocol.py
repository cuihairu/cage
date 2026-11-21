"""Wire contract for the Cage plugin JSON IPC protocol (version 1).

Mirrors ``crates/cage-plugin-protocol`` (the Rust side of the same contract).
One request per worker process: the host writes one compact JSON object to
the worker's stdin and closes it; the worker replies with one compact JSON
object on stdout and exits. See ``docs/plugins.md`` for the full spec.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional

PROTOCOL_VERSION = 1
METHOD_RUN = "run"

#: Default diagnostic code when a plugin emits a diagnostic without one.
#: E1601 is the gamerule "custom validator plugin returned error" code.
DEFAULT_DIAGNOSTIC_CODE = "E1601"

ERR_PROTOCOL_VERSION_MISMATCH = "PROTOCOL_VERSION_MISMATCH"
ERR_PROTOCOL_MALFORMED = "PROTOCOL_MALFORMED"
ERR_PROTOCOL_UNKNOWN_METHOD = "PROTOCOL_UNKNOWN_METHOD"
ERR_PLUGIN_ERROR = "PLUGIN_ERROR"
ERR_PLUGIN_INVALID_RESULT = "PLUGIN_INVALID_RESULT"

VALID_SEVERITIES = ("error", "warning", "info")


class ProtocolError(Exception):
    """A protocol-level failure (as opposed to a plugin *finding*).

    ``code`` is one of the stable ``ERR_*`` constants so the host can map it
    onto its own diagnostic model.
    """

    def __init__(self, code: str, message: str) -> None:
        self.code = code
        self.message = message
        super().__init__(f"{code}: {message}")

    @classmethod
    def version_mismatch(cls, request_protocol: Any, supported: int) -> "ProtocolError":
        return cls(
            ERR_PROTOCOL_VERSION_MISMATCH,
            f"plugin protocol version {request_protocol} is not supported "
            f"(host speaks {supported})",
        )

    @classmethod
    def malformed(cls, detail: str) -> "ProtocolError":
        return cls(ERR_PROTOCOL_MALFORMED, f"malformed plugin request: {detail}")

    @classmethod
    def unknown_method(cls, method: str) -> "ProtocolError":
        return cls(ERR_PROTOCOL_UNKNOWN_METHOD, f"unknown plugin method '{method}'")

    @classmethod
    def plugin_error(cls, message: str) -> "ProtocolError":
        return cls(ERR_PLUGIN_ERROR, message)

    @classmethod
    def invalid_result(cls, message: str) -> "ProtocolError":
        return cls(ERR_PLUGIN_INVALID_RESULT, message)


def parse_request(data: bytes) -> Dict[str, Any]:
    """Parse one raw request payload into a validated request dict.

    Raises :class:`ProtocolError` with a stable code when the payload is not
    valid JSON, declares an unsupported ``protocol``, or is missing required
    fields / names an unknown method. The ``protocol`` check runs before the
    shape check so version mismatches are reported as such.
    """
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ProtocolError.malformed(f"request is not valid UTF-8: {exc}") from exc
    try:
        value = json.loads(text)
    except ValueError as exc:
        raise ProtocolError.malformed(str(exc)) from exc
    if not isinstance(value, dict):
        raise ProtocolError.malformed("request must be a JSON object")
    protocol = value.get("protocol")
    if not isinstance(protocol, int) or isinstance(protocol, bool):
        raise ProtocolError.malformed("missing or non-integer 'protocol' field")
    if protocol != PROTOCOL_VERSION:
        raise ProtocolError.version_mismatch(protocol, PROTOCOL_VERSION)
    request_id = value.get("id")
    if not isinstance(request_id, str):
        raise ProtocolError.malformed("missing or non-string 'id' field")
    method = value.get("method")
    if not isinstance(method, str):
        raise ProtocolError.malformed("missing or non-string 'method' field")
    if method != METHOD_RUN:
        raise ProtocolError.unknown_method(method)
    params = value.get("params")
    if not isinstance(params, dict):
        raise ProtocolError.malformed("missing or non-object 'params' field")
    plugin = params.get("plugin")
    if not isinstance(plugin, dict):
        raise ProtocolError.malformed("missing or non-object 'params.plugin' field")
    if not isinstance(plugin.get("id"), str) or not isinstance(plugin.get("version"), str):
        raise ProtocolError.malformed("'params.plugin' needs string 'id' and 'version'")
    if not isinstance(params.get("schema"), (dict, list)):
        raise ProtocolError.malformed("missing or non-object 'params.schema' field")
    tables = params.get("tables", [])
    if not isinstance(tables, list):
        raise ProtocolError.malformed("'params.tables' must be an array")
    for table in tables:
        if not isinstance(table, dict) or not isinstance(table.get("name"), str):
            raise ProtocolError.malformed("each table needs a string 'name'")
    if not isinstance(params.get("options", {}), (dict, list)):
        raise ProtocolError.malformed("'params.options' must be an object or array")
    return value


def response_ok(request_id: str, diagnostics: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Build a successful response carrying plugin findings."""
    return {
        "protocol": PROTOCOL_VERSION,
        "id": request_id,
        "ok": True,
        "diagnostics": diagnostics,
    }


def response_err(request_id: str, error: ProtocolError) -> Dict[str, Any]:
    """Build a failed response carrying a protocol/plugin error."""
    return {
        "protocol": PROTOCOL_VERSION,
        "id": request_id,
        "ok": False,
        "error": {"code": error.code, "message": error.message},
    }


def encode_response(response: Dict[str, Any]) -> bytes:
    """Serialize a response to the wire form: compact JSON + trailing newline."""
    return (json.dumps(response, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")


def make_diagnostic(
    severity: str,
    message: str,
    *,
    code: Optional[str] = None,
    table: Optional[str] = None,
    row: Optional[int] = None,
    field: Optional[str] = None,
    hint: Optional[str] = None,
) -> Dict[str, Any]:
    """Build one wire diagnostic; absent optional fields are omitted so the
    JSON matches the Rust contract's ``skip_serializing_if`` shape."""
    diag: Dict[str, Any] = {"severity": severity, "code": code or DEFAULT_DIAGNOSTIC_CODE, "message": message}
    if table is not None:
        diag["table"] = table
    if row is not None:
        diag["row"] = row
    if field is not None:
        diag["field"] = field
    if hint is not None:
        diag["hint"] = hint
    return diag


def validate_diagnostics(diagnostics: List[Any]) -> List[Dict[str, Any]]:
    """Check plugin findings against the wire shape.

    Raises :class:`ProtocolError` with ``ERR_PLUGIN_INVALID_RESULT`` when a
    finding is not an object, lacks a string ``message``, or carries an
    unknown severity — the host must never receive output it cannot decode.
    """
    checked: List[Dict[str, Any]] = []
    for diag in diagnostics:
        if not isinstance(diag, dict):
            raise ProtocolError.invalid_result(f"diagnostic must be an object, got {type(diag).__name__}")
        if not isinstance(diag.get("message"), str):
            raise ProtocolError.invalid_result("diagnostic needs a string 'message'")
        if diag.get("severity") not in VALID_SEVERITIES:
            raise ProtocolError.invalid_result(
                f"diagnostic severity must be one of {VALID_SEVERITIES}, got {diag.get('severity')!r}"
            )
        checked.append(diag)
    return checked
