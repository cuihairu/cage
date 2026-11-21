"""Context + runtime tests: the ctx API, serve() behaviours, truncation."""

from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cage_sdk import (  # noqa: E402
    ERR_PLUGIN_ERROR,
    ERR_PROTOCOL_MALFORMED,
    ERR_PROTOCOL_UNKNOWN_METHOD,
    ERR_PROTOCOL_VERSION_MISMATCH,
    Plugin,
    ProtocolError,
    RunContext,
    serve,
)
from cage_sdk.runtime import truncate_utf8  # noqa: E402


def request_bytes(**param_overrides) -> bytes:
    params = {
        "plugin": {"id": "demo", "version": "1.0.0"},
        "schema": {"name": "Monster", "tables": []},
        "tables": [
            {
                "name": "Monster",
                "primary_key": ["id"],
                "rows": [
                    {"index": 0, "fields": {"id": 1, "level": 3}},
                    {"index": 1, "fields": {"id": 2, "level": 99}},
                ],
            }
        ],
        "options": {"cap": 10},
    }
    params.update(param_overrides)
    return json.dumps(
        {"protocol": 1, "id": "run-1", "method": "run", "params": params}
    ).encode("utf-8")


class LevelCap(Plugin):
    id = "level_cap"
    version = "2.1.3"

    def validate(self, ctx: RunContext) -> None:
        cap = ctx.options().get("cap", 10)
        table = ctx.table("Monster")
        if table is None:
            ctx.warning("no Monster table")
            return
        for row in table["rows"]:
            level = row["fields"].get("level")
            if isinstance(level, (int, float)) and level > cap:
                ctx.error(
                    f"level {level} exceeds cap {cap}",
                    table="Monster",
                    row=row["index"],
                    field="level",
                )


class ServeTests(unittest.TestCase):
    def serve(self, plugin, data: bytes):
        out = io.BytesIO()
        code = serve(plugin, input_stream=io.BytesIO(data), output_stream=out)
        return code, json.loads(out.getvalue())

    def test_runs_a_plugin_and_returns_findings(self):
        code, response = self.serve(LevelCap(), request_bytes())
        self.assertEqual(code, 0)
        self.assertTrue(response["ok"])
        self.assertEqual(response["id"], "run-1")
        self.assertEqual(response["protocol"], 1)
        self.assertEqual(len(response["diagnostics"]), 1)
        diag = response["diagnostics"][0]
        self.assertEqual(diag["severity"], "error")
        self.assertEqual(diag["code"], "E1601")
        self.assertEqual(diag["table"], "Monster")
        self.assertEqual(diag["row"], 1)
        self.assertEqual(diag["field"], "level")
        self.assertIn("exceeds cap 10", diag["message"])

    def test_a_clean_run_returns_no_findings(self):
        # cap=1000 lets every row (level 3 and 99) pass, so nothing is reported.
        code, response = self.serve(LevelCap(), request_bytes(options={"cap": 1000}))
        self.assertEqual(code, 0)
        self.assertTrue(response["ok"])
        self.assertEqual(response["diagnostics"], [])

    def test_a_missing_table_is_reported_as_a_warning(self):
        code, response = self.serve(LevelCap(), request_bytes(tables=[]))
        self.assertEqual(code, 0)
        self.assertEqual(response["diagnostics"][0]["severity"], "warning")

    def test_a_plugin_exception_becomes_a_plugin_error(self):
        class Boom(Plugin):
            id = "boom"
            version = "1.0.0"

            def validate(self, ctx):
                raise RuntimeError("kaboom")

        code, response = self.serve(Boom(), request_bytes())
        self.assertEqual(code, 1)
        self.assertFalse(response["ok"])
        self.assertIsNone(response.get("diagnostics"))
        self.assertEqual(response["error"]["code"], ERR_PLUGIN_ERROR)
        self.assertIn("RuntimeError", response["error"]["message"])
        self.assertIn("kaboom", response["error"]["message"])

    def test_the_plugin_base_requires_an_implementation(self):
        class Incomplete(Plugin):
            id = "incomplete"
            version = "1.0.0"

        code, response = self.serve(Incomplete(), request_bytes())
        self.assertEqual(code, 1)
        self.assertEqual(response["error"]["code"], ERR_PLUGIN_ERROR)
        self.assertIn("NotImplementedError", response["error"]["message"])

    def test_a_version_mismatch_is_refused(self):
        data = json.dumps({"protocol": 9, "id": "x", "method": "run", "params": {}}).encode()
        code, response = self.serve(LevelCap(), data)
        self.assertEqual(code, 1)
        self.assertFalse(response["ok"])
        self.assertEqual(response["error"]["code"], ERR_PROTOCOL_VERSION_MISMATCH)
        self.assertIn("9", response["error"]["message"])

    def test_an_unknown_method_is_refused(self):
        data = json.dumps(
            {"protocol": 1, "id": "x", "method": "compile", "params": {}}
        ).encode()
        code, response = self.serve(LevelCap(), data)
        self.assertEqual(code, 1)
        self.assertEqual(response["error"]["code"], ERR_PROTOCOL_UNKNOWN_METHOD)
        self.assertIn("compile", response["error"]["message"])

    def test_garbage_input_is_malformed(self):
        code, response = self.serve(LevelCap(), b"{nope")
        self.assertEqual(code, 1)
        self.assertEqual(response["error"]["code"], ERR_PROTOCOL_MALFORMED)


class ContextTests(unittest.TestCase):
    def test_reads_the_four_capabilities(self):
        ctx = RunContext(
            {
                "plugin": {"id": "demo", "version": "1.0.0"},
                "schema": {"name": "Monster"},
                "tables": [
                    {"name": "Monster", "rows": [], "primary_key": ["id"]},
                    {"name": "Item", "rows": [], "primary_key": []},
                ],
                "options": {"cap": 7},
            }
        )
        self.assertEqual(ctx.plugin["id"], "demo")
        self.assertEqual(ctx.schema(), {"name": "Monster"})
        self.assertEqual(ctx.table_names(), ["Monster", "Item"])
        self.assertEqual(ctx.options(), {"cap": 7})
        table = ctx.table("Monster")
        self.assertEqual(table["primary_key"], ["id"])
        self.assertIsNone(ctx.table("Nope"))

    def test_collects_and_copies_diagnostics(self):
        ctx = RunContext({"plugin": {}, "schema": {}, "tables": [], "options": {}})
        ctx.error("hard stop", code="E9999", table="T", row=2, field="f", hint="fix it")
        ctx.warning("soft nudge", table="T")
        ctx.info("fyi")
        collected = ctx.diagnostics()
        self.assertEqual(len(collected), 3)
        self.assertEqual(
            collected[0],
            {
                "severity": "error",
                "code": "E9999",
                "message": "hard stop",
                "table": "T",
                "row": 2,
                "field": "f",
                "hint": "fix it",
            },
        )
        self.assertEqual(collected[1]["code"], "E1601")
        self.assertNotIn("row", collected[1])
        self.assertEqual(collected[2], {"severity": "info", "code": "E1601", "message": "fyi"})
        collected.append({"tampered": True})
        self.assertEqual(len(ctx.diagnostics()), 3, "diagnostics() must return a copy")


class TruncateTests(unittest.TestCase):
    def test_short_text_passes_through(self):
        self.assertEqual(truncate_utf8("hello", 10), "hello")

    def test_ascii_cut_is_exact(self):
        self.assertEqual(truncate_utf8("a" * 50, 10), "a" * 10)
        self.assertEqual(len(truncate_utf8("a" * 50, 10).encode("utf-8")), 10)

    def test_cjk_cut_stays_on_a_character_boundary(self):
        text = "爆" * 300  # 900 bytes of 3-byte characters
        cut = truncate_utf8(text, 400)
        self.assertLessEqual(len(cut.encode("utf-8")), 400)
        cut.encode("utf-8").decode("utf-8")  # must not raise
        self.assertEqual(cut, "爆" * 133)  # 133 * 3 = 399 bytes


if __name__ == "__main__":
    unittest.main()
