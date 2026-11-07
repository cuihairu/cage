# Source Layer Analysis

This page abstracts the implementation into nine responsibility layers. It is
anchored to the source tree, not to directory names. Chinese mirror:
[简体中文源码分层分析](./zh/source-layer-analysis.md).

The workspace itself names the implementation slices as core, input adapters,
output adapters, and the binary application: `Cargo.toml:3-27`. Source and
target adapters depend on core, while only the CLI binary assembles all of
them (for example, Excel and JSON adapters in `crates/cage-cli/Cargo.toml:20-40`).

## Layer Map

| Layer | Responsibility | Representative entry |
| --- | --- | --- |
| Application Interfaces | Accept CLI/HTTP/MCP/LSP requests and drive shared pipelines | `crates/cage-cli/src/main.rs:575` |
| Project Loading | Read project config, resolve schemas/sources, produce the compiler input | `crates/cage-cli/src/main.rs:787` |
| Core Model | Represent schema and canonical data independently of any file format | `crates/cage-core/src/schema/mod.rs:9`, `crates/cage-core/src/value/mod.rs:134` |
| Normalization | Materialize defaults, split cells, coerce values to declared types | `crates/cage-core/src/normalize/mod.rs:116`, `crates/cage-core/src/normalize/mod.rs:232` |
| Validation | Run L0-L7 checks and return locatable diagnostics | `crates/cage-core/src/validation/mod.rs:168` |
| Build & Distribution | Hash inputs, incrementally regenerate, pack/verify, and publish artifacts | `crates/cage-cli/src/main.rs:2184`, `crates/cage-core/src/snapshot/mod.rs:83` |
| Target Generation | Turn schema/document into data, bindings, contracts, or templates | `crates/cage-cli/src/main.rs:3727` |
| Source Ingestion | Parse external files/tables/APIs into canonical documents | `crates/cage-cli/src/main.rs:1037`, `crates/cage-source-excel/src/lib.rs:54` |
| Governance & Diagnostics | Track assets, migrate schema/data, and normalize error reporting | `crates/cage-core/src/assets/mod.rs:86`, `crates/cage-core/src/diagnostics/mod.rs:13` |

## Application Interfaces

`main` parses CLI arguments and dispatches commands (`crates/cage-cli/src/main.rs:575`).
Four protocol surfaces share the compiler core rather than reimplementing it:

- CLI commands call `build_project`, `run_gen`, migration, snapshot, registry,
  asset, and verify flows. `build` is the broadest route (`crates/cage-cli/src/main.rs:2184`).
- HTTP serves editor/status APIs on loopback with a single-threaded request
  loop and embedded UI assets (`crates/cage-cli/src/web.rs:61-77`, `crates/cage-cli/src/web.rs:49-58`).
- MCP is a stateless rmcp tool router over stdio; each tool reuses project
  loading, validation, or target planning (`crates/cage-cli/src/mcp.rs:130-240`).
- LSP has no persistent compiler state: framed JSON-RPC requests trigger full
  project validation and publish diagnostics (`crates/cage-cli/src/lsp.rs:97-230`).

There is no Redis or MySQL runtime cache here. The HTTP context keeps only
project root and deserialized config (`crates/cage-cli/src/web.rs:84-87`);
MCP has explicitly stateless tools (`crates/cage-cli/src/mcp.rs:130-135`);
LSP holds root, env/level, and open-document URIs (`crates/cage-cli/src/lsp.rs:97-103`).

## Project Loading

`load_project_impl` resolves one config file, fills environment DSN/credential
names, loads the schema, and merges each logical source root. Registry roots
go through ledger verification before their `data/` directory is selected as a
source (`crates/cage-cli/src/main.rs:1133-1192`). Local and remote roots are
dispatched through the CLI-owned loader, while registry resolution is core
logic (`crates/cage-cli/src/main.rs:1020-1065`).

`cage.toml` is the top-level configuration schema: project identity, profiles,
source roots, schema path, output, registry, dependency pins, remote source
environment-variable names, and format conventions (`crates/cage-core/src/manifest/mod.rs:523-632`).
Remote configuration deliberately stores names, not credentials: `dsn_env`,
`credential_env`, and named read-only queries are declared in config while
secrets resolve at load time (`crates/cage-core/src/manifest/mod.rs:610-631`).

## Core Model

`Schema` holds ordered table definitions; `Value` is the canonical data shape;
`Document` aggregates insertion-ordered tables and source metadata
(`crates/cage-core/src/schema/mod.rs:9-132`, `crates/cage-core/src/value/mod.rs:134-364`).
Core re-exports these as the public compiler boundary (`crates/cage-core/src/lib.rs:57-80`).
The model has no process-wide singleton state. Lifetime is per command or
request; inputs arrive from loading and outputs are immutable borrow inputs to
normalization, validation, and generation.

The schema editor boundary is also a model conversion, not a separate UI
model: editor JSON and canonical YAML both map through core functions
(`crates/cage-core/src/edit/mod.rs:35-90`).

## Normalization

Normalization happens after loading and before validation/generation. It fills
field defaults (`crates/cage-core/src/normalize/mod.rs:116`), splits arrays
encoded in one cell (`crates/cage-core/src/normalize/mod.rs:160`), and coerces
`Value` instances to field types (`crates/cage-core/src/normalize/mod.rs:232`).
It is stateless: each function maps a value/document to another value/document.
CLI build calls it immediately after profile validation (`crates/cage-cli/src/main.rs:2226-2240`).

## Validation

`validate_with_profile` builds a primary-key/reference cache and executes
levels in L0-L7 order, then optionally escalates warnings and sorts diagnostics
(`crates/cage-core/src/validation/mod.rs:168-227`). Levels are an ordered enum
(`crates/cage-core/src/validation/mod.rs:22-89`). Game-rule validators are
invoked as L7 from the same pipeline. Validation stores no durable state; its
cache lives only inside `ValidationContext`.

CLI/MCP/LSP all route to this entry rather than re-decoding diagnostics:
build uses it (`crates/cage-cli/src/main.rs:2226`), MCP tool bodies call
`validate_project` (`crates/cage-cli/src/mcp.rs:270-420`), and LSP maps its
result to editor diagnostics (`crates/cage-cli/src/lsp.rs:109-177`).

## Build & Distribution

The CLI build orchestrates the lifecycle: validate full data, normalize,
compute schema/source hashes, compare the prior manifest, regenerate only
affected tables/targets, write artifacts, and write a deterministic manifest
(`crates/cage-cli/src/main.rs:2184-2520`). Incremental state is the prior
`manifest.json` on disk plus artifact files; safe-path checks reject unsafe
prior records (`crates/cage-cli/src/main.rs:2300-2355`). Snapshots package the
manifest/schema/artifact set with a BLAKE3 ledger and verify it again on load
(`crates/cage-core/src/snapshot/mod.rs:83-206`). The registry is a directory
of self-verifying snapshots plus per-package `index.json`; it resolves only
ledger-verified entries (`crates/cage-core/src/registry.rs:6-31`,
`crates/cage-core/src/registry.rs:1526-1569`).

Remote distribution and remote reads use one bounded HTTP/retry policy. Cache
keys/directories, GET/conditional GET, and PUT live in core
(`crates/cage-core/src/remote.rs:35-310`, `crates/cage-core/src/remote.rs:524-547`).
Registry push uploads entry files and then a merged index via PUT
(`crates/cage-core/src/registry.rs:1078-1330`).

Configuration inputs are the profile/target list plus registry auth-env name;
artifacts and manifests are the durable state. No daemon holds build state in
memory between requests.

## Target Generation

`generate_target_items` is the shared in-memory dispatch used by build and MCP
dry runs (`crates/cage-cli/src/main.rs:3727-3768`). Data formats are JSON, CSV,
and MessagePack; language/contract/template generators are schema-driven
(`crates/cage-cli/src/main.rs:3637-3725`). Each target is an independent crate
and constructs its generator from `TargetConfig`
(`crates/cage-core/src/manifest/mod.rs:508-521`). Target state is only the
project's Tera template directory for the template format; all generated bytes
return through the CLI and are written by one artifact writer.

## Source Ingestion

Every local parser produces `Document` through a source adapter. Excel uses
calamine and convention metadata (`crates/cage-source-excel/src/lib.rs:54`,
`crates/cage-source-excel/src/lib.rs:1284`); CSV/JSON/YAML/MessagePack are
format-specific parser crates. HTTP, database, and Sheets adapters fetch
remote bytes/rows and reuse the JSON adapter for canonical caching/conversion
(`crates/cage-source-http/src/lib.rs:133`, `crates/cage-source-db/src/lib.rs:252-360`,
`crates/cage-source-sheets/src/lib.rs:298-330`).

Remote state is a project-local cache path derived from a deterministic
source key (`crates/cage-core/src/remote.rs:35-60`). Databases are read-only
query/introspection inputs, not persistent compiler state. The
`RemoteSourceConfig` schema carries only environment-variable names and named
read-only queries (`crates/cage-core/src/manifest/mod.rs:610-631`).

## Governance & Diagnostics

Asset governance inventories rules, migrations, and local source roots; BLAKE3
hashes become a deterministic baseline and verification report
(`crates/cage-core/src/assets/mod.rs:26-177`). Migration parses a declarative
spec, applies ordered steps, revalidates the document, and reports per-step
results (`crates/cage-core/src/migrate/mod.rs:35-90`,
`crates/cage-core/src/migrate/mod.rs:513-880`). Schema diffing derives migration
drafts (`crates/cage-core/src/migrate/diff.rs:88-220`).

Diagnostics are first-class pipeline results carrying code, source location,
table/field context, severity, and rendering. The type lives in core
(`crates/cage-core/src/diagnostics/mod.rs:13-110`) and every interface layer
renders or maps the same object. Error-code tables are the stable文案 source:
see `crates/cage-core/src/error/codes.rs`.

## Configuration, State, and Lifecycles

| Concern | Input | State | Channel |
| --- | --- | --- | --- |
| CLI | argv, `cage.toml`, schemas, sources, registry pins | output/manifest/snapshot/registry files | process → filesystem |
| Web | HTTP request + `cage.toml`, editor JSON, uploaded bytes | project files, prior manifest/artifacts | browser fetch ↔ loopback HTTP |
| MCP | JSON-RPC tool arguments + project files | none beyond loaded command state | stdio MCP |
| LSP | initialize options + project files | open-document URI set only | stdio LSP framing |
| Sources | files, SQL result sets, HTTP payloads, Sheets rows | project-local remote cache / canonical cache JSON | ureq HTTP, MySQL/Postgres protocol, filesystem |
| Registry | snapshot bytes, dependency pins, bearer auth-env name | package directories, `index.json`, `HASHES.json` | filesystem plus optional HTTP PUT |

CLI processes end after one dispatch (`crates/cage-cli/src/main.rs:575-778`);
Web loops until killed (`crates/cage-cli/src/web.rs:61-79`); MCP ends when the
stdio client disconnects (`crates/cage-cli/src/mcp.rs:215-242`); LSP exits on
EOF or after shutdown/exit (`crates/cage-cli/src/lsp.rs:199-278`).

## Related Pages

- [Architecture](./architecture.md) explains the conceptual pipeline.
- [Glossary](./glossary.md) defines the terms used by each layer.
- [Source](./source.md), [Schema](./schema.md), [Validation](./validation.md),
  [Target](./target.md), [Build](./build.md), [CLI](./cli.md), and
  [Web](./web.md) document each concern in depth.
