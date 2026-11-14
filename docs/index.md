---
layout: home

hero:
  name: Cage
  text: A configuration compiler and validator for games
  tagline: Reliably compile scattered, heterogeneous, human-authored configuration data into validated, deterministically built runtime configuration assets
  actions:
    - theme: brand
      text: Start with the architecture
      link: /architecture
    - theme: alt
      text: 中文
      link: /zh/
    - theme: alt
      text: CLI at a glance
      link: /cli
    - theme: alt
      text: GitHub
      link: https://github.com/cuihairu/cage

features:
  - title: Source Adapter plugins
    details: Excel / CSV / JSON / YAML / MessagePack plus HTTP API / MySQL / PostgreSQL / Google Sheets remote sources. They only read and parse, and never perform business validation; XML and SQLite come later.
  - title: Schema decoupled from Source
    details: One Schema serves many configuration sources — types, required fields, defaults, ranges, enums, uniqueness and references are each defined independently.
  - title: Eight-level validation pipeline
    details: Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule run level by level; cage check --level validates by level.
  - title: Cross-configuration reference validation
    details: Existence plus compatibility checks — a referenced object that exists but has an invalid type is an error too; reference predicates are evaluated semantically (E1410 when unsatisfied).
  - title: Diagnostics as a first-class citizen
    details: Error codes plus precise locations (file / Sheet / cell / field / value) plus fix hints, consumed directly by the CLI, the Web UI and the IDE (`cage lsp`).
  - title: Deterministic builds
    details: Same input → same artifacts, hashes and Manifest. Traceable, rollback-friendly and incremental.
  - title: Profile mechanism
    details: client / server profiles configure different targets and field visibility, so one configuration is reused on both ends instead of maintaining two sets of tables.
  - title: Target plugins
    details: Starting from JSON / CSV data artifacts, extended to MessagePack (deterministic binary), Protobuf `.proto`, FlatBuffers `.fbs` and standard JSON Schema (draft-07 / 2020-12) definition files, plus C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java code bindings and a Template Target (custom Tera templates); Binary / SQLite and the rest come later. Data serialization and code generation are kept separate.
  - title: Declarative data migration
    details: cage migrate steps through a versioned chain applying rule transforms (rename / fill defaults / drop fields / widen types / remap values) to validated configuration; it reports as a dry run by default, --write writes back in place and re-validates the whole stack.
---




[中文](/zh/) | English

![Cage configuration compiler and validator architecture](/images/architecture-en.svg)

## Core flow

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline（L0 Parse ~ L7 Game Rules）
      → Normalize / IR → Target Generators → Manifest / Hash
```

Excel is only one kind of input source for Cage. Every input enters the unified Canonical Model, and once validation passes, deterministic assets are produced for the target platform — beyond the conversion itself, validation, references and normalization all happen on this same chain.

## Interface previews

`cage init` — terminal question-and-answer wizard for new-project scaffolding (9 questions, one answer per line, back/defaults supported):

![cage init interactive wizard](/design/init-wizard.png)

`cage check` — validation result view: error list with error code, source location and fix suggestion:

![cage check validation result (error list)](/design/validation-result.png)

`cage check` — environment view: field-level constraint comparison across the dev/test/staging/prod `env_overrides` sets:

![cage check validation result (environment overrides)](/design/validation-result-env.png)

`cage check` — structure tree view: schema and field hierarchy with types, constraints and profile visibility:

![cage check validation result (structure tree)](/design/validation-result-tree.png)

## Quick start

```bash
cage init my-game                    # new project: interactive scaffold (terminal Q&A); check/build pass with zero edits
cd my-game                          # run inside the project root (where cage.toml lives)
cage check .                   # validate only, generate nothing
cage build . --profile client  # validate and generate the target artifacts
cage gen . --profile server    # code bindings and definition files only (cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema/template)
cage diff build/a build/b            # compare two configuration versions
cage graph .                   # table reference dependency graph (text/dot/json, topo order and cycle list)
cage verify build/                   # audit the generated artifacts (re-hash every file against the ledger)
cage inspect . Item            # schema and configuration structure (project path required, table name optional)
cage assets . --write assets.json   # asset inventory baseline (--check reports missing / tampered / extra)
cage snapshot . --profile client    # build + pack a self-verifying snapshot (--verify before loading, --list lists packages)
cage registry publish . --registry reg/        # multi-version registry (publish / list / verify / GC / remove / export / import / push)
cage migrate .                 # preview the migration plan (dry-run; --write persists and re-verifies, --all runs the whole chain)
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0   # draft a migration rule file from two schema versions
cage schema-draft . mysql:Items      # draft a schema by introspecting tables read-only (MySQL / PostgreSQL)
cage web ./                          # local Schema editor and project status page (binds 127.0.0.1 only)
cage lsp ./                          # stdio Language Server (IDE diagnostics, same pipeline as check)
cage mcp --stdio                     # AI assistant MCP server (validation / structure / build preview as read-only tools)
```

## Documentation sections

- Assets: three kinds of input assets — Schema / source data / build rules — plus asset management with artifact traceability (`docs/design.md §50`: the `cage assets` asset manifest and baseline re-check, four layers of backup defense, snapshot and registry versioning capabilities; [`examples/asset-workspace`](https://github.com/cuihairu/cage/tree/main/examples/asset-workspace) demonstrates the full lifecycle)
