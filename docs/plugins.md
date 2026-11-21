# Plugins: Versioned JSON IPC Protocol

Cage's extension points are layered by stability and safety (see the review
`2026-10-22-schema-workbench-python-plugins.md` §五). This page documents the
**business-validation plugin** layer: a language-agnostic, versioned JSON IPC
protocol plus a Python SDK that speaks it.

| Extension layer | Mechanism | Status |
|---|---|---|
| Rule template package | Declarative JSON/Schema AST | Landed with P1-3 (built-in templates) |
| **Business validation plugin** | **Versioned JSON IPC worker** | **P2-1: protocol + Python SDK (this page)** |
| Rust native validator | Trait / built-in registry | Existing `GameRuleValidator` |
| Data source plugin | Independent Source Adapter | Existing `cage-source-*` |
| Generator plugin | Built-in Target / template | Existing `cage-target-*` |

A validation plugin is meant to be a *pure function*:

```text
Diagnostics = f(Schema, Data, Options)
```

The plugin never opens `Monster.xlsx`, never re-implements CSV/Excel/JSON
parsing, and never writes data back. Cage Core normalizes the data first and
hands the plugin read-only, already-typed tables. This keeps one plugin
producing consistent results on a designer's laptop, a build server, and CI.

## Architecture

```text
Cage Studio / CLI / CI
        │  build request (schema + normalized tables + options)
        ▼
   Rust Core (L0–L6) ──► unified L7 executor ──► Python Worker (this protocol)
        │                                              │  Diagnostics = f(...)
        └──────────────► unified diagnostics ◄─────────┘
```

The protocol is defined once in `crates/cage-plugin-protocol` (Rust, the
canonical wire types) and mirrored by `sdk/python` (`cage_sdk`). Any language
can implement the same contract; the protocol is deliberately language-agnostic.

## Protocol v1

### Framing (MVP)

A plugin run is a **single request/response exchange**:

1. The host writes exactly one compact UTF-8 JSON object followed by `\n` to
   the worker's stdin, then closes stdin.
2. The worker reads all of stdin (one request), runs the plugin, writes exactly
   one compact UTF-8 JSON object followed by `\n` to stdout, then exits.

The MVP is single-shot: no keep-alive, no streaming, no batching. A future
persistent worker would layer its own framing on the same message types.

### Versioning

Every message carries a `protocol` integer, currently `1`. A worker receiving
a request whose `protocol` differs from its supported version MUST respond
with an error whose `code` is `PROTOCOL_VERSION_MISMATCH` and exit non-zero;
it MUST NOT attempt to interpret the message. The host's version is the
contract: bump `PROTOCOL_VERSION` on any breaking change to the message
shapes.

### Methods

The MVP exposes exactly one method, `run`.

#### Request (host → worker)

```json
{
  "protocol": 1,
  "id": "build-2026-11-22-0001",
  "method": "run",
  "params": {
    "plugin": { "id": "monster_balance", "version": "1.0.0" },
    "schema": { "name": "Monster", "tables": [] },
    "tables": [
      {
        "name": "Monster",
        "primary_key": ["id"],
        "rows": [
          {
            "index": 0,
            "location": "Monster.xlsx#Sheet1!A2",
            "fields": { "id": 1, "level": 3, "cost": 144 }
          }
        ]
      }
    ],
    "options": { "base_cost": 100, "growth": 1.2, "max_level": 10 }
  }
}
```

| Field | Meaning |
|---|---|
| `protocol` | Must equal the supported version (`1`). |
| `id` | Opaque correlation id, echoed verbatim on the response. |
| `method` | Must be `run`. |
| `params.plugin` | Plugin id and version, taken from the project configuration. |
| `params.schema` | The canonical Schema as JSON (the same shape the editor stores). |
| `params.tables` | Normalized tables, in declaration order. |
| `params.rows[].index` | 0-based row index within the table (diagnostics reference it). |
| `params.rows[].location` | Optional human-readable source position (e.g. `Monster.xlsx#Sheet1!A2`). |
| `params.rows[].fields` | Field name → normalized JSON value. |
| `params.options` | Project-configuration options for this plugin (read-only). |

#### Response (worker → host), success

```json
{
  "protocol": 1,
  "id": "build-2026-11-22-0001",
  "ok": true,
  "diagnostics": [
    {
      "severity": "error",
      "code": "E1601",
      "message": "cost 100 is below the expected curve 144.0 (base 100, growth 1.2)",
      "table": "Monster",
      "row": 1,
      "field": "cost",
      "hint": "raise cost, or adjust base_cost/growth in the plugin options"
    }
  ]
}
```

A diagnostic's `severity` is one of `error` / `warning` / `info`. `code`
defaults to `E1601` (the gamerule "custom validator plugin returned error"
code) when omitted. `table`/`row`/`field` are optional and let the host map a
finding back to a concrete source location.

#### Response (worker → host), failure

```json
{
  "protocol": 1,
  "id": "build-2026-11-22-0001",
  "ok": false,
  "error": { "code": "PLUGIN_ERROR", "message": "ValueError: bad options at plugin.py:41" }
}
```

### Error codes

| `code` | Meaning |
|---|---|
| `PROTOCOL_VERSION_MISMATCH` | The request's `protocol` differs from the supported version. |
| `PROTOCOL_MALFORMED` | The request was not parseable (bad JSON, missing fields). |
| `PROTOCOL_UNKNOWN_METHOD` | The request named a method the protocol does not implement. |
| `PLUGIN_ERROR` | The plugin raised an exception (message capped at 400 UTF-8 bytes). |
| `PLUGIN_INVALID_RESULT` | The plugin produced output outside the protocol shape. |

### Exit codes

| Code | Meaning |
|---|---|
| `0` | A response was written (even one carrying `error`-severity *findings* — the host decides what findings mean). |
| `1` | Protocol or plugin failure; `error` carries the reason. |

## Python SDK (`sdk/python`)

`cage_sdk` mirrors the protocol and gives plugin authors the four read-only
capabilities and the controlled diagnostic writes.

```bash
pip install -e sdk/python
```

```python
from cage_sdk import Plugin, RunContext, run

class MonsterBalance(Plugin):
    id = "monster_balance"
    version = "1.0.0"

    def validate(self, ctx: RunContext) -> None:
        table = ctx.table("Monster")
        if table is None:
            ctx.warning("no Monster table to check")
            return
        for row in table["rows"]:
            level = row["fields"].get("level")
            if isinstance(level, (int, float)) and level > 10:
                ctx.error(
                    "level too high",
                    table="Monster", row=row["index"], field="level",
                )

if __name__ == "__main__":
    run(MonsterBalance())
```

### Context API

| Method | Purpose |
|---|---|
| `ctx.schema()` | The canonical Schema as JSON (read-only). |
| `ctx.table_names()` | Names of the tables handed to this run. |
| `ctx.table(name)` | One normalized table (`{"name","primary_key","rows"}`), or `None`. |
| `ctx.options()` | Project-configuration options for this plugin (read-only). |
| `ctx.error(message, *, code, table, row, field, hint)` | Report a hard violation. |
| `ctx.warning(message, *, ...)` | Report a soft finding. |
| `ctx.info(message, *, ...)` | Report an informational notice. |

`run(plugin)` serves one request from stdin and exits with the protocol exit
code. `serve(plugin, input_stream, output_stream)` is the testable core.

### Running the bundled example

The example plugin implements a monster power/level curve:

```bash
python3 examples/plugin-python/monster_balance.py \
    < examples/plugin-python/sample_request.json
```

## What P2-1 covers (and what does not)

In scope for this protocol MVP:

- The versioned JSON IPC contract (Rust `cage-plugin-protocol` + Python mirror).
- The Python SDK (context API, version/malformed/unknown-method rejection,
  plugin-exception capture, diagnostic validation).
- Cross-language interop tests: a Rust-built request is decoded by the Python
  SDK over a real subprocess and its response decodes back in Rust.

Delivered since P2-1 (each in `cage_core`, wired into the CLI):

- **P2-2** — the host-side worker: spawn, timeout, crash recovery, and the
  failure-behaviour table (missing plugin, version mismatch, timeout, crash,
  invalid output) mapped onto Cage diagnostics (`cage_core::plugin`).
- **P2-3** — the unified L7 executor that runs built-in Rust validators and
  worker plugins on the same diagnostic surface (`GameRuleRegistry`).
- **P2-4** — `ValidationAttestation`: plugin id/version, config digest,
  dependency lock, environment fingerprint in an auditable record.
- **P2-5** — the `[validation]` project policy: `required_level` raises the
  CLI default so L7 actually runs, and `required_plugins` fails validation
  with `E1602` when a declared id is missing — applied by `check`, `build`,
  the LSP lane and the web editor's data precheck.
