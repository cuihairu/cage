# cage-plugin-sdk

Python SDK for **Cage** validation plugins.

It speaks Cage's language-agnostic, versioned JSON IPC protocol (v1) over
the worker's stdin/stdout: the host writes one request, the worker replies
with one response, then exits. The protocol is defined once in
`crates/cage-plugin-protocol` (Rust) and mirrored here, so any language can
implement the same contract.

## Install

```bash
pip install -e .
```

Or use it directly from a repo checkout (the examples do this via a
`sys.path` insert).

## Write a plugin

```python
from cage_sdk import Plugin, RunContext, run

class MonsterBalance(Plugin):
    id = "monster_balance"
    version = "1.0.0"

    def validate(self, ctx: RunContext) -> None:
        opts = ctx.options()
        base = opts.get("base_cost", 100)
        table = ctx.table("Monster")
        if table is None:
            ctx.warning("no Monster table to check")
            return
        for row in table["rows"]:
            level = row["fields"].get("level")
            cost = row["fields"].get("cost")
            if isinstance(level, (int, float)) and isinstance(cost, (int, float)):
                expected = base * (1.2 ** max(level - 1, 0))
                if cost < expected * 0.9:
                    ctx.error(
                        f"cost {cost} is below the expected curve {expected:.1f}",
                        table="Monster", row=row["index"], field="cost",
                    )

if __name__ == "__main__":
    run(MonsterBalance())
```

## Test

```bash
python3 -m unittest discover -s tests -v
```
