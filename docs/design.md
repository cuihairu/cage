[中文](/zh/design.md) | English

# Cage

> Game Configuration Compiler, Validation & Transformation Framework

**Cage** is a general-purpose configuration compilation, validation, and transformation framework for game development.

It parses all kinds of **human-maintainable configuration sources (Authoring Sources)** into a unified intermediate configuration model, then — after multi-level validation, cross-configuration reference resolution, business semantic validation, and normalization — generates configuration assets or code in various formats according to the needs of the target platform and runtime.

Cage is not tied to Excel, and it is not tied to JSON.

Excel is only one of Cage's input sources; JSON, YAML, CSV, databases, spreadsheet services, and any future configuration sources can plug in through a Source Adapter.

---

## 1. Positioning

The problem Cage solves is not:

> Converting Excel to JSON

It is:

> **How to reliably compile the scattered, heterogeneous, human-oriented configuration data produced during game development into validated, deterministically built configuration assets that different runtimes can consume directly.**

Core flow:

```text
Authoring Sources
       |
       v
+------------------+
|  Source Adapters |
+--------+---------+
         |
         v
+------------------+
| Canonical Model  |
+--------+---------+
         |
         v
+--------------------------+
| Validation Pipeline      |
|                          |
| Syntax / Schema / Type   |
| Value / Table / Reference|
| Semantic / Game Rules    |
+------------+-------------+
             |
             v
+------------------+
| Normalize / IR   |
+--------+---------+
         |
         v
+------------------+
| Target Generators|
+--------+---------+
         |
    +----+----+--------+
    |    |    |        |
   JSON CSV  C#       Lua
             Python   Protobuf
             ...
```

---

# 2. Why Cage Is Needed

Configuration in a game project typically serves all of these at once:

- Designers
- Programmers
- The client
- The server
- The toolchain
- CI/CD
- Test environments
- Operations

Different roles need different representations.

For example, the same Item configuration:

```text
策划：
    Excel

客户端：
    JSON / CSV / C#

服务端：
    JSON / CSV / Lua / Python

工具：
    JSON / SQLite / Binary
```

Without a unified configuration build system, a project easily drifts into:

```text
Excel -> Python Script
Excel -> C# Script
Excel -> Lua Script
Excel -> JSON Script
Excel -> CSV Script
```

and then every script:

- parses on its own
- converts on its own
- validates on its own
- handles default values on its own
- handles references on its own
- reports errors on its own

which ultimately produces a whole set of problems:

1. Different outputs may produce different results.
2. Validation logic is scattered across separate scripts.
3. Cross-table references cannot be validated in one place.
4. Configuration errors only surface when the game runs.
5. Changing one field means changing multiple converters.
6. CI gets no unified configuration quality gate.
7. Configuration never becomes a traceable, reproducible build artifact.

Cage's goal is to unify this whole chain.

---

# 3. Core Concepts

Cage divides the configuration system into six core concepts:

```text
Source
Schema
Model
Validation
Transform
Artifact
```

## 3.1 Source

Where configuration enters from.

For example:

```text
Excel
CSV
JSON
YAML
TOML
XML
SQLite
Database
Remote API
Google Sheets
Custom Format
```

A Source is only responsible for:

> **Reading external data into Cage.**

It should not be responsible for business validation.

### Ruling on Excel Sheet-Level Metadata and Cell Comments (2026-10-12)

The two items previously marked "not implemented" (source.md: cell comments Comment / sheet-level metadata Sheet Metadata) are hereby ruled on together: one is chartered for implementation, and the other is explicitly rejected.

**Sheet Metadata → chartered for implementation: hidden sheets never enter the Canonical Model.**
calamine 0.36 exposes `sheets_metadata()` (`SheetVisible::Visible / Hidden / VeryHidden`; ODS maps `style:display`). The ruling: any sheet with `visible != Visible` is skipped before parsing — hiding is the author's deliberate expression, and the draft/staging surface is never delivered; a hidden sheet landing in the artifacts is a behavior defect, not a capability gap. No warning is emitted (skipping is the semantics, not an anomaly), and no escape hatch for "explicitly naming a hidden sheet" is introduced (if you really want it back, unhide the sheet — author intent gets exactly one place to be expressed, inside Excel).

**Cell comments (Comment) → explicitly rejected (rationale registered, plus conditions for reopening).**
Comments are author-side annotations, not data: validation never consumes them, artifacts never contain them, diffs never compare them — the only effect of carrying them through the Canonical Model and the whole pipeline is a wider `TypedValue` surface and more work at every serialization. Comment anchoring also depends on hand-written OOXML parsing of `xl/comments*.xml` + VML shapes (ClientData Row/Col), which calamine does not expose — bringing in that parsing for zero pipeline consumers is disproportionate. **Reopen condition**: charter it when the editor wants an inline display of author intent; at that point it belongs to the editor exchange layer (W1 adds a field to the exchange model), not the build chain.

---

## 3.2 Schema

Defines the structure and constraints of configuration.

For example:

```yaml
table: Item

primary_key: id

fields:
  id:
    type: uint32
    required: true

  name:
    type: string
    required: true

  price:
    type: uint32
    min: 0

  type:
    type: enum
    values:
      - Weapon
      - Armor
      - Consumable
```

A Schema defines:

- Fields
- Types
- Required
- Default values
- Ranges
- Enums
- Arrays
- Objects
- Uniqueness
- References
- Output information

---

## 3.3 Canonical Model

All Sources should land in one unified intermediate model.

For example:

```text
Excel
JSON
YAML
CSV
SQLite
       |
       v
Canonical Model
```

The Canonical Model should not be equated directly with JSON.

Recommended basic types:

```text
Null
Bool
Int
UInt
Float
String
Bytes
Array
Object
```

It also carries:

```text
Source Location
Schema Information
Type Information
Metadata
```

That is what makes this possible:

```text
monster.xlsx
Sheet: Monster
Row: 27
Column: DropItemID
```

pinpointing the exact location in error messages.

---

# 4. Source Adapter

The Source Adapter is Cage's input plugin.

Architecture:

```text
                  Cage Core
                     |
        +------------+-------------+
        |            |             |
     Excel         JSON          YAML
        |            |             |
        +------------+-------------+
                     |
              Canonical Model
```

The first phase can support:

```text
Excel
CSV
JSON
YAML
```

Future extensions:

```text
XML
TOML
SQLite
MySQL            ← 已实装（§45，S2 cage-source-db）
PostgreSQL       ← 已实装（§45，S2 cage-source-db）
Google Sheets    ← 已实装（§45，S3 cage-source-sheets）
HTTP API         ← 已实装（§45，S1 cage-source-http）
Custom Binary
```

Important principle:

> The Source Adapter is only responsible for reading and parsing, not for game business logic.

---

# 5. Excel

Excel is a very common Authoring Source in the game industry, but it is not Cage's core abstraction.

For example:

```text
Item.xlsx
Monster.xlsx
Skill.xlsx
Quest.xlsx
```

The Excel Adapter can handle:

- Workbook
- Worksheet
- Header
- Cell
- Row
- Column
- Merged Cells
- Formula
- Comment
- Cell Type
- Sheet Metadata

It is ultimately converted into:

```text
Document
  |
  +-- Table: Item
  +-- Table: Monster
  +-- Table: Skill
```

---

# 6. Decoupling Schema from Source

Do not hard-code the Schema into Excel.

For example:

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

The same Schema can be used for multiple Sources.

---

# 7. Validation Pipeline

The core of Cage is not Transform, but Validation.

It is recommended to split validation into multiple stages.

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

---

# 8. Syntax Validation

First, check whether the input can be parsed correctly.

For example:

```yaml
foo:
    - a
      - b
```

The YAML itself is invalid.

Cage:

```text
ERROR E0001

Source:
    config/test.yaml

Location:
    line 3

Invalid YAML syntax.
```

---

# 9. Schema Validation

Check whether the configuration structure conforms to the Schema.

For example, a missing field:

```text
ERROR E1001

Item[10001]

Missing required field:
    price
```

An unknown field:

```text
ERROR E1002

Item[10001]

Unknown field:
    pric
```

---

# 10. Type Validation

For example:

```text
Level = "abc"
```

Schema:

```yaml
type: uint32
```

Error:

```text
ERROR E1101

Item[10001].Level

Expected:
    uint32

Actual:
    string

Value:
    "abc"
```

---

# 11. Value Validation

A correct type does not mean the value is reasonable.

For example:

```yaml
level:
  type: uint32
  min: 1
  max: 100
```

The data:

```text
level = 999
```

The result:

```text
ERROR E1201

Item[10001].level = 999

Allowed range:
    1..100
```

Supported:

```text
min
max
min_length
max_length
regex
enum
unique
required
```

---

# 12. Table Validation

Check the internal consistency of a single table.

For example:

```text
ID 必须唯一
```

The data:

```text
10001
10002
10001
```

The result:

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
组合唯一
字段组合约束
排序要求
空值规则
```

---

# 13. Reference Validation

One of the most important capabilities in game configuration.

For example:

### Item

```text
ID
10001
10002
10003
```

### Monster

```text
ID       DropItemID
20001    10001
20002    10002
20003    99999
```

Schema:

```yaml
DropItemID:
  type: uint32

  reference:
    table: Item
    field: ID
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

---

# 14. Reference Should Not Only Validate "Existence"

For example:

```text
Monster.DropItemID
        |
        v
Item.ID
```

The Item exists:

```text
Item[10001]
```

But:

```text
Item[10001].Type = QuestItem
```

while Monster drops only allow:

```text
Weapon
Armor
Consumable
```

In this case, it should continue to report:

```text
ERROR E1410

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

Therefore, the Reference system should support:

```text
Existence
Type
Predicate
Cardinality
Compatibility
```

---

# 15. Configuration Dependency Graph

Cage should build a configuration dependency graph.

For example:

```text
Monster
   |
   | DropTableID
   v
DropTable
   |
   | ItemID
   v
Item
```

Ultimately:

```text
Monster
   |
   +----> DropTable
              |
              +----> Item
```

This dependency graph can be used for:

- Reference checking
- Build order
- Incremental builds
- Change impact analysis
- Deletion checks
- Cycle detection
- Debugging

---

# 16. Semantic Validation

Schema solves structural problems.

Semantic Validation solves:

> Whether the data makes sense in combination.

For example:

```text
MinLevel <= MaxLevel
```

```text
BuyPrice >= SellPrice
```

```text
Skill.Cost >= 0
```

```text
Monster.DropTableID 必须存在
```

Expressions can be defined:

```yaml
rules:
  - name: level_range
    assert: min_level <= max_level

  - name: price_range
    assert: sell_price <= buy_price
```

---

# 17. Game Rule Validator

Complex game business logic should not be crammed entirely into the Schema DSL.

Plugins should be supported:

```text
Schema
Expression
Code Validator
```

For example:

```cpp
class Validator {
public:
    virtual void validate(
        const ConfigContext& context,
        const Document& document,
        Diagnostics& diagnostics
    ) = 0;
};
```

A game project can implement:

```text
ItemValidator
SkillValidator
MonsterValidator
QuestValidator
DropTableValidator
MapValidator
```

That way, Cage Core does not need to understand game-specific business logic.

## 17.1 Final Decision on the Plugin Sandbox Plan

Comparison and selection of the three candidate execution models:

| Dimension | A. In-process trait | B. Dynamic library (C ABI shim) | C. Script sandbox (WASM/Rhai/Lua) |
| --- | --- | --- | --- |
| Type safety | Full Rust types, zero ABI layers | Needs a C ABI shim (Rust trait objects cannot cross the dylib boundary), data serialized across the boundary | Host API needs a dedicated projection, values converted back and forth |
| Deployment | Changing a plugin requires recompiling the host | Compile-time decoupling, plugins delivered independently and composed per project | Distributed with the config, no compilation needed |
| Isolation | None (trusted code) | None (loading implies trust) | Resource limits / capability trimming, can run untrusted code |
| Performance | Native | Native + one serialization across the boundary | At least an order of magnitude slower |
| Failure mode | Panics surface directly | Boundary catches panic/timeout (E1603) | Sandbox can be killed once limits are exceeded |
| Implementation cost | Already in place | Medium (shim + loader + error mapping) | High (runtime embedding + API design) |

Decision (layered, not either/or):

1. **Now: A (in-process trait)**. The trait `GameRuleValidator` + the registry
   `GameRuleRegistry` are implemented, and the built-in sample rule
   `power_curve` (when any table contains both `level` and `attack` fields,
   assert `attack <= level * 100 + 50`) runs end to end:
   `cage check --level gamerule` loads → validates → emits `E1601`
   diagnostics (row-level location + hint carrying the computation).
   Embedders (game servers / toolchains) likewise inject their own rules via
   `registry.register(...)`.
2. **Third-party distribution: B (dynamic library + C ABI shim)**. Game
   teams write plugins in their own language stack and compile them into
   dylibs; the host loads them through the shim and adapts them back to the
   same trait. Error codes are already reserved: `E1602` (plugin not found),
   `E1603` (panic/timeout), `E9904` (load failure). The trust model matches
   the architecture page: **locally trusted code**.
3. **Untrusted code (Registry/community rules): not executed**. If a hosted
   runtime need genuinely arises, C will be evaluated then, with WASM
   preferred (capability trimming + resource limits are mature); it is not
   in this phase.

The trait is the stable contract — all three layers A/B/C share the same
interface; migrating from in-process to dynamic library does not change
business plugin code, only the packaging.

---

# 18. Validation Level

Suggested definitions:

```text
L0 Parse
L1 Schema
L2 Type
L3 Value
L4 Table
L5 Reference
L6 Semantic
L7 Game Rule
```

But these do not all have to be executed.

You can:

```bash
cage check
```

to run full validation.

Or:

```bash
cage check --level schema
```

to run only the Schema layer.

---

# 19. Diagnostics

Diagnostics should be a first-class citizen.

Every error contains at least:

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

For example:

```text
ERROR E1401

File:
    monster.xlsx

Sheet:
    Monster

Cell:
    G27

Row:
    Monster[20003]

Field:
    DropItemID

Value:
    99999

Reference:
    Item.ID

Message:
    Target does not exist.

Hint:
    Add Item[99999] or change DropItemID.
```

Supported:

```text
ERROR
WARNING
INFO
```

This way they can be directly consumed by:

- CLI
- IDE
- CI
- Web UI

---

# 20. Normalize

After validation passes, normalization begins.

For example:

```text
"100"
100
100.0
```

Unified as:

```text
UInt32(100)
```

Booleans:

```text
yes
YES
true
1
```

Unified as:

```text
Bool(true)
```

The goal of Normalize:

> Data with the same semantics should produce the same Canonical Representation.

---

# 21. Transform

Transform is responsible for:

> Canonical Model → Target Artifact

Not for re-validating data.

Architecture:

```text
                  Valid Model
                       |
       +---------------+---------------+
       |               |               |
      CSV             JSON             Code
                                       |
                              +--------+--------+
                              |        |        |
                             C#      Python    Lua
```

---

# 22. Template Target

The Target Generator is also a plugin. The list of data and code targets
stays unchanged:

Phase one:

```text
CSV
JSON
C#
Python
Lua
```

Later:

```text
TypeScript
C++
Go
Protobuf
MessagePack
FlatBuffers
Binary
SQLite
```

## Templated Form (G Series, Chartered 2026-10)

Tera (Jinja-style, filters / inheritance / macros) unifies the official and
user-defined code generation surface (new crate `cage-target-template`):

- **The entire IR as template variables**: the Schema is serialized in
  full — tables / fields / types / descriptions / default values are all
  referenceable inside templates — plus a top-level `schema_hash`; per-table
  templates additionally receive the current `table` variable. The iteration
  order of fields and tables = the schema declaration order (serde_json
  preserve_order explicitly declared, the same policy as each language's
  official generator, no drifting with dependency-graph feature unification)
- **The template file name is the output file name template**:
  `{table}.py.tera` produces one file per table in table-name order;
  templates without `{table}` render once globally (output name = file name
  minus `.tera`) — following the same `{table}` placeholder policy as
  `file_template`
- **No logic written in templates**: naming conventions (snake_case /
  camelCase / PascalCase / kebab_case / SCREAMING_CASE) + the `field_order`
  field-name order and per-language type mappings / default value literals
  are made into Tera filters (convention layer always registered, language
  layer mounted via `options.lang_filters`); decisions stay in Rust (the old
  plan layer's job gets a new mount point, not moved into templates)
- **Determinism**: artifact order = template registration order (filesystem
  mode in path-name order, official in-memory templates in the order passed
  in) × table-name order; the Tera workspace is version-locked; templates
  ship with the source — same schema + same templates is byte-for-byte
  identical; after the official templates were rewritten, existing tests
  remaining byte-for-byte unchanged is the acceptance anchor

Implementation status:

- G1 template engine integration: delivered — `cage-target-template` (Tera
  instance wrapper, IR context bridge, both rendering forms of file name
  mapping, first landing of the three naming-convention filters, from_config
  reads `options.template_dir`, 10 unit tests)
- G2 official template rewrite: delivered — the engine gains
  `generate_official` (in-memory templates + two hooks: `setup` registers
  language filters, `extras` merges per-context language-precomputed
  decisions, output order = order passed in × table-name order); all 7
  crates rewritten (`templates/table.{lua,cs,py}.tera` +
  `enums.{lua,cs,py}.tera` shipped in-package via `include_str!`, each with
  18 tests unchanged byte-for-byte; TS/JS one crate dual form, 6 templates
  `table.{ts,js,d.ts}.tera` + `enums.{ts,js,d.ts}.tera`, 28 tests unchanged
  byte-for-byte, the interleaved .js+.d.ts order reproduced by zipping two
  calls in order; C++ `table.h.tera` + `enums.h.tera`, 19 tests unchanged
  byte-for-byte; Go `table.go.tera` + `enums.go.tera`, 20 tests unchanged
  byte-for-byte — gofmt alignment math (a tabwriter port) stays in Rust,
  extras feeds the template the final aligned lines; Java
  `table.java.tera` + `enums.java.tera`, 20 tests unchanged byte-for-byte +
  real-javac verification — paths remapped by class ident (file stem =
  class ident, the engine replaces `{table}` with the schema name,
  artifacts zipped back in emission order))
- G3 custom template loading: delivered — `format = "template"` enters the
  CLI: `code_target_items` gains a Result branch (the bundled seven
  languages' `Some(Ok(…))` infallible policy kept as-is; the template branch
  carries the fallible result of
  `TemplateTargetGenerator::from_config(target).generate(…)`), build panel
  template errors go through `BuildFailure::Io`, gen panel eprintln + exit
  code 2; `template_dir` defaults to `.cage/templates/`, relative paths
  resolve against the project root (the path parameter of build/gen),
  overridable via `options.template_dir`; artifact paths carry the
  `output_dir` prefix (engine `render_registered` uniformly `join_output`;
  the build/gen/diff three panels are covered naturally via the manifest —
  diff only compares the manifest, no wiring needed). Error channel
  enhancements: Tera's top-level Display only says `Failed to render
  '<name>'`; the crate adds `tera_err` to flatten the cause chain (real
  failures such as unknown variables / missing filter arguments surface
  directly), and filesystem mode `add_template_file` errors carry the full
  template path. 5 new CLI tests (conventional directory rendering +
  `options.template_dir` override + build/manifest recording format
  "template" + missing directory / empty directory two forms + syntax errors
  pointing at the template file + two runs byte-for-byte identical); the
  from_config default converges with the conventional location
  (`templates` → `.cage/templates`)
- G4 filter library: delivered — decision functions become a library; no
  logic is written in templates. Language-agnostic layer
  (`cage-target-template`, always registered): the convention filters gain
  `kebab_case` / `SCREAMING_CASE`, plus the new `field_order` (`fields` map
  → an array in field-name order, the unified emission order across official
  languages); `generate_with_setup` opens up the setup hook (`generate` =
  the no-hook special case). Language layer (each `cage-target-*` crate
  exposes `pub fn register_filters(tera, schema[, holder])`): seven
  languages `{lang}_type` (field object → the same type text as the official
  generator, enum references carrying post-allocation identifiers; lua's
  type surface is tags) + seven languages `{lang}_default` (field object →
  the same literal as the official generator; no default / not renderable →
  null); mounted via CLI `options.lang_filters` (comma-separated; unknown
  keys error before rendering with exit code 2), java's holder is derived
  from the `options.enums_file` stem (same source as from_config). Enum
  allocation was refactored out as a zero-behavior-change extraction: go
  `allocate_names` / cpp `allocate_names` (a package-level shared set,
  generate and the filters share the same source), java filter side
  `filter_enum_idents` (the holder seed set), ts reuses `enum_exports`. The
  input convention is unified on field objects (a serialized `FieldSchema`),
  with crate public helpers `field_parts` / `field_default`. 11 new tests
  (seven crates type/default smoke + convention filters and field_order +
  the setup hook + CLI mounting and the unknown-key two forms), workspace
  545 all green; the filter tables went into the target.md Template section
- G5 documentation closeout: delivered — the target.md Template section
  (config samples / output-name and render-order conventions / the IR
  variable table / two tables for convention + language filters / error
  policy, with G3/G4 landed under this sign-off and rechecked); the
  `需求整理.md` status line + generator list + crate landing policy gains
  cage-target-template; the architecture.md structure tree gains
  cage-target-template; the README Target plugin line gains Template Target;
  §22/§23 reconciled against the implementation and rechecked (this panel's
  G1–G4 all delivered, naming conventions / filter list synced to the G4
  reality)

---

# 23. Data Serialization and Code Generation

These two classes of Target should be clearly distinguished.

## Data Targets

```text
JSON
CSV
YAML
MessagePack
Protobuf
FlatBuffers
Binary
```

## Code Targets

```text
C#
Python
Lua
TypeScript / JavaScript
C++
Go
Java
JSON Schema（标准文档，2026-10 实装）
```

### JSON Schema Target Decision Record (2026-10)

**What was decided**: schema export in standard JSON Schema form goes through the `format = "jsonschema"` target (`cage-target-jsonschema`, in the code-binding family panel) — one **self-contained** `{table}.schema.json` per table, draft-07 by default with 2020-12 switchable via `options.draft`; enums are always inlined as `enum` arrays (**instances are always the member name** — L2 fixes the type of enum fields as string, integer literals are only code-generation metadata, so integer enums are also emitted as string enums); both typed Object and table documents emit `additionalProperties: false` (same policy as the validator's "unknown keys rejected"); keys of `map<int, V>` emit a `propertyNames.pattern` numeric-string constraint (int keys are numeric strings in the data model); bytes emits `contentEncoding: base64` under draft-07 (that keyword was removed in 2020-12, so it is not emitted there); documents carry no custom extension keys, and cross-table references and semantic rules have no standard spelling so they do not enter the document (still enforced by the cage pipeline at the source of truth). Determinism = table-name order × fixed key insertion order (serde_json preserve_order); two runs are byte-for-byte identical. **Why**: so that non-cage toolchains (editors, ajv, frontend forms) can consume the schema with zero runtime — this is the off-chain form of "Schema as an asset"; the self-contained single file avoids the `$ref` resolution burden; draft-07 has the widest compatibility surface, with 2020-12 kept as an upgrade path. **Alternatives**: a merged single document with `$defs` + `$ref` (consumers would have to do reference resolution, rejected); a shared enum file (JSON Schema has no standard cross-file import mechanism, and relative `$ref` paths are fragile in distribution scenarios, rejected); defaulting to 2020-12 (tool support is narrower than 07, kept only as an option).

For example, the same data:

```text
Canonical Model
      |
      +---- JSON
      |
      +---- CSV
      |
      +---- C#
      |
      +---- Python
      |
      +---- Lua
      |
      +---- TypeScript / JavaScript
      |
      +---- C++ / Go / Java
```

### Code Generation Approach: plan → render → verify Direct Rendering (current; from G2 on, official generators migrate to bundled templates)

The generator for a Code Target does not build an AST of the target language, nor does it rely on the dynamic capabilities of templates; instead it renders strings directly across the three layers of **plan → render → verify** (for the templated form and its division of labor, see §22 Template Target: from G2 on, the render layer of official generators switches to bundled official templates, while decisions stay in Rust):

```text
Schema
  |
  v
plan（mini-IR：一次算完所有决策——标识符、类型、import、默认值字面量）
  |
  v
render（String + writeln 逐行渲染，多处产物共享同一份 plan）
  |
  v
verify（dev-only：tsc / javac / g++ / gofmt 回验产物，不进 CI 依赖）
```

Why not the official per-language AST library + printer:

- **Build dependencies**: the TypeScript compiler API / javac TreeMaker / go/ast would mean a pure Rust workspace has to carry the Node / JDK / Go toolchains
- **Determinism killer**: AST printers evolve with versions; a formatter upgrade changes output bytes, invalidating manifest hashes / golden byte comparison / incremental-build skipping all at once
- **API mismatch**: these APIs are designed for rewriting existing code (refactoring, rename); assembling declarations by hand is actually more cumbersome (building a field declaration with javac JCTree is far more expensive than writing a line of text)

Why not a template engine (Handlebars/Tera and the like): the output surface of declarative bindings is fixed (about ten statement shapes), so the dynamic capabilities of templates go unused, while pushing escaping and branching logic down into templates would forfeit Rust's type checking; `file_template = "{table}.ts"` in configuration is only a filename placeholder and has nothing to do with code templates.

> Decision update (2026-10, G series, see §22): the conclusion above about "generator internal structure" still holds — the three plan → render → verify layers are unchanged, and decisions (identifiers / types / imports / default-value literals) stay in Rust; but the conclusion about "who may change the generated text" no longer holds: once the need appeared for users to change templates rather than code, the dynamic capabilities of templates (loops / branches / macros / inheritance) are exactly what that scenario wants. The new policy is that official and custom generation share one Tera engine (`cage-target-template`): the official per-language bindings are rewritten as bundled official templates (G2), user-defined templates plug in via `template_dir` (G3), language type mappings and literals become filters (G4), and **no logic is written inside templates** — the worry above about "branching logic sinking into templates" is blocked by "decisions stay in Rust, templates only express text shape + golden byte-for-byte anchoring". The determinism anchor is not relaxed: version-locked Tera workspace + templates shipped with the source + existing golden tests byte-identical. The `file_template` placeholder policy carries over as-is (the template filename `{table}.py.tera` uses the same `{table}` substitution).

Failure-mode analysis for reliability: the generated artifacts are purely declarative code (fields, types, default values) with no control flow — either they fail to compile (exposed on the spot) or they are correct; there is no intermediate state of "compiles fine but quietly misbehaves at runtime". Syntactic correctness is therefore guaranteed by dev-only verification with real compilers (`tsc --strict`, `javac -Xlint:all -Werror`, `g++ -Wall -Wextra -Werror`, `gofmt -l`, all tested in practice against adversarial schemas — keyword field names, `i64::MIN`, non-ASCII, naming collisions), rather than guaranteed by the construction layer. Industry precedent of the same kind: protoc's Java/C++ generators are also string concatenation internally.

Upgrade signal: when some target needs to generate code with logic (inline validation functions, complex runtime support code) or to rewrite users' existing code, introduce an IR layer for that target alone — under the plugin architecture this is a local decision that does not affect the other targets. That signal merged with the "users change templates" need and has landed as the Template Target (§22, G series).

---

# 24. Frontend / Backend Profiles

Do not hard-code "client" and "server" into Core.

A Build Profile can be defined:

```yaml
profile: client

targets:
  - json
  - csv
  - csharp
```

Server side:

```yaml
profile: server

targets:
  - json
  - csv
  - python
  - lua
```

Then:

```bash
cage build --profile client
cage build --profile server
```

---

# 25. Field Visibility

Different targets may not need all fields.

For example:

```yaml
fields:

  id:
    type: uint32

  name:
    type: string

  admin_note:
    type: string
    targets:
      - server
```

Client side:

```text
id
name
```

Server side:

```text
id
name
admin_note
```

This avoids:

> Maintaining two sets of configuration for client and server.

---

# 26. Build Manifest

Each build produces a Manifest:

```json
{
  "project": "game",
  "profile": "client",
  "cage_version": "0.1.0",
  "schema_hash": "...",
  "source_hash": "...",
  "content_hash": "...",
  "artifacts": {
    "item.json": "...",
    "monster.json": "...",
    "skill.json": "..."
  }
}
```

Uses:

- Version tracking
- Deployment
- Rollback
- Client/server version matching
- CI
- Caching
- Incremental build

---

# 27. Deterministic Build

Given the same:

```text
Source
Schema
Cage Version
Profile
```

It should produce the same:

```text
Artifact
Hash
Manifest
```

That is:

```text
Build(A) == Build(A)
```

Avoid:

- Timestamps causing file changes
- Non-deterministic Map ordering
- Random IDs
- Unstable sorting

This is an important foundation for a configuration build system.

---

# 28. Incremental Build

With a Dependency Graph in place, incremental build can be supported.

For example:

```text
Item.xlsx 修改
```

Affects:

```text
Item
 |
 +--> DropTable
 |
 +--> Monster
```

Meanwhile:

```text
Skill
Quest
Map
```

need not be regenerated.

Finally:

```text
Changed Sources
      |
      v
Dependency Graph
      |
      v
Affected Tables
      |
      v
Incremental Build
```

This is a later-stage capability.

---

# 29. Configuration Registry

A remote configuration registry can be added in a later phase:

```text
Cage Registry
```

It stores:

```text
Schema
Source Metadata
Build Manifest
Artifacts
Hash
Version
```

For example:

```text
game-config/
    v1.0.0/
    v1.1.0/
    v1.2.0/
```

But the Registry should not be part of the MVP.

**Implemented (Phase 3 R series)**: The R1 local registry has landed — `<registry>/<包>/<版本>/` is a self-verifying Configuration Snapshot (the same packaged artifact and the same blake3 ledger as `cage snapshot`), and the package directory carries a deterministic `index.json`; `cage registry publish` publishes (idempotent for identical bytes at the same version, E1801 conflict for different bytes), `cage registry list` lists entries, and consumers resolve via `source_roots: registry:<包>[@<版本>]` (ledger verification before loading; E1802 unresolved / E1803 verification failed). R2 schema-side resolution and dependency pins have landed — `schema_path: registry:<包>[@<版本>]` reads the entry's `schema.json` directly (the profile-projected schema captured at publish time), and `[dependencies]` declares version pins for packages (comparator ranges AND-ed together, `^`/`~` expanded; omitting `@版本` picks the highest version satisfying the range, and an explicit `@版本` must also fall within the pin, otherwise E1802). R3 remote read-only resolution has landed — `[registry] path` supports http(s) roots, the protocol is anonymous GET over three resources (package index, entry ledger, entry files), and entry bytes are immutable; resolution = fetch the index → select a version via the same `select_version` → download file by file per the ledger and verify blake3 byte by byte → land the result in the `.cage-cache/registry/<url 指纹>/` cache → pass the `verify_snapshot` trust gate (nothing is loaded unverified); if cache re-verification comes back clean, the cache is reused offline. Remote roots are read-only: publish/list are refused (the protocol has no package enumeration; publishing completes in the local registry and is then hosted by a static server). Authentication (tokens/signatures) is reserved for a separate future project. R4 rollback and cleanup have landed — `cage registry verify` audits the entire registry (re-running ledger verification for every entry + cross-checking index records against entry ledgers + an orphan-directory report, with E1803 items listed one by one), `cage registry gc` applies a rolling-window policy (keep the newest `--keep N` versions per package, minimum 1 — the window is the rollback surface: consumers pinning an old version can still resolve it, while versions squeezed out of the window become E1802 for consumers; orphan directories are swept up as well, and --dry-run reports the same list without writing), and `cage registry remove` performs explicit administrative removal (the version directory is deleted along with its index record while the package index is kept; identical bytes can be cleanly re-published into the registry; the registry itself never implicitly rewrites history). The red line is unchanged: the Registry does not execute uploaded Validators (§35).

---

# 30. CLI

Recommended core CLI:

```bash
cage check
cage build
cage inspect
cage diff
cage verify
cage graph
```

## check

```bash
cage check config/
```

Validates only; does not generate Runtime Artifacts.

## build

```bash
cage build config/
```

Validates and generates targets.

## inspect

```bash
cage inspect Item
```

View the Schema and configuration structure.

## diff

```bash
cage diff build/a build/b
```

Compare configuration versions.

## verify

```bash
cage verify runtime/
```

Verify already-generated Artifacts.

**Landed (2026-10-12)**: Implemented as `cage verify <path>`. Project root → reads the configuration's `output_dir` ledger; it also accepts an output directory or a manifest.json path (keys are resolved relative to the project, with a deployment-tree-compatible fallback that strips the first path segment). Recomputes blake3 file by file + checks sizes; missing / tampered / size-mismatched files are each reported as FAILED, exit code 1; it only reads ledger keys (the same verify-first stance as `/api/artifact`); snapshot packages still go through `cage snapshot --verify` (a separate self-verifying ledger).

## graph

```bash
cage graph
```

Output the configuration dependency graph.

**Landed (2026-10-12)**: Implemented as `cage graph <project> [--format text|dot|json]`. Schema reference declarations are turned into a graph via `DependencyGraph::from_schema` (the same entry point as the L5 reference check and the second incremental layer), with edges carrying reference-field labels; the text view includes the topological build order and a list of cycles, dot is for Graphviz rendering, and json is machine-readable. Cycles are reported but not fatal — raising errors belongs to the reference check; this command is a view. It is schema-driven and does not read data sources.

---

# 31. CI/CD

Cage should be a natural fit for CI.

```text
Git Push
    |
    v
CI
    |
    v
cage check
    |
    +---- Error ---> Build Failed
    |
    v
cage build
    |
    v
Artifacts
    |
    v
Package / Deploy
```

For example:

```bash
cage check --profile client
cage check --profile server

cage build --profile client
cage build --profile server
```

Configuration errors fail in CI first and never get carried into the game.

---

# 32. Warning Policy

Recommended to support:

```text
warning
error
```

and allow CI configuration:

```yaml
ci:
  warnings_as_errors: false
```

Production builds:

```yaml
ci:
  warnings_as_errors: true
```

---

# 33. Project Structure

Recommended:

```text
cage/
├── README.md
├── LICENSE
├── CMakeLists.txt
│
├── docs/
│   ├── architecture.md
│   ├── source.md
│   ├── schema.md
│   ├── validation.md
│   ├── transform.md
│   ├── targets.md
│   ├── cli.md
│   └── build.md
│
├── src/
│   ├── core/
│   │   ├── value/
│   │   ├── document/
│   │   ├── schema/
│   │   ├── diagnostics/
│   │   ├── validation/
│   │   ├── reference/
│   │   ├── dependency/
│   │   ├── normalize/
│   │   └── manifest/
│   │
│   ├── source/
│   │   ├── excel/
│   │   ├── csv/
│   │   ├── json/
│   │   └── yaml/
│   │
│   ├── target/
│   │   ├── csv/
│   │   ├── json/
│   │   ├── csharp/
│   │   ├── python/
│   │   └── lua/
│   │
│   ├── validator/
│   │   └── ...
│   │
│   └── cli/
│
├── schemas/
├── examples/
├── tests/
└── plugins/
```

---

# 34. Plugin Model

The Cage core should remain as stable as possible.

```text
                    Cage Core
                       |
       +---------------+----------------+
       |               |                |
 Source Plugins   Validator Plugins   Target Plugins
       |               |                |
     Excel          SkillValidator      JSON
     CSV            QuestValidator      CSV
     YAML           MonsterValidator    C#
     JSON                                Python
                                         Lua
```

This lets third parties extend Cage without modifying the Core.

---

# 35. Security / Isolation

If projects are allowed to write business Validators, the following needs attention:

```text
Validator
    |
    v
可能执行任意代码
```

Therefore:

- Local development can use Native Plugins
- CI can consider a Sandbox
- Untrusted project code should not be executed by default
- The Remote Registry should not directly execute uploaded Validators

This is an issue that needs serious consideration in a later phase.

---

# 36. MVP

The first version should not try to solve every problem.

Proposed MVP:

### Source

```text
Excel
JSON
YAML
CSV
```

### Schema

```text
type
required
default
enum
min
max
unique
reference
```

### Validation

```text
Syntax
Schema
Type
Value
Unique
Reference
Expression
```

### Target

```text
JSON
CSV
```

### CLI

```text
check
build
diff
inspect
```

### Build

```text
Deterministic
Manifest
Hash
```

This is already enough to form genuinely valuable infrastructure.

---

# 37. Phase Two

Add:

```text
C#
Python
Lua
Protobuf
MessagePack
```

As well as:

```text
Plugin SDK
Game Rule Validator
Dependency Graph
Incremental Build
CI Integration
```

---

# 38. Phase Three

Then consider:

```text
Web UI                 ← 已实装（W 系列）
Schema Editor          ← 已实装（W 系列）
Configuration Registry ← 已实装（R1–R4）
Remote Source          ← 三源已实装（S1–S3：HTTP / DB / Sheets）（§45）
Google Sheets          ← 已实装（§45，S3 cage-source-sheets）
Database Source        ← 已实装（§45，S2 cage-source-db）
Migration              ← 已立项（2026-10 拍板，M 系列，§46）
Artifact Distribution  ← 已立项（2026-10 拍板，A 系列，§47）
```

---

# 39. What Not to Do

Cage should not become:

### Excel Editor

Excel itself is already a mature Authoring Tool.

### Game Database

Cage builds configuration; it is not meant to be the game's runtime database.

### Secret Manager

Passwords, tokens, and keys do not belong to ordinary game configuration.

### Game Logic Framework

Cage can validate business rules, but it should not become the game server's logic framework.

### Hard Binding to a Specific Game Engine

Not bound to:

```text
Unity
Unreal
Godot
Cocos
```

It should serve all game clients and servers.

---

# 40. Difference from an Ordinary Configuration Converter

An ordinary converter:

```text
A -> B
```

Cage:

```text
Source
  |
  v
Parse
  |
  v
Canonical Model
  |
  +--> Schema
  |
  +--> Type
  |
  +--> Constraint
  |
  +--> Reference
  |
  +--> Semantic
  |
  +--> Game Rules
  |
  v
Validated Model
  |
  +--> JSON
  +--> CSV
  +--> C#
  +--> Python
  +--> Lua
  +--> Protobuf
  +--> ...
```

So Cage's value is not:

> **Conversion.**

But rather:

> **Establishing a reliable configuration compilation chain.**

---

# 41. The Casino Cage Metaphor

Cage takes its name from the **Cage / Cashier's Cage** in a casino.

In a casino, the Cage is:

```text
Cash
  |
  v
Cage
  |
  +--> Verify
  +--> Exchange
  +--> Record
  |
  v
Chips
```

Cage:

```text
Authoring Data
  |
  v
Cage
  |
  +--> Parse
  +--> Validate
  +--> Normalize
  +--> Transform
  +--> Audit
  |
  v
Runtime Assets
```

So it is not:

> "An Excel converter"

It is more like:

> **The exchange and settlement boundary before configuration enters the runtime world.**

The Cage in a casino does not care what you end up playing:

```text
Poker
Blackjack
Baccarat
```

Likewise, Cage should not care what the configuration ultimately serves:

```text
Unity
Cocos
Unreal
Game Server
Tool
```

It is only responsible for:

> **The input data must be valid, complete, and verifiable before it can be exchanged for runtime assets.**

---

# 42. One-Sentence Definition

In English:

> **Cage is a configuration compiler and validation framework for game development. It transforms heterogeneous authoring data into validated, deterministic runtime artifacts.**

In Chinese:

> **Cage 是一个面向游戏研发的配置编译与验证框架，将异构的配置源转换为经过验证、确定性构建的运行时配置资产。**

---

# 43. Core Design Principles Summary

```text
1. 不绑定 Excel
2. Source 与 Target 解耦
3. 所有输入进入统一 Canonical Model
4. Validation 与 Transformation 分离
5. Schema 负责结构
6. Reference 负责跨配置关系
7. Semantic Rule 负责组合逻辑
8. Plugin Validator 负责复杂游戏业务
9. Target Generator 负责输出
10. Build 必须可重复
11. Artifact 必须可追踪
12. Diagnostics 必须精确到 Source Location
13. Client / Server 使用 Profile，而不是硬编码
14. Core 不绑定具体游戏引擎
15. Excel 只是第一种 Source
```

---

# 44. Final Architecture

```text
                             CAGE
              Game Configuration Compiler
                                  |
       +--------------------------+--------------------------+
       |                          |                          |
       v                          v                          v
  Authoring Sources           Schema / Rules             Profiles
       |                          |                          |
  +----+----+----+                |                    +-----+-----+
  |    |    |    |                |                    |           |
Excel CSV JSON YAML               |                 Client       Server
  |    |    |    |                |                    |           |
  +----+----+----+----------------+--------------------+-----------+
                                  |
                                  v
                         +----------------+
                         | Canonical IR   |
                         +-------+--------+
                                 |
                                 v
                      +-----------------------+
                      | Validation Pipeline   |
                      |                       |
                      | Parse                 |
                      | Schema                |
                      | Type                  |
                      | Value                 |
                      | Table                 |
                      | Reference             |
                      | Semantic              |
                      | Game Rules             |
                      +-----------+-----------+
                                  |
                                  v
                         Validated Model
                                  |
                    +-------------+-------------+
                    |             |             |
                    v             v             v
                  JSON          CSV           Code
                                              |
                                      +-------+-------+
                                      |       |       |
                                     C#     Python   Lua
                    |
                    +-----------> Protobuf
                    |
                    +-----------> MsgPack
                    |
                    +-----------> Binary
                                  |
                                  v
                           Runtime Artifacts
                                  |
                                  v
                         Manifest / Hash / CI
```

---

## Conclusion

The abstraction Cage most needs to establish is not:

```text
Excel -> JSON
```

But rather:

```text
                 Any Source
                     |
                     v
               Canonical Model
                     |
                     v
           Validate Everything
                     |
                     v
              Validated Model
                     |
                     v
               Any Target
```

**Excel, CSV, YAML, and JSON are just Sources; JSON, CSV, C#, Python, Lua, Protobuf, and so on are just Targets.**

What truly belongs to Cage Core is the middle part:

> **Schema + Canonical Model + Reference Graph + Validation Pipeline + Semantic Rules + Deterministic Build**

This is also the part of the project most worth building into standalone open-source infrastructure.


---

# 45. Remote Source

The MVP's four Sources all read local files. There are three scenarios in which designer data actually lives in online systems: numeric tables hosted in Google Sheets, operations and account data in business databases (MySQL / PostgreSQL), and toolchains that have exposed some tables as JSON APIs. Remote Source brings these three classes of input into the same compilation chain; together with the HTTP API that makes four sources. All four sources are strictly **read-only**.

## Positioning and Red Lines

- The remote end is a data source, not a trusted source. Retrieved bytes go through the same Parse → Schema → … → Game Rule pipeline as local files, with L0-L7 executed in full and no bypass. "Do not load what has not been validated" holds for remote bytes just the same (the same policy as R3's cache reverification).
- Read-only. No write-back, no DDL/DML; editing of configuration data still happens in author-side tools, and the `cage web` save surface is unchanged.
- No remote code execution. No stored-procedure calls, no eval of response content — the same red line as §35's Validator.
- Credentials never go into cage.toml. The config file only records environment variable names; connection strings and keys are resolved from env at runtime. An unset env fails outright (E1904) — no guessing, no logging.

## Source Syntax and Mapping

All four sources are declared through schemes in `[source_roots]`, in the same style as the `registry:` prefix. An `http(s) URL` appearing in `[source_roots]` is an HTTP API source; appearing in `[registry].path` it is a registry root — the two are never mixed:

```toml
[source_roots]
items    = "https://api.example.com/v1/items.json"   # HTTP API：GET JSON
monsters = "mysql:Monsters"          # MySQL：表名或具名查询
drops    = "pg:drop_tables"          # PostgreSQL
levels   = "gsheet:1AbC...xz/Levels" # Sheets：spreadsheet_id / tab

[remote.mysql]
dsn_env = "CAGE_MYSQL_URL"           # 只存 env 名，DSN 不进仓库

[remote.pg]
dsn_env = "CAGE_PG_URL"

[remote.gsheets]
credential_env = "CAGE_SHEETS_CREDENTIAL"   # API key（service account 需
                                            # OAuth JWT 交换，留待实现期）
```

| Source | Fetch | Table / row mapping | Type policy |
| --- | --- | --- | --- |
| HTTP API | A single GET; the response body is the table | Same shape as the local JSON source: `{表名: 行数组}` / single object → `Root` / row array → `Data`; field names = fields | JSON value types map directly |
| MySQL / PostgreSQL | Read-only SELECT; a table name expands to `SELECT * FROM t`; named queries live in `[remote.<scheme>.queries]` | Column names = fields, NULL = Null, rows = records | DECIMAL / NUMERIC rendered as strings, never routed through Float (floating point loses precision; how to interpret them is left to Schema) |
| Google Sheets | Sheets API v4 `values` (`gsheet:<spreadsheet_id>/<tab>`), `valueRenderOption=UNFORMATTED_VALUE` (cached formula values, no recomputation, same policy as the Excel adapter); the API key is read from the env named by `[remote.gsheets].credential_env` (E1904), and error diagnostics cite only the spec, never the URL (the key never reaches logs) | tab = table, first row = header (an empty header cell falls back to `col<i>`, same convention as Excel); empty rows are skipped, short rows padded with null to the header width, and the tab's natural row order = the author-promised order, preserved as-is; a non-row-set / missing header refuses to load (E1903) | Initial values are strings (numbers / booleans kept as JSON text); L2/L3 validation narrows the types |

Named queries and table names are statically validated at load time: only a single statement beginning with `SELECT` is accepted; semicolons, comments, multiple statements, row-lock clauses (`FOR UPDATE` / `FOR SHARE`), and `INTO` are all rejected (E1905). At runtime the connection is pinned to a read-only transaction (PostgreSQL `default_transaction_read_only`, MySQL `SESSION TRANSACTION READ ONLY`) — double insurance. Row-order determinism: if the statement carries its own `ORDER BY`, the author-promised order is respected; otherwise rows are sorted by their serialized form — the build never depends on server-side return order. The row set is serialized into canonical JSON and landed in the cache (row-object key order = SELECT column order, the same ordering policy as CSV / Excel / local JSON; DECIMAL / NUMERIC text kept faithful, binary columns base64), then goes through standard JSON parsing — the same cache anchor and parsing chain as the HTTP source. The cache key covers scheme + DSN (one-way hash) + statement, so different servers / queries never share a slot. The Sheets source uses the same anchor and chain: canonical JSON lands in `.cage-cache/source/<gsheets+id+tab 指纹>/` (the API key does not enter the fingerprint — it does not change byte semantics), and the tab's natural row order, the author-promised order, is preserved as-is.

## Determinism and Cache

The anchor of deterministic builds is **the fetched bytes**, not "the remote's current state". Response bytes land in the cache as-is, and parsing consumes only those bytes — there is no "fetch and compute at the same time":

```text
fetch
   |
   v
字节落 .cage-cache/source/<源指纹>/<指纹>.json
   |
   v
同一套 Parse / 校验流水线（无旁路）→ Canonical Model
   |
   v
内容进 source_hash → build_id → 产物
```

source_hash covers the parsed Canonical Model content (table names / row order / primary keys / fields — `ManifestGenerator::input_hashes` → the private `hash_source`, the same measuring stick). If the remote data changes, the parsed content changes, source_hash follows, and build_id rotates — the change is visible in the manifest; there is no "quietly swapped data". Repeated builds over the same bytes stay byte-for-byte identical, and the golden contracts hold. The source fingerprint reuses R3's `cache_key` policy (first 12 hex of the URL's blake3), and fetching, retries, and cache paths are factored into one shared helper (`cage_core::remote`, reused by both the R3 registry and the per-source adapters) instead of four separate implementations.

Offline semantics (closed out in S4): only transport-class failures (connection / DNS / timeout — connection refused, network down, service unreachable) fall back to the cache and emit an `E1906` WARNING; 404 (the remote has been deleted) and 401/403 (access may have been revoked) do not fall back — stale bytes would fail silently, so these stay hard failures. DB-source fetch failures are all treated as transport-class (credential E1904 / query whitelist E1905 fire before any network is touched, so stale bytes never mask configuration errors). A cache file produced by fallback goes through the same standard JSON parsing as the online path (cached bytes pass the validation gate too). The three sources share one gate, `cage_core::remote::source_cache_fallback`; the WARNING carries only the spec / table name (the URL for http) — credentials never reach logs. `--no-cache` (the four commands check / build / gen / inspect) turns the fallback off: if the fetch fails, the build fails. A freshness cap (max_age) is deferred to implementation.

## Error Codes (the E19xx family)

| Code | Meaning | Status |
| --- | --- | --- |
| `E1901` | Remote fetch failed (network / DNS / retry exhaustion, non-auth error statuses such as 404, DB connection / statement failures) | Implemented (HTTP / DB sources) |
| `E1902` | Authentication / authorization rejected (HTTP 401 / 403) | Implemented (HTTP / Sheets sources) |
| `E1903` | Invalid response shape (not a row set / missing header) | Implemented (Sheets source shape gate) |
| `E1904` | Missing credentials (env unset, or credential file unreadable) | Implemented (DB source dsn_env, Sheets source credential_env) |
| `E1905` | Illegal query (a non-read-only statement was configured) | Implemented (DB source: SELECT whitelist + table-name validation) |
| `E1906` | Remote unreachable but the previous cached copy was used as fallback (offline fallback WARNING, non-fatal; `--no-cache` disables the fallback and restores a hard E1901) | Implemented (one `source_cache_fallback` gate shared by the three sources) |

The full E1901–E1906 family is registered in `codes.rs` and validation.md and all wired up and live (E1901/E1902 with S1, E1904/E1905 with S2, E1903 with S3, E1906 with S4). Bad JSON from the HTTP source does not go through E1903 — it takes the same Parse diagnostics as local files (E0001 with row/column positions). The DB source's row set is canonical JSON produced by the adapter itself, so its shape cannot be illegal and it likewise never passes through E1903; E1903 is consumed by the Sheets source's shape gate (not a row set / missing header / majorDimension not ROWS).

Retries cover only connection-class failures (bounded attempts + backoff, the same policy as the curl retries in ci.yml); 4xx is never retried.

## Deferred to Implementation (Not in the First Release)

- Incremental fetching (row-level): fetch only the changed rows by revision / updated_at — document-level incremental fetch is already implemented (S8 conditional GET, see the decision record below); row-level increments need per-source data contracts
- OAuth user-authorization flow, mTLS, and service account credentials (Sheets ships with API key only in the first release — service accounts need an OAuth JWT exchange)
- ~~Connection pooling, cursor pagination over large tables~~ (landed, 2026-10-13: process-level pooling per DSN + mysql streaming row fetch, see the landing note in the blocker log below); concurrent multi-source fetching (measured data supports staying serial, see the same note)
- Sheets rich text and formula recomputation (the first release caches values only)
- ~~Auto-generating Schema drafts from database table introspection~~ (implemented, S9, see the decision record below)

**Implementation status** (2026-10): S1 delivered — HTTP API source implemented (`cage-source-http`, http(s) URLs written directly in `[source_roots]`); the shared fetch / retry / cache helper first landed in `cage_core::remote` (reused by the R3 registry from the same source). S2 delivered — MySQL / PostgreSQL sources implemented (`cage-source-db`, both backends sharing the row-set mapping: SELECT whitelist E1905 → dsn_env resolution E1904 → connection + session read-only pin → row set as canonical JSON into the cache → standard JSON parsing; CLI error codes E1901 / E1904 / E1905 wired and live). S3 delivered — Google Sheets source implemented (`cage-source-sheets`, `gsheet:<id>/<tab>`, UNFORMATTED_VALUE, first-row header following the Excel convention, initial-values-as-strings policy, tab natural row order preserved, API key via credential_env, E1902 / E1903 / E1904 wired and live; service accounts deferred to implementation). S4 delivered — offline semantics closed out: transport-class fetch failures in all three adapters fall back to the previous cached copy (E1906 WARNING; 404 / 401/403 never fall back); `--no-cache` (check / build / gen / inspect) disables the fallback and restores hard failure; end-to-end tests for remote change → source_hash / build_id rotation were already anchored with S1 (identical-rebuild manifest byte-for-byte identical + both hashes rotating on data change), and this sign-off adds fallback / strict dual-mode tests across the three adapters and at the CLI process level. S5 delivered — error-code wiring closed out and reconciled: repo-wide (codes.rs / validation.md / design.md) "reserved" annotations on the E19xx family zeroed out, with a systematic audit of code coverage over the four sources' error paths on record — HTTP: E1901 (transport / 404 / other statuses / non-http URL) + E1902 (401/403) + E0001 (bad JSON takes the same parse diagnostics as local files) + E9902 (cache write) + E1906 (fallback WARNING); DB: E1905 (spec / whitelist / table name) + E1904 (dsn_env) + E1901 (illegal DSN / connection / session read-only pin / prepare / query — mysql.rs and pg.rs all carry codes) + E9902; Sheets: E1901 (spec / id / transport) + E1904 (credential_env) + E1902 + E1903 (shape gate) + E9902 + E1906 — every path has an existing test asserting its code (crate-level assert-contains + CLI process-level stderr assertions), and diagnostic rendering through `load_project` Err → `error: {e}` is fully covered. S6 delivered — documentation closed out (the source.md future-extension list promoted to official, a Remote Source section added to cli.md, the Remote Source row in `需求整理.md` checked off, and the architecture.md structure tree reconciled); the S series S1–S6 all delivered. S7 delivered — HTTP pagination protocol and rate-limit negotiation (formerly deferred to implementation, see the decision record below). S8 delivered — HTTP conditional GET incremental fetch (document-level: ETag/Last-Modified revalidation, 304 reuses the cached bytes; row-level increments remain on the deferred list, see the decision record below). S9 delivered — database-table introspection Schema drafts (formerly deferred to implementation, `cage schema-draft`, see the decision record below).

### S7 Decision Record (HTTP Pagination and Rate-Limit Negotiation)

**Pagination protocol = the RFC 8288 `Link: <...>; rel="next"` response header.** What was decided: after a successful GET, if the HTTP source's response carries a `Link` header with `rel="next"`, the chain of pages is followed — each page body must be a JSON row array, and pages are concatenated in order into one merged document (bare array → `Data` table); a relative next target is resolved against the current page URL per RFC 3986; the visited set is deduplicated (a loop → hard error); the hard cap `MAX_PAGES` = 1000 pages. **Why**: the Link header is a standard protocol and independent of the response body's shape — no need to guess each API's `next`/`next_page`/`cursor` key names, and key-name guessing is exactly the implicit magic cage has always avoided; relative resolution, loops, and page-count bounds are all locally controllable deterministic semantics. **Alternatives**: a response-body cursor convention (`{"next": url}`) — every provider names it differently, rejected; `X-Next-Page`-style custom headers — non-standard, rejected. **Compatibility**: when the first response has no next, the bytes are returned as-is — the existing contract of single-GET sources (any acceptable shape) stays byte-for-byte unchanged, and the pagination contract only takes effect once a next appears. **No participation in offline fallback**: a pagination-contract violation (non-array page / loop / over the bound / bad link) means "the server has answered", in the same class as 404/401/403, and never falls back to the cache — only transport-class failures (server unreachable) fall back (S4 policy unchanged).

**Rate-limit negotiation = 429 + `Retry-After`, honored within a cap.** What was decided: when a 429 carries a parseable `Retry-After` (delta-seconds or the RFC 7231 IMF-fixdate HTTP-date) and the wait is ≤ `MAX_RATE_LIMIT_WAIT` (30s), retry after backing off for exactly the duration the server asked for (`FetchFailure::RateLimited`, sharing the same `attempts` budget as transport-class failures, with `RetryClassify::cooldown` carrying the negotiated wait); missing / unparseable / over the cap fail immediately in every case (`Status(429)` → E1901) — no guessing, no long sleeps. **Why**: rate limiting is the server's explicit "when to come back" negotiation, and honoring it is the correct retry; the cap protects builds from being dragged to death by an hour-long throttle. **Alternatives**: ignore `Retry-After` and always back off on a fixed schedule — disrespects the server's rhythm, rejected; never retry a 429 — too fragile against short throttles, rejected. **HTTP-date parsing**: IMF-fixdate only (the one form that appears in server practice); the two obsolete formats are not implemented; civil date → epoch seconds via Howard Hinnant's `days_from_civil`, no date library introduced. **Fallback semantics**: once 429 retries are exhausted it is a hard E1901 with no cache fallback — the server is reachable (merely throttling), consistent with the "fall back only when the server is unreachable" policy.

**Acceptance**: cage-core unit tests ×5 (a `Retry-After` table covering both forms plus garbage values; HTTP-date with known epoch anchors 1970/2015/2020 leap day/2030; cooldown mapping and retry classification; rate limiting consuming the same budget) + cage-source-http unit tests ×7 new (two-page merge and merged cache bytes; non-array page hard error naming the page URL; loop hard error; page-bound enforcement (small bound injected); success after rate-limit backoff with count assertions; over-cap immediate hard error without sleeping; hard error on a missing 429 header) + RFC 8288 parsing unit tests (quoting / multiple members / multiple tokens / casing / commas inside the target) + CLI integration ×1 (paginated source end-to-end build → Data.json containing the rows of both pages → cache = merged array → same-state rebuild with a byte-for-byte identical manifest).

---

# 46. Migration (M series, decided 2026-10)

**Implementation status**: M1 migration-rules model and parsing (`parse_spec` / `validate_spec` / `parse_migration_dir`, E2001/E2002), M2 migration executor and re-verification (`apply` with the six transform semantics + `reverify` L0–L6 re-verification, E2003/E2004), and M3 CLI and writing to disk (`cage migrate` dry-run/`--write`/`--all`/`--to`, local text sources written back to their original paths) are all delivered, with all four E20xx codes wired (the E20xx table in validation.md is authoritative); additional deliveries: `--to latest` (resolved to the full chain, same as `--all`; a literal version `latest` on the chain takes precedence) and cell-level locations in the Excel report (`StepReport.affected_locations` — `Sheet`/`Row` locations of affected rows in Excel tables emitted with the report, not collected for text sources); further additions: `cage migrate-draft` (the `migrate::diff` module — a structural diff of two schema versions → a migration-rules **draft**: mechanically safe transforms become steps, ambiguous items are left as `# TODO`; see the rules-draft decision record below); the interface block and signatures below follow the implementation — `apply` actually takes a `from_schema` parameter (used for `widen_type` direction checks), and `reverify(doc, schema)` is a standalone entry point.

**Implementation decision record (M3, added alongside the code)**:

- **Two widen paths**: the schema loaded by the CLI is always the **new version** (the workflow is: change the schema first, then write rules to migrate the data), so `widen_type`'s `to` equals the schema's current type — if it still went through the direction table, `X → X` would be wrongly rejected as not a "safe widening". Decision: when `current == to`, skip the direction table and run only the per-row value-range check (the value range is the safety net and runs on both paths); only when `current != to` does the direction table apply (the library-caller-holding-the-old-schema scenario is unchanged).
- **The CLI does not run `validate_spec`**: its from-schema reference validation needs the **old schema**, while only one new schema exists on disk (historical schemas are not stored). Decision: from-schema validation remains a library API capability (for editors/toolchains holding both schemas); bad references on the CLI path are picked up in relay by the apply-layer defenses — a rename target already occupied (from/to coexisting in one row, or from/to coexisting as table names — both rejected outright with `E2003`, never merging the two values), the referenced table missing from the documents (`E2003`), and field-level residue backstopped by re-verify `E2004`. Failure is failure; there is no silent channel.
- **Idempotence**: `apply`'s `rows_changed` counts only rows whose bytes actually changed — re-running an already-migrated document makes every step a no-op (rename targets already in place / defaults already filled in / values already mapped / tables already renamed); before `--write`, files are compared byte by byte and identical ones are skipped. A second `migrate --write` = 0 rows changed + everything unchanged.
- **Write-to-disk render order**: reading a JSON source does not preserve in-file key order, so the original order cannot serve as the basis for re-rendering; decision: rows are rendered in **schema field declaration order** (the contract order, stable), with fields outside the schema appended at the end in load order. The first `--write` is a one-time format normalization (reported as wrote); from the second run on, the bytes are constant.
- **Multi-segment chain re-verification**: re-verification always runs against the one (final) schema on disk — every intermediate state on the chain must be self-consistently valid under the current schema; a chain that would halt partway at an intermediate state (one that does not satisfy the final schema) should use `--all` to land there in one go.
- **The `latest` symbol vs. literal versions**: `--to latest` resolves to the full chain by default (the same segment selection as `--all`); if a segment on the chain genuinely has `to: "latest"`, the literal version takes precedence and the chain is taken only up to that segment — a concrete target beats the symbol, so a chain that carries its own latest version can still be migrated up to just that point.
- **Excel cell-level locations**: Excel sources are never written back, and the report is their only visible surface of change — for Excel tables (detected by the .xlsx/.xls suffix), `apply` collects `Row.location` of affected rows (rendered as `file | Sheet | Row`); text sources collect nothing (the on-disk diff is the change record); the CLI renders the rows under each step in source order, folding beyond 8 rows into `… +K more row(s)` (a deterministic cap — no timestamps, no random order).

**Implementation decision record (rules draft `migrate-draft`, added alongside the code)**:

- **The draft is not auto-inference** (what alternative ① rejected was "auto-inference becoming rules directly"): the structural diff proposes only **mechanically safe** transforms — `remove_field` (present in the old, absent in the new), `widen_type` (within the direction table), `set_default` (new field with a default / required false→true with a default); ambiguous items always become `# TODO` (field and table renames — only the author knows rename vs. delete-old-add-new; non-widening type changes; enum member additions and removals; required flips without a default; dropped tables and new tables), **renames are never guessed**. An all-TODO draft renders `steps: []`, which `parse_spec` rejects under E2001 — a draft is never run directly.
- **The tagged form of UInt default values**: bare YAML scalars carry no signedness (`yaml_to_value` tries i64 first), so `5` reads back as `Int(5)` — if a UInt column's default went through as a bare scalar, re-verification would inevitably hit E1101. Decision: when parsing a `set_default` value from a mapping, first try the canonical adjacent tagged form (`{type: UInt, value: 5}`) and fall back to a plain read on failure; on the draft-rendering side, any value containing `UInt` is always emitted in tagged form. String values are always single-quoted (so `yes`/numeric-looking forms do not flip type on read-back). The render → `parse_spec` round trip is backstopped by unit tests.
- **The draft command does not run `validate_spec`** (the same rationale as the M3 CLI precedent, one layer deeper): `set_default` references new fields of the to-schema while `remove_field` references old fields of the from-schema — neither schema version alone validates everything. The defense is relayed by apply's E2003 and re-verification's E2004.

**Decision record (decided 2026-10, under the authorization "pending decision items may be settled per the recommended option + recorded, with the user reviewing and adjusting later")**

- **What was decided**: Migration = **declarative data migration** of existing Source data under schema evolution — migration rules (the `migrations/` directory, where filename order is the version-stepped chain: `0001-…`, `0002-…`) explicitly declare each segment's transforms (field rename / fill in a default / remove a field / safe type widening / enum value mapping / table rename); `cage migrate` applies the transforms to the Canonical Model, reports table by table and row by row, and re-verifies under the new schema after migration (done only when check is all green); read-only report by default, and only `--write` writes to disk.
- **Why**: what to do with existing data after a schema change is an evolutionary must-have for the compiler — manual editing is unauditable and auto-inference is unreliable; explicit rules + re-verification = auditable (rules go into the version repository), deterministic (same rules + same data = same artifacts, isomorphic to the whole-repo determinism contract), and failure is failure (no silent data loss).
- **Alternatives**: ① auto-infer migration steps from the schema diff (rejected — at ambiguous points it silently walks the wrong way: field rename vs. delete-old-add-new cannot be told apart mechanically); ② report only, change no data (weak — no closed loop, users still hand-edit); ③ a runtime compatibility layer (rejected — out of bounds, per the red line "no game-runtime database / game-logic framework").
- **Implementation order**: the A series (§47) goes first — reusing the R series' registered facilities, zero new concepts; the M series follows (ramping up a new module).

**Capability boundaries**:

- In scope: declarative transforms at the Canonical Model layer; sources rewritable as text (JSON / YAML / CSV) support `--write` writing to disk; Excel sources get a report only, nothing written to disk (red line "no Excel editor" — edit manually per the report, then re-run migrate after editing so validation converges); the post-migration registry entry = re-run build + `cage registry publish` (reusing the R series, no new channel created).
- Out of scope: auto-inference (rules must be explicit); evolution of the schema itself (schema editing goes through the Schema Editor / hand editing; migration handles data only); segment selection taken as needed (`--all` full chain, `--to <ver>` prefix, `--to latest` full chain).

**Interface**:

```text
cage-core::migrate
├── MigrationSpec      # 一段迁移：from / to / steps[]
├── Step               # rename_field / set_default / remove_field /
│                      # widen_type / remap_values / rename_table
├── parse_spec(...)    # 迁移文件 → MigrationSpec（E2001 解析失败）
├── validate_spec(...) # 规则引用对 schema 校验（E2002——表/字段存在性、
│                      # rename 撞名；库 API，持 from-schema 的调用方用）
├── parse_migration_dir(...)  # 目录 → 文件名序版本步进链
├── apply(spec, doc, from_schema)  # Canonical Model 原位变换 + 逐步骤
│                      # 行数报告（E2003 变换不满足：方向不安全、值超
│                      # 域、表/行字段缺失、rename 目标撞名；幂等——
│                      # 重跑已迁移文档 0 行变更）
└── reverify(doc, schema)  # 迁移产物对新 schema L0–L6 回验（E2004 附诊断）

migrate::diff
├── diff_schemas(from, to)    # 表名/字段名对齐的结构 diff（FieldDelta 五类）
├── draft_migration(diff)     # 机械安全变换成 Step、歧义项成 TODO 注释
└── render_draft(draft)       # 规则稿文本（TODO 块 + steps:；空稿 `[]`）

cage migrate [--all | --to <ver|latest>] [--write]   # 默认 dry-run 只报告
└── 载入工程 → 按文件名序逐段应用 → 当前 schema 回验（E2004 迁移后
    校验失败不落盘）→ 报告（每步骤行数 / 每源文件处置）；
    --write 对本地 JSON/YAML/CSV 源按 source_file 原路写回（schema
    字段序渲染、字节未变跳过），Excel 源与 registry/远程源恒报告

cage migrate-draft <from-schema> <to-schema> --from <ver> --to <ver> [-o file]
└── 两版 schema 的规则**草稿**：机械安全变换成步、歧义项 `# TODO`；
    默认 stdout，`-o` 落盘（父目录自动创建）；空稿 `steps: []` 需手写
    步骤后才能被 cage migrate 解析
```

**Error codes (E20xx family)**: E2001 migration-rules file parse failure / E2002 invalid rule reference / E2003 transform requirements not met / E2004 post-migration re-verification failure. The whole family enters `codes.rs`'s `error::codes::migration` module + validation.md with M1, one doc per code (reserved annotations are cleared once wiring lands at M1, the same discipline as the S series).

**Deferred to implementation**: ~~joint validation of migration rules against `[dependencies]` version pins~~ (implemented; see the decision record below). (`--to latest` and cell-level locations in the Excel report were filled in with M3.1; schema-diff-assisted rules-draft generation was filled in with `cage migrate-draft` — see the implementation status and the rules-draft decision record.)

---

# 47. Artifact Distribution (A series, decided 2026-10)

**Implementation status**: A1 bundle export (`cage registry export`, deterministic tar), A2 bundle import (`cage registry import`, admission through the trust gate), and A3 direct push (`cage registry push`, probe + per-file PUT + remote index merge) are all delivered, with all five E21xx codes wired (the E21xx table in validation.md is authoritative); A1's deferred item, the compression container, is now filled in — `export_bundle` gains a `compression` parameter (CLI `--compress zstd`, a single zstd frame at fixed level 19; same tar + same level + same zstd library version → same container bytes), and `import_bundle` sniffs the container by frame magic (the extension is ignored; the ledger hashes the decompressed contents, and both forms get the same trust-gate judgment); push is unchanged (it pushes individual files, not bundles); A6, the signature ledger, is filled in — `cage registry keygen` (the seed is written only to the user-specified file, the public key goes to stdout), `export --sign --key-env` (a detached ed25519 signature as `<bundle>.sig`), and `import --verify-sig --key-env` (a signature gate ahead of the ledger trust gate), with E2106/E2107 wired — seven codes in total (the interface block and signatures below follow the implementation).

**Decision record (decided 2026-10, same authorization policy as §46)**

- **What was decided**: two distribution routes for registry entries — ① **offline bundle**: `cage registry export <包>[@<版本>] -o <file>` produces a deterministic tar (all of the entry's files + the ledger + an excerpt of the package index; mtime/uid/gid zeroed, members ordered by name — same entry = same bytes), and `cage registry import <file>` first passes the same ledger trust gate (`verify_snapshot`; nothing enters the register unverified) and is then admitted (identical bytes idempotent / different bytes an E1801 conflict; a bad ledger E2103); ② **direct push**: `cage registry push <project> [包[@版本]] --registry <远端根>` reads the entry from the local registry declared by the project's `[registry].path` (project-path loading mirroring publish; the package name defaults to `project.name`, the version defaults to the newest under dot-separated ordering), then PUTs file by file to the http(s) remote root and writes the package `index.json` last, after all entry files succeed (the entry-bytes-immutability discipline is unchanged); `--registry` accepts only http(s) roots (local targets are publish's territory); authentication is `Authorization: Bearer $TOKEN`, with the token resolved from the environment variable named by `--auth-env` (override) or `[registry].auth_env` (same policy as E1904: credentials never enter cage.toml, never land in logs or error text, and a missing one is checked before any network contact — E2105); before pushing, an anonymous GET of the remote package index serves as the status probe (the R3 read protocol is unchanged; 404 = the remote does not yet have this package → full upload; same content_hash → idempotent with zero PUTs; different content_hash → push rejected with E1801 — the remote already holds an entry with the same version but different bytes), and the index PUT rewrites the file wholesale after merging with the remote's existing entries (remote historical entries are never rewritten or deleted).
- **Why**: R3 already closed the loop on "how consumers fetch" (anonymous read-only GET parsing of the three resources); what was missing is "how it gets to a remote, how it travels non-network channels" — the bundle covers offline / audit / object-storage channels (the artifact is already a self-verifying snapshot, so copying it is distribution), and push covers automated publishing (CI-produced packages go straight into the remote registry); both routes reuse the same ledger trust gate, introducing no second trust model.
- **Alternatives**: ① build our own registry server process (rejected — red line: no server-side products; the protocol fixes only the client-side write shape, and any static gateway that can accept PUTs / nginx WebDAV / CI job works as the server); ② git as the distribution backend (rejected — implicit history rewriting conflicts with the registry's "never implicitly rewrite history" discipline); ③ gzip/zstd compression containers (rejected in the initial version — concerns over compression-frame timestamps / dictionary state were later confirmed not to apply to bulk single-frame encoding: the frame header has no timestamp and no dictionary state; now implemented — `--compress zstd` wraps the deterministic tar in a single frame at fixed level 19, container bytes are byte-for-byte reproducible within the same library version, and import sniffs by frame magic; see implementation status).
- **Implementation decision record (signature ledger A6, added alongside the code)**:

- **The signature sits at the bundle byte layer, not the ledger file layer**: the HASHES.json ledger remains the integrity anchor (the import trust gate is unchanged), while the detached ed25519 signature covers the bundle's **exact on-disk bytes** (plain tar / zstd container judged alike) — the signed object includes the ledger, so a tampered ledger also fails signature verification; no second trusted manifest is introduced.
- **The sidecar's public_key is not a trust anchor**: the public key inside `<bundle>.sig` exists only for human cross-checking and transport convenience; verification is always pinned to the key the consumer supplies via `--key-env` — "who is trusted" is the consumer's call, and the signature only answers "who signed". Without a trusted key it does not run (`--verify-sig` forces `--key-env`, clap `requires`); there is no silent fallback to "trust the sidecar's own public key" — that would amount to verifying integrity only, selling out non-repudiation.
- **Key discipline**: keys are read only from environment variables (base64 32-byte seed/public key), never entering cage.toml or logs; `keygen`'s seed is written only to the file named by `-o` (unix 0600, best-effort and non-blocking), and stdout never emits the seed. E2106 (key material unavailable) and E2107 (signature verification fails / sidecar missing or malformed / algorithm not recognized) are separate codes — no key available and key mismatch are two different kinds of failure.
- **The signature gate comes before the ledger gate**: an unattributable bundle is rejected before integrity is even discussed — the ordering determines the failure message: E2107 appears before E2103/E1801.

**Server-side conventions (documented, not implemented)**: GET `<root>/<包>/index.json` anonymous as the status probe (404 = package does not exist); PUT `<root>/<包>/<版本>/<文件>` uploads file by file, and after the whole entry succeeds, PUT `<root>/<包>/index.json` (the server should replace that file wholesale — the client has already merged the remote's existing entries); 405/501 = the server has not implemented the write channel (E2104, with a hint to fall back to "local publish + static hosting"); 401/403 = credentials rejected (E2102; the probe and PUTs map identically); unauthenticated deployments keep working as before (the R3 read path is unchanged; push's optional auth_env sends no header by default).

**Capability boundaries**: export / import / push touch only "already-admitted entries" — no rebuilding (publishing remains `cage registry publish`, exclusive to local roots); the read path stays R3 anonymous read-only; no server-side implementation and no incremental deltas. Signatures are implemented (A6 detached ed25519 signatures; see implementation status) — the blake3 ledger remains the integrity anchor, and the signature adds non-repudiation on top of it.

**Interface**:

```text
cage-core::registry 增
├── export_bundle(root, package, version, compression, out)  # E2101 条目读取失败
│    # 条目全文件 + HASHES.json + 包 index 摘录 → 确定性 tar
│    # compression = Plain | Zstd（固定级别 19 单帧）
├── import_bundle(root, file, dry_run)          # verify_snapshot 信任门
│    # → 入册（坏账本 E2103；同字节幂等、异字节 E1801）
├── push_entry(source_root, remote_root, package, version, auth_env,
│              dry_run)
│    # resolve_entry + 账本重建 index 记录 → 匿名探针 GET 远端包 index
│    #   （同 hash 幂等早退 / 异 hash E1801 / 404 空远端 / 401·403 E2102）
├── parse_presign_map(bytes) + push_entry_presigned(source_root, map,
│              package, version, dry_run)      # presigned 路线（A3 拍板）
│    # 同一 push 主体（PushRoute::Direct | Presigned）：每对象一个
│    # presigned URL，index 自带 GET/PUT 对——合并 / E1801 / 幂等不变；
│    # coverage 缺口在任何 PUT 之前失败关闭；无 Bearer（URL 即凭据）
└── 签名账本（A6）：generate_signing_key + key_material（keygen，种子只落
     用户指定文件）、signing_key_from_env / verifying_key_from_env（E2106
     密钥材料不可用）、sign_bundle_bytes / verify_bundle_bytes（E2107
     验签不过）、read/write_bundle_signature（`<bundle>.sig` sidecar）
     # → 逐文件 PUT（remote::http_put，RetryPolicy 与 http_get 同口径，
     #   Bearer 仅随 PUT）→ index 合并远端条目最后 PUT
     # E2101 传输 / 远端 index 不可读 / 非 http(s) 根 / presign map 畸形
     # E2102 鉴权被拒（401/403，探针与 PUT 同映射）
     # E2104 服务端拒写（405/501 等明确 4xx）
     # E2105 凭据缺失（env 未设或空，网络触达前失败）
     # dry_run 走完整本地读 + 状态探针，零 PUT

cage registry export <pkg>[@<ver>] -o <file> [--compress zstd]
    [--sign --key-env <VAR>] [--registry <local-root>]
cage registry import <file> [--verify-sig --key-env <VAR>] [--registry <local-root>] [--dry-run]
cage registry keygen -o <file>
cage registry push <project> [pkg[@ver]] (--registry <远程根>
    | --presign-map <file>) [--auth-env <VAR>] [--dry-run]
    # presign map（JSON）：{"uploads": {"pkg/1.0.0/文件": "https://…"},
    #   "index": {"get": "https://…（可省）", "put": "https://…"}}
```

**Error codes (E21xx family)**: E2101 distribution transport / entry read failure / malformed or coverage-gapped presign map / E2102 authentication rejected (HTTP 401/403) / E2103 bundle ledger verification failure / E2104 server refuses writes (405 / 409 / explicit 4xx) / E2105 push credentials missing (auth_env unset; the presigned route reads no credentials) / E2106 signing key unavailable (A6) / E2107 bundle signature verification failure (A6). The whole family is signed into `codes.rs`'s `error::codes::distribution` module + validation.md one code at a time starting with A1, one doc per code; when A6 closes, all seven codes flip to wired.

**A3 batch decision record (decided 2026-10-10)** — all three "deferred to implementation" items closed out:

- **Incremental delta distribution → rejected**. push is already per-file PUT (4 small files + 1 index), which is sufficient granularity; `<版本>/<文件>` paths never share bytes across versions, so there is no common base to delta against; actually building content-addressed storage (CAS) would amount to swapping the protocol — that is a matter for a new protocol version, never changed silently. Entries are small (configuration snapshots, not binary assets); the delta payoff does not cover the complexity.
- **S3 presigned direct push → implemented as the provider-agnostic `--presign-map`**. Native SigV4 is rejected: credential custody belongs to CI / cloud roles, and cage holds no cloud keys; the presigned URL is the industry-standard delegation and covers S3 / GCS / Azure Blob alike. Shape: the uploader (a CI job) first obtains from the object store a URL table of `{uploads, index.get, index.put}`, and `cage registry push --presign-map` PUTs strictly per that list — the push body (probe / merge / E1801 / idempotence / index last) changes not one character; only the URLs and the authentication (no Bearer) switch sources. A coverage gap fails closed before any PUT — never leaving half an entry behind for the index to point at thin air.
- **Pull-through caching proxy → not implemented** (red line: no server-side products). The read protocol is naturally CDN / proxy friendly — object bytes are immutable (the same version never has different bytes, guaranteed by E1801), the index is the only mutable file, and GETs are fully anonymous: any static hosting / CDN / reverse proxy works as-is, with no cage-side component needed.

**Deferred to implementation**: (the three §47 items closed out with the A3 batch — delta rejected, presigned implemented, proxy rejected; see the decision records above; the compression container is implemented — bulk encoding at fixed level 19 never depended on dictionary state to begin with, so the "zstd deterministic dictionary" variant is moot along with it; the signature ledger is implemented with A6 — see implementation status and the A6 decision record.)

---

# 48. Environment Validation (per-environment validation, decided 2026-10)

A game's configuration is per-environment: for the same table, the dev environment allows `hp` to be omitted, while the prod environment not only makes it required but also demands a floor of 100. Schema is the single carrier of rules, so environment semantics land on the Schema — each environment is one notch of a "strictness knob" applied to the same set of field shapes, not another Schema.

**Decision: constraint overrides (option 1).** In 2026-10 the user decided to take the "constraint overrides" direction (per-environment = overrides applied to the existing constraints, rather than either of the two alternatives, "an independent Schema per environment" or "data forking"), and authorized the inspection pass to settle implementation details on the user's behalf per the decision record's policy. **Authorized policy**: if during implementation option 1 is found to conflict with the existing constraint mechanism → record the conflict and the rationale, stop, and escalate; if no conflict is found, proceed with batched delivery.

**Wire format (ruled at implementation time; a carrier deviation under identical semantics)**: adopt a **table-level side table** `env_overrides` (a new TableSchema field: environment name → field name → constraint patch `FieldOverride`), instead of the "per-key inline `{default, envs}`" shape from the decision preview. **Why the deviation**: the inline form would require reshaping every constraint field of `FieldSchema` into a wrapper structure, touching roughly a hundred `.required` / constraint-read sites across 12 crates repo-wide (including 7 codegen crates locked by golden tests) — a large volume of mechanical changes that also trips golden; the side-table form carries exactly the same semantics (one set of constraint deltas per environment) but has zero blast radius — validators, codegen, and the editor round-trip all stay unchanged. **Classification**: a choice of implementation carrier within the settled direction, not a mechanism conflict (constraint overrides and `rules`/`reference`/profile projection do not encroach on one another: overrides only tune strictness on the same shape — no fields added or removed, no types changed), so delivery is not halted; recorded here.

**Semantics**: `FieldOverride` is a **partial patch** of a field — it lists only the constraint keys an environment wants to move (required/min/max/min_length/max_length/pattern/enum_values/min_items/max_items); keys not listed keep their baseline values; the listed keys are **whole-value replacements** (e.g. `enum_values` overridden to the production whitelist, not a union with the baseline). **Type is not in the patch**: an environment only tunes strictness and never reshapes a field. An empty patch (not a single key set) is an error at schema structure checking.

**Resolution point**: `Schema::resolve_env(env)` runs exactly once after load and before profile projection and validation — it clones the Schema and applies that environment's patches onto the plain fields, so what is handed downstream is always "a plain Schema". All validators (L0–L7, profile semantics, E9006), codegen, and hashing consume the resolved result with **zero changes**; the baseline Schema is never modified. E9006 (projection stripping a structurally required field) is judged against the resolved `required` — the environment tightens first, then projection triggers the conflict, so the ordering is naturally correct.

**Where environment names come from**: the Schema declares them itself — the union of the per-table key names in `env_overrides` (`declared_envs`); no second list is registered in cage.toml (single source of truth). An undeclared `--env` → usage error, exit 2, with the error listing every declared environment; when the Schema has no `env_overrides` at all, `--env` → exit 2, "declares no environments". **No new E codes**: a wrong environment name is the same class of thing as an unknown profile (a CLI usage error, not a document-validation failure); on the `--env` path, the overrides of all environments get structure checks (field exists, patch non-empty; errors carry table/env/field locations) — a bad override is guaranteed to be caught when that environment is selected. Warning about bad overrides in unselected environments (lint without `--env`) is already implemented (2026-10): `Schema::lint_env_overrides` collects all structural problems; the baseline path (no environment selected) reports each as a non-blocking `warning:`, while the `--env` path keeps them hard errors — the same structural defect under two policies, isomorphic to the warn/escalate treatment of unknown fields; this follows the existing "no new E codes" ruling (plain-text diagnostics at load time, presented consistently with `--env` structure errors).

**Build ledger**: `schema_hash` fully hashes the resolved Schema (environment changes → field constraints change → schema_hash changes → build_id changes); the manifest gains `environment: Option<String>` (serde default; baseline builds do not write this key), explicitly recording the environment the build ran in; the L1 / layer-2 incremental guards compare environment in addition to profile — **switching environments never reuses the previous environment's build artifacts**. Same-environment determinism is unchanged: same input, same environment → a byte-identical manifest. `generator_version` 1.1.0 → 1.2.0 (layout evolution: new `environment` field). ~~`cage snapshot` / `registry publish` temporarily package from baseline builds (no `--env`); per-environment packaging is left for later.~~ **Per-environment packaging is now implemented (2026-10; see the decision record below).**

**Acceptance**: cage-core unit tests (declared_envs union / structure checks catch unknown fields and empty patches / resolve_env applies the patches while the baseline stays unmodified / an environment with no overrides resolves equal to the baseline) + CLI integration ×4 (three rule sets for the same table: baseline E1001, dev passes, prod switches to E1201/E1204; an undeclared environment and a Schema without overrides both exit 2 and list the declared set; the manifest records `environment`, switching environments rotates build_id, and the incremental guard mismatches by environment; a dev → prod incremental build converges byte-for-byte with the full build).

### Per-environment packaging decision record (2026-10)

**One version, one package; sameness judged by the whole-package fingerprint.** What was decided: `cage snapshot` and `cage registry publish` both take `--env` (same semantics as check/build/gen: `env_overrides` is first resolved into the schema, then full validation and packaging run); the snapshot directory name carries an env segment, `<profile>-<env>-<build_id[..12]>` (baseline carries no env segment); the `environment` field of the in-package `manifest.json` and the resolved-form `schema.json` travel with the package; publish keeps the existing flow — when a per-environment entry is registered, the manifest itself declares the environment.

**E1801 widens its judgment surface from "same content_hash" to the whole-package fingerprint**: idempotence/conflict detection becomes a dual comparison on `build_id + content_hash` (publish and the import pre-check change in lockstep). Why: content_hash covers only the artifact bytes — when repackaging per environment the artifacts can come out byte-identical (the data did not change, only the rules did), so looking only at content_hash would misjudge the new package as "identical, no-op" and the prod bytes would never land. `build_id` covers profile+schema_hash+source_hash+content_hash; change the schema and it necessarily changes — the same version in a different environment (or any schema-only change) now correctly reports E1801 and prompts a version-number change (e.g. `0.1.0-prod`; the version grammar already allows a label segment).

Alternatives and trade-offs: attaching multiple per-environment subdirectories to one version (`<version>/<env>/`) — rejected: it breaks the addressability of "one version, one package" and the bundle/export shape, and complicates consumer-side pin semantics; resolve/verify/gc/remove/push keep working by bytes and the ledger, unaware of environments — the environment is just one build parameter at packaging time, and once registered it is plain bytes. `--env` and `snapshot --verify` are mutually exclusive (re-verification looks at bytes only, not at environment).

**Acceptance**: core unit tests (same artifacts with different build_id conflict, with the message naming the build_id; same build_id+content is an idempotent no-op) + CLI integration ×2 (`snapshot --env`: env segment in the directory name, in-package manifest records the environment, schema.json carries the resolved constraints, re-verification passes, baseline snapshots unaffected; `publish --env`: the same version in a different environment reports E1801, a new version registers with the manifest recording the environment, an idempotent re-publish is a no-op, registry verify all green) + example run.sh steps 6/11 extended (live demos of per-environment snapshots and E1801).

### S8 decision record (HTTP conditional GET incremental fetching)

**Document-level incremental = RFC 7232 revalidation, per-URL cached validators.** What was decided: every cached byte blob from an HTTP source gets a sidecar meta alongside it (`.cage-cache/source/<key>/<key>.meta.json`, `{etag, last_modified}`, with the keys taken from the response headers); when the same URL is fetched again, the validators go out as `If-None-Match` / `If-Modified-Since` — 304 → the cached bytes are reused as-is (not even the meta is rewritten); 200 → new bytes replace the old bytes and the new validator replaces the old meta. **Why document-level only**: row-level increments (fetching only the changed rows by revision/updated_at) would require every source to expose a data contract (revision keys, changed-row-set semantics), and HTTP APIs have no universal policy for that — validators are response-level and independent of the response body's shape, the only layer where the benefit can be had without changing each source's contract; row-level is left to implementation time.

**Only the first-page GET carries conditions**: in a paginated source, only the first page's validator describes the state of the whole collection; later pages are relative URLs from the first page's bytes, carrying their own link-level semantics, so conditioning them has no reliable basis — when the first page 304s, the whole pagination chain need not be walked again.

**meta without an accompanying payload is treated as no cache**: meta and payload must both be present before revalidation is allowed — an assertion built on half a cache cannot be verified against the server, so we honestly treat it as no cache (an ordinary GET rebuilds both), adding no new state and silently reusing nothing.

**`--no-cache` (strict) still revalidates**: strict means "cached bytes do not serve as the answer", not "do not talk to the server" — a 304 is the server confirming the cache, which is a legitimate answer; strict and normal mode share the same revalidation path and differ only in whether they fall back on transport failure (see S4).

**Relation to offline fallback**: a 304 does not go through transport-failure classification — it is not a fallback candidate and does not trigger E1906; only genuine transport errors fall under S4's semantics. A validator write failure (E9902) does not affect the payload — next round, with no meta, it simply takes the ordinary GET path.

**Acceptance**: cage-source-http unit tests ×2 (a rotating server in five steps: first fetch unconditional, landing a meta ETag; second fetch sends INM and gets a 304 with the bytes untouched; after rotation the stale INM gets a 200 landing new bytes and new meta; the next round carries the new ETag; strict still revalidates — server hit counts asserted step by step; after the meta is deleted, rebuilt as no-cache) + all-green gates (fmt / clippy -D warnings / 689 tests / run.sh live exit 0).

### Migration and dependency pinning linked-validation decision record (2026-10, closing out the leftover from design §46)

**The chain endpoint must land within the pin.** What was decided: `cage migrate` adds a linkage gate after segment selection and before any rule is applied — when `schema_path` comes from `registry:<包>[@<版本>]` and `[dependencies]` declares a pin for that package, the selected chain's endpoint version (the last segment's `to`; the default single segment, `--to`, and `--all` are judged the same way) must satisfy the pin, otherwise `E1802` is a hard error (exit 2; dry-run and `--write` judged alike, with the message naming the version / pin / package name). **Why**: the chain endpoint and the pin are two independent declarations of the same thing ("which schema version the data migrates to") — the pin governs which schema versions this project accepts, and the migration chain governs where the data actually ends up; if data migrates to a version outside the pin, neither re-verification nor any later build can hold under a schema the project can resolve. Until now these two declarations intersected only at load-time version resolution; the migration target itself had no gatekeeper.

**Alternatives and trade-offs**: warn without blocking (same policy as the baseline env lint) — rejected: migration rewrites source data, the dry-run report already does the "heads-up" job, and letting through a version doomed to have no path to re-verification only postpones the failure to reverify (E2004) or the next build; hanging it on a new E20xx code — rejected: this is a pin violation, not a rule-reference/transform/re-verification problem, and E1802's existing semantics ("explicit versions must land within the pin") already cover it as-is.

**Boundaries**: a local schema (not `registry:`) has no pin to link against, so the gate silently skips it; for `registry:` without a pin, the norm is governed by the explicit version alone (R2's existing semantics). The migration chain's own continuity (from-linkage) remains `parse_migration_dir`'s responsibility; this gate looks only at the endpoint.

**Acceptance**: CLI integration ×1 (publish a schema entry → consumer side `registry:cfg@1.0.0` + pin `>=1.0.0, <1.1.0` → `migrate --to 1.1.0` exits 2 with the message carrying E1802/version/pin/package name → after loosening the pin, the same chain's dry-run passes) — delivered this round, checked off.

### S9 decision record (database table introspection schema draft)

**Draft = mechanical on structure, semantics left to the author.** What was decided: `cage schema-draft <project> <spec>... [-o <file>]` — each `mysql:<表>` / `pg:<表>` spec (same syntax as source_roots; `schema.表` resolves to the named schema) is introspected over a read-only session (a bound-parameter `information_schema` SELECT; table names are never concatenated into SQL) and rendered into one deterministic YAML draft: table order = spec order, column order = declaration order, primary key from constraints, `NOT NULL → required: true`; DECIMAL / date-time / JSON / unknown types are flagged in inline comments as "this column is the author's call"; a table without a primary key is written out with an empty `primary_key: []` plus a reminder comment. **Why mechanical on structure**: introspection can only answer shape (column names / types / nullability / primary key) — min/max/pattern/enum_values/references are business semantics, and the DB does not know cage's validation domain; deciding on the author's behalf is precisely the opposite of a draft. The type mapping takes the "minimally faithful policy": integers land by width on Int8–Int64 / UInt8–UInt64 (the unsigned suffix promotes the family), `tinyint(1)` lands on Bool per MySQL convention, floats land by precision on Float32/64; DECIMAL / temporal / JSON / text all land on String — **consistent with the actual shape the adapters land in the cached JSON**; the draft must be able to pass validation on our own pipeline, not describe the server's worldview.

**Credentials and read-only discipline carry over wholesale from S2**: the DSN comes from the environment variable named by `[remote.<scheme>].dsn_env` (E1904; never enters cage.toml), the session is pinned read-only first, and the self-produced introspection SQL must pass our own `validate_select` whitelist (self-checked by unit tests) — one discipline for everything that runs over the wire. A table not found (zero columns returned) is an E1901 lookup failure. No new E codes: the three cases — spec/DSN/connection — all land in the existing semantics of E1905 / E1904 / E1901.

**Determinism**: the rendered bytes depend only on the introspected shape — the header comment carries no timestamp (consistent with the repo-wide "same input, same bytes" discipline), and the same introspection always renders the same YAML. `-o` parent directories are created automatically (same policy as migrate-draft); the default is stdout.

**Acceptance**: cage-source-db unit tests ×15 (draft 7 + mysql 4 + pg 4: a 30-case MySQL COLUMN_TYPE classification table, PG data_type classification + varchar/numeric size reassembly, introspection SQL passing the whitelist + a self-check that consts cannot be concatenated, byte-exact rendered shapes, **rendered YAML parsed back into a Schema with table/primary-key/required shapes asserted**, quote round-trips for unusual names, an empty list when there is no PK, comments appearing only in the tiers that need a human decision) + CLI integration ×2 (five error-code tiers + exit 2: no [remote] / env unset / non-db spec / mysql+pg unreachable; two usage-error tiers: missing cage.toml and missing spec) — delivered this round, checked off.

---

# 49. Decisions on the Four Roadmap Wrap-up Items (2026-10-11 follow-up batch, ruled item by item)

The four items previously left "for later" (IDE support / FlatBuffers / registry authentication and the remote publishing protocol / the three §45 external dependencies) are ruled in this batch under this repo's decision-record policy — items that can land autonomously are chartered and implemented; items that genuinely require external input are registered as blocked, with the reason recorded. Ruling: three items chartered and implemented, one item blocked.

## IDE support → chartered and implemented: `cage lsp` (stdio LSP, zero new dependencies)

**What was decided**: a new `cage lsp` subcommand — a testable subset of LSP over stdio (`initialize` / `initialized` / `textDocument/didOpen` / `didChange` / `didClose` / `textDocument/diagnostic` or `publishDiagnostics` / `shutdown` / `exit`). The JSON-RPC 2.0 frame protocol is hand-written (`Content-Length` header + newline separation, identical shape on the read and write sides), synchronous and single-threaded; `serde_json` is already a workspace dependency — **zero new dependencies, no async runtime**. The diagnostic source = full project validation (reusing the single entry point of the check pipeline); diagnostics are mapped into the schema YAML documents open in the editor (file-level zero-width range, body carrying the E code + location + hint; the editor sidebar is the diagnostics panel). The `--env` / `--level` policy matches `check` (the LSP initialization parameters carry `env` / `level`; defaults = default environment + all levels).

**Why**: diagnostics have long been first-class citizens (E code + file / Sheet / cell / field / value location + fix hints); what the IDE side lacks is only the **transport layer** — the three consumers, CLI / CI / Web UI, each already consume the same diagnostics, and the editor has no reason to keep waiting. None of the three red lines is touched: no binding to Unity / Unreal / Godot / Cocos (no client-side plugins), no server-side product (the LSP runs locally in the editor process), no node build chain (the W3 editor frontend deliberately has no node dependency; a VS Code extension would drag in a TS compilation chain, in conflict with repo-wide discipline). A hand-written frame protocol rather than pulling in `lsp-types` + `tokio`: the latter would introduce new dependencies and an async runtime (the repo is async-free overall), while the LSP frame protocol is nothing but "two header lines + fixed-length body" and can be tested exhaustively within about a hundred hand-written lines (subprocess stdio round trip, assertions in both directions).

**Alternatives**: a VS Code extension (needs a TS build chain, violates the no-node precedent, and covers only one IDE); delivering only a machine-readable diagnostics contract document (too thin — machine-readable diagnostics are already a library contract (the serde shape of `Diagnostics`) and a Web UI contract (POST `/api/validate`); CLI `check` itself has no `--json`; what has always been missing is not the data shape but the editor transport layer and process model); waiting for the flatc / lsp ecosystem to mature (no date; the payoff does not justify the wait).

**Boundaries**: diagnostics first — **no** completion / hover / go-to-definition / formatting / rename (these require the LSP side to hold a schema semantic model, a new capability surface to be chartered separately); no multi-file incremental validation (every didChange runs full validation — configuration scale is on the order of a hundred tables, and full validation is milliseconds); no editor built inside the editor (the W3 Web UI is already the interactive editing surface; the LSP feeds diagnostics only).

**Landed (2026-10-11, I series complete, including one policy refinement)**: implemented as `cage lsp <root>` (`crates/cage-cli/src/lsp.rs`, synchronous single-threaded, zero new dependencies, a hand-written JSON-RPC 2.0 frame protocol of roughly a hundred lines, unit tests covering frame round trips). The protocol subset = the list decided above, push-style `publishDiagnostics`; unknown requests -32601, bad JSON -32700, unknown notifications silently ignored; `exit` without a preceding `shutdown` exits with code 1 (the LSP spec's misuse signal). The diagnostics pipeline was extracted into a single entry point `validate_project` (shared by `run_check` and lsp — editor diagnostics and CI diagnostics never diverge), mapped per the decision as a file-level zero-width range with the message body carrying the E code + location + hint. **Refinement — panel ownership**: the earlier "diagnostics are mapped into the schema YAML documents open in the editor," read literally, means per-file anchored filtering; the landed decision is to **push the full diagnostic set to every open document** (every open document = the project diagnostics panel): validation diagnostics are mostly anchored to data files rather than schema files, so per-file filtering would leave an open schema panel blank for long stretches and hide cross-file errors of the reference / dependency kind — exactly what the panel exists for; a zero-width range never claims "this line is broken" in the first place, and the risk of misreading is carried by the location in the message body. didOpen / didChange fully re-validate the on-disk state (editor buffers are not virtualized; textDocumentSync honestly declares Full), and didClose clears that document's panel; `env` / `level` go through initializationOptions (a bad level becomes an error diagnostic rather than silence), and a project load failure (bad cage.toml / unresolvable registry reference) is likewise a single error diagnostic. Acceptance: 5 unit tests in lsp.rs + 7 end-to-end subprocess stdio round trips in tests/lsp.rs (a real binary, run against the example project and a bad project).

## FlatBuffers → chartered and implemented: `cage-target-flatbuffers` (definition generation, no wire encoding)

**What was decided**: a new target crate `cage-target-flatbuffers` (format `flatbuffers`) that generates FlatBuffers `.fbs` **definition files** from the cage Schema — one `table` per configuration table, enums landing as `enum`, field types translated per the mapping table, defaults / descriptions / constraints preserved via comments and `default`; file names go through the target-generic `file_template` (`{table}.fbs`, split per table), with a deterministic-generation header comment (same policy as the proto target: same schema, same bytes, no timestamp in the comment). **Data wire encoding is explicitly not done** — fully consistent with the existing Protobuf precedent (Protobuf landed as schema-driven `.proto` definition-file generation and explicitly does not do data wire encoding; see docs/target.md, "Data Targets and Code Targets Are Separate").

**Why**: a FlatBuffers consumer **must** have the compiled schema before it can read data — the `.fbs` definition file is the first required artifact of any FlatBuffers pipeline, and everything in the cage Schema — tables / enums / types / defaults / descriptions — translates over mechanically (the type mapping takes the minimal-faithful policy, unknown types fall back to String, consistent with the type policy of the schema-draft draft). The payoff of wire encoding does not justify the complexity: first, consumers still need the .fbs (without the definition file, a .bin is a black box); second, a homegrown flatbuffers builder is roughly 400 lines (vtable / table / vector layout + length prefixes), and **this repo has no flatc to serve as a cross-check baseline** — serialization without a baseline can only vouch for itself; third, the configuration payload is small text on the order of a hundred tables, and the deterministic-binary need is already covered by the MessagePack target (deterministic bytes + a repo-wide hash anchor). Definition-file generation = complete delivery; wire encoding = do it once a baseline exists.

**Alternatives**: a homegrown flatbuffers builder emitting `.bin` (no flatc cross-check, and the .fbs is still needed — half the payoff); documentation only, no code (too thin — definition-file generation is isomorphic to proto and costs little at the margin); waiting for a community crate (the `flatbuffers` rust crate depends on flatc-generated code, which amounts to needing a generator first — the definition file cannot be bypassed).

**Documentation policy change**: docs/target.md originally listed FlatBuffers under Data Targets as "planned" — this batch corrects that to the same group as Protobuf (schema-driven definition generation), leaving only Binary / SQLite as planned Data Targets items, with a note that FlatBuffers wire encoding is deferred (to be chartered again once a flatc baseline exists).

**F0 type mapping table (finalized)** — cage type → `.fbs` shape:

| cage type | `.fbs` shape | Notes |
| --- | --- | --- |
| Bool | `bool` | |
| Int8 / Int16 / Int32 / Int64 | `byte` / `short` / `int` / `long` | fbs short-name spelling (flatc also accepts aliases like int8; the short name is chosen) |
| UInt8 / UInt16 / UInt32 / UInt64 | `ubyte` / `ushort` / `uint` / `ulong` | |
| Float32 / Float64 | `float` / `double` | |
| String | `string` | |
| Bytes | `[ubyte]` | |
| Enum (all integer-valued members legal: non-empty, no duplicates, in range) | shared `enum Ident : <smallest base type>`, file header `include "cage_enums.fbs"` | base-type ladder byte→ubyte→short→ushort→int→uint→long→ulong picks the smallest that fits; flatc enums have no allow_alias, **a duplicate value makes the whole family illegal → the entire enum degrades to a string field** (the ruling corresponding to proto allow_alias) |
| Enum (string members / missing literals / out of range) | `string` + degradation comment | fbs enums carry integers only |
| `Array<T>` | `[T']` | when nested arrays / Maps are elements, they land as the wrapper table `{Field}Value` (proto same-name same-order rule) |
| `Map<K,V>` | `[{Field}Value]` | fbs has no native map; the idiom is an entry table: `table {Field}Value { key: K; value: V; }`; integer keys widen to `long` (the data model stores number strings; proto int64 same-family rule) |
| Object / Any / Null | `string` + comment | in cage these three are free-form payloads and fbs has no free-form type → carried as JSON text (the simplification corresponding to proto going through WKT Struct) |

**Attribute mapping**: a single-field primary key whose field maps to `string` or an integer scalar → append `(key)` (flatc restricts key to string/integer only); `required` with the field typed `string` / vector / table → append `(required)` (fbs syntax permits required only on non-scalars; scalars carry optional-absence via default-value semantics). Each file closes with `root_type <Table>;` pointing at the main table. Wrapper and field identifiers all go through the same name-order allocator and keyword escaping as proto (fbs keywords + scalar-name list).

## Registry authentication and the remote publishing protocol → chartered and implemented: remote package enumeration (the last missing link in the publishing loop)

**Current-state inventory** (reconciled before this batch's ruling, to avoid duplicate delivery): read — R3 remote http(s) read-only resolution (index → pick version → per-file blake3 verification → land in the project cache → trust gate, offline fallback semantics the same as S4); write — A3 direct push (per-file PUT, index last, idempotent on same hash, E1801 on a different hash, Bearer credentials via `[registry].auth_env`, E2102 / E2104 / E2105 already wired); delegated write — A7 `--presign-map` provider-agnostic presigned direct push; integrity — A6 signature ledger + `verify_registry` whole-ledger audit; delta rejection / pull-through proxy rejection are likewise closed (A7 decision record). **Only one real gap remains**: under R3, `registry publish/list` reports a usage error against a remote root (the protocol has no package enumeration) — the publisher cannot see what has already been published remotely, and overwrites / conflicts can only be inferred from local records.

**What was decided**: `cage registry list --registry <远程根>` — anonymous GET of the remote index (URL convention shared with R3 reads; the bytes land in the `.cage-cache/registry/` same-origin cache directory, reused without refetching) → rendering in exactly the same shape as local `list` (project / package / version / environment / build_id / content_hash), with `--json` supported. The authentication policy follows push: when `--auth-env` names an environment variable, the Bearer token rides along on the GET, 401/403 → E2102, missing credentials → E2105 (exactly the same codes and semantics as push). **The server side still is not implemented** (red line: no server-side product) — a read-only GET is naturally CDN / reverse-proxy friendly (object bytes immutable, the index the only mutable file, as argued in the A7 decision record); any static hosting works as-is.

**Why**: what the remote publishing loop lacks is pre-publish **visibility** — without remote enumeration, CI cannot audit "does this version already exist remotely" or "which environment has been published," and an E1801 conflict is discovered only at the PUT stage (under the discipline that remote history is never rewritten, the later a conflict is found, the more it hurts). This link is cage-side code that needs no external system (the protocol is just a GET of an index.json, the same convention R3 already validated).

**Authentication boundary (explicitly not done, registered as blocked)**: token issuance / refresh / rotation, OAuth user authorization flows, mTLS, service account credential custody — these need an external IdP and hosted credentials and fall under the established policy that "credential custody is CI's / the cloud role's business; cage holds no cloud keys" (same as the A7 presign decision). cage's authentication surface **stops at the Bearer token** (named via env, never written into cage.toml, never logged, absence checked before any network contact, per the E1904 / E2105 policy); who issues the token itself has nothing to do with cage.

**Alternatives**: do nothing (local `list` already suffices) — but then the remote publishing loop lacks enumeration, CI cannot audit remote state, and the read-only surface of R3 "remote resolution" is left mismatched (it can read contents but not the listing); implement the server side — red line, no server-side product.

**Landed (2026-10-11, L series complete, including two policy refinements)**: implemented as `cage registry list [--registry <远程根>] --package <NAME>… [--auth-env] [--json] [path]`. Refinement one — **enumeration source**: the earlier "GET an index.json" was incomplete: the read-only protocol has only **per-package** index resources, no master catalog of package names (a master catalog would require either a server side or extra root-index maintenance in the push protocol — both over the line). Decision: remote list enumerates by naming packages with `--package` (CI knows what it published; naming is the audit); naming none reports usage with exit code 2; a missing package name is not an error — a 404 renders as an empty list of `(no versions)` and exits 0 (same semantics as push probing's "no remote index yet," the normal pre-publish state). Refinement two — **cache semantics**: implementing "reuse the cache without refetching" literally would have list report stale remote state, manufacturing exactly the "the later a conflict is found, the more it hurts" this ruling set out to avoid. Decision: fetching fresh online is the truth, written to `.cage-cache/registry/<url-key>/<pkg>/index.json` (same origin and key as R3 resolve), **falling back to the cache only on transport failure** (S4 offline semantics, output marked offline); the `path` positional anchors the cache (omit it to fetch fresh every time and write nothing to disk). The rest landed per the original ruling: rendering in the same shape as local list, `--json` isomorphic between local and remote, the Bearer riding on the GET only when `--auth-env` names one (401/403 → E2102, missing → E2105, checked before the network), zero server-side implementation.

## The three §45 external dependencies → blocked (external input needed, not chartered this batch)

All three are registered as blocked together, for one and the same reason: they **depend on the real state of external systems, while this repo's testing discipline is mock servers / temp directories with no real network contact** — a change that cannot be verified inside the gate does not enter main (the repo carries zero "unverifiable-in-tests" code debt).

- **Row-level incremental** (fetch only changed rows by `revision` / `updated_at`): needs a **data contract** per source — which columns count as revision and how change semantics are defined (DB sources need per-engine timestamp / sequence-column conventions, Sheets sources have no notion of revision at all, HTTP sources offer only document-level conditional GET). That contract is two things — a business-semantics decision plus validation against a real database — not pure implementation. **Document-level incremental is already implemented** (S8 conditional GET; a 304 reuses the cached bytes); at configuration scale document-level already suffices, and the row-level payoff needs support from load-test data on real large tables. **Unblock condition**: data-contract samples from at least two real sources (column names / semantics / timezone policy) + one connectable real database instance (or reproducible fixture data).
- **OAuth user authorization flow / mTLS / service account credentials**: needs Google-hosted service account credentials and OAuth consent configuration (the Sheets API's JWT exchange needs `subject_token` + granted scopes). This is a credential-custody problem, under the established policy that "credential custody is CI's / the cloud role's business; cage holds no cloud keys" (§47 A3 / A7 decision records); cage's credential surface already stops at "named via env (credential_env / dsn_env / auth_env) + absence checked before network contact (E1904) + diagnostics never contain credentials". For Sheets, an API key alone in the first phase already covers public data and intranet-hosted scenarios. **Unblock condition**: one trial-usable service account JSON (or OAuth client configuration) + scope authorization.
- **Connection pooling / concurrent multi-source / large-table cursor pagination**: needs real MySQL / PostgreSQL instances for concurrency load testing — a connection pool's payoff is measurable only under real I/O contention; mocks cannot measure pooling gains. The current S2 dual backend is "one connection per query," which at configuration scale is the correct minimal implementation (each query is milliseconds; pooling trades CI time for complexity). **Unblock condition**: one connectable real DB instance + large-table (hundred-thousand-row) load-test data. **Landed (2026-10-13)**: the unblock condition was met on this machine (MariaDB 11.8 system service + docker PG 18), the block was lifted and the work implemented —
  - Pooling: one pool per DSN at process level (`cage-source-db::pool`). mysql uses the driver's built-in `Pool` (bounded 0–8, pool created lazily, health check on checkout); postgres 0.19 has no synchronous pool, so a 60-line idle-stack pool was hand-written (`IdlePool`, nesting-safe, idle cap, discarded rather than returned to the pool when a fetch fails). Zero new dependencies.
  - Zero semantic change: every connection checkout still pins read-only (the driver runs `COM_RESET_CONNECTION` on return / session GUCs persist, so re-pinning idempotently is the fallback); a DSN parse failure errors every time, and a bad pool is never registered.
  - Large tables: on the mysql side, `query` with full buffering was switched to `query_iter` row-by-row streaming, so a hundred thousand rows no longer reside twice in memory; pg's simple protocol has no streaming API in 0.19, so buffering stays (sufficient at configuration scale, the bound set by measurement).
  - Measured (a 100k-row table, env-gated live tests `CAGE_DB_TEST_*_DSN`, auto-skipped without the env so they stay out of the gate): byte-deterministic round trips on both backends, a server-side connection-count increment ≤1 across 5 loads (reuse proven), the read-only pin rejecting writes on a real server, introspection working over the same pool. mysql raw fetch of 100k rows in 1.6–1.9s (handshake amortized ≈0.3s each), pg in 0.5–1.3s; full load time is dominated by canonical JSON serialization + cache read-back (on the order of 6–10s) — connection cost is not the bottleneck.
  - The live tests caught and fixed one real bug: MariaDB's text protocol tags temporal columns with charset 63 (binary), and `map_cell` was landing DATETIME into the cache base64-encoded as a binary column — a violation of the "dates → Text" discipline; temporal columns are now exempted from the binary determination by column type, pinned by a live assertion.
  - Concurrent multi-source is not chartered (the data says so): load time is dominated by the serialization pipeline and serial fetching is milliseconds; concurrency would trade deterministic output order for complexity; revisit when a real project shows N sources × network RTT dominating build time (unblock condition: real large tables + load-test data from a networked deployment).

**Red-line alignment**: each of the three touches one of "no server-side product / no cloud keys held / what tests cannot verify does not enter main"; the blocked rulings agree with the existing decision records and need no new policy decision — this batch only registers them and spells out the "unblock conditions," so they can be chartered directly once the input exists.

---

# 50. Asset Governance (2026-10-11, user decision)

The user set the direction: Schema, validation rules, enum definitions, and cross-table constraints are all **formal assets** — brought under version control, off-site copies, historical reproduction, and disaster recovery as four layers of defense — but no large asset-management platform is to be built for backup. Three capabilities are to be added natively to cage: an **asset inventory** (which assets make up the project, each individually hashable), **versioned builds** (builds referencing explicit asset versions), and **integrity verification and recovery** (missing / tampered items detectable, history recoverable).

## The four asset categories × cage's current state, reconciled item by item

| Asset category | User policy | cage landing point | Current state |
| --- | --- | --- | --- |
| Source-data assets | The data proper, maintained by designers | The directories `[source_roots]` points at (xlsx/csv/json/yaml/msgpack under config/) | Already versioned (rides with Git); the hash is anchored into `source_hash` (rotation visible via build_id) |
| Rule assets | Schema / enums / validation rules / cross-table constraints | The schema file set `schema_path` points at | Already versioned; **enums / constraints / references / business rules are embedded in the schema files**, with no separate rules/ directory |
| Build-rule assets | Target platforms / output formats / field mappings | `cage.toml` (profiles / targets / registry / remote) + the `migrations/` declarative migration chain | Already versioned |
| Artifacts and traceability records | Deliverable, reproducible, verifiable | `build/` (rebuildable) + snapshot/ (self-verifying freeze: manifest + schema projection + artifacts + HASHES.json) + registry entries (byte-immutable, E1801) + offline bundle (`.tar[.zst]` + `.sig` ed25519 signature, A6) | All delivered |

Two directory-structure rulings (answering "directory names may be adjusted to the cage implementation; do not restructure just to fit a template"):

- **No separate rules/ directory**: cage's source of truth for rules is the schema files — enums in `enums:`, constraints on fields, references in `references`, business rules registered in `rules` / gamerule plugins. Splitting a `rules/` out would mean two ledgers (which one is the source of truth?), violating the established principle that "the Schema is the single source of truth." The "rule assets" in the user's policy = the full `schema_path` set + `migrations/`.
- **No rename to cage.yaml/cage.lock**: the project entry point is `cage.toml` (it carries all build rules); "locking dependency and rule versions" is carried by the registry's **version pins + byte-immutable entries** (`[dependencies]` range pins, E1801 refusing overwrites, entries never rewritten — stronger than a lock file: what is locked is not a version-number string but a content hash).

## Backup policy (four layers of defense; cage builds only the tooling surface of the first two)

1. **Git for versioning**: rule assets + source-data assets + build-rule assets all ride with the repo (review / diff / rollback treated like ordinary code). Excel is binary — Git storing history is fine, but cell-level diff is not Git's job (LFS and the like wait for a real pain point; nothing is pre-built).
2. **Remote repository for off-site copies**: GitHub / self-hosted Git keeps the remote copy.
3. **Release snapshots for historical reproduction**: `cage snapshot` (a self-verifying frozen view) + `cage registry publish` (immutable entries) answer "which version of the rules was in use, and what was compiled" — build_id / cage_version / schema_hash / source_hash / per-artifact hashes are all already on record in the manifest.
4. **Independent backup for disaster recovery**: the `cage registry export` bundle + `.sig` is a self-verifying offline artifact (it re-passes the ledger trust gate on import); copied to another machine / independent storage, it is the disaster-recovery copy. **GitHub is not disaster recovery** (accidental deletion / account compromise / service unavailability) — layer 4 cannot be skipped.

**Recovery drills = acceptance of the backup loop**: periodically restore from a bundle into a clean directory (`registry import` → `verify`) + rebuild from a snapshot and diff the bytes (the determinism contract guarantees same input → same bytes; an empty diff means reproduction holds). For a complete example see `examples/asset-workspace/` (one-shot `examples/asset-workspace.sh`).

## Landing points for cage-native capabilities

- **Asset inventory → new command `cage assets`** (B1): takes stock of the project's three asset categories — rules / source data / build rules — that is, the full schema set + migrations/ + the `[source_roots]` file set + cage.toml, each item blake3-hashed (consistent with the repo-wide hash policy) + size + category; deterministic ordering and rendering; `--json` for machine reading; `--write <file>` writes the inventory file (committed with Git = the backup baseline); `--check <file>` re-verifies against the current tree — missing / tampered / extra (an unregistered new file) each named item by item, exit code 1 on any mismatch.
- **Versioned builds → already present**: the manifest records build_id / cage_version / schema_hash / source_hash / content_hash / per-artifact hashes + snapshot freezing.
- **Integrity verification and recovery → already present + one addition**: `cage build --verify` / snapshot HASHES.json re-verification / `registry verify` whole-ledger audit are the existing stock; `cage assets --check` fills the "source-side assets" link (the existing verification is all anchored on build artifacts and the registry; source-side assets previously had no native verification command).

## git commit stays out of the manifest (determinism-contract ruling)

The determinism contract of the manifest / snapshot is "same input → same bytes" (the snapshot directory name = profile + build_id; a rerun overwrites the same directory byte for byte). A git commit is a **historical coordinate, not content**: rerunning the same content on different commits (cherry-pick, rebase, dual-repo mirroring) produces different manifest bytes, breaking the byte-for-byte reproduction assertion. Therefore:

- the compiler does not read git, and the manifest records no commit;
- commit traceability is carried by **process**: the release workflow records `git rev-parse HEAD` next to the bundle (`<bundle>.commit`, plain text, part of the same sidecar family as `.sig`), and the build_id in the snapshot directory name is itself a content fingerprint — the "commit + build_id" key pair retrieves any history;
- whoever needs machine-readable provenance generates it in their own workflow (a CI job naturally knows the commit); cage does not do it for them.

## Boundaries (out-of-scope list)

- No asset-management platform / backup scheduler / backup-rotation policy engine (cron and object-storage lifecycle are ops' business);
- No Git LFS integration / binary diff (nothing is pre-built before a real large-table pain point appears);
- No server side (red line restated: snapshots / bundles / the registry protocol are all static-hosting friendly);
- `.cage-cache/` and `build/` stay out of routine backups (rebuildable; snapshots and bundles are the backup artifacts).

---

# 51. Excel Meta-Row Convention (X series, decided 2026-10-27)

Section 3.1 chartered Excel *sheet-level* metadata (hidden sheets never enter the Canonical Model) and rejected Excel *cell comments* as a rule channel. What it did not settle is where the *rule itself* lives for a table authored in Excel. Section 39's "Excel Editor" red line forbids building an editor; it never said Excel cannot carry the rules the compiler checks. This section decides the rule-carrying surface.

**Governing ruling (2026-10-26, user directive):** the convention must be *configurable*. Cage may not assume the author's Excel layout — no preset format is baked in as if it were "the" Excel format. Two consequences are fixed here and bind every later implementation in the X series:

- **vocabulary is not new.** Every rule keyword is an existing Cage Schema DSL field name — `type`, `required`, `default`, `enum`, `min`, `max`, `unique`, `primary_key`, `reference`, `description`, `targets` (T3.1; the same name set as JSON Schema's core keywords where they overlap). No aliasing, no invented terms, no Luban-isms carried over wholesale.
- **the adapter layer invents no rule semantics.** It decodes *layout and position* into that vocabulary, and stops. Cross-table references and L7 game rules are declared as metadata here, but *resolved and enforced* by the schema and validation layers. Meta rows never reach the data plane of the IR (X3 asserts this).

**Boundary of v1:** column-level only. No cross-column expressions, no row-level rules, no game logic. A table that carries no convention declaration is a plain data table — byte-identical to today, no warning, no silent detection.

## Q1 — built-in presets and the cage.toml switch

One grammar, four location presets. The grammar is the same in all of them; only *where convention rows may sit* differs.

**Grammar (one, shared):**

1. A cell whose text starts with the marker (`##` by default) introduces a **keyword** = the remainder of that cell. If a space follows the keyword, the remainder is the value for *that cell's own column*.
2. A plain (non-marker) cell inside a convention row is the value of the nearest marker to its left in the same row.
3. A convention row is a row holding at least one marker cell. Rows with no marker cell are ignored.
4. Column identity is positional — the column of the cell, against the data header row.

**Presets:**

| Preset | Convention territory | Default |
| --- | --- | --- |
| `data` | none — today's behavior | yes |
| `header_block` | the contiguous non-empty run of rows immediately above the data header row (stops at the first empty row) | |
| `marker_rows` | every row above the data header row, scanned in full; marker rows may be interleaved with author notes | |
| `meta_sheet` | a sibling sheet named `<sheet>.meta`, parsed with the same grammar (its first row is the column-name row and must match the main sheet's header row exactly) | |

`marker_rows` is the intended everyday shape: it is the row-of-`##`-keywords layout the 2026-10-26 sourcing directive points at (Luban-style marker rows — cage adopts the marker char and the row-per-keyword shape; the exact Luban dialect is not otherwise relied on or documented here). `header_block` is for authors who keep a single contiguous metadata band. `meta_sheet` is for authors who would rather not share rows with data at all.

**cage.toml** (all optional; `preset` unset = `data`):

```toml
[source.excel]
preset = "marker_rows"            # data | header_block | marker_rows | meta_sheet
marker = "##"                      # every built-in preset; must start with '#'
workbooks = { "Skill.xlsx" = "header_block" }   # per-workbook override, keyed by workbook file name

# — instead of the built-in presets: the script adapter (mutually exclusive with preset) —
script = "tools/excel-meta.py"    # relative to the project root, forward slashes
script_args = [ "--strict" ]      # extra arguments, appended after the script path
script_timeout_secs = 10          # default 10; floored at 1
script_config = { }               # any TOML table, passed to the script verbatim
```

Selecting a preset is *explicit*: no `[source.excel]` means no convention, and the source byte stream is read exactly as today. There is no auto-detection of marker rows anywhere — a stray `##` in a data cell of a `data`-preset table is data.

`workbooks` matches by file name; if the same file name exists under two source roots the override is ambiguous (E2201), because a file-name key cannot say which workbook it means.

## Q2 — the script adapter: contract, vehicle, determinism

**Vehicle: an external subprocess.** cage writes one JSON document to the script's stdin and reads one from stdout. Language-agnostic: Python, Node, PowerShell, a compiled binary — anything the author can run.

*Rejected, explicitly:* an embedded interpreter (a Python/JS runtime inside cage). It would make cage a code-execution surface with a supply-chain dependency it cannot vet, and it buys nothing the subprocess does not already give — the same contract either way, plus a large dependency and a security posture the repo does not want.

**Trust boundary, stated plainly:** the script is *author tooling*. cage does not sandbox it and makes no sandboxing claim; the author runs it with their own privileges because they chose to. What cage does do:

- the script path is project-root-relative and explicit; absolute paths, `..` escapes, and non-existent / non-executable targets fail at start (E2203). No implicit search paths, no `$PATH` lookup;
- the only channel back is stdout JSON, and it is **validated as data** — first against the contract schema, then against the vocabulary table. Unknown keys, an unknown `kind`, a malformed `reference` object: E2202 / E2206. Output is never interpreted as code, never re-serialized into a shell, never written anywhere.

**The contract** is versioned `cage.excel-meta/1` and published as a machine-checkable document: [`schemas/excel-meta-adapter-v1.schema.json`](schemas/excel-meta-adapter-v1.schema.json) (JSON Schema 2020-12, `$defs.request` and `$defs.response`).

- **request → script**: `workbook`, `sheet`, `header_row` (as cage resolved it, 0-based), and the whole visible sheet as `grid` — a 2-D array of number / string / boolean / `null` (JSON has a single numeric type, so integer and non-integer are one thing: a whole number stays whole as long as the emitter writes no fractional part). Empty cells are `null`; merged cells are back-filled to the top-left value (xlsx only, `.ods` never); dates arrive in the same form the data plane renders them in — the `Debug` form of the Excel serial (`ExcelDateTime { value: 44927.0, datetime_type: DateTime, is_1904: false }`), not a normalized ISO-8601 string — so a script sees what the IR sees; formula cells carry their cached value, nothing is re-evaluated. Plus `config` = the `script_config` table verbatim, so authors tune their own script without new cage knobs.
- **script → cage**: `table`, `header_row` (0-based, as the script says the data header really is), and `columns` keyed by data column name, each value carrying the vocabulary fields plus a required `cell` anchor in A1 form (`"B2"`, `"G27"`) so every later diagnostic points at the exact cell. The value of `type` is the Schema field's own `type` slot **verbatim** — the adjacently tagged `{ kind, value }` object (`{ kind: Int32 }`, `{ kind: Array, value: { kind: Int32 } }`, `{ kind: Enum, value: Rarity }`, `{ kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }`), one of its 19 kinds. The contract invents no shorthand: there is no translation step between the contract and the schema, so there is nowhere for the two spellings to drift.

**Determinism:** the source fingerprint that feeds `source_hash` / `build_id` gains four inputs — the blake3 of the script's bytes, the resolved script path, the canonical form of `script_config`, and the contract version. Editing the script or its config invalidates caches and changes `build_id`, exactly as editing a workbook does. cage cannot *prove* a script is deterministic (a script that embeds a timestamp will produce a `build_id` flip per run); it can make the failure obvious, and the cache chain guarantees no stale metadata ever masquerades as current. That is the same standard as every other source input, and it is all that the determinism contract ever claims.

## Q3 — failure semantics: one standard, both sides

Every failure is reported. Nothing is silently dropped, and nothing silently degrades to "data". The built-in presets and the script adapter emit the **same** codes for the **same** condition, so a diagnostic is read the same way whichever decoder produced it. Cell-level conditions carry the A1 anchor; declaration-level conditions carry workbook + sheet.

| Code | Condition |
| --- | --- |
| **E2201** | convention *syntax*: unknown keyword, a value before any keyword, a duplicate keyword on one column, a marker cell with an unparseable form, `preset` and `script` both set, a preset declared while `has_header = false` |
| **E2202** | convention *vocabulary*: a value that does not resolve into the vocabulary — an unknown `kind`, an `enum` that is not a value list, a malformed `reference` object |
| **E2203** | script will not start: path outside the project root, `..` escape, not found, not executable, spawn failure |
| **E2204** | script exceeded `script_timeout_secs` |
| **E2205** | script exited non-zero (stderr is quoted, truncated) |
| **E2206** | script output is unusable: not valid JSON, not the contract shape, or a contract version cage does not know |
| **E2207** | convention contradicts the data: a declared column name that is not in the header row, a `meta_sheet` name row that does not match, a response whose `table` is not the sheet's table |

E22xx is the source-convention family; E10xx–E19xx (validation), E20xx (migration) and E21xx (distribution) are untouched.

## Q4 — relation to the existing knobs

The four knobs in `ExcelSourceAdapter` (`header_row`, `has_header`, `eval_formulas`, `skip_empty_rows`) keep their exact meaning and their exact defaults. What changes is what *surrounds* them:

- `header_row` still names the **data** header row. Convention territory is computed *above* whatever `header_row` resolves to — so raising `header_row` to make room for the first type row is **not** how conventions are declared, and rows above it are never silently swallowed as data;
- `has_header = false` cannot carry a convention (there is nothing to key columns by), so a preset alongside it is E2201 rather than a mystery;
- `eval_formulas` and `skip_empty_rows` are untouched. A blank row still ends the `header_block` run; cached formula values are what the adapter sees.

## Q5 — as-built notes (X1, 2026-10-27)

Deltas between this decision and what X1 ships, recorded so the decision record and the code cannot drift.

**The comment row is `##description`.** The vocabulary is the Schema field-name set, so a comment row is declared with the field the schema actually has. `##comment` is E2202 (unknown variant) — cage keeps no alias table and adds no keywords beyond the ones in the table above.

**Cell type notation, presets only.** A preset cell accepts two spellings, both resolved into the Schema's own `FieldType`: the shorthand (`Int32`, `Any`, `Array<T>`, `Enum<Name>`, `Map<string|int, T>`, nestable — `Array<Array<String>>`), and the Schema expression form (`{ kind: Int32 }`, `{ kind: Array, value: { kind: Int32 } }`). This is the one place cage accepts a shorthand; the script contract of Q2 still demands the tagged form verbatim, so the contract keeps its no-translation property. `Object` has no cell notation and is refused with E2202 — a nested object is declared in the schema, not in a cell.

**`marker` is not `marker_rows`-only.** It is the marker char of the shared grammar and applies to all three built-in presets that read marker cells (`marker_rows`, `header_block`, `meta_sheet`). `data` ignores it.

**`script_args` and the timeout floor.** `script_args` is appended after the script path at spawn; it is not part of the stdin contract and not part of the fingerprint. `script_timeout_secs` below 1 is floored to 1 second — there is no zero, i.e. no unlimited wait, because a hung script must be reportable as E2204 rather than stall the build.

**A hidden meta sheet is read; a hidden data sheet is not.** Hidden sheets never enter the Canonical Model as data (3.1), and a `<sheet>.meta` carrier is never parsed as a table — but the carrier *is* read even when hidden, because a convention carrier is authoring metadata rather than data, and hiding a meta sheet is a plausible authoring artifact. Only the data-plane carve-out applies.

**The script places the header row.** With a script, `response.header_row` is the data header row: the script runs before header extraction, so it can sit below a type band it owns. Built-in presets never move the data header row (Q4).

**The source fingerprint lands with the consumer.** X1 decodes metadata and does not read it — `parse_file` is the data-plane projection of `parse_file_with_meta`, the metadata dropped. No `build_id` byte changes in X1; the four fingerprint inputs above are wired in X2, where the metadata is first consumed. That is also why X1's byte-level regression is a structural property rather than a coincidence.

**Deferred to X4 — the ambiguous workbook override.** `workbooks` is keyed by file name, so a name present in two source roots cannot say which workbook it means. X1 applies the override to every workbook whose file name matches, in every source root, and does not raise E2201: deciding it needs a cross-root pass over all source roots, which the per-file adapter does not have. Until then, a duplicate file name with a `workbooks` override is a per-root application, not a rejection.

**`cage-source-sheets` keeps the same contract, later.** The convention is Excel-only in v1: sheets carry no meta rows, and the sheets adapter reads `UNFORMATTED_VALUE` from the Sheets API with no place to put declarations. The grammar and vocabulary are unchanged, so extending it is an adapter decision, not a redesign.

This section is the architecture decision. X1 landed the preset parsers, the subprocess runner, the E22xx codes and the cage.toml switch — the deltas are recorded in Q5. X2 lands the `schema-draft` introspection and the drift check against an adopted schema, X3 the merge into the three-source parity suite, X4 the docs and the runnable example.
