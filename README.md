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
- Target plugins: JSON / CSV data artifacts to start, extended with C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java code bindings and a Template Target (user-defined Tera templates + a language filter library), MessagePack data artifacts, Protobuf `.proto` definition files, and standard JSON Schema documents (draft-07 / 2020-12); FlatBuffers and others later (data serialization and code generation are separate)

## Quick Start

```bash
bash examples/run.sh   # one command through the full example project: validate → build 12 targets → generate code in 9 languages → E1501/E1601 bad-data diagnosis demo
```

See [`examples/game-config/`](examples/game-config/README.md) and the full-example page of the documentation site for a walkthrough. Other common commands:

```bash
cage check config/                 # validate only, generate nothing
cage check config/ --env prod        # validate under a schema-declared environment (env_overrides)
cage build config/ --profile client  # validate and generate the target artifacts
cage snapshot config/ --profile client  # build + pack a self-verifying snapshot; load a snapshot directory with --verify to check it first
cage gen config/ --profile server    # generate code bindings only (cs/python/lua/ts/js/cpp/go/java)
cage diff build/a build/b          # compare two configuration versions
cage inspect config/ Item          # inspect the schema and structure (project path required, table name optional)
cage migrate config/               # preview the migration plan (--write persists, --all runs the whole chain)
cage web ./                        # start the local Schema editor service (binds 127.0.0.1 only)
```

## One-Command Install

Download the binary over an anonymous direct link from the rolling [nightly release](https://github.com/cuihairu/cage/releases/tag/nightly) (no Actions artifacts, no token). The installer verifies SHA256 automatically; no Rust toolchain is required.

**Linux / macOS** (`sh`):

1. Run the installer (latest nightly by default):

   ```sh
   curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh | sh
   ```

   Expected output (last two lines):

   ```text
   安装完成：/home/<你>/.local/bin/cage
   cage 0.1.0
   ```

2. If the installer reports that the install directory is not on PATH, reopen the terminal, or add it manually:

   ```sh
   export PATH="$HOME/.local/bin:$PATH"
   ```

3. Verify:

   ```sh
   cage --version
   ```

**Windows** (PowerShell 5.1+, no administrator required):

1. Run the installer (latest nightly by default):

   ```powershell
   irm https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.ps1 | iex
   ```

   Expected output (last two lines):

   ```text
   安装完成：C:\Users\<你>\AppData\Local\cage\cage.exe
   cage 0.1.0
   ```

2. The script has already added the install directory to the user PATH; reopen the terminal for it to take effect.

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

The install directory can be overridden with the `CAGE_INSTALL_DIR` environment variable (on Windows it is fixed to `%LOCALAPPDATA%\cage`); `--path` (sh) / automatic (ps1) writes the install directory into your shell profile / user PATH; `--uninstall` (sh) / `-Uninstall` (ps1) uninstalls — after the binary is removed, remove the corresponding entry from your profile / user PATH manually.

Platform coverage: Linux x86_64 (primary track), macOS aarch64 and Windows x86_64 (trial tracks); no assets for other architectures yet. The scripts ship with the `nightly` release just like the binaries themselves (`install.sh` / `install.ps1` assets); their sources live in [`scripts/`](scripts/).

## Nightly Builds

The binaries can be fetched without a Rust toolchain from the [Nightly Release](https://github.com/cuihairu/cage/releases/tag/nightly), which rolls forward with main daily (Linux x86_64 as the primary track, macOS / Windows as trial tracks). The archive contains a single `cage` file plus a `SHA256SUMS` checksum list.

## Documentation

Full design and usage documentation: **https://cuihairu.github.io/cage/**

- [Design notes](docs/design.md) · [Requirements](docs/需求整理.md) · [Roadmap](todo.md)

## Foundations

Cage is built on open-source libraries from the Rust ecosystem: Excel reading via [calamine](https://crates.io/crates/calamine), serialization via [serde](https://serde.rs) / serde_yaml / toml, command-line parsing via [clap](https://docs.rs/clap), content hashing via [blake3](https://crates.io/crates/blake3), the local HTTP service via tiny_http, the remote registry client via ureq, and CSV reading/writing via csv; the documentation site is built with [VitePress](https://vitepress.dev). The Source/Target adapters, the validation pipeline, the registry, and the rest of the project logic are implemented in this repository.

## License

[Apache-2.0](LICENSE)
