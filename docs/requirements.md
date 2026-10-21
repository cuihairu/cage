[中文](/zh/requirements.md) | English

# Cage Requirements Digest (based on `docs/design.md`, Rust implementation)

> In one sentence: Cage is a configuration compiler and validator for game development — it turns heterogeneous configuration sources (Excel/CSV/JSON/YAML/…) into validated, deterministically built runtime configuration assets.
> Tech stack: **Rust** (Cargo workspace; the C++ directory layout in the design draft is mapped to multiple crates following Rust conventions).

Status as of the 2026-10 code: the MVP is fully delivered, phase two is fully delivered (2026-10 wrap-up: the Protobuf / MsgPack generators landed), phase three delivers the Web UI / Schema Editor / Registry (R1–R4); the Remote Source S series (S1 HTTP API source / S2 MySQL, PostgreSQL source / S3 Google Sheets source / S4 offline semantics / S5 error-code consolidation / S6 documentation consolidation / S7 pagination and rate-limit negotiation / S8 conditional GET / S9 database-table introspection) is fully delivered, per-environment validation `--env` (design §48, constraint override) is delivered, and the Template Target G series (Tera engine, rewritten official seven-language templates, user custom templates, filter library) is fully delivered (design §22); the Artifact distribution A series (bundle export/import + direct http(s) push, design §47) and the migration M series (declarative data migration, design §46) are fully delivered.

## Core flow

`Authoring Sources → Source Adapters → Canonical Model → Validation Pipeline（L0-L7）→ Normalize/IR → Target Generators → Manifest/Hash`

## MVP (phase one, see design draft §36) — fully delivered

### Source Adapter (reads and parses only, no business validation)
- [x] Excel (workbook/sheet/header/cell/row/columns/merged cells (.xlsx only)/cached formula values/hidden sheets skipped (decided 2026-10-12); cell comments explicitly not done (same decision))
- [x] CSV (header row; Bool/integer/float type inference during parsing, then strict type-family matching at L2 (mismatch reports E1101, no automatic conversion))
- [x] JSON / YAML
- Unified Canonical Model output: Null/Bool/Int/UInt/Float/String/Bytes/Array/Object + Source Location + metadata

### Schema (decoupled from Source, standalone DSL file)
- [x] Field types (including uint32/int64/float/string/bool/enum/array/object/map), required, default, min/max, min_length/max_length, unique, enum, primary_key, reference

### Validation Pipeline (core, executed level by level)
- [x] L0 Parse (syntax errors, E0001 family) / L1 Schema (missing field E1001, unknown field E1002) / L2 Type (E1101) / L3 Value (E1201 range/regex/enum) / L4 Table (E1301 primary key/composite uniqueness) / L5 Reference (E1401 existence, E1410 semantic predicate)
- [x] L6 Semantic (expression rules: asserts such as min_level <= max_level) / L7 Game Rule (plugin Validator trait, the built-in sample rule power_curve reports E1601)
- [x] `cage check --level N` can run a chosen level

### Diagnostics (a first-class citizen)
- [x] All fields Code/Severity/Source/Location/Table/Row/Column/Field/Value/Message/Hint; errors pinpointed to file/Sheet/cell (e.g. monster.xlsx Sheet Monster Cell G27); supports ERROR/WARNING/INFO

### Normalize
- [x] Representation-layer normalization: string trim, -0.0 → 0.0, object keys sorted via BTreeMap; the same semantics → the same Canonical Representation (cross-type coercion is the library API `coerce_to_type`, not wired into the pipeline; L2 validates strictly by type family)

### Target Generator
- [x] JSON (stable key order), CSV, MessagePack (`rmp` minimal-form binary, deterministic byte output)
- Phase-two language bindings are in the section below; Protobuf is in the §37 checklist items (.proto definition files)

### Build (determinism is a hard requirement)
- [x] The same Source+Schema+Version+Profile → the same Artifact+Hash+Manifest (no timestamps/random IDs/unstable sorting/Map ordering; golden tests lock byte equality)
- [x] Build Manifest: project/profile/cage_version/generator_version/build_id/schema_hash/source_hash/content_hash/dependencies/table_hashes/artifacts

### CLI (clap)
- [x] `cage check` (validate only) / `cage build` (validate + generate) / `cage inspect` (view Schema and structure) / `cage diff` (compare two builds)
- [x] `--profile client|server` (the Profile configures targets, and field visibility filters by targets)

### Profile and Field Visibility
- [x] The profile config file defines targets; field-level `targets: [server]` controls visibility (avoiding separate client/server configurations); structurally required fields hidden by the profile report E9006

## Phase two (see §37)

- [x] Code-binding generators: C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java
- [x] Template Target (G series, design §22): the Tera engine is wired in, the official seven-language templates are rewritten (byte-for-byte identical output), `format = "template"` user custom templates (conventional location `.cage/templates/`), and a language filter library (`options.lang_filters`)
- [x] MsgPack generator (decided 2026-10: a data-type target, Canonical Model →
      MessagePack binary, `rmp` minimal form with byte-for-byte determinism;
      `format = msgpack`/`messagepack`, the only option is `sort_keys`;
      non-finite floats are encoded as nil, matching the JSON target's
      non-finite → null rule, and Bytes go out as native bin without base64)
- [x] Protobuf generator (decided 2026-10: design draft §23 has no explicit wire-format
      statement, so it landed as emitting .proto3 definition files — schema-driven and in
      the same family as the C#/Go code bindings; it enters the build/gen code surface via
      `format = proto`/`protobuf`; the type-mapping table is self-defined and merged into
      `docs/target.md`: narrow integers collapse into int32/uint32, object/null/any go through
      google.protobuf.Struct/Value, int map keys → int64, arrays/maps wrap the element
      or value in a nested `{字段}Value` message, integer enums go into the shared
      cage_enums.proto (missing 0 prefix gets a synthesized `_UNSPECIFIED`, duplicate
      values allow_alias, domains beyond int32 or members containing strings fall back to
      string))
- [x] JSON Schema generator (2026-10: standard JSON Schema documents, entering the
      build/gen code surface via `format = jsonschema`; each table is self-contained in
      `{table}.schema.json`, draft-07 by default / 2020-12 via `options.draft`, enums
      inline member names (an instance is always a member name), closed objects,
      deterministic bytes; zero runtime consumption by non-cage toolchains, see the
      `docs/design.md` §Code Targets decision record)
- [x] Plugin surface: the SourceAdapter / TargetGenerator / GameRuleValidator traits (registered in-process)
- [x] Game Rule Validator implemented (E1601, `cage check --level gamerule`)
- [x] Dependency Graph (reference graph → build order/change impact/cycle detection, E1403 wired into L5)
- [x] Incremental builds (`cage build --incremental`: three layers — skip the whole run by hash / rebuild only affected tables through dependency propagation / propagate target configuration)
- [x] CI integration (a top-level `warnings_as_errors` switch + CI gating in the repo itself)

## Phase three (see §38)

- [x] Web UI (`cage web` local service, bound to 127.0.0.1 only)
- [x] Schema Editor (W1–W4: exchange model / local service / editor frontend / samples and smoke)
- [x] Registry (R1–R4: local multi-version registry, the `registry:` source with schema resolution, `[dependencies]` version pinning, remote http(s) read-only resolution, verify/gc/remove)
- [x] Remote Source (Google Sheets/DB/API) — started (2026-10): the four-source shape
      was finalized in design §45, the S1 HTTP API source (`cage-source-http`),
      the S2 MySQL / PostgreSQL source (`cage-source-db`) and the S3 Google Sheets
      source (`cage-source-sheets`) are delivered, S4 offline semantics is consolidated
      (transport failures fall back to a cached copy + an E1906 WARNING, with a strict
      `--no-cache` mode), the S5 error-code wiring is delivered (the reserved E19xx family
      is cleared out and the inventory covering all four sources is on record),
      and S6 documentation is delivered (the follow-up extension list in `docs/source.md` was
      promoted, the remote-source chapter in `docs/cli.md`, the Remote Source checkbox in
      `docs/需求整理.md`, and the three crates already appear in the project structure tree
      in `docs/architecture.md`)
- [x] Migration — delivered (decided 2026-10, design §46: the M series of declarative data
      migration, explicit rule transforms at the Canonical Model layer plus re-validation,
      the E20xx family; M1 rule model / M2 executor / M3 CLI consolidation all delivered,
      `cage migrate` dry-run/--write/--all/--to, local text sources written back in place,
      Excel always report-only, see the M series in `todo.md`)
- [x] Artifact distribution — delivered (decided 2026-10, design §47: the A series of
      registry distribution — offline bundle export/import + direct http(s) push
      (auth_env credentials), the E21xx family; A1 export / A2 import / A3 push /
      A4 documentation consolidation / A5 compressed containers / A6 signature ledger /
      A7 the three §47 closures (incremental delta rejection, `--presign-map` presigned
      push implemented, pull-through proxy rejection) all delivered, see the A series in
      `todo.md`)
- [x] Per-environment validation — delivered (decided 2026-10, design §48: constraint
      override direction, table-level `env_overrides` environment constraint patches,
      `--env` selects the environment for validation/build/generation, the manifest records
      `environment` and incremental builds invalidate on an environment mismatch, see the
      per-environment validation section in `todo.md`)

## Red lines (design draft §39-40)
- No Excel editor, no game runtime database, no Secret Manager, no game logic framework; no binding to Unity/Unreal/Godot/Cocos
- Source Adapters do no business validation; Transform does not re-validate; Validator plugins do not execute untrusted code by default (CI sandbox is a later consideration)

## Rust implementation baseline
- Cargo workspace: `cage-core` (value/document/schema/diagnostics/validation/reference/normalize/manifest/snapshot/edit/registry/remote), `cage-source-{excel,csv,json,yaml,http,db,sheets,msgpack}`, `cage-target-{json,csv,msgpack,cs,py,lua,ts,cpp,go,java,proto,jsonschema,template}`, `cage-cli`
- Determinism: BTreeMap/stable sorting, byte-for-byte reproducible output (golden tests)
- Test gating: all tests must be green before commit/push (no force push, no version tags/releases)
