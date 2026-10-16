
# Validation: Validation Pipeline

The core of Cage is not Transform, but **Validation**.

Validation is split into stages and executed level by level:

```text
Parse
  |
  v
Schema
  |
  v
Type
  |
  v
Value
  |
  v
Table
  |
  v
Reference
  |
  v
Semantic
  |
  v
Game Rules
```

Each level corresponds to a validation level and a family of error codes:

| Level | Name | What Is Checked | Error Code Family |
| --- | --- | --- | --- |
| L0 | Parse | whether the input can be parsed correctly | `E0001` family |
| L1 | Schema | whether the structure conforms to the Schema | `E1001` family |
| L2 | Type | whether types match | `E1101` family |
| L3 | Value | whether values are within constraints | `E1201` family |
| L4 | Table | internal consistency of a single table | `E1301` family |
| L5 | Reference | cross-configuration references | `E1401` family |
| L6 | Semantic | expressions / combinatorial logic | `E1501` family |
| L7 | Game Rule | business plugin validation | `E1601` family |

## L0 Syntax

First, check whether the input can be parsed correctly. For example:

```yaml
foo:
    - a
      - b
```

The YAML itself is invalid. Cage:

```text
ERROR E0001

Source:
    config/test.yaml

Location:
    line 3

Invalid YAML syntax.
```

For text-based Sources (JSON/YAML/CSV), syntax errors are pinpointed down to the line number.

## L1 Schema

Checks that the configuration structure conforms to the [Schema](/en/schema).

A missing field:

```text
ERROR E1001

Item[10001]

Missing required field:
    price
```

Unknown fields (warning by default, escalated to an error when `warnings_as_errors = true`):

```text
WARNING E1002

Item[10001]

Unknown field:
    pric
```

### E9006 Field Visibility Conflict (Profile Semantics)

`targets` decides which profile views a field/table is visible in (empty = all profiles; the `"*"` wildcard is equivalent to empty).
Trimming views is a routine operation, but **when a field that is structurally indispensable gets hidden by a profile, that is a conflict** —
it reports `E9006` instead of silently filtering it out:

- a `required` field without a `default` is hidden by the current profile;
- a primary key field is hidden;
- a unique-constraint member is hidden;
- for a reference field in a visible table, its target table or target field is hidden (a dangling reference after projection).

A required field that has a `default`, a non-required field, or a whole table dropped by `targets` are all
legitimate views and raise no error. The check runs against the complete schema (`profile` only trims the
artifact view and does not exempt data validation), and both `cage check/build --profile <p>` and `cage gen`
run with this semantics. `--env` (per-environment constraint overrides, design §48) is orthogonal to
profiles and takes effect before them: environment patches are applied onto the baseline fields first
(`required` changes accordingly), and E9006 is judged by the resolved `required` — once the environment
tightens things, being hidden by the sub-profile is a conflict. Example:

```text
ERROR E9006

Account.secret

Required field hidden by profile (targets: ["server"], profile: "client")
```

## L2 Type

For example:

```text
Level = "abc"
```

Schema:

```yaml
      level:
        name: level
        type: { kind: UInt32 }
```

The error:

```text
ERROR E1101

Item[10001].Level

Expected:
    UInt32

Actual:
    string

Value:
    "abc"
```

Map fields are validated key by key and value by value: keys are gated by `key_type` (string keys are
always valid; int keys must be numeric strings parseable via `parse::<i64>`), values are recursively
validated against `value_type`, and mismatches likewise go through `E1101` (the hint shows the expected
type spelled as `map<key, value>`); an empty map is always valid.

## L3 Value

A correct type does not mean a reasonable value. Schema:

```yaml
      level:
        name: level
        type: { kind: UInt32 }
        min: 1
        max: 100
```

Data:

```text
level = 999
```

Result:

```text
ERROR E1201

Item[10001].level = 999

Allowed range:
    1..100
```

Supported value constraints:

```text
min / max               数值范围（E1201）
min_length / max_length 字符串长度（E1202）
pattern                 正则匹配（E1203）
enum_values             枚举取值域（E1204）。命名枚举（type: {kind: Enum, value: 名}）
                        的成员资格同在本层、同走 E1204——对顶层 enums: 声明的成员做
                        检查；只查字符串值，非字符串类型失配归 L2（E1101），
                        悬空枚举名归 L1（E1004）
min_items / max_items   数组元素个数（E1205）
```

`unique` belongs to L4 (E1301/E1302) and `required` to L1 (E1001); neither is checked at this level.

## L4 Table

Checks the internal consistency of a single table. For example, IDs must be unique. The data:

```text
10001
10002
10001
```

Result:

```text
ERROR E1301

Duplicate primary key:

Item.ID = 10001

Rows:
    2
    4
```

It can also check:

```text
组合唯一（unique_constraints，E1302）
排序要求（order_by，E1304）
```

## L5 Reference

One of the most important capabilities in game configuration. For example:

**Item**

```text
ID
10001
10002
10003
```

**Monster**

```text
ID       DropItemID
20001    10001
20002    10002
20003    99999
```

Schema:

```yaml
      drop_item_id:
        name: drop_item_id
        type: { kind: UInt32 }
        reference:
          table: Item
          field: id
```

Cage:

```text
ERROR E1401

Monster[20003].DropItemID

Value:
    99999

Reference:
    Item.ID

Target does not exist.
```

### References Should Not Only Validate Existence

```text
Monster.DropItemID
        |
        v
Item.ID
```

The Item exists (`Item[10001]`), but `Item[10001].Type = QuestItem`, while Monster drops only allow `Weapon / Armor / Consumable`. Use `compatible_with` to declare the values the target field must be compatible with; violations report `E1411`:

```text
ERROR E1411

Monster[20001].DropItemID = 10001

Referenced Item exists,
but its type is not valid for Monster drops.

Actual:
    QuestItem

Allowed:
    Weapon
    Armor
    Consumable
```

The reference check surface breaks into five categories; implementation status:

```text
Existence        存在性          已实装（E1401）
Type             引用对象类型     已实装（E1411，compatible_with）
Predicate        谓词约束         已实装（E1410，断言在引用目标行上求值）
Cardinality      基数            已实装（E1404，源侧口径：one/optional 单值、many 数组逐元素）
Compatibility    兼容性          已实装（E1411，字段级约束）
```

### Table-Level Reference Cycles (E1403, Implemented)

After the row-by-row reference checks, L5 performs schema-level cycle detection: when the reference
graph (`DependencyGraph::from_schema`) contains cycles, each cycle reports one `E1403` WARNING,
with the cycle path written into the diagnostic message (starting from the smallest table name,
deduplicated across starting points — two tables referencing each other report only one):

```text
WARNING E1403 — Circular Reference
  Source: schema | Table: Item

Circular reference: Item → Kit → Item
```

Self-references (such as an unlock chain `Stage.next → Stage.id`) count as cycles under the reference
graph's own contract and are flagged the same way. Cycles do not block the build — generation is
single-pass and incremental propagation is safe for cycles — but they make the library API's
`topological_sort` / `build_order` fail, so the author is warned with a WARNING rather than an
error (an ERROR would wrongly reject legitimate mutually-referencing schemas);
with `warnings_as_errors = true` it escalates to an error (exit code 1).

## L6 Semantic

Schema solves the structural problem; Semantic Validation solves: **whether the data makes sense when combined.**

For example:

```text
MinLevel <= MaxLevel
BuyPrice >= SellPrice
Skill.Cost >= 0
Monster.DropTableID 必须存在
```

Expression rules can be defined:

```yaml
      level:
        name: level
        type: { kind: Int32 }
        rules:
          - name: level_range
            assert: min_level <= max_level
```

Rule declarations are parsed along with the Schema (`name`/`assert`/`message`/`warning_only`
are all present); the evaluator is implemented around **single comparison assertions**
`operand OP operand` (operands are field references — bare names or backtick-quoted names —
and numeric/string/boolean literals; operators
`== != <= >= < >`). Evaluation semantics:

- The numeric family (Int/UInt/Float) is compared numerically: Int/UInt use exact i128
  arithmetic, and once Float is involved f64 is used; strings compare lexicographically; booleans order false < true.
- If a referenced field is missing on that row or is null (an optional field left at its default) → the rule takes no stance on that row, treated as passing.
- If the two sides are not comparable in type (e.g. string vs int, arrays/objects/bytes) → an `E1501` violation is recorded and the hint names the types on both sides; never silently passed.
- If the assertion is false → `E1501` (`message` is preferred as the hint; the default renders
  `Assertion '<assert>' failed`); `warning_only: true` downgrades to WARNING.
- **A defective rule is a schema error**: if the assertion fails to parse or references a field not
  declared in the table, `E1004` is reported once per rule (independent of row count, reported even
  for empty tables), and that rule is not evaluated against any row. The same defect in a reference
  predicate (L5) is reported at L5 under the same code.

More complex business judgments (cross-row, arithmetic, multi-condition combinations) still belong to the L7 game-rule plugins.

## L7 Game Rule (Business Plugins, Implemented)

Complex game business logic should not all be crammed into the Schema DSL. The layering should be:

```text
Schema
Expression
Code Validator
```

Business validation is built as plugins (Rust traits, registered in-process):

```rust
pub trait GameRuleValidator: Send + Sync {
    fn name(&self) -> &'static str;
    fn validate(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic>;
}

// cage check --level gamerule 运行内建注册表；嵌入方亦可自行组装：
let mut registry = GameRuleRegistry::with_builtins();
registry.register(Box::new(DropTableValidator));
let diagnostics = registry.run(&schema, &document);
```

The built-in sample rule `power_curve` works end to end: when any table contains both `level`/`attack`
fields, it validates `attack <= level * 100 + 50`, and violations report `E1601` (row-level location,
hint includes the computation):

```text
ERROR E1601 — Game Rule Validation Failed
  Source: config/monster.json | Row: 1
  Table: Monster
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)
```

This way Cage Core does not need to understand specific game business. For the execution model
choice (in-process trait → dynamic library → sandbox) and the trust boundary, see
[Architecture: Security and Isolation](/en/architecture); for the full comparison table and the layered decision,
see the repository design draft `docs/design.md` §17.1 "Finalized Plugin Sandbox Plan".

## Level-by-Level Execution

These levels do not all have to be executed:

```bash
cage check                      # 执行 L0–L6（默认 --level semantic）
cage check --level schema       # 只做 Schema 层
cage check --level reference    # 只做到引用层
cage check --level gamerule     # 加上 L7 业务规则插件
```

See [CLI](/en/cli) for details.

## Diagnostics: First-Class Citizens

Each error carries at least:

```text
Code
Severity
Source
Location
Table
Row
Column
Field
Value
Message
Hint
```

A full example (as it actually renders):

```text
ERROR E1401 — Reference Target Missing
  Source: config/Monster.json | Sheet: Monster | Col: G27
  Table: Monster
  Row: 20003
  Field: DropItemID
  Value: 99999
  Message: reference target not found: Item.id = 99999
  Hint: Add Item[99999] or change DropItemID.
```

Supported severity levels:

```text
ERROR
WARNING
INFO
```

The CLI and Web UI already consume these diagnostics directly; IDE plugin integration is left for later.

## Warning Policy (Implemented)

The warning → error escalation policy is a top-level switch in `cage.toml` (default `false`):

```toml
# 日常开发：警告只提示，不阻断构建
warnings_as_errors = false

# 生产构建 / CI 门禁：警告升级为错误
warnings_as_errors = true
```

When enabled, the same warning is treated as an error in `cage build` (exit code 1, no
artifacts produced); when disabled, `cage build` succeeds as usual for configurations that
contain only warnings. The `warnings_as_errors` gate on the CI side refers to GitHub Actions'
`-D warnings` (the rustc lint layer), a different layer from this configuration option;
both are running (see the repository's ci.yml).

## Error Code Table

Error codes have the format `E{category}{number}`, where the category corresponds to the validation
level; each code has its own documentation entry and is defined in
`cage-core::error::codes`. The table is the **full registry of definitions**; codes marked
"reserved" are defined (constant + title complete) but not yet thrown by any current code path —
capability status is marked as it truly is, and the marker is removed once wired up.

### L0 Parse

| Code | Meaning |
| --- | --- |
| `E0001` | Syntax error (also: L1 reuses this code to emit a Warning when "the schema has a required field but the document is missing the entire table") |
| `E0002` | Unexpected end of input (reserved) |
| `E0003` | Invalid character encoding (reserved) |
| `E0004` | Malformed structure (e.g. a bad YAML mapping) |

### L1 Schema

| Code | Meaning |
| --- | --- |
| `E1001` | Missing required field |
| `E1002` | Unknown field not defined in the Schema |
| `E1003` | Duplicate field definition in the Schema (reserved) |
| `E1004` | Dangling references within the Schema itself (pk/unique/enum/reference pointing to a nonexistent field, enum, or table; executed only on the Web API loading path, while cross-table circular dependencies are reported during build ordering as a code-less error); malformed assertions in semantic rules / reference predicates, or assertions referencing undeclared fields (reported once per rule during L5/L6 evaluation) |
| `E1005` | Schema file missing or unreadable (reserved) |

### L2 Type

| Code | Meaning |
| --- | --- |
| `E1101` | Type mismatch |
| `E1102` | Cannot coerce to the target type (reserved) |
| `E1103` | Numeric overflow / underflow (reserved) |
| `E1104` | Invalid enum variant (reserved) |

### L3 Value

| Code | Meaning |
| --- | --- |
| `E1201` | Numeric value out of range (min/max) |
| `E1202` | String length out of bounds |
| `E1203` | Regex does not match |
| `E1204` | Enum value not allowed |
| `E1205` | Array length out of bounds |

### L4 Table

| Code | Meaning |
| --- | --- |
| `E1301` | Duplicate primary key |
| `E1302` | Duplicate composite unique key |
| `E1303` | Missing required row (reserved) |
| `E1304` | Row order violation |
| `E1305` | Required table is empty (reserved) |

### L5 Reference

| Code | Meaning |
| --- | --- |
| `E1401` | Reference target does not exist |
| `E1402` | Reference to a deleted entity (reserved) |
| `E1403` | Circular reference (implemented: table-level reference cycle WARNING, escalated by warnings_as_errors) |
| `E1404` | Cardinality violation (implemented, **source-side semantics**, settled on 2026-10-10: `cardinality` declares how this field references its target — `one` (default) and `optional` take a single value, where `optional` tolerates null while null is a violation under `one`; `many` requires an array and validates each element through E1401/E1410; shape mismatches (array with `one`, scalar with `many`, null with `many`) report this code and name the fix; unknown spellings report `E1004`. The target-side 1:1 reading was rejected — as the default `one`, it would wrongly reject all legitimate many-to-one configurations) |
| `E1410` | Referenced target exists but the semantic predicate is not satisfied (the predicate is evaluated on the target row; a missing optional field on the target row is treated as taking no stance; incomparable types on the two sides also report this code) |
| `E1411` | Referenced target violates a field constraint |

### L6 Semantic

| Code | Meaning |
| --- | --- |
| `E1501` | Assertion expression is false (or the two sides are incomparable in type; missing optional fields take no stance; `warning_only` downgrades to WARNING) |
| `E1502` | Cross-field constraint violation (reserved) |
| `E1503` | Expression evaluation error (e.g. division by zero) (reserved) |

### L7 Game Rule

| Code | Meaning |
| --- | --- |
| `E1601` | Plugin validator reported an error |
| `E1602` | Plugin not found (reserved) |
| `E1603` | Plugin execution failed (panic, timeout, etc.) (reserved) |

### Build / Target (Generation Time)

| Code | Meaning |
| --- | --- |
| `E9001` | No generator found for the target format (reserved) |
| `E9002` | Target generation failed |
| `E9003` | Deterministic build broken (output not reproducible) |
| `E9004` | Manifest generation failed (reserved) |
| `E9005` | Profile does not exist (reserved) |
| `E9006` | Field visibility conflict |

### Editor (Editor Interchange, Phase Three)

| Code | Meaning |
| --- | --- |
| `E1701` | Editor-state document cannot be parsed into a Schema (syntax/shape/unknown kind), located by JSON path |

### Registry (Configuration Registry, Phase Three R Series)

| Code | Meaning |
| --- | --- |
| `E1801` | Publish conflict: republishing the same version with a different whole-package fingerprint (any difference in `build_id`/`content_hash` — data, schema, and `--env` environment changes all count; the registry does not rewrite history), illegal package/version names, index write failure |
| `E1802` | Registry reference cannot be resolved: illegal `registry:` spec, package/version does not exist, illegal version requirement, explicit version not satisfying the `[dependencies]` pin, the migration chain's terminal version falling outside the pin (linked gate, design §46), no version in the range satisfies the requirement, `[registry].path` not configured, corrupted index, unreachable remote root (no cache fallback) |
| `E1803` | Registry entry ledger verification failed (local tampering / added or removed files, or downloaded remote bytes not matching the ledger, or a corrupted remote ledger) — rejected on both the publish and the resolve sides; the `registry verify` full-ledger audit reports under the same code: corrupted entries, index records drifting from the ledger, and orphan directories with no index record |

### Remote Source (Remote Data Sources, S Series)

| Code | Meaning |
| --- | --- |
| `E1901` | Remote fetch failure: network / DNS / timeout (bounded retries exhausted), 404 or other non-auth error status, 429 without `Retry-After` / unparseable / above the 30s cap or rate-limit retries exhausted, pagination contract violations (non-array page / cycles / over 1000 pages / broken next links), non-canonical gsheet spec / illegal spreadsheet id (HTTP / Sheets sources wired up); DB connection failure / illegal DSN / session read-only pin failure / statement execution failure (DB source wired up) |
| `E1902` | Remote authentication / authorization rejected (HTTP 401 / 403) (HTTP / Sheets sources wired up) |
| `E1903` | Remote response shape invalid: Sheets response is not a row set / empty header row / majorDimension not ROWS (Sheets source shape gate wired up) |
| `E1904` | Missing credentials: `[remote.<scheme>].dsn_env` / `[remote.gsheets].credential_env` not declared, or the declared env unset / empty (DB / Sheets sources wired up) |
| `E1905` | Illegal remote query: not starting with SELECT, multiple statements (semicolons), comments, row-lock clauses, `INTO`, non-safe table names outside named queries (DB source wired up) |
| `E1906` | Remote unreachable but fell back to the previous cached copy (offline-fallback WARNING — non-fatal, noted on stderr; happens only for transport-class failures, 404 / 401/403 never fall back; `--no-cache` disables the fallback and restores a hard `E1901`) |

Bad JSON from the HTTP source (`cage-source-http`, design §45 S1) does not go through E1903 — it goes through the same Parse diagnostic as local files (`E0001` with line/column location). The DB source's (`cage-source-db`, §45 S2) row set is canonical JSON produced by the adapter itself, so its shape cannot be invalid, and it likewise bypasses E1903. DB source loading order: parse spec → named query / table name resolution (E1905) → SELECT whitelist (E1905) → DSN env resolution (E1904) → connect (E1901) — the whitelist and credential checks both happen before any network contact. Sheets source (`cage-source-sheets`, §45 S3) loading order: parse spec and spreadsheet id (E1901) → credential env resolution (E1904) → fetch (401/403 → E1902, everything else → E1901) → shape gate (E1903) — spec and credential checks likewise happen before any network contact, error diagnostics only cite the `gsheet:<id>/<tab>` spec, and the API key never lands in logs. For all three sources, transport-class fetch failures (connection / DNS / timeout) fall back to the cached copy when one exists and emit an `E1906` WARNING (`--no-cache` disables the fallback); 404 and 401/403 never fall back — stale bytes must not mask a remote deletion or a revoked access.

### Artifact Distribution (Distribution, A Series)

| Code | Meaning |
| --- | --- |
| `E2101` | Distribution transport failure: bundle export could not read an entry (package / version missing, entry directory or ledger missing, illegal package / version name), could not write the bundle file; push transport failure (retries exhausted), status probe unreadable, remote index unreadable; presigned push with a malformed map / non-absolute http(s) URL / missing `index.put` / coverage gap (a failure before any PUT closes it) (wired up) |
| `E2102` | Distribution target rejected credentials: the push status probe or any PUT received HTTP 401 / 403; the token only ever appears in request headers, never in error text or logs (wired up) |
| `E2103` | Bundle ledger verification failed: the import side rejects unverified bytes at the `verify_snapshot` trust gate — structural problems (member path escapes, malformed index, missing ledger), trust-gate rejection, and index excerpts drifting from the ledger are all rejected before the ledger is written (wired up) |
| `E2104` | Distribution target refused writes: a push PUT received 405 / 409 or another explicit 4xx — the server has no write channel; fall back to "local publish + static hosting" (wired up) |
| `E2105` | Distribution credentials missing: `--auth-env` / `[registry].auth_env` not declared, or the declared env unset — fails before any network contact (wired up) |
| `E2106` | Signing key unavailable: the environment variable named by `--key-env` is unset / empty / not base64 / not a 32-byte Ed25519 key, or the library rejected the key material, or keygen persistence and sidecar writing failed (wired up) |
| `E2107` | Bundle signature verification failed: the `.sig` sidecar is missing or malformed, the algorithm name is unrecognized, or the detached ed25519 signature fails to verify under the trusted public key — the bundle was signed by a different key or its bytes were altered (wired up) |

### Migration (Migration, M Series)

| Code | Meaning |
| --- | --- |
| `E2001` | Migration rule file invalid: unreadable, not valid YAML, missing `from` / `to` / `steps`, `from` equal to `to`, empty steps, unknown transform name, malformed payload (wired up) |
| `E2002` | Migration rule references invalid: a table / field referenced by a step does not exist in the from-schema, or the rename target collides with an existing table / field name (wired up) |
| `E2003` | Migration step not satisfied by the data: the widening direction is unsafe (narrowing / cross-family / Int64·UInt64→Float64 precision loss), a row value exceeds the widened target's value domain, or a table or row field referenced by a step does not exist in the document — the whole run fails rather than silently dropping data (wired up) |
| `E2004` | Post-migration revalidation failed: the migration artifacts do not pass the full L0–L6 stack under the new schema, and the error list is emitted alongside E2004 (wired up) |

### Internal (System Level)

| Code | Meaning |
| --- | --- |
| `E9901` | Internal invariant broken (bug) (reserved) |
| `E9902` | Source read I/O error |
| `E9903` | Configuration error (CLI arguments, missing file) (reserved) |
| `E9904` | Plugin loading failed (reserved) |
