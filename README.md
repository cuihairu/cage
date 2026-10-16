[English](README.md) | [中文](README.zh.md)

<div align="center">

<img src="docs/public/logo.svg" width="64" alt="Cage logo" />

# Cage

[![Rust](https://img.shields.io/badge/Rust-cargo-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/cuihairu/cage/actions/workflows/ci.yml/badge.svg)](https://github.com/cuihairu/cage/actions/workflows/ci.yml)
[![Codecov](https://codecov.io/gh/cuihairu/cage/branch/main/graph/badge.svg)](https://codecov.io/gh/cuihairu/cage)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://cuihairu.github.io/cage/)
[![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)](LICENSE)

**A configuration compiler and validation framework for game development. It transforms heterogeneous authoring data into validated, deterministic runtime artifacts.**

</div>

---

Cage parses human-maintained configuration sources (Excel / CSV / JSON / YAML / …) into a unified intermediate configuration model, then runs multi-level validation, cross-configuration reference resolution, and normalization, and finally emits ready-to-consume configuration assets or code for each target platform.

It is not bound to one input or output format: Excel is just one kind of input source. The goal is to compile scattered, heterogeneous, human-oriented configuration data into validated, deterministically built runtime configuration assets.

## Core Pipeline

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline (L0 Parse ~ L7 Game Rules)
      → Normalize / IR → Target Generators → Manifest / Hash
```

## Capabilities

- Source adapter plugins: Excel / CSV / JSON / YAML, plus the remote sources HTTP API / MySQL / PostgreSQL / Google Sheets (XML, SQLite, … later); they read and parse only, and do no business-rule validation
- Schema decoupled from Source: one schema serves multiple configuration sources (types / required / defaults / ranges / enums / uniqueness / references)
- Eight-level validation pipeline: Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule, executable level by level (`cage check --level`)
- Cross-configuration reference checks: existence plus compatibility (a reference whose target exists but has an invalid type is still an error; semantic predicate checking is reserved)
- Diagnostics as a first-class concern: error codes + precise locations (file / sheet / cell / field / value) + fix hints
- Deterministic builds: same input → same artifacts, hashes, and manifest (traceable, revertible, incremental)
- Profile mechanism: different targets and field visibility for client / server; one configuration serves both ends
- Configuration Snapshot: `cage snapshot` packs a self-verifying configuration snapshot (manifest + schema + artifacts + a per-file hash ledger); a server verifies it before loading, and tampering or added/removed files are exposed
- Configuration Registry: `cage registry` is a multi-version configuration repository (publish / list / audit / GC / remove + bundle export / import / direct push to a remote, including presigned-URL pushes via `--presign-map`); consumers reference `registry:package@version` and pin versions via `[dependencies]`, with remote http(s) root resolution (the consumer-side cache works offline, and `registry push` pushes directly to a remote)
- Schema draft from a live database: `cage schema-draft` introspects MySQL / PostgreSQL tables (read-only, bound parameters) into a reviewable schema draft — tables, primary keys, `NOT NULL → required`, with inline comments where the server's type carries a decision (DECIMAL / temporal / JSON)
- Declarative data migration: `cage migrate` applies rule transforms to validated configuration along a version-stepped chain (rename / fill defaults / drop fields / widen types / remap values); it reports a dry-run plan by default, and `--write` writes the sources back in place and re-verifies the full stack
- Target plugins: JSON / CSV data artifacts to start, extended with C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java code bindings and a Template Target (user-defined Tera templates + a language filter library), MessagePack data artifacts, Protobuf `.proto` and FlatBuffers `.fbs` definition files, and standard JSON Schema documents (draft-07 / 2020-12); Binary and others later (data serialization and code generation are separate)

## Quick Start

```bash
bash examples/run.sh   # one command through the full example project: validate → build all 14 targets → generate code in 8 languages plus proto/fbs/jsonschema definition files → E1501/E1601 bad-data diagnosis demo
```

See [`examples/game-config/`](examples/game-config/README.md) and the full-example page of the documentation site for a walkthrough. Other common commands:

```bash
cage init my-game            # new project: interactive scaffold (terminal Q&A)
cage init my-game --source csv --envs four --targets json,csv --git   # all flags, non-interactive
```

A scaffolded project runs `cage check .` and `cage build . --env dev` with zero edits. More common commands:

```bash
cage check config/                 # validate only, generate nothing
cage check config/ --env prod        # validate under a schema-declared environment (env_overrides)
cage build config/ --profile client  # validate and generate the target artifacts
cage snapshot config/ --profile client  # build + pack a self-verifying snapshot; load a snapshot directory with --verify to check it first
cage gen config/ --profile server    # generate code bindings and definition files only (cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema)
cage diff build/a build/b          # compare two configuration versions
cage inspect config/ Item          # inspect the schema and structure (project path required, table name optional)
cage migrate config/               # preview the migration plan (--write persists, --all runs the whole chain)
cage web ./                        # start the local Schema editor service (binds 127.0.0.1 only; /status/ shows project status and build freshness)
cage lsp ./                        # serve the diagnostics-only Language Server over stdio (IDE support)
cage assets config/ --write assets.json   # inventory rule/source assets with blake3 hashes; --check audits the working tree against the baseline
cage graph config/ --format dot    # print the table reference dependency graph (text/dot/json, with topo order and cycle list)
cage verify build/                 # audit generated artifacts against the manifest ledger (re-hash every file)
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0   # draft a migration rule file from two schema versions
```

## AI Assistant (MCP)

`cage mcp --stdio` exposes the check / structure / build-preview capabilities as [Model Context Protocol](https://modelcontextprotocol.io) tools, so an AI client (Claude Desktop, Claude Code, …) drives the real compiler: it reads JSON-structured results — error code + `file:line` location + actionable suggestion — and can tell you exactly what to change. Every tool is read-only: nothing is written to disk.

Add this to your MCP client config (Claude Desktop / Claude Code `claude_desktop_config.json` or a `.mcp.json`):

```json
{
  "mcpServers": {
    "cage": {
      "command": "cage",
      "args": ["mcp", "--stdio"]
    }
  }
}
```

Tool list (the `path` argument is always the configuration project root, default the current directory):

| Tool | Purpose |
| --- | --- |
| `check_schema` | Validate (same pipeline as `cage check`): error codes, `file:line` / row-column locations, reasons and suggestions (missing field / type mismatch / undeclared environment / …) |
| `list_tables` | All tables: primary keys, fields, row counts, cross-table references, declared environments |
| `list_fields` | Field definitions: type, required, default, range/length/pattern, enum, reference, profile visibility |
| `list_envs` | Declared environments (valid `--env` values) and the per-table `env_overrides` diff |
| `build_dry_run` | Build preview (nothing written): artifact list (path/size/format/table), targets, schema/source hashes and build_id |
| `explain_config` | What each `cage.toml` section means, this project's actual values, and the config precedence chain |

Example: the AI calls `check_schema` and gets

```json
{
  "code": "E1201",
  "file": "config/Item.csv",
  "where": "config/Item.csv:row 1:price",
  "message": "Value below minimum",
  "suggestion": "Minimum allowed: 0"
}
```

and can answer directly: "set `price` in row 1 of `config/Item.csv` back to ≥ 0, or loosen the schema's `min`". Running `cage mcp` without `--stdio` just prints the tool list.


## Local Test Scope

CI and local development are split three ways by change surface — a one-line doc change must not
trigger a full build.

1. **Docs only** (`*.md`, `docs/**`, images): no build. The main CI workflow's `paths-ignore`
   already skips those paths.
2. **Code**: run only the changed subset, never the full suite.
   - `cargo test -p <changed crate>`
3. **Full** (`cargo test --workspace --locked`): only for these — `Cargo.toml` / `Cargo.lock`,
   `.github/**`, Docker/Compose config, pre-release.

## One-Command Install

Download the binary over an anonymous direct link from the rolling [nightly release](https://github.com/cuihairu/cage/releases/tag/nightly) (no Actions artifacts, no token). The installer verifies SHA256 automatically; no Rust toolchain is required.

**Linux / macOS** (`sh`):

1. Run the installer (latest nightly by default):

   ```sh
   curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh | sh
   ```

   The installer adds the install directory to your PATH **by default**: it appends one `export PATH=...` line to your shell profile (`~/.zshrc` / a login profile for bash / `~/.profile`), backing the file up to `<profile>.bak-<date>` first. Re-running is idempotent — at most one line. It prints the written file and tells you to `source` it or reopen the terminal; if the write fails (missing profile, no permission) it says so explicitly with the manual line instead of staying silent. Pass `--no-path` to skip the PATH write entirely (it then prints the manual `export` line).

   Expected output (last two lines):

   ```text
   Installed to: /home/<you>/.local/bin/cage
   cage 0.1.0
   ```

2. Verify (after `source`-ing the profile or reopening the terminal):

   ```sh
   cage --version
   ```

   To roll back the PATH line: remove the `# added by cage install.sh` line from your profile, or restore the `.bak-<date>` backup the installer left next to it.

**Windows** (PowerShell 5.1+, no administrator required):

1. Run the installer (latest nightly by default):

   ```powershell
   irm https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.ps1 | iex
   ```

   Expected output (last two lines):

   ```text
   Installed to: C:\Users\<you>\AppData\Local\cage\cage.exe
   cage 0.1.0
   ```

2. The script adds the install directory to the user PATH by default (idempotent; `-NoPath` opts out and prints the manual command); reopen the terminal for it to take effect.

3. Verify:

   ```powershell
   cage --version
   ```

**Install a specific version** (latest nightly by default; a specific release tag requires that release to carry the assets for your platform):

```sh
sh <(curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh) v0.1.0   # Linux / macOS
```

```powershell
irm https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.ps1 -OutFile install.ps1; .\install.ps1 -Version v0.1.0   # Windows
```

The install directory can be overridden with the `CAGE_INSTALL_DIR` environment variable (on Windows it is fixed to `%LOCALAPPDATA%\cage`); writing the install directory into your shell profile / user PATH is the default on both platforms (`--path` (sh) is kept as an accepted alias of the default; `--no-path` / `-NoPath` opt out); `--uninstall` (sh) / `-Uninstall` (ps1) uninstalls — after the binary is removed, remove the corresponding entry from your profile / user PATH manually.

Platform coverage: Linux x86_64 (primary track), macOS aarch64 and Windows x86_64 (trial tracks); no assets for other architectures yet. The scripts ship with the `nightly` release just like the binaries themselves (`install.sh` / `install.ps1` assets); their sources live in [`scripts/`](scripts/).

## Nightly Builds

The binaries can be fetched without a Rust toolchain from the [Nightly Release](https://github.com/cuihairu/cage/releases/tag/nightly), which rolls forward with main daily (Linux x86_64 as the primary track, macOS / Windows as trial tracks). The archive contains a single `cage` file plus a `SHA256SUMS` checksum list.

## Documentation

Full design and usage documentation: **https://cuihairu.github.io/cage/**

- [Design notes](docs/design.md) · [Requirements](docs/%E9%9C%80%E6%B1%82%E6%95%B4%E7%90%86.md) · [Lifecycle](https://cuihairu.github.io/cage/lifecycle.en) · [UI prototypes](https://cuihairu.github.io/cage/design-prototypes.en) · [Roadmap](todo.md)

## Foundations

Cage is built on open-source libraries from the Rust ecosystem: Excel reading via [calamine](https://crates.io/crates/calamine), serialization via [serde](https://serde.rs) / serde_yaml / toml, command-line parsing via [clap](https://docs.rs/clap), content hashing via [blake3](https://crates.io/crates/blake3), the local HTTP service via tiny_http, the remote registry client via ureq, and CSV reading/writing via csv; the documentation site is built with [VitePress](https://vitepress.dev). The Source/Target adapters, the validation pipeline, the registry, and the rest of the project logic are implemented in this repository.

## License

[Apache-2.0](LICENSE)
