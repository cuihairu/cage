"""Cage plugin Python SDK — speaks the language-agnostic JSON IPC protocol.

Quickstart (see ``docs/plugins.md`` for the full spec):

.. code-block:: python

    from cage_sdk import Plugin, RunContext, run

    class MonsterBalance(Plugin):
        id = "monster_balance"
        version = "1.0.0"

        def validate(self, ctx: RunContext) -> None:
            table = ctx.table("Monster")
            if table is None:
                ctx.warning("no Monster table")
                return
            for row in table["rows"]:
                level = row["fields"].get("level")
                if isinstance(level, (int, float)) and level < 0:
                    ctx.error(
                        "level cannot be negative",
                        table="Monster", row=row["index"], field="level",
                    )

    if __name__ == "__main__":
        run(MonsterBalance())
"""

from .context import RunContext
from .protocol import (
    DEFAULT_DIAGNOSTIC_CODE,
    ERR_PLUGIN_ERROR,
    ERR_PLUGIN_INVALID_RESULT,
    ERR_PROTOCOL_MALFORMED,
    ERR_PROTOCOL_UNKNOWN_METHOD,
    ERR_PROTOCOL_VERSION_MISMATCH,
    PROTOCOL_VERSION,
    ProtocolError,
)
from .runtime import Plugin, run, serve

__all__ = [
    "DEFAULT_DIAGNOSTIC_CODE",
    "ERR_PLUGIN_ERROR",
    "ERR_PLUGIN_INVALID_RESULT",
    "ERR_PROTOCOL_MALFORMED",
    "ERR_PROTOCOL_UNKNOWN_METHOD",
    "ERR_PROTOCOL_VERSION_MISMATCH",
    "PROTOCOL_VERSION",
    "Plugin",
    "ProtocolError",
    "RunContext",
    "run",
    "serve",
]
