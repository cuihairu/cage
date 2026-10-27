
# Schema: Structure Definitions

A Schema defines the structure and constraints of configuration data: fields,
types, requiredness, defaults, ranges, enums, arrays, objects, uniqueness,
references, and output metadata. It is a DSL file independent of the input
source (YAML or JSON; `schema_path` may be a single file or a directory — a
directory merges and loads every `yaml`/`yml`/`json` inside it in filename
order) and does not depend on any particular Source.

> All examples here are real wire format and can be loaded directly. A
> runnable, complete project is at
> [`examples/game-config/schemas/`](https://github.com/cuihairu/cage/blob/main/examples/game-config/schemas).

## Decoupling Schema from Source

Do not hard-code the Schema into Excel:

```text
Excel
   |
   +---- Source Adapter
   |
   v
Canonical Model
   ^
   |
Schema
```

This way, in the future:

```text
Excel + Schema
JSON + Schema
YAML + Schema
CSV + Schema
```

**The same Schema can serve multiple Sources.** Designers maintain numbers in
Excel, programmers write tool configuration in YAML, and both can share the
same structure and validation rules.

## DSL Example

The root keys of a Schema file are `tables` / `enums` / `metadata`. Every
table must carry `name` and `primary_key`, and every field must carry
`name`:

```yaml
tables:
  Item:
    name: Item
    description: 道具表
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 }, required: true, min: 1 }
      title: { name: title, type: { kind: String }, required: true, max_length: 64 }
      price: { name: price, type: { kind: UInt32 }, min: 0 }
      type:
        name: type
        type: { kind: Enum, value: ItemType }
        required: true

enums:
  ItemType:
    name: ItemType
    values:
      - { name: Weapon, value: 1 }
      - { name: Armor, value: 2 }
      - { name: Consumable, value: 3 }
```

Fields can declare reference relationships to one another:

```yaml
      drop_item_id:
        name: drop_item_id
        type: { kind: UInt32 }
        reference:
          table: Item
          field: id
```

`reference` has three further optional sub-keys: `predicate` (the semantic
predicate the referenced object must satisfy; L5 evaluates it on the target
row and yields `E1410` — see
Validation for the exact semantics), `cardinality`
(`one`/`many`/`optional`, default `one`; **source-side semantics** are
implemented — `one`/`optional` require a single-value reference (`optional`
tolerates null, while under `one` a null reports `E1404`), and `many` requires
an array and validates existence and the predicate element by element; an
unknown spelling reports `E1004`), and `compatible_with` (referenced-object
type compatibility check, yielding `E1411`). See
`examples/game-config` `Stage.loot_item_ids` for an example
(`cardinality: many` referencing `Item.id`).

## Field Types (19 kinds)

Types are always written in adjacent-tag form `type: { kind: X }` (parameterized
types pass their parameter in `value`); the lower-case scalar form
(`type: uint32`) **cannot be parsed**:

| kind | Description |
| --- | --- |
| `Null` / `Bool` | Null value / boolean |
| `Int8` `Int16` `Int32` `Int64` | Signed integers |
| `UInt8` `UInt16` `UInt32` `UInt64` | Unsigned integers |
| `Float32` `Float64` | Floating-point numbers; an integer value (e.g. an Excel whole-number cell) is accepted — the number tower unifies upward, a fractional value into an integer field is still rejected |
| `String` / `Bytes` | UTF-8 string / raw bytes |
| `Array` | Array; `value` is the element type: `{ kind: Array, value: { kind: Int32 } }` |
| `Object` | Object; `value` is the property table: `{ kind: Object, value: { x: { kind: Int32 } } }` |
| `Map` | Mapping `map<K,V>`; `value: { key_type: string\|int, value_type: { kind: … } }`, nestable |
| `Enum` | Named enum reference; `value` is the enum name (must be registered under top-level `enums:`) |
| `TaggedUnion` | Tagged union — a flat object whose `tag` key selects one branch, and only that branch's payload keys may appear next to it (回流-6, the Luban `$type` polymorphism). `value: { tag: "$type", branches: { Damage: { amount: { kind: Int32 } } } }`. A key from another branch, an unknown tag, or a wrong payload type is rejected (the type safety the enum + `Any`-payload workaround loses). With a `separator`, a table cell splits single-cell polymorphism (`"Damage\|13\|wind"` → `{"$type":"Damage","amount":13,"element":"wind"}`) |
| `Any` | Arbitrary value |

## Field Constraint Reference

| Constraint | Description |
| --- | --- |
| `type` | Field type (see table above, `{ kind: X }` form) |
| `required` | Required (missing reports `E1001`) |
| `nullable` | Accepts a present `null` (e.g. an Excel empty cell) without weakening the type to `Any`; a `nullable` reference field should also use `cardinality: optional` |
| `default` | Declares a default value. **Backfills** a missing key or a present `null` (Excel empty cell) during build, so a field with a default always carries it; also used for the `E9006` visibility exemption and member initial values in generated code |
| `min` / `max` | Numeric range (`E1201`) |
| `min_length` / `max_length` | String length range (`E1201`) |
| `pattern` | Regex a string must match (`E1203`). Note: writing `regex:` is treated as custom metadata and **silently ignored** |
| `enum_values` | Inline enum value domain (`E1204`). Note: writing `values:` is silently ignored; named enums use top-level `enums:` + `{ kind: Enum, value: name }` |
| `min_items` / `max_items` | Array element count range (`E1205`) |
| `items` | Field schema for array elements (recursive) |
| `separator` | Split a scalar table cell (Excel/CSV cannot hold arrays) into an `Array` on this separator — the equivalent of Luban's `(list#sep=,)`. Each token is trimmed and coerced to the element type; a cell that fails to split cleanly is left as-is and the type check reports it. Only meaningful on `Array` fields |
| `properties` / `additional_properties` | Object property schema and unknown-key switch |
| `reference` | Cross-table reference (`table` + `field`, `E1401`; see the sub-key description above) |
| `targets` | Which profiles the field is visible to (empty = all; `"*"` is equivalent to empty; a hidden structurally required field reports `E9006`) |
| `rules` | Field-level semantic rules (`name`/`assert`/`message`/`warning_only`; L6 evaluates single-comparison assertions, a false assertion or an incomparable type reports `E1501`, a malformed assertion or undeclared field reports `E1004` — see Semantic) |

## Table-Level Constraints

| Constraint | Description |
| --- | --- |
| `name` / `description` | Table name (required) and description |
| `primary_key` | Primary key field list (required; duplicates report `E1301`) |
| `unique_constraints` | List of composite uniqueness constraints: `- { name: name_unique, fields: [name] }` (`E1302`). Note: there is no `unique:` key on fields; writing one is silently ignored |
| `order_by` | Row ordering requirement: list of field names (`E1304`) |
| `targets` | Which profiles the table is visible to (empty = all; dropping a whole table or hiding a required table reports `E9006`) |

Unknown keys do not error — they fall into custom metadata via
`#[serde(flatten)]`. Misspelling a constraint key (e.g. `regex`, `values`, or
field-level `unique`) is therefore **silently ineffective**, and at validation
time the constraint is as if it never existed.

Schema validation is executed in tiers by the
Validation pipeline: required at L1, types at L2, value
domains (min/max/length/pattern/enum/items) at L3, uniqueness and primary keys
at L4, and cross-table references at L5.

## Per-Environment Constraint Overrides (`env_overrides`, design §48)

The same table can use different strictness in different environments — dev
loosened, prod tightened. An environment is one notch of a "strictness knob" on the same
field shape: the table-level side table `env_overrides` declares
**environment name → field name → constraint patch**, and the patch lists only
the constraint keys that environment wants to change
(`required` / `min` / `max` / `min_length` / `max_length` /
`pattern` / `enum_values` / `min_items` / `max_items`); keys not listed keep
their baseline value, and listed keys are replaced in their entirety. **Types
are not in the patch** (an environment only tunes strictness, it does not
reshape the shape); an empty patch is an error, and the field names in a patch
must exist in the table.

```yaml
tables:
  Hero:
    # … 基线字段 …
    env_overrides:
      dev:
        hp: { required: false }          # 放宽
      prod:
        hp: { required: true, min: 100 } # 收紧
        rarity: { enum_values: [common, rare, epic] }
```

Environment names come from the union of the `env_overrides` keys across all
tables (the Schema declares them itself; they are not registered in
cage.toml). `cage check` / `build` / `gen` select an environment with
`--env <name>`: resolution happens before validation and generation (baseline
fields are replaced with that environment's values, while the baseline itself is
not modified), so every validator and code generator sees one ordinary Schema.
Passing `--env` with an undeclared environment name, or with a Schema that has
no `env_overrides`, is a usage error (exit 2). The build ledger rotates with
the environment: `schema_hash` is computed over the resolved Schema, the
manifest records an `environment` field, and switching environments does not
reuse the previous environment's build artifacts. Structural bad overrides
(pointing at a nonexistent field / empty patch) have two guards: without
`--env`, each one is reported as a `warning:` at load time (non-blocking, so a
bad override does not stay latent until someone actually selects an
environment); with `--env`, it is a hard error (exit 2). `cage snapshot` /
`registry publish` also support `--env` to produce environment-specific
packages (see
build and cli).
