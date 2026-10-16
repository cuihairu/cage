[中文](target.md) | English

# Target: Normalization and Artifacts

Once validation passes, the pipeline moves into normalization (Normalize), after which the Target Generator produces the target assets (Artifacts).

## Normalize

The Normalize step in the pipeline performs **representation-layer** normalization, not cross-type conversion:

- String trimming and removal of trailing `\r`;
- Float `-0.0` → `0.0`;
- Object keys are emitted sorted via `BTreeMap`.

Type assignment is decided at the source and strictly guarded at L2: type inference runs during text-source parsing (CSV's
`true`/`false`/`yes`/`no` → Bool, integer/float text → Int/Float, empty →
Null), and L2 validates by **type family** (Int only pairs with Int8–Int64, UInt only with UInt8–
UInt64; crossing families raises `E1101`. There is no string↔number recalibration — numeric text
written into a String column, or `100` written into a uint32 field, both raise type errors).

Cross-type coercion (`"100"` → Int32, `1` → Bool(true), etc.) is a cage-core
library API (`normalize::coerce_to_type`) that is **not currently wired into the build pipeline** —
embedders may call it themselves; pipeline integration is a planned item.

The goal of Normalize:

> **Data with identical semantics should produce the same Canonical Representation** (this holds
> within one representation layer: strings identical after trim and object keys identical after
> sorting share one canonical form).

Normalization uses deterministic rules (`BTreeMap`, stable sorting), so identical inputs always yield identical outputs — the groundwork for [deterministic builds](/build.en).

## Transform

Transform is responsible for:

> Canonical Model → Target Artifact

rather than re-validating data. Validation is already complete in the [Validation pipeline](/validation.en); Transform only converts:

```text
                  Valid Model
                       |
       +---------------+---------------+
       |               |               |
      CSV             JSON             Code
                                       |
                  +--------------------+--------------------+
                  |         |         |         |          |
                 C#      Python      Lua    TS/JS   C++  Go  Java
```

## Target Generator

The Target Generator is also a plugin.

MVP stage (JSON / CSV):

```text
CSV
JSON
```

Second-stage expansion (implemented):

```text
C#
Python
Lua
TypeScript
JavaScript
C++
Go
Java
```

Definition file generation (implemented, schema-driven; data does not participate in generation):

```text
Protobuf      (.proto)
FlatBuffers   (.fbs)
JSON Schema   (.schema.json)
```

Later:

```text
Binary
SQLite
```

### Data Targets and Code Targets Are Separate

These two kinds of Target should be clearly distinguished.

Data Targets (data serialization):

```text
JSON            ← 已实装
CSV             ← 已实装
MessagePack     ← 已实装
YAML            ← 规划
Binary          ← 规划
```

(Protobuf / FlatBuffers land as schema-driven definition file generation (see the "Protobuf
Target" and "FlatBuffers Target" sections below) and are not in this list — they do not do data
wire encoding.)

Code Targets (code generation):

```text
C#              ← 已实装
Python          ← 已实装
Lua             ← 已实装
TypeScript      ← 已实装
JavaScript      ← 已实装
C++             ← 已实装
Go              ← 已实装
Java            ← 已实装
Protobuf        ← 已实装（.proto 定义文件）
FlatBuffers     ← 已实装（.fbs 定义文件）
JSON Schema     ← 已实装（标准 JSON Schema 文档）
Template        ← 已实装（用户自定义模板）
```

(Items marked as planned are outside `format =` support; the build reports
`unsupported target format '<fmt>'` and fails with exit code 2.)

The three data targets each have their own options:

- `json`: `pretty` (default `true`, indented pretty-printed output), `sort_keys` (default
  `true`, object keys in name order; `false` preserves source field order). The library API
  additionally offers `generate_combined` producing `all.json` (table name → row array); the CLI
  does not expose it yet.
- `csv`: `delimiter` (single character, default `,`), `write_header` (default
  `true`), `quote_style` (`always` / `never` / `non_numeric`; invalid values silently fall back to
  the default "quote as needed").
- `msgpack`: `sort_keys` (default `true`, row fields enter the map in name order; `false`
  preserves source field order — nested object keys are still sorted by normalize, same policy as
  `json`).

### MessagePack Target (implemented)

A data target: Canonical Model → MessagePack binary, one
`{table}.msgpack` per table (table = row array, row = field map).

This format doubles as a source format: `cage-source-msgpack` re-consumes the
`.msgpack` files it wrote itself (registry entries under `data/**` are packaged in this format; see
the fidelity choice in [CLI · registry](/cli.en)), with lossless round-trips for
floats/Bytes/integers beyond i64.

```toml
[[profiles.client.targets]]
format = "msgpack"              # 别名 messagepack
output_dir = "build/msgpack"
file_template = "{table}.msgpack"

[profiles.client.targets.options]
sort_keys = true                # 默认 true：行字段按名序
```

Encoding and determinism notes:

- Encoding uses the `rmp` smallest-form: fixed values mean fixed bytes; the same input document
  always yields byte-identical artifacts (locked by golden tests)
- Type mapping: Null → nil, Bool → bool, Int/UInt → smallest integer-family representation,
  Float → f64, String → str, Bytes → native bin (no base64 detour),
  Array → array, Object → map
- Non-finite floats (NaN / ±Inf) encode as nil — NaN bit patterns are not stable across platforms,
  and direct binary encoding would break the determinism contract (same rule as the JSON target:
  non-finite → null)
- With `sort_keys = false`, row fields keep source field order; nested object keys are still sorted
  by normalize's `BTreeMap` (same policy as `json`)

### Protobuf Target (implemented)

Design draft §23 listed Protobuf under Data Targets without settling the wire-encoding question; the
implementation decision (2026-10) is: **emit .proto3 definition files** — schema-driven, in the
same family as code bindings like C#/Go (data does not participate in generation; consumers compile
with `protoc` and their own runtime).
One `{table}.proto` per table (table → message, field numbers 1..n assigned in name order); shared
integer enums are centralized in `cage_enums.proto`, and table files `import` it as needed.

```toml
[[profiles.client.targets]]
format = "proto"               # 别名 protobuf
output_dir = "build/proto"

[profiles.client.targets.options]
package = "cage.generated"     # 默认 cage.generated（原样写入 package 子句）
enums_file = "cage_enums.proto" # 默认 cage_enums.proto（import 路径随之）
```

Type mapping table (proto3):

| Cage type | proto3 type | Notes |
|-----------|-------------|------|
| int8 / int16 / int32 | `int32` | value range fully covered, lossless |
| int64 | `int64` | |
| uint8 / uint16 / uint32 | `uint32` | |
| uint64 | `uint64` | |
| float32 | `float` | |
| float64 | `double` | |
| bool / string / bytes | `bool` / `string` / `bytes` | |
| array\<T\> | `repeated T'` | T' is the mapping of T; when T is an array/map, wrap in one nested message |
| object | `google.protobuf.Struct` | free-form policy (same family decision as C++'s `std::map<string, any>`); imports the well-known type |
| null / any | `google.protobuf.Value` | same as above |
| map\<string, V\> | `map<string, V'>` | |
| map\<int, V\> | `map<int64, V'>` | key int → int64 (same family policy as go/cpp/java) |
| enum (all members integer and within int32 range) | corresponding proto enum | see below |
| enum (has string members / missing literals / beyond int32 range) | `string` | same-family fallback policy; a comment on the field notes the original enum name |

Key generation rules:

- Determinism: tables in name order, fields in name order, field numbers 1..n assigned by name
  order; identifiers share one deduplication set (table names first, enum names after, appending
  `_` on conflict); the same Schema always produces byte-identical files, and file headers stamp the
  schema hash recorded in the manifest
- proto3 requires the first enum member to be 0: when no member has value 0, a synthetic
  `{Enum}_UNSPECIFIED = 0;` is prepended; when member values repeat, `option allow_alias = true;`
  is emitted
- When arrays/nested maps land in a repeated element or map value position (proto forbids
  `repeated repeated` / map wrapping map), a `{field}Value` nested message is generated to carry
  them (suffix stacks with depth: `{field}ValueValue`)
- Optional (non-required) fields carry `optional` (proto3 explicit presence); repeated/map never do
- Reserved words and scalar type names used as identifiers → trailing `_`; illegal characters
  normalized to `_`; a leading digit gets a `_` prefix; same-named members deduplicated
  deterministically
- When a `repeated` element or map value type is object/null/any, the well-known type
  message is used — legal, no wrapping needed

### FlatBuffers Target (implemented)

Same family as Protobuf: **emit `.fbs` definition files** — schema-driven, in the same family as
code bindings like C#/Go (data does not participate in generation; consumers compile
with `flatc` and their own runtime).
One `{table}.fbs` per table (table → `table`, fields in name order); shared integer enums are
centralized in `cage_enums.fbs`, and table files `include` it as needed.

```toml
[[profiles.client.targets]]
format = "flatbuffers"         # 别名 fbs
output_dir = "build/fbs"

[profiles.client.targets.options]
namespace = "cage.generated"    # 默认 cage.generated（写入 namespace 子句）
enums_file = "cage_enums.fbs"   # 默认 cage_enums.fbs（include 路径随之）
```

Type mapping table (`.fbs`):

| Cage type | `.fbs` type | Notes |
|-----------|-------------|------|
| bool | `bool` | |
| int8 / int16 / int32 / int64 | `byte` / `short` / `int` / `long` | fbs short-name spellings |
| uint8 / uint16 / uint32 / uint64 | `ubyte` / `ushort` / `uint` / `ulong` | |
| float32 / float64 | `float` / `double` | |
| string | `string` | |
| bytes | `[ubyte]` | |
| array\<T\> | `[T']` | T' is the mapping of T; when T is an array/map, a wrapper table `{Field}Value` is used |
| map\<string, V\> | `[{Field}Value]` | fbs has no native map; the idiomatic entry table: `table {Field}Value { key: string; value: V'; }` |
| map\<int, V\> | `[{Field}Value]` | integer keys widened to `long` (the data model stores numeric strings; same family rule as proto int64) |
| object / null / any | `string` | free-form payload; fbs has no free-form type → carried as JSON text |
| enum (all members integer, non-empty, no duplicates, in range) | shared `enum Ident : <smallest base type>` | base-type ladder byte→ubyte→short→ushort→int→uint→long→ulong, take the smallest that fits |
| enum (has string members / missing literals / out of range / **duplicate values**) | `string` | flatc has no `allow_alias`; duplicate values make the whole family illegal → the entire enum degrades to a string field (the corresponding proto ruling) |

Key generation rules:

- Determinism: tables in name order, fields in name order; identifiers share one deduplication set
  (table names first, enum names after, appending `_` on conflict); the same Schema always produces
  byte-identical files, and file headers stamp the schema hash recorded in the manifest
- Attributes: single-field primary key mapping to `string` or an integer scalar → `(key)`;
  `required` with field type `string` / vector / table → `(required)` (fbs syntax only allows
  non-scalar required); each file ends with `root_type <Table>;`
- Nested arrays / map value positions land in wrapper tables (`{Field}Value`, suffix stacking with
  depth as `{Field}ValueValue`; composite map values are further wrapped in `{Field}Entry` to carry them)
- Reserved words and scalar type names used as identifiers → trailing `_` (case-insensitive
  comparison); illegal characters normalized to `_`; a leading digit gets a `_` prefix; same-named
  members deduplicated deterministically
- No wire encoding: this repo has no flatc to serve as a byte-level oracle; deterministic binaries
  are covered by MessagePack

### JSON Schema Target (implemented)

Standard JSON Schema document generation: one **self-contained** `{table}.schema.json` per table
(`properties` in schema field declaration order), consumed directly by editors, ajv, and other
non-cage toolchains — no cage runtime needed. draft-07 by default (widest tool support),
2020-12 via `options.draft`. Documents are pure standard JSON Schema (no custom extension keys);
determinism = table name order × declaration-order key insertion order; enums are always inlined as
`enum` arrays, with no shared enum file.

```toml
[[profiles.client.targets]]
format = "jsonschema"          # 别名 json-schema / json_schema
output_dir = "build/jsonschema"

[profiles.client.targets.options]
draft = "07"                   # 默认 07；2020-12 切换；未知值报错（exit 2）
```

Type mapping table (JSON Schema keywords):

| Cage type | JSON Schema | Notes |
|-----------|-------------|------|
| null | `{"type": "null"}` | |
| bool | `{"type": "boolean"}` | |
| int8–64 / uint8–64 | `{"type": "integer"}` | integer family shares one type, `min`/`max` → `minimum`/`maximum` (integral bounds rendered as integers) |
| float32 / float64 | `{"type": "number"}` | same as above |
| string | `{"type": "string"}` | `min_length`/`max_length`/`pattern` → `minLength`/`maxLength`/`pattern` |
| bytes | `{"type": "string", "contentEncoding": "base64"}` | `contentEncoding` is draft-07 only (removed as a keyword in 2020-12) |
| array\<T\> | `{"type": "array", "items": T}` | `min_items`/`max_items` → `minItems`/`maxItems` |
| object | `{"type": "object", "properties": …, "additionalProperties": false}` | closed form for typed objects with declared properties (consistent with the validator: unknown keys rejected); an empty Object is free-form |
| map\<string, V\> | `{"type": "object", "additionalProperties": V}` | open-form object; the value domain is V |
| map\<int, V\> | same as above + `propertyNames.pattern: ^-?[0-9]+$` | int keys are numeric strings in the data model, same policy as the validator |
| enum | `{"type": "string", "enum": [member names…]}` | **instances are always member names** (L2 fixes the enum field type as string; integer literals are only code generation metadata); dangling enum names degrade to plain `string` (L1 reports that separately) |
| any | `{}` | no constraints applied |

Key generation rules:

- Table document key order is fixed: `$schema` → `title` → `description` → `type` →
  `properties` → `required` → `additionalProperties`; key order within a property is fixed likewise
  (type → description → default → constraint keywords) — the fixed insertion order locks byte
  determinism
- `required` collects required fields (declaration order); non-finite min/max are not written out
  (JSON has no NaN/Inf); an inline `enum_values` integer-string domain on an integer field renders
  as a numeric enum (instances are JSON numbers; a string enum would mismatch)
- Cross-table references and semantic rules (rules) have no standard JSON Schema spelling and do
  not enter the documents — they are still enforced by cage's own pipeline against the source of truth
- An unknown `options.draft` value is the only failure path (same family as template: a config
  error reports exit 2 at generate time), and not a single artifact is written

For the Code Targets' current generation approach (plan → render → verify direct rendering, no AST
library dependency) and the rationale behind it, see the Code Targets chapter of the repo design
draft `docs/design.md`; the templated form (Template Target, Tera) has shipped in full — official
language templates ship with the package (G2), and user-defined templates plug in via
`format = "template"` (G3, see the next section). Decisions still live in the Rust filter layer;
see also §22.

For example, the same data:

```text
Canonical Model
      |
      +---- JSON     （稳定键序、确定性字节输出）
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

### C# Target (implemented)

The first Code Target to land: generates C# class bindings from the Schema (data does not
participate in code generation; types and metadata all come from the Schema). One `{table}.cs` per
table, shared enums in their own compilation unit; output is deterministically
sorted (tables/fields in name order, enum values keep Schema order), so the same Schema always
produces byte-identical files, and file headers stamp in the schema hash recorded in the manifest.

```toml
[[profiles.client.targets]]
format = "csharp"            # 别名 cs
output_dir = "build/cs"
file_template = "{table}.cs"

[profiles.client.targets.options]
namespace = "Cage.Generated"  # 默认即此值
enums_file = "CageEnums.cs"  # 默认 CageEnums.cs
```

Key generation rules:

- Integer enums (every member has an integer value) generate an `enum`, with the
  `int`/`long`/`ulong` underlying type auto-selected by value range; string/valueless enums generate
  `static class` constants (Cage enums compare by string)
- Fields that are optional and have no default generate nullable types (`string?`); required
  reference types get an empty initializer (`= string.Empty;`), guaranteeing zero-warning
  compilation under `#nullable enable`
- Schema defaults render as initializer literals (objects/type mismatches skipped); unresolved
  enums fall back to `string` with a doc comment noting it
- Reserved words get an `@` prefix, illegal identifier characters are normalized to `_`, and
  same-named members are deduplicated deterministically
- Map fields (`Map<K,V>`, the 19th field type): `Dictionary<K, V>` (keys
  string→`string`, int→`long`), value types recursing
  (`map<string, Array<Int32>>` → `Dictionary<string, IReadOnlyList<int>>`); an empty map default
  renders `= new();`, non-empty scalar members render as collection initializers, with keys in
  deterministic sorted order

### Python Target (implemented)

Generates `@dataclass(frozen=True)` bindings from the Schema. One `{table}.py` per table,
shared enums centralized in `cage_enums.py`; the determinism policy matches the C# Target exactly
(tables in name order, fields in name order, enum values keep Schema order, file headers stamp the
schema hash).

```toml
[[profiles.server.targets]]
format = "python"            # 别名 py
output_dir = "build/python"
file_template = "{table}.py"

[profiles.server.targets.options]
enums_file = "cage_enums.py" # 默认 cage_enums.py
```

Key generation rules:

- dataclass syntax requires fields without defaults first: required fields with no renderable
  default come first (name order within the group), fields with defaults follow; optional fields
  without defaults are annotated `T | None = None`
- Integer enums (every member has an integer value) generate `IntEnum`; the rest generate constant
  classes (the member name/string value is the constant value; Cage enums compare by string)
- Schema defaults render as initializer literals: array defaults go through
  `field(default_factory=lambda: [...])` (avoiding mutable objects shared across instances),
  objects/type mismatches skipped; unresolved enums fall back to `str` with a comment noting it
- Reserved words get a trailing `_` (PEP 8), illegal identifier characters are normalized to `_`,
  and same-named members are deduplicated deterministically
- Map fields: `dict[K, V]` (keys string→`str`, int→`int`), value types recursing
  (`dict[str, list[int]]`); an empty map default is `field(default_factory=dict)`, non-empty
  `field(default_factory=lambda: {...})` (not shared across instances)

### Lua Target (implemented)

Generates table binding modules from the Schema. One `{table}.lua` per table, shared enums
centralized in `cage_enums.lua` (flat tables of the form `M.<Enum> = { 成员 = 值, ... }`);
the determinism policy matches C#/Python.

```toml
[[profiles.server.targets]]
format = "lua"
output_dir = "build/lua"
file_template = "{table}.lua"

[profiles.server.targets.options]
enums_file = "cage_enums.lua" # 默认 cage_enums.lua
```

Each table module exports:

- `M.name` / `M.description` / `M.primary_key`: meta information
- `M.fields`: list of field metadata (name order: `name` the original schema name, `key` the
  normalized Lua key, `type` the type tag, `required`), with a constraint summary kept as comments
- `M.defaults`: table of renderable Schema defaults (objects/type mismatches/non-finite floats
  have no Lua literal and are skipped)
- `M.new(t)`: row constructor; caller-supplied fields win, missing fields fall back to `M.defaults`
  (array defaults are copied in, rows share no state)

Key generation rules:

- Integer enum values keep their numbers; string/bool/valueless members use their string form (Cage
  enums compare by string, and the member name is the last-resort fallback)
- Reserved words get a trailing `_` (Lua has no `@`-style escaping), illegal identifier characters
  are normalized to `_`, and same-named members are deduplicated deterministically
- String escaping uses `\n` / `\ddd` (Lua has no `\xHH`)
- Map fields: type tag `map<K, V>` (keys string/int, values recurse); the field comment states
  "a key-value table (hash part), distinct from Array's array table (sequence part)", and int
  keys prompt the consumer to convert with `tonumber`; an empty map default is `{}`, and `M.new`
  deep-copies via `copy_default`, so rows share no tables

### TypeScript / JavaScript Target (implemented)

One crate, two flavors of typed module bindings. `format = typescript` (alias `ts`)
emits one `{table}.ts` per table (`export interface` + a Defaults constant + a factory function);
`format = javascript` (alias `js`) emits a `{table}.js` with JSDoc types per table, paired by
default with a `.d.ts`. Shared enums are centralized in `cage_enums.ts` / `cage_enums.js`;
the determinism policy matches C#/Python/Lua exactly.

```toml
[[profiles.client.targets]]
format = "typescript"          # 别名 ts
output_dir = "build/ts"
file_template = "{table}.ts"

[profiles.client.targets.options]
enums_file = "cage_enums.ts"   # 默认 cage_enums.ts（js 形态默认 cage_enums.js）
```

```toml
[[profiles.server.targets]]
format = "javascript"          # 别名 js
output_dir = "build/js"

[profiles.server.targets.options]
emit_dts = false               # 默认 true：为每个 .js 配对 .d.ts
```

Key generation rules:

- One `export interface` per table: required fields or fields with renderable defaults are
  mandatory properties, the rest optional with `?`; accompanied by
  `export const {Table}Defaults: Partial<T>` and an `export function new{Table}(init?)` factory —
  default literals are inlined into the factory, each call allocates fresh arrays, and rows share
  nothing
- Enums export as `as const` constant objects + derived type aliases (`ItemKind.Sword` works as
  both a value and a type), with numeric/string buckets partitioned by member value; table files
  reference only the enums they actually use, via `import type`
- The JS flavor uses `@typedef` / `@property` (optional properties written `[name]`), with enums
  referenced as JSDoc `import("…")` types and no runtime import produced; `emit_dts` by
  default pairs every file with a `.d.ts` (`declare const` + literal types)
- Property names get no reserved-word escaping (legal in property position, guaranteeing 1:1 with
  Schema fields); binding identifiers (interface/const/function names) get a trailing `_` for
  reserved words, illegal characters normalized to `_`, and same-named members deduplicated
  deterministically
- Non-finite floats have native literals: `NaN` / `Infinity` / `-Infinity` render as usual
- Map fields: `Map<K, V>` (keys string→`string`, int→`number`), value types recursing
  (`Map<string, number[]>`); defaults render as `new Map()` / `new Map([["k", v]])` rather than
  object literals, same policy in JSDoc / `.d.ts`

### C++ Target (implemented)

Generates header-file bindings from the Schema: one `{table}.h` per table (`struct` + member
initializers), one `cage_enums.h` for shared enums; the determinism policy matches
C#/Python/Lua.

```toml
[[profiles.client.targets]]
format = "cpp"                 # 别名 c++ / cxx
output_dir = "build/cpp"
file_template = "{table}.h"

[profiles.client.targets.options]
namespace = "cage::generated"  # 默认 cage::generated
enums_file = "cage_enums.h"    # 默认 cage_enums.h
```

Key generation rules:

- Required fields or fields with renderable defaults are plain members (value-initialized `{}`
  when there is no default, a default rendered as a member initializer `{"pvp"}`); optional fields
  without defaults are wrapped in `std::optional<T>`
- Integer enums (all members have integer values) generate an `enum class` with the
  `std::int32_t` / `std::int64_t` / `std::uint64_t` underlying type auto-selected by value range;
  string/valueless enums generate a `namespace` + `inline constexpr std::string_view` constants
- Type mapping: `std::string`, `std::vector<T>`, bytes →
  `std::vector<std::uint8_t>`, objects → `std::map<std::string, std::any>`,
  Map fields → `std::unordered_map<K, V>` (keys string→`std::string`,
  int→`std::int64_t`, value types recursing; kept distinct from the object's `std::map`),
  string-bucket enum fields → `std::string_view` (the bucket itself is a namespace with no
  standalone type, and the field holds a view of its constants)
- The include set is trimmed to actual usage (`cstdint` / `optional` / `string` /
  `vector` / `map` / `unordered_map` / `any` / `string_view` / `limits`),
  system headers in name order, quoted headers in their own group; the enum header is referenced
  by the relative path configured in `enums_file`
- Non-finite floats use `std::numeric_limits<T>::quiet_NaN()` / `infinity()`;
  string escaping uses `\n` and three-digit octal (C++'s `\x` swallows following hex digits)
- Reserved words get a trailing `_`, illegal characters normalized to `_`; table structs and enums
  share one namespace's deduplication set, deterministically dodging same-name collisions

### Go Target (implemented)

Generates struct bindings from the Schema: one `{table}.go` per table (exported fields + `json`
tags + a `New{Table}` default-value constructor), one `cage_enums.go` for shared enums (`type` +
`const` blocks); all files share one package, and package-level declarations share a single
deduplication set. The determinism policy matches C#/Python/Lua, and output is byte-identical to
gofmt.

```toml
[[profiles.server.targets]]
format = "go"                  # 别名 golang
output_dir = "build/go"

[profiles.server.targets.options]
package = "config"             # 默认 config
enums_file = "cage_enums.go"   # 默认 cage_enums.go
file_template = "{table}.go"   # 默认 {table}.go
```

Key generation rules:

- Field names export in PascalCase (`player_id` → `PlayerId`), always carrying a
  `json:"原始字段名"` tag; required fields or fields with defaults are plain types, optional
  value types without defaults use pointers (`*int32` / `*string` / `*枚举`),
  and slices/maps use nil to express absence
- `New{Table}()` is always generated: it fills only fields with renderable defaults, with literals
  allocated fresh on each call (not shared across rows); non-finite floats use
  `math.NaN()` / `math.Inf(±1)` (the only scenario that introduces `import "math"`)
- Integer enums pick an `int32` / `int64` / `uint64` underlying `type` by value range + prefixed
  consts (`ItemKindSword`, unique at package level); the rest generate `string`-underlying const
  blocks
- The first line of each file header carries the Go toolchain recognition mark
  `// Code generated by Cage — DO NOT EDIT.`; field alignment, const blocks, and trailing
  comments are all byte-identical to gofmt output
- Map fields: `map[K]V` (keys string→`string`, int→`int64` — `encoding/json` natively supports
  decoding quoted strings for integer keys), value types recursing (`map[string][]int32`); like
  slices, nil expresses absence and no pointers are emitted; `New{Table}` defaults render as
  `map[K]V{}` / entry literals

### Java Target (implemented)

Generates class bindings from the Schema: one `{table}.java` per table (public fields +
field-initializer defaults), one `CageEnums.java` for shared enums (a holder class nesting
`public enum`s with a value payload); the determinism policy matches C#/Python/Lua.

```toml
[[profiles.server.targets]]
format = "java"
output_dir = "build/java"

[profiles.server.targets.options]
package = "cage.generated"     # 默认 cage.generated
enums_file = "CageEnums.java"  # 默认 CageEnums.java
file_template = "{table}.java" # 默认 {table}.java
```

Key generation rules:

- **File name = public class name** (a hard javac constraint): the `{table}` placeholder uses the
  final class identifier rather than the raw table name; table classes and the enum holder share a
  package, deterministically dodging collisions via the unified deduplication set
- Required fields or fields with renderable defaults are plain types; optional primitives without
  defaults are promoted to wrappers (`Integer` / `Double` / …), reference types are nullable by
  nature; array defaults use `new ArrayList<>(List.of(…))`, allocated fresh per instance
- Unsigned types widen one step: `UInt8 → short`, `UInt16 → int`, `UInt32 / UInt64 → long`
  (Java is all-signed; u64 **enum values** beyond `Long.MAX_VALUE` are written as
  two's-complement long literals with a trailing comment `wrapped from unsigned …`; if a
  field default exceeds the range, the default is skipped entirely)
- Integer enums pick an `int` / `long` payload by value range (`public final value` +
  a constructor); string/valueless enums carry a `String` payload (member name as fallback);
  non-finite floats use `Double.NaN` / `…POSITIVE_INFINITY`
- Reserved words get a trailing `_` (a lone `_` is illegal and normalizes to `__`), illegal
  characters normalized to `_`, and same-named members deduplicated deterministically
- Map fields: `HashMap<K, V>` (keys string→`String`, int→`Long`, value types boxed and recursing:
  `HashMap<String, List<Integer>>`); an empty default is `new HashMap<>()`, scalar members
  `new HashMap<>(Map.of(…))` allocated fresh per instance (skipped entirely beyond
  `Map.of`'s 10-pair overload limit); imports are injected by recursively scanning nested
  field types

## Template: user-defined templates

`format = "template"` renders the Schema IR with [Tera](https://keats.github.io/tera/) templates —
any text artifacts beyond the language bindings (config files, docs, DSL scripts) go through this
channel. Templates load from disk and do not ship with the package:

```toml
[[profiles.client.targets]]
format = "template"
output_dir = "build/tpl"

[profiles.client.targets.options]
template_dir = "my_templates"   # 默认 .cage/templates（相对项目根）
lang_filters = "py,go"          # 挂载语言过滤器库（见下），缺省不挂载
```

Template file conventions:

- **Output name = the template's relative path minus the `.tera` suffix** (`defs.tera` → `defs`,
  `{table}.tpl.tera` → `Item.tpl`), with subdirectory structure preserved as-is
- Templates whose file name contains the `{table}` placeholder **render once per table** (table
  name order); those without **render once globally** (template name order). Artifact order =
  template name order × table name order, byte-identical across two runs
- Missing directory, no `*.tera` in the directory, or a template syntax error → exit code 2, with
  the error pointing at the corresponding template file (Tera's cause chain fully expanded)

Template context (Schema IR serialized as-is + two supplemental keys):

| Variable | Shape | Availability |
|------|------|--------|
| `tables` | table name → table (`name` / `description` / `primary_key` / `fields` / `unique_constraints` / `order_by` / `targets`) | always present |
| `table` | current table (same shape as above) | `{table}` templates only |
| `enums` | enum name → enum definition | always present |
| `metadata` | Schema-level metadata | nullable |
| `schema_hash` | content hash (deterministic-build policy) | always present (null in the default `(unavailable)` case; pair with a `default` filter as a fallback) |

`fields` is a mapping of field name → field, containing at least `name` / `type`, with the
remaining keys present as the Schema dictates (`description` / `required` / `default` / `min` /
`max` / `min_length` / `max_length` / `pattern` / `enum_values` / `min_items` / `max_items` /
`reference` / `targets` / `rules`, etc.; the `required` key is omitted when it is `false`).

### Filters

All built-in Tera filters are available. Convention filters are always registered
(language-agnostic layer):

| Filter | Input | Output |
|--------|------|------|
| `snake_case` / `camelCase` / `PascalCase` | string | identifier normalized on word boundaries |
| `kebab_case` | string | `http-server` form |
| `SCREAMING_CASE` | string | `HTTP_SERVER` form |
| `field_order` | `fields` mapping | array sorted by field name (the emission order shared by all official languages) |

Language filters mount via `options.lang_filters` (comma-separated multiple languages, e.g.
`"py,go"`; unknown keys fail the build with exit code 2). Decisions still live in Rust — the
filters call the same mapping functions as the official generators, so the type names /
literals obtained in templates match the official artifacts:

| Filter | Input | Output |
|--------|------|------|
| `py_type` / `cs_type` / `ts_type` / `go_type` / `java_type` / `cpp_type` / `lua_type` | field object | the language's type text (enum references carry the officially assigned identifiers; `go_type` includes the pointer-optional form, `java_type` includes wrapper-class promotion) |
| `py_default` / `cs_default` / `ts_default` / `go_default` / `java_default` / `cpp_default` / `lua_default` | field object | the language literal for the field default; no default or not renderable → null |

(`lua_type` outputs the type tag on the official Lua side; `cpp_type` does not aggregate
per-file includes — that is a generator-level concern.)

```text
.cage/templates/{table}.tpl.tera   →   build/tpl/Item.tpl
────────────────────────────────────────────────────────────
table={{ table.name }} pk-snake={{ table.name | snake_case }}
{% for f in table.fields | field_order %}{{ f.name }}: {{ f | py_type }}
{% endfor %}
```

## Profile: frontend / backend

Do not hard-code "client" and "server" into Core. Build Profiles can be defined (real syntax in
[CLI: build](/cli.en), project config `cage.toml`):

```toml
[profiles.client]
name = "client"

[[profiles.client.targets]]    # JSON + CSV 数据产物
format = "json"
output_dir = "build/client/json"

[[profiles.client.targets]]    # C# 代码绑定
format = "csharp"
output_dir = "build/cs"
file_template = "{table}.cs"

[profiles.server]
name = "server"

[[profiles.server.targets]]    # 服务端语言组合
format = "python"
output_dir = "build/python"
```

Then:

```bash
cage build --profile client
cage build --profile server
```

## Field Visibility

Different targets may not need every field. For example:

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

Omitting `targets` = visible to all profiles; the `"*"` wildcard is equivalent to all.
Tables also have a table-level `targets`: when the list does not include the current
profile, the entire table is removed from that profile's view.

Client output:

```text
id
name
```

Server output:

```text
id
name
admin_note
```

This avoids:

> maintaining two sets of configuration for client and server.

One set of configuration, filtered by Profile, reused on both ends.
