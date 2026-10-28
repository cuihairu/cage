#!/usr/bin/env python3
"""Cage Excel meta-row adapter contract demo (cage.excel-meta/1, design §51 Q2).

This is a runnable example of the *script* adapter: the built-in `marker_rows`
preset decodes the same shape in Rust, and this script proves the contract is
language-agnostic — cage writes one JSON request on stdin and reads one JSON
response on stdout, never code.

Run standalone (self-check, no cage needed):

    python3 tools/excel-meta-demo.py --self-check

Run as a cage adapter: declare it in cage.toml and let cage spawn it —

    [source.excel]
    script = "tools/excel-meta-demo.py"

The decoder here is deliberately minimal (the `##var` header + `##type` row of
a `marker_rows` sheet). A real script may place convention rows wherever the
author likes — the request carries the whole grid, so the script owns the
layout. Output is validated against the published contract schema
(docs/schemas/excel-meta-adapter-v1.schema.json) plus the vocabulary table
(unknown keywords fail, E2202).
"""

import json
import sys

CONTRACT = "cage.excel-meta/1"

# The only vocabulary a script may emit: the Schema DSL field names the
# ColumnMeta contract carries (docs/schemas/excel-meta-adapter-v1.schema.json).
VOCABULARY = {
    "cell", "type", "required", "primary_key", "default", "enum", "min",
    "max", "unique", "reference", "description", "targets", "sub_columns",
    "multi_row",
}


def _parse_type(text):
    """Map a `##type` cell to the adjacently-tagged FieldType object.

    The contract carries the exact `{ kind, value }` shape the Schema
    deserializes — there is no translation, so the two spellings cannot drift.
    Scalars carry no `value`; `Enum` carries the name string.
    """
    text = text.strip()
    if text.startswith("Enum<") and text.endswith(">"):
        name = text[5:-1].strip()
        if not name or not (name[0].isalpha() or name[0] == "_"):
            raise ValueError(f"{text!r} is not a valid Enum name")
        return {"kind": "Enum", "value": name}
    scalars = {
        "Null", "Bool", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16",
        "UInt32", "UInt64", "Float32", "Float64", "String", "Bytes", "Any",
    }
    if text in scalars:
        return {"kind": text}
    raise ValueError(f"{text!r} is not a Schema type name")


def decode(request):
    """Decode a `marker_rows`-shaped grid into a TableMeta response."""
    grid = request["grid"]
    sheet = request["sheet"]
    marker = "##"

    # The data header row is the `##var` row; field names follow the marker
    # cell (Q6 ruling (a)). Everything at or above it, up to the first pure
    # data row, is convention territory.
    header_row = None
    for i, row in enumerate(grid):
        if any(
            isinstance(c, str) and c.startswith(marker + "var")
            for c in row[:1]
        ):
            header_row = i
            break
    if header_row is None:
        raise ValueError("no ##var header row found")

    # Field names: every non-marker cell right of the ##var cell.
    header = grid[header_row]
    names = [c for c in header[1:] if isinstance(c, str) and c]
    if not names:
        raise ValueError("##var header declares no columns")

    # Data starts at the first row at/below the header with no marker cell.
    data_start = len(grid)
    for i in range(header_row, len(grid)):
        if not any(isinstance(c, str) and c.startswith(marker) for c in grid[i]):
            data_start = i
            break

    # The `##type` row carries the per-column types, keyed by column index.
    types = {}
    for i in range(header_row, data_start):
        row = grid[i]
        if row and isinstance(row[0], str) and row[0].startswith(marker + "type"):
            for j, t in enumerate(row[1:], start=1):
                if isinstance(t, str) and t:
                    types[j - 1] = t
            break

    columns = {}
    for idx, name in enumerate(names):
        # Column letter for the cell anchor (idx 0 is B, the ##var marker is A).
        letter = chr(ord("A") + 1 + idx)
        ftype = _parse_type(types.get(idx, "String"))
        columns[name] = {"cell": f"{letter}{header_row + 1}", "type": ftype}

    return {
        "contract": CONTRACT,
        "table": sheet,
        "header_row": header_row,
        "columns": columns,
    }


def self_check():
    """Feed a canned Skill grid, decode it, and assert the contract holds."""
    grid = [
        ["##var", "id", "name", "cost", "tier", "kind"],
        ["##type", "Int32", "String", "Int32", "Int32", "Enum<SkillKind>"],
        ["##min", "", "0", "", "", ""],
        ["", "1", "Fireball", "10", "1", "Fire"],
        ["", "2", "Icebolt", "15", "2", "Ice"],
    ]
    request = {
        "contract": CONTRACT,
        "workbook": "config/Skill.xlsx",
        "sheet": "Skill",
        "header_row": 0,
        "grid": grid,
        "config": {},
    }
    response = decode(request)

    assert response["contract"] == CONTRACT, response
    assert response["table"] == "Skill", response
    assert response["header_row"] == 0, response
    assert list(response["columns"]) == ["id", "name", "cost", "tier", "kind"], response
    assert response["columns"]["id"]["type"] == {"kind": "Int32"}, response
    assert response["columns"]["kind"]["type"] == {"kind": "Enum", "value": "SkillKind"}, response

    # Every key the script emitted must be in the vocabulary (else E2202).
    for col in response["columns"].values():
        unknown = set(col) - VOCABULARY
        assert not unknown, f"out-of-vocabulary keys: {unknown}"

    print(json.dumps(response, indent=2))
    print("self-check OK: response conforms to the cage.excel-meta/1 contract", file=sys.stderr)


def main():
    if "--self-check" in sys.argv:
        self_check()
        return 0
    request = json.load(sys.stdin)
    if request.get("contract") != CONTRACT:
        print(f"unknown contract {request.get('contract')!r}", file=sys.stderr)
        return 2
    json.dump(decode(request), sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
