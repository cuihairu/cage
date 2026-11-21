"""Protocol layer tests: parsing, framing, diagnostics, error codes."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cage_sdk import RunContext  # noqa: E402
from cage_sdk.protocol import (  # noqa: E402
    DEFAULT_DIAGNOSTIC_CODE,
    ERR_PLUGIN_INVALID_RESULT,
    ERR_PROTOCOL_MALFORMED,
    ERR_PROTOCOL_UNKNOWN_METHOD,
    ERR_PROTOCOL_VERSION_MISMATCH,
    METHOD_RUN,
    PROTOCOL_VERSION,
    ProtocolError,
    encode_response,
    make_diagnostic,
    parse_request,
    response_err,
    response_ok,
    validate_diagnostics,
)


def valid_request(**overrides) -> dict:
    request = {
        "protocol": PROTOCOL_VERSION,
        "id": "run-1",
        "method": METHOD_RUN,
        "params": {
            "plugin": {"id": "demo", "version": "1.0.0"},
            "schema": {"name": "Monster", "tables": []},
            "tables": [
                {
                    "name": "Monster",
                    "primary_key": ["id"],
                    "rows": [{"index": 0, "fields": {"id": 1, "level": 3}}],
                }
            ],
            "options": {"base_cost": 100},
        },
    }
    request.update(overrides)
    return request


def encode(request: dict) -> bytes:
    return json.dumps(request, ensure_ascii=False).encode("utf-8")


class ParseRequestTests(unittest.TestCase):
    def test_happy_path_parses(self):
        parsed = parse_request(encode(valid_request()))
        self.assertEqual(parsed, valid_request())
        self.assertEqual(parsed["id"], "run-1")

    def test_tables_and_options_may_be_absent(self):
        params = {"plugin": {"id": "p", "version": "1"}, "schema": {}}
        parsed = parse_request(
            encode({"protocol": 1, "id": "x", "method": "run", "params": params})
        )
        # parse_request only *validates* the shape; RunContext applies the
        # default so a plugin asking for tables on an empty run still works.
        self.assertNotIn("tables", parsed["params"])
        self.assertNotIn("options", parsed["params"])
        ctx = RunContext(parsed["params"])
        self.assertEqual(ctx.table_names(), [])
        self.assertEqual(ctx.options(), {})

    def test_rejects_invalid_utf8(self):
        with self.assertRaises(ProtocolError) as caught:
            parse_request(b"\xff\xfe")
        self.assertEqual(caught.exception.code, ERR_PROTOCOL_MALFORMED)

    def test_rejects_bad_json(self):
        with self.assertRaises(ProtocolError) as caught:
            parse_request(b"{nope")
        self.assertEqual(caught.exception.code, ERR_PROTOCOL_MALFORMED)

    def test_rejects_a_non_object(self):
        with self.assertRaises(ProtocolError) as caught:
            parse_request(b"[1, 2, 3]")
        self.assertEqual(caught.exception.code, ERR_PROTOCOL_MALFORMED)

    def test_rejects_a_missing_protocol(self):
        with self.assertRaises(ProtocolError) as caught:
            parse_request(encode({"id": "x", "method": "run", "params": {}}))
        self.assertEqual(caught.exception.code, ERR_PROTOCOL_MALFORMED)

    def test_rejects_a_non_integer_protocol(self):
        with self.assertRaises(ProtocolError):
            parse_request(encode({"protocol": "1", "id": "x", "method": "run", "params": {}}))

    def test_rejects_a_boolean_protocol(self):
        # True is an int in Python; it must not count as protocol 1.
        with self.assertRaises(ProtocolError):
            parse_request(encode({"protocol": True, "id": "x", "method": "run", "params": {}}))

    def test_version_mismatch_is_reported_before_shape(self):
        request = valid_request()
        request["protocol"] = 2
        with self.assertRaises(ProtocolError) as caught:
            parse_request(encode(request))
        self.assertEqual(caught.exception.code, ERR_PROTOCOL_VERSION_MISMATCH)
        self.assertIn("2", caught.exception.message)
        self.assertIn(str(PROTOCOL_VERSION), caught.exception.message)

    def test_unknown_method_is_named(self):
        request = valid_request()
        request["method"] = "compile"
        with self.assertRaises(ProtocolError) as caught:
            parse_request(encode(request))
        self.assertEqual(caught.exception.code, ERR_PROTOCOL_UNKNOWN_METHOD)
        self.assertIn("compile", caught.exception.message)

    def test_missing_plugin_identity_is_malformed(self):
        with self.assertRaises(ProtocolError):
            parse_request(
                encode(
                    {
                        "protocol": 1,
                        "id": "x",
                        "method": "run",
                        "params": {"schema": {}},
                    }
                )
            )

    def test_schema_and_options_must_be_json_objects(self):
        request = valid_request()
        request["params"]["schema"] = "Monster"
        with self.assertRaises(ProtocolError):
            parse_request(encode(request))
        request["params"]["schema"] = {"name": "Monster"}
        request["params"]["options"] = "base_cost=100"
        with self.assertRaises(ProtocolError):
            parse_request(encode(request))

    def test_tables_must_carry_a_string_name(self):
        request = valid_request()
        request["params"]["tables"] = [{"name": 7}]
        with self.assertRaises(ProtocolError):
            parse_request(encode(request))


class DiagnosticTests(unittest.TestCase):
    def test_defaults_to_e1601_and_omits_absent_fields(self):
        diag = make_diagnostic("error", "level too high", table="Monster", row=0, field="level")
        self.assertEqual(diag["code"], DEFAULT_DIAGNOSTIC_CODE)
        self.assertEqual(diag["code"], "E1601")
        self.assertEqual(diag["severity"], "error")
        self.assertEqual(diag["message"], "level too high")
        self.assertEqual(diag["table"], "Monster")
        self.assertEqual(diag["row"], 0)
        self.assertEqual(diag["field"], "level")
        self.assertNotIn("hint", diag)

    def test_explicit_code_wins(self):
        diag = make_diagnostic("warning", "near cap", code="E1602", hint="raise it")
        self.assertEqual(diag["code"], "E1602")
        self.assertEqual(diag["hint"], "raise it")

    def test_validates_findings(self):
        good = make_diagnostic("error", "nope")
        self.assertEqual(validate_diagnostics([good]), [good])

    def test_rejects_a_non_object_finding(self):
        with self.assertRaises(ProtocolError) as caught:
            validate_diagnostics(["nope"])
        self.assertEqual(caught.exception.code, ERR_PLUGIN_INVALID_RESULT)

    def test_rejects_a_finding_without_a_message(self):
        with self.assertRaises(ProtocolError):
            validate_diagnostics([{"severity": "error"}])

    def test_rejects_an_unknown_severity(self):
        with self.assertRaises(ProtocolError):
            validate_diagnostics([{"severity": "fatal", "message": "x"}])

    def test_response_shapes(self):
        ok = response_ok("run-1", [make_diagnostic("error", "x")])
        self.assertTrue(ok["ok"])
        self.assertIsNone(ok.get("error"))
        self.assertEqual(len(ok["diagnostics"]), 1)

        err = response_err("run-1", ProtocolError.malformed("bad"))
        self.assertFalse(err["ok"])
        self.assertIsNone(err.get("diagnostics"))
        self.assertEqual(err["error"]["code"], ERR_PROTOCOL_MALFORMED)

    def test_encode_is_compact_json_with_trailing_newline(self):
        raw = encode_response(response_ok("run-1", []))
        self.assertTrue(raw.endswith(b"\n"))
        self.assertEqual(raw.count(b"\n"), 1)
        self.assertEqual(json.loads(raw), {"protocol": 1, "id": "run-1", "ok": True, "diagnostics": []})


if __name__ == "__main__":
    unittest.main()
