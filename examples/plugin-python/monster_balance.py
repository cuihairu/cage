#!/usr/bin/env python3
"""Monster power-curve validation plugin (P2-1 protocol MVP demo).

This is a *plugin*, not a host: it implements one validation rule and is
driven by the versioned JSON IPC protocol. The host (Cage's L7 executor, the
CLI/Web/CI) is the thing that builds the request from the normalized
schema/data/options and spawns this process; this file only reads one request
on stdin, runs the rule, and writes one response on stdout.

Run it directly against a sample request (no host yet):

    python3 examples/plugin-python/monster_balance.py \
        < examples/plugin-python/sample_request.json

See ``docs/plugins.md`` for the protocol spec and this plugin's walkthrough.
"""

import pathlib
import sys

# Allow running from a repo checkout without installing the SDK.
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "sdk" / "python"))

from cage_sdk import Plugin, RunContext, run  # noqa: E402


class MonsterBalance(Plugin):
    """Monster attack power must follow a cost/level curve.

    The curve parameters come from the project configuration
    (``ctx.options()``), so the same plugin checks any game's curve without
    code changes — the project declares ``base_cost`` and ``growth``.
    """

    id = "monster_balance"
    version = "1.0.0"

    def validate(self, ctx: RunContext) -> None:
        options = ctx.options()
        base_cost = options.get("base_cost", 100)
        growth = options.get("growth", 1.2)
        max_level = options.get("max_level", 10)

        table = ctx.table("Monster")
        if table is None:
            ctx.warning("no Monster table to check")
            return

        for row in table["rows"]:
            fields = row["fields"]
            level = fields.get("level")
            cost = fields.get("cost")
            if not isinstance(level, (int, float)) or not isinstance(cost, (int, float)):
                # Non-numeric rows are the host's type layer's concern
                # (L3); a plugin only sees normalized numbers.
                continue
            if level > max_level:
                ctx.error(
                    f"level {level} exceeds the configured max {max_level}",
                    table="Monster",
                    row=row["index"],
                    field="level",
                    hint=f"lower the level or raise `max_level` in the plugin options",
                )
                continue
            expected = base_cost * (growth ** max(level - 1, 0))
            if cost < expected * 0.9:
                ctx.error(
                    f"cost {cost} is below the expected curve {expected:.1f} "
                    f"(base {base_cost}, growth {growth})",
                    table="Monster",
                    row=row["index"],
                    field="cost",
                    hint="raise cost, or adjust base_cost/growth in the plugin options",
                )
            elif cost > expected * 1.1:
                ctx.warning(
                    f"cost {cost} is above the expected curve {expected:.1f}",
                    table="Monster",
                    row=row["index"],
                    field="cost",
                )


if __name__ == "__main__":
    run(MonsterBalance())
