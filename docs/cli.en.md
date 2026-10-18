
# CLI

Cage's command-line entry point (the `cage-cli` crate, built on [clap](https://crates.io/crates/clap)):

```bash
cage init
cage mcp
cage check
cage build
cage gen
cage inspect
cage assets
cage diff
cage graph
cage verify
cage snapshot
cage web
cage lsp
cage registry
cage migrate
cage migrate-draft
cage schema-draft
```

The first four landed in the MVP (`check` / `build` / `inspect` / `diff`); `gen` / `snapshot`
are phase two (both implemented); `web` is phase three (the Schema editor local service);
`registry` is the phase-three R series (local Configuration Registry publish/list,
R1–R4 all implemented); A-series distribution (bundle export/import, direct push to
remote, design §47) landed with A1–A3; M-series migration (`cage migrate` execution +
`cage migrate-draft` drafting, design §46) landed with M1–M3; `cage schema-draft`
(schema drafting from database-table introspection, design §45 / S9) is implemented;
`cage graph` (reading the dependency graph) is implemented; see [verify](#verify)
for `cage verify runtime/` (verifying generated artifacts), [init](#init) for
`cage init` (interactive scaffolding), and [mcp](#mcp) for `cage mcp` (the
AI-assistant MCP server).

## mcp

> **Lifecycle**: cross-cutting · ②④⑤ (AI-driven validation/generation/dry-run). See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage mcp            # 打印工具清单（人看的）
cage mcp --stdio    # 以 stdio JSON-RPC 起 MCP server（给 MCP 客户端）
```

AI assistant (MCP): `cage mcp --stdio` uses the official [rmcp](https://crates.io/crates/rmcp)
SDK to wrap the existing check / structure / build dry-run capabilities as
[Model Context Protocol](https://modelcontextprotocol.io)
tools; an AI client (Claude Desktop, Claude Code, other MCP clients) can call them
after configuring one snippet:
`{"mcpServers": {"cage": {"command": "cage", "args": ["mcp", "--stdio"]}}}`.
Output is always structured JSON, and every tool is **read-only** — it writes nothing
to disk and modifies no configuration:

- **check_schema**: the same pipeline as `cage check` (identical
  level/profile/env/--no-cache semantics); returns `{ok, error_count, errors: [{code,
  file, where, line, row, column, table, field, message, suggestion, …}], warnings,
  next}` — error code + location + cause + actionable suggestions (missing field /
  type mismatch / undeclared environment / nonexistent template variable, …); `where`
  is a compound location (text sources use `file:line`; sources without line numbers
  use `file:row N:col`).
- **list_tables / list_fields**: schema structure (tables/fields/primary keys/row
  counts/enums/references/profile visibility).
- **list_envs**: declared environments (the legal values for `--env`) and each table's
  `env_overrides` differences; how to fix an unknown environment is also spelled out
  in the suggestions.
- **build_dry_run**: runs the same generation chain as `cage build`
  (`generate_target_items`) to compute the artifact list and all hashes, but **writes
  nothing to disk** (`written: false`); on validation failure it returns structured
  errors the same way.
- **explain_config**: the semantics of each `cage.toml` section + the project's actual
  values + the configuration precedence chain (discovery order → level → profile →
  env → source merge → artifacts → incremental).

See the integration tests in `crates/cage-cli/tests/mcp_server.rs`: they start a real
server and walk initialize → tools/list → six tools/call calls (including one
deliberately miswritten configuration, asserting the error carries file and
suggestion). The Chinese and English READMEs' "AI Assistant (MCP)" sections carry the
client configuration snippet and the tool list.

## init

> **Lifecycle**: before ① · project scaffolding. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage init my-game                                   # 终端里交互式问答生成项目骨架
cage init my-game --source csv --envs four \
  --targets json,csv --template game --git          # 全 flag 非交互（CI/脚本友好）
cage init my-game --no-examples                     # 只要空骨架（不生成示例表与样例数据）
```

`cage init` generates a ready-to-use configuration project: `cage.toml` (same
structure as `examples/game-config/cage.toml` — no second configuration format
introduced) + the `schemas/item.yaml` example table + `config/Item.*` example data +
`README.md` + `.gitignore`. With **zero changes**, the generated project passes
`cage check .` and `cage build . --env dev`.

- **Interactive mode**: with a TTY and not a single flag passed, it enters a Q&A
  flow — project directory → project name → configuration source type
  (excel/csv/json/yaml/sheets/db/http) → environment set (`four` = the four
  dev/test/staging/prod sets, built in) → target generators (listed by the
  `cage-target-*` crates actually present in the repo:
  json/csv/msgpack/proto/flatbuffers/csharp/python/lua/typescript/javascript/cpp/go/java/jsonschema/template)
  → template style → whether to `git init` → whether to include example tables. Each
  question takes its default on Enter, `b` steps back to the previous question, and
  before finishing it prints the list of files to generate and asks for confirmation
  (choosing `n` writes no files).
- **Non-interactive mode**: pass any flag (or run with stdin not a TTY, e.g. in
  CI/pipelines) and it generates directly from flags and defaults — it never blocks
  on a prompt. Unknown values for `--source` / `--envs` / `--targets` / `--template`
  → exit 2.
- **No overwriting**: if the directory already contains any of `cage.toml` /
  `schemas/` / `config/` / `README.md` / `.gitignore`, it refuses (exit 1), lists the
  conflicting items, and touches not a single file.
- The `excel` / `sheets` / `db` / `http` source types have no zero-dependency
  samples, so the local text sample data (`config/Item.csv`) is kept and a
  commented-out remote-source block is left in `cage.toml`; the project still passes
  validation out of the box — un-comment it to switch sources.
- `--envs four` writes `env_overrides` into `schemas/item.yaml` (a price cap of
  dev 1000 → test 800 → staging 600 → prod 500), and validation runs for all four
  environment sets; `--envs none` declares no environments at all.

## check

> **Lifecycle**: ② · validation. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage check .
```

Validates only, without generating Runtime Artifacts. Can run per level:

```bash
cage check --level schema       # 只做 Schema 层
cage check --level reference    # 只做到引用层
cage check --profile client     # 按 Profile 验证
cage check --env prod           # 按 Schema 的 prod 环境约束验证
```

`--level` values: parse / schema / type / value / table / reference /
semantic / gamerule (default `semantic`, i.e. it executes through L6 by default;
including L7 business rules requires an explicit `--level gamerule`). `--profile`
defaults to `client` (same for check / build / gen). `--env` (same for check /
build / gen) validates after applying the constraint overrides declared by the
Schema's `env_overrides` for that environment — an undeclared environment name is a
usage error (exit 2); see [Schema per-environment constraint overrides](/en/schema).
Even without `--env`, bad overrides are guarded: structurally bad overrides in any
declared environment (pointing at a nonexistent field / empty patch) are reported
one by one as `warning:` at load time (without blocking); with `--env`, that same
defect is a hard error (exit 2).

## build

> **Lifecycle**: ④⑤ · target generation + incremental. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage build . --profile client
cage build . --profile server
cage build . --env prod          # 按环境约束验证并生成
cage build . --incremental       # 哈希与上次 manifest 一致时跳过重新生成
```

`--level`, with the same semantics as check (default semantic), can raise or lower
the validation level.

Validates and generates target artifacts, and outputs the [Build Manifest](/en/build)
at the same time. `--incremental` performs three-layer incrementality based on the
schema/source/target fingerprints in the manifest (validation still runs in full; a
schema change falls back to a full build, and a target configuration change only
regenerates the corresponding target — see [incremental build](/en/build) for
details).

## gen

> **Lifecycle**: ④ · code binding generation. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage gen . --profile server
cage gen . --env prod    # 按环境约束生成
```

Generates only code-class artifacts ([C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java / Protobuf](/en/target),
plus the `template` user-defined Tera template target), and runs no data validation:
code generation is Schema-driven — types and metadata all come from the Schema, and
no configuration row data is consumed (source files still must load and parse
normally). Data-class targets in the Profile (json/csv/msgpack) are skipped; use
`cage build` when you need data artifacts. Artifacts are likewise written into the
Build Manifest (same rules as build), last writer wins.

## inspect

> **Lifecycle**: cross-cutting · ② (structure inspection). See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage inspect . Item     # 项目路径必填；表名可选，省略时列出全部表
```

Inspect the Schema and configuration structure.

## assets

> **Lifecycle**: cross-cutting · asset baseline. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage assets <project>                       # 文本清单：路径 / 类别 / blake3 / 大小
cage assets <project> --json                # 同形 JSON（基线即此形态）
cage assets <project> --write assets.json   # 基线落盘（随仓入库）
cage assets <project> --check assets.json   # 对基线复核工作树
```

The asset manifest (design §50; see [Assets · classification and backup strategy](/en/assets)
for details): takes the project's build rules (`cage.toml`), rule assets (the
`schema_path` file set + `migrations/`), and source data assets (the
`[source_roots]` file set), inventories each file's blake3 hash + size, and sorts
deterministically by path — the same tree always yields the same manifest. Remote
specs (`registry:` / `gsheet:` / `mysql:` / `pg:` / `http(s)://`) are not
inventoried, and rebuildable directories such as `build/` and `.cage-cache/` never
enter the ledger.

`--check` re-verifies three classes of problems and fails with exit code 1:
`missing` (in the baseline, not in the tree), `changed` (same path, different hash
or size), and `unlisted` (in the tree, not in the baseline); a baseline project-name
mismatch or an unparseable file yields exit code 2. The three modes are mutually
exclusive (clap conflict rules).

## `--no-cache`: Offline Fallback Toggle for Remote Sources

```bash
cage build . --no-cache    # check / build / gen / inspect 通用
```

All four commands `check` / `build` / `gen` / `inspect` take `--no-cache`: it
disables the offline fallback for remote sources (HTTP / MySQL / PostgreSQL /
Google Sheets, see [source](/en/source)) — by default a transport-class fetch
failure falls back to the last cached copy and emits an `E1906` WARNING; under
`--no-cache`, failing to reach the remote is a failure (`E1901`, exit code 2), even
if a cached copy exists. This suits the pre-release check of "must build from the
remote's latest bytes". Strict does not mean no caching at all: HTTP sources default
to conditional GET (ETag/Last-Modified, see [source](/en/source)), and under
`--no-cache` conditional requests are still sent — a 304 is the server confirming
the cache, not the cache answering itself.

## schema-draft

> **Lifecycle**: before ① · schema drafting from database-table introspection. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage schema-draft . mysql:Items pg:public.Monsters   # 内省库表 → stdout
cage schema-draft . mysql:Items -o schemas/draft.yaml
```

Generates a schema draft from live database table shapes (design §45): each
`mysql:<table>` / `pg:<table>` spec introspects `information_schema` over a read-only
session (the table name is bound as a parameter) and renders tables, primary keys,
and `NOT NULL → required`; DECIMAL / date-time / JSON / unknown types are flagged
with inline comments as semantics requiring a decision (min / max / pattern /
enum_values / references still belong to the author). The type mapping matches the
source adapters (DECIMAL / temporal / JSON → `String`, i.e. the actual shape of the
cached JSON). Credential discipline matches the remote sources: the DSN is read only
from the environment variable named by `[remote.<scheme>].dsn_env` (`E1904`); a
table not found is `E1901`; a non-db spec is `E1905`. Rendering is deterministic
(no timestamps); `-o` parent directories are created automatically. A draft must be
human-reviewed before it can serve as `schema_path`.

## diff

> **Lifecycle**: cross-cutting · version comparison. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage diff build/a build/b
```

Compares two configuration versions; the two positional arguments also accept
manifest.json file paths (not limited to build directories). After the file list
comes a two-line layered summary — `tables changed` (tables hit by the per-table
`table_hashes` comparison) and `targets affected` (the `output_dir` of affected
files) — sharing its data source with the status page's diff section.

## snapshot

> **Lifecycle**: ⑥ · self-verifying snapshot. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage snapshot <project> --profile client    # 构建 + 打包自校验快照
cage snapshot <project> --env prod          # 环境化出包（env_overrides 先解析再打包）
cage snapshot <snapshot-dir> --verify       # 载入前校验（服务器入口）
cage snapshot <project> --list              # 列已打包快照并逐包自验
```

Packs a [Configuration Snapshot](/en/build): the profile's build artifacts are
packed into a directory that loads standalone and carries its own blake3 ledger,
`<output_dir>/snapshot/<profile>[-<env>]-<build_id[..12]>` (manifest.json /
schema.json / data/ / generated/ / HASHES.json), and self-verifies after building.
`--verify` performs the pre-load check on an existing snapshot directory: every file
is re-hashed and compared; tampering or added/removed files are listed one by one
and it fails with exit code 1. `--profile` defaults to `client`. `--env` produces an
environment-scoped package: the schema's `env_overrides` are resolved into the
schema first, then validation and packing run; the directory name carries the env
segment, the package's `manifest.json` records `environment`, and `schema.json` is
exactly that environment's rules; `--env` and `--verify` are mutually exclusive
(re-verification ignores the environment). `--list` lists every package under
`<output_dir>/snapshot/` (profile / env / build_id / content_hash / artifact
count), self-verifying each package on the spot and printing a verdict — the
listing never claims a package it did not just verify is intact; it is mutually
exclusive with `--verify` / `--env`.

## verify

> **Lifecycle**: ⑥ · artifact verification. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage verify <project-root>         # 审计 output_dir 的构建产物
cage verify build/                 # 直接审计构建输出目录
cage verify build/manifest.json    # 或 manifest 文件本身
```

Audits generated artifacts: every file in the manifest ledger has its blake3
recomputed and its size checked — missing, modified, or byte-count-mismatched files
are each reported `FAILED` with exit code 1; a clean tree yields 0. Manifest keys
are project-relative paths; inside a build tree they resolve against the project
root, and when given an output directory directly, the deployment layout with the
first segment (`build/`) stripped is also attempted. **Ledger keys only are read** —
files outside the ledger are never touched, the same posture as `/api/artifact`.
Snapshot packages have their own load-side check:
[`cage snapshot <dir> --verify`](#snapshot).

## graph

> **Lifecycle**: cross-cutting · ③ (dependency graph). See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage graph <project>               # 文本视图（默认）
cage graph <project> --format dot  # Graphviz DOT
cage graph <project> --format json # 机器可读
```

Outputs the [configuration dependency graph](/en/build): schema reference
declarations form the graph via `DependencyGraph::from_schema` — the same entry
point as the reference check (L5) and incremental planning (the second layer of
`build --incremental`). Edges carry the referencing-field label (in schema order),
and the text view appends the topological build order and a cycle list; **cycles are
reported but not fatal** — the reference check is what errors; this command is a
view. The graph is schema-driven: it reads no data sources and only errors if the
schema itself is invalid.

## Remote Source

Cage supports four kinds of remote input sources, all declared in `cage.toml`'s
`[source_roots]` with unified syntax; only environment variable names are stored for
credentials (they never enter `cage.toml` or land in logs):

| Syntax | crate | Key features |
| --- | --- | --- |
| `https://...` / `http://...` | `cage-source-http` | GET response bytes are cached, then parsed as standard JSON; `ETag`/`Last-Modified` conditional GET (304 reuses the cache); `Link: rel="next"` pagination follows the chain and merges (a page = a row array); 429 with `Retry-After` backs off and retries up to 30s; `E1901` fetch failure / pagination violation, `E1902` 401/403, `E0001` bad JSON |
| `mysql:<table|named query>` | `cage-source-db` | SELECT whitelist (a single statement, starting with SELECT; semicolons/comments/row locks/CTE/`INTO` rejected) → `dsn_env` resolution → session pinned read-only (`SESSION TRANSACTION READ ONLY`); DECIMAL preserved as text, `E1905` whitelist, `E1904` credentials, `E1901` connection/statement |
| `pg:<table|named query>` | `cage-source-db` | Same as above; on the PG side, read-only uses `default_transaction_read_only` |
| `gsheet:<spreadsheet_id>/<tab>` | `cage-source-sheets` | Sheets API v4 `values` + `UNFORMATTED_VALUE`; first row is the header, blank rows skipped, short rows padded with null; shape gate `E1903` (not a row set / empty header / majorDimension not ROWS) |

The three sources share the cache layout `.cage-cache/source/<fingerprint>/`: remote bytes
hit disk first, then go through standard JSON parsing and the full L0-L7 validation,
**no bypass**. See [Source](/en/source) for details.

### Offline fallback and `--no-cache`

See the [`--no-cache` section](#no-cache-offline-fallback-toggle-for-remote-sources) above.

## web

> **Lifecycle**: ⑥ · Schema editor service. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage web ./ --port 8765
```

Starts the local service for the Schema editor and the project status page
(phase-three W series + U1..U3); it binds only to `127.0.0.1` and needs no
authentication. Opening the printed address in a browser gives the editor
single-page app;
`/status/` is the project status page (validation panel + per-profile build
freshness + build button, embedded into the binary at compile time), sharing one
exchange document with the HTTP API; the API contract is in
[Web UI](/en/web):

```text
GET  /api/schema      → 合并 Schema 的编辑器文档（每请求重载，保存后立即可见；
                         ?profile=NAME 附 filter_by_profile 投影版）
POST /api/validate    → 编辑态校验（E1701 / E1004 诊断）
POST /api/schema      → 保存回环 canonical YAML（单文件 schema_path 才可写）
GET  /api/status      → 项目状态：校验面板 + 每 profile 构建新鲜度（?profile=&env=&level=）
POST /api/build       → 触发一次构建（与 cage build 同一 build_project 管线）
GET  /api/artifacts   → 产物清单：profile 的 build manifest 账本（每文件 hash/大小/所属表）
GET  /api/artifact    → 单文件伺服：只认 manifest 键（无路径穿越），伺服前重算 blake3 比对
GET  /api/snapshots   → 快照包列表：每包现场 verify_snapshot + manifest 事实
POST /api/snapshot    → 打包快照：与 cage snapshot 同一管线（完整构建 → pack）
GET  /api/diff        → 分层 diff：changed_tables / affected_targets / files / roots
GET  /api/registry    → registry 条目浏览（读 [registry] path；未配置出说明）
```

The status page and the build API consume the same entry points as `cage check` /
`cage build` (`validate_project` / `build_project`) — diagnostics from the page,
the CLI, and the LSP never disagree. Ctrl+C stops the service. `--port` defaults to
8765.

## lsp

> **Lifecycle**: cross-cutting · ② (IDE diagnostics). See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage lsp <project>
```

Starts a **diagnostics-only** Language Server over stdio (design §49, the I-series
IDE project): a hand-written JSON-RPC 2.0 framing protocol (`Content-Length` header +
fixed-length body), synchronous single-threaded, zero new dependencies, no async
runtime. Protocol subset: `initialize` / `initialized` / `textDocument/didOpen` /
`didChange` / `didClose` / `shutdown` / `exit`; diagnostics use push-style
`textDocument/publishDiagnostics`; unknown requests get `-32601`, bad JSON gets
`-32700`, unknown notifications pass silently; `exit` without a preceding `shutdown`
counts as misuse and the process ends with exit code 1 (the LSP spec's signal).

The diagnostics are exactly the check pipeline's output (a single validation entry
point shared with `cage check` — editor diagnostics and CI diagnostics never
disagree), mapped to a **file-level zero-width range** ((0,0)-(0,0), not anchored to
a specific row/column); the message body carries the E code + the real location + a
fix hint. Full diagnostics are pushed to **every** open document — every open
document is the project diagnostics panel (validation diagnostics mostly anchor to
data files; filtering by open file would hide cross-file reference / dependency
errors). A failed project load (a broken cage.toml, an unresolvable registry
reference) likewise becomes an error diagnostic rather than silence.

`didOpen` / `didChange` trigger a full re-validation (milliseconds at the
hundred-table scale; the editor buffer is not virtualized, so what gets diagnosed is
the on-disk state — refresh after saving); `didClose` clears that document's panel.
`env` / `level` are carried by the initialize request's `initializationOptions`
(`{"env": "prod", "level": "semantic"}`); defaults match `check` (base environment /
semantic level / client profile), and a bad level surfaces as an error diagnostic.

**Not implemented**: completion / hover / go-to-definition / formatting / rename
(these need a schema semantic model — a separate project). stderr is a free-form
logging channel.

### VSCode extension (thin-shell client)

[`editors/vscode/`](https://github.com/cuihairu/cage/tree/main/editors/vscode)
is a thin-shell client for `cage lsp`: the official `vscode-languageclient` library +
about 50 lines of TS, zero Rust changes — activation condition
`workspaceContains:**/cage.toml`, spawns `cage lsp <workspace root>`, and the
`cage.env` / `cage.level` settings map to initializationOptions (changes require a
window reload). The Problems panel is the cage diagnostics panel (E code + location
+ suggestion; the file-level zero-width range means no inline squiggles). Contract
tests (`npm test`: behavior-level activate/deactivate + an LSP handshake against the
real binary + VSIX packaging assertions) and packaging are gated in CI (the editor
leg of ci.yml / daily.yml); the VSIX ships as a nightly Release attachment (same
binary channel, not an official release).

## registry

> **Lifecycle**: ③ · configuration package registry. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
cage registry publish <project> [--registry <dir>] [--package name] [--version 1.0.0] [--profile client] [--env prod]
#                                     └ 可省：缺省回落 cage.toml [registry].path
cage registry list    --registry <root> [--package NAME]… [--auth-env VAR] [--json] [path]
cage registry verify  --registry <dir>
cage registry gc      --registry <dir> [--keep 3] [--dry-run]
cage registry remove  <package> <version> --registry <dir> [--dry-run]
cage registry export  <package>[@<version>] -o <file> --registry <dir> [--compress zstd] [--sign --key-env VAR]
cage registry import  <file> [--verify-sig --key-env VAR] [--dry-run] --registry <dir>
cage registry keygen  -o <file>
cage registry push    <project> [package[@version]] (--registry <remote-url> | --presign-map <file>) [--auth-env VAR] [--dry-run]
```

The local Configuration Registry (phase-three R series,
[design §29](https://github.com/cuihairu/cage/blob/main/docs/design.md#29-configuration-registry)).
`publish` runs a full build → packs a [self-verifying snapshot](/en/build) → once
the ledger check passes, registers it under `<registry>/<package>/<version>/` (package
defaults to `project.name`, version defaults to `project.version`); only one package
per version — republishing judges idempotency by the whole-package fingerprint
(`build_id` + `content_hash`): an identical fingerprint is a no-op, and any
difference (data, schema, `--env` environment) reports an `E1801` version conflict:
the registry never rewrites history. `--env` produces an environment-scoped package
(the registered entry's `manifest.json` records `environment`); switching
environments on the same version is a conflict — publish under a different version
number (e.g. `0.1.0-prod`). `list` lists packages/versions/build_id/content_hash/
file counts in deterministic order; on a local root it lists every package, and
`--json` outputs an index array isomorphic with the remote's.

Consumers declare the registry root in cage.toml and reference packages as source
roots (R1 source resolution):

```toml
[registry]
path = "../registry"          # 相对项目根

[source_roots]
main = "registry:common@1.0.0"   # 省略 @版本 = 最高点分序版本
```

Before loading, resolution passes the entry's ledger check (tampering/added/removed
files → `E1803`, refused load; nonexistent package/version or no `[registry].path`
wired up → `E1802`). The entry's `data/` loads in the highest-fidelity format
(msgpack > json > yaml > csv > excel; msgpack round-trips floats/Bytes/integers
beyond i64 losslessly), and table names follow the artifact records in the entry's
manifest.json.

The Schema side can likewise come from an entry (R2):
`schema_path = "registry:common"` reads the entry's `schema.json` (the
profile-projected schema packed at publish time). In this case the schema belongs to
the publisher; `cage web`'s POST /api/schema returns 409 for such projects — to
change the schema, change it in the publishing project and publish again.

### Remote registry (R3, read-only)

`[registry].path` may also be an HTTP(S) root: resolution goes over the network,
publishing stays local-only.

```toml
[registry]
path = "https://registry.example.com/config"   # http(s):// 前缀 = 远程根
```

The protocol is anonymous GET (design §29): `GET <root>/<package>/index.json`,
`GET <root>/<package>/<version>/HASHES.json`, `GET <root>/<package>/<version>/<file>`; an entry's
bytes are immutable once published. Resolution flow: fetch the index → pick the
version per the pin (same rules as local) → download file by file per the ledger and
verify each blake3 (mismatch → `E1803`) → land in the in-project cache
`.cage-cache/registry/<url fingerprint>/` → deliver only after passing the
`verify_snapshot` trust gate. A cache that re-verifies clean is reused directly;
after the first online pull, **offline builds work** (the index also has a local
copy as a fallback); unreachable with no cache → `E1802`.

A remote root is read-only: `cage registry publish` against a remote root errors
out; the publisher publishes to a local registry and hosts it with any static
server, or uses `cage registry push` to direct-push to the remote (see bundle
distribution and direct push below).

`registry list` also works against a remote root (§49 L1): name packages with
`--package NAME` (repeatable) — the read-only protocol has only per-package index
resources and no global package-name directory (a global directory would need
either a server side or extra root-index maintenance in the push protocol, both
over the line), and naming packages is exactly how CI audits what it has published;
naming none reports usage with exit code 2. A missing package name is not an error:
a remote 404 renders as an empty `(no versions)` list and exits 0 (the normal state
before publishing). One GET per package, anonymous by default; when `--auth-env VAR`
names one, GETs carry the same Bearer as the push protocol (`VAR` unset/empty →
`E2105`, 401/403 → `E2102`, reporting only the variable name, never the value). The
`path` positional argument anchors
`.cage-cache/registry/<url fingerprint>/<package>/index.json`, shared with resolution: fetching
fresh online wins and is written to the cache; on transport failure it falls back to
the cached copy and marks the output offline; omitting `path` fetches fresh every
time and writes nothing to disk. Network failures other than 401/403,
`E2102`/`E2105` report `E2101`.

### Dependency declarations and version ranges (R2)

`[dependencies]` declares version pins for registry packages: a reference that
omits `@<version>` resolves per the pin (taking the highest version satisfying the
range); an explicit `@<version>` must also fall inside the pin, otherwise `E1802`.

```toml
[dependencies]
common = ">=1.0, <2.0"   # 区间 AND，取满足的最高版本
items   = "^1.2"         # >=1.2.0, <2.0.0（左起首个非零分量 +1，其后归零）
monsters = "~1.2"        # >=1.2.0, <1.3.0（锁定到次末位给定分量）
stages  = "1.2.3"        # 精确等于（裸版本 = 精确匹配）
```

Comparison operators `=` `>` `>=` `<` `<=` are supported, comma-separated as AND;
`^` caret and `~` tilde expand into closed-below ranges; comparison goes by
dotted-numeric order with per-component zero padding (`>=1.2` does not exclude
`1.2.0`). No published version satisfying the range → `E1802` (the error message
appends the list of published versions).

### Verification, rollback, and cleanup (R4)

`cage registry verify` is a whole-registry audit: every entry of every package
re-passes the ledger check (bytes re-checked against blake3), and index.json records
are cross-checked against each entry ledger's build_id/content_hash (drift →
`E1803`); an entry directory on disk with no index record (an interrupted remove/gc,
manual edits) also reports `E1803`. The audit is read-only; any problem found is
listed item by item and fails with exit code 2.

`cage registry gc` implements the GC policy for coexisting versions: a **rolling
window** keeps the newest `--keep N` versions per package (default 3, floor 1 — any
GC run leaves every package with at least one version); versions outside the window
are deleted and removed from the index, and orphan directories are swept along. The
window is the rollback surface: a consumer pinned to an old version like `@1.0.0`
can still resolve and build, while a version squeezed out of the window by gc
becomes `E1802` for consumers. Before narrowing the window, confirm no consumer is
still pinned to a version about to be removed. `--dry-run` reports the exact same
removal list without touching the registry.

Entry removal is an explicit administrative operation
(`cage registry remove <package> <version>`); the registry itself never rewrites history
(same version, different bytes is always `E1801`). remove deletes the version
directory together with the index record (`--dry-run` only checks existence and name
legality); the package's index is kept (even if it becomes empty); a version slot
after an explicit removal can be cleanly re-registered by publishing an
identical-byte snapshot again — republication is not a conflict; any other
same-version republication is still rejected as a version conflict. verify / gc /
remove, like publish / list, apply only to local registries; a remote root exits
with a read-only error.

### bundle export (A series, [design §47](https://github.com/cuihairu/cage/blob/main/docs/design.md#47-artifact-distributiona-系列2026-10-拍板))

```bash
cage registry export common@0.1.0 -o common-0.1.0.tar --registry ../registry
cage registry export common       -o common-latest.tar --registry ../registry   # 省略 @版本 = 最高点分序版本
cage registry export common -o common.tar.zst --compress zstd --registry ../registry   # zstd 压缩容器（固定压缩级别）
```

Packs a registered entry into a **deterministic tar bundle**: all the entry's files
(`data/`, `generated/`, `manifest.json`, `schema.json`, the `HASHES.json` ledger)
plus a package-index extract (`index.json`, containing only the exported entry),
with member paths `<package>/<version>/<file>`. The same entry always yields the same
bytes — members are written in name order, mtime/uid/gid zeroed, permission bits
fixed at 0o644 — independent of the exporting machine and time, so it can go
straight into object storage or diff/audit pipelines. The bundle carries its own
ledger: the receiving side (`cage registry import`) passes the `verify_snapshot`
trust gate before registering; unverified bytes are never registered.

`--compress zstd` wraps the very same deterministic tar, as-is, into a single zstd
frame (`zstd::stream::encode_all`, compression level pinned at 19 and not exposed to
users): same tar + same level + same zstd library version → identical container
bytes, byte-for-byte across two exports. `--compress` accepts only the literal
`zstd`; other values exit 2 as an argument error; without the flag it stays a plain
tar, behavior unchanged.

`--sign`, after the export lands on disk, applies one ed25519 signature to the
bundle's **file bytes** and writes the detached signature to `<bundle>.sig` (JSON:
`algorithm`/`public_key`/`signature`, base64). The signature covers the exact
bytes — both plain tar and zstd containers are signed over the on-disk bytes, so
any change anywhere in the file fails verification. The signing key resolves only
from the environment variable named by `--key-env` (base64 of a 32-byte Ed25519
seed; the key never enters cage.toml or logs) — `--sign` without `--key-env` is a
usage error exiting 2; a key unset / empty / not base64 / wrong length → `E2106`.
The companion `cage registry keygen -o <file>` generates a new key pair: the seed
(private) is written only to the file named by `-o` (0600 permissions on unix) and
never printed; stdout emits just the public key and the environment-variable usage
for export/import, two lines.

Export reads the registry and rebuilds nothing (publishing remains
`cage registry publish`); nonexistent package or version, an entry missing its
ledger, or an unwritable bundle → `E2101`. Remote roots do not support export (the
read-only protocol has no file enumeration); remote consumption goes through
`registry:` source resolution.

### bundle import (A series)

```bash
cage registry import common-0.1.0.tar --registry ../registry          # 入册
cage registry import common-0.1.0.tar --registry ../registry --dry-run # 只报告不落笔
```

Registers a bundle into the local registry: unpack into a temporary staging area →
ride the ledger through the `verify_snapshot` trust gate → cross-check the index
extract against the ledger (build_id / content_hash / file count agree) → on
passing, take the same registration path as publish. **Unverified bytes never touch
the target registry** — every rejection path (tampering, missing ledger,
member-path escape, extract/ledger drift) touches only the staging area, with
`E2103` reporting the list of problems. Re-importing identical bytes is an
idempotent no-op; same version with different bytes reports an `E1801` conflict
(the registry does not rewrite history, distribution channels included). An
imported entry is fully identical in kind to a locally published one: `registry:`
source resolution, `schema_path`, and `registry verify` all work as usual.

The import target is a local root (import is a write; remote roots take no writes);
`--dry-run` runs the full trust gate and reports the entry and file count to be
registered, writing not a single byte.

`--verify-sig --key-env <VAR>` passes the signature gate **before** the ledger
trust gate: it reads the `<bundle>.sig` detached signature and verifies the
bundle's file bytes against a trusted public key the consumer brings (base64 of a
32-byte Ed25519 public key, resolved from an environment variable). **The
public_key riding along in the sidecar is not a trust anchor** — verification
honors only the key given by `--key-env`; a failing signature, a missing or
malformed sidecar, or an unrecognized algorithm name → `E2107`, and the bundle
never touches the registry. The blake3 ledger guarantees integrity (the bytes were
not changed); the ed25519 signature adds **non-repudiation** on top (it can prove
which key produced this bundle) — the two layers each mind their own business, and
only passing both counts as passing.

Import sniffs the container form by **file header**, not by extension: if the first
four bytes are the zstd frame magic (`28 B5 2F FD`), it decompresses first and then
untars; otherwise it parses as a plain tar — both containers import identically
under any extension. Compression lives only at the container layer; the
`HASHES.json` ledger hashes the decompressed content, so a zstd-wrapped bundle and
its plain-tar form pass the same trust gate. A corrupt container that declares a
zstd frame but won't decompress is rejected with `E2103`, never silently parsed as
plain tar.

### Direct push to remote (A series)

```bash
# 从项目 [registry].path 声明的本地注册表推送（镜像 publish 的项目路径加载）
cage registry push ./game                  --registry https://registry.example.com/config --auth-env CAGE_TOKEN
cage registry push ./game common@0.1.0     --registry https://registry.example.com/config --auth-env CAGE_TOKEN
cage registry push ./game common           --registry https://registry.example.com/config --dry-run
```

Direct-pushes a locally registered entry to an http(s) registry root (design §47
A3): the source is the **local** registry declared by `[registry].path` in the
project's cage.toml (undeclared → an error prompting you to configure `[registry]`
and `publish` first); `--registry` is the **remote** target and accepts only
http(s) roots (a local target is `cage registry publish` territory — mixing them is
rejected outright). The package name defaults to `project.name`, the version to the
newest in dotted order.

Push flow: an anonymous GET of the remote package index as a status probe (the R3
read protocol unchanged) → PUT file by file (`Authorization: Bearer $TOKEN` rides
only with PUTs, not the probe) → **merge** the index with the remote's existing
entries and PUT the whole thing last (remote history entries are never rewritten or
deleted; the index is written only after every entry file succeeds, so an
interruption leaves no half-done entry references). The probe result decides the
action: package absent remotely (404) → full upload; same version, same
content_hash → idempotent zero PUTs; same version, different content_hash →
`E1801` refuses the push (the immutable-bytes discipline is uniform across
distribution channels).

Authentication: the token resolves from the environment variable named by
`--auth-env`, falling back to the project's `[registry].auth_env` declaration —
cage.toml stores **only the variable name**; the token never enters configuration
files, error text, or logs (same discipline as E1904). An env unset or empty →
`E2105` fails before any network access. Error mapping: transport failure (retries
exhausted) / unreadable remote index → `E2101`; a 401/403 on the probe or a PUT →
`E2102` (the token never enters error text); an explicit 4xx such as 405/409 →
`E2104` (the server has no write channel; fall back to the "local publish + static
hosting" deployment form). `--dry-run` runs the full local read and status probe,
zero PUTs.

#### presigned direct push (--presign-map)

```bash
cage registry push ./game common@0.1.0 --presign-map presign.json
```

Another route for direct push to object storage (the design §47 A3 decision): the
CI job first asks the object store for a **presigned URL table** (the
industry-standard delegation common to S3 / GCS / Azure Blob — cage holds no cloud
keys; credential custody belongs to the party issuing the URLs), saves it as a JSON
map, and hands it to `--presign-map`:

```json
{
  "uploads": {
    "common/0.1.0/manifest.json": "https://bucket…X-Amz-Signature=…",
    "common/0.1.0/schema.json":   "https://bucket…X-Amz-Signature=…"
  },
  "index": {
    "get": "https://bucket…/common/index.json?…X-Amz-Signature=…",
    "put": "https://bucket…/common/index.json?…X-Amz-Signature=…"
  }
}
```

`uploads` maps entry-relative paths (`package/version/file`; keys with or without a leading
`/` both work) to presigned PUT URLs; `index.get` may be omitted (the remote does
not have the package yet), `index.put` is required. The push discipline is
**word-for-word identical** to direct push — probe GET (via `index.get`), merge,
`E1801` conflict, idempotent zero PUTs for identical bytes, index PUT last — only
the URL and authentication change source: **no Bearer is read or sent**, the URL is
the credential (mutually exclusive with `--auth-env`, rejected outright at the clap
layer). Incomplete entry-file coverage (the map missing any file's URL) fails
closed with `E2101` before any PUT — never leaving half an entry for the index to
point at thin air. A malformed map / a URL that is not absolute http(s) / a missing
`index.put` likewise reports `E2101`. Immutability is guaranteed by the party
issuing the URLs (S3 conditional writes / versioned buckets / one-time key
conventions all work) — the presigned PUT itself carries no cage-side status
probing.

## migrate (M series)

> **Lifecycle**: ①→② · data migration execution. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
# 先改 schema.yaml 到新版，再写 migrations/0001-xxx.yaml（from/to/steps）
cage migrate .                    # 单段 dry-run 预演（默认：只报告，不落盘）
cage migrate . --write            # 单段执行：数据变换 + 落盘本地文本源
cage migrate . --all --write      # 整条链一次跑完
cage migrate . --to 1.2.0 --write # 链前缀：跑到指定目标版本（含）为止
cage migrate . --to latest --write # 整链（latest 解析为链终点，与 --all 同选段）
```

Runs declarative migration over existing source data (design §46): the
`migrations/` directory forms a version-stepping chain in filename order
(`0001-…`, `0002-…`; non-`.yaml`/`.yml` files are ignored); each segment declares
`from`/`to`/`steps`, with six step kinds — `rename_field` (order-preserving rename),
`set_default` (fills only missing or null), `remove_field`, `widen_type`
(safe-widening direction table + per-row value domain), `remap_values` (unmapped
values pass through as-is), and `rename_table` (table order preserved). After
execution, it re-validates against the **current schema** (the full L0–L6 stack).
When the schema comes from `registry:<package>` and `[dependencies]` declares a pin for
that package, the selected chain's end version must fall inside the pin, otherwise
`E1802` rejects it (dry-run and `--write` judged alike) — data must not be migrated
onto a schema version the project does not accept.

- **Flow**: load the project → apply segment by segment (each step reports affected
  row counts; a failure fails the whole segment and aborts, `E2003`; for Excel
  tables, the step's line carries cell-level locations of affected rows —
  `file.xlsx | Sheet: <name> | Row: <n>`, in source order, with more than 8 rows folded
  into one `… +K more row(s)` line; JSON/YAML/CSV sources keep file-level reporting
  without row-level noise) → re-validate (`E2004` carries the full diagnostics; on
  failure, **nothing is written**) → report the disposition of every source file. A
  missing `migrations/` directory or an empty chain → prints nothing to migrate and
  exits 0; a bad rule file → `E2001`; an illegal rule reference → `E2002`
  (library-side `validate_spec`; on the CLI path the apply-time `E2003` defense line
  takes over — a rename target already occupied, or a referenced table absent from
  the documents, both get rejected at transform time).
- **Segment selection**: by default runs the **first** segment in filename order
  (explicit segment-by-segment progress); `--all` runs the whole chain; `--to <ver>`
  takes a chain prefix (through the target version, inclusive); `--to latest`
  resolves to the whole chain (same selection as `--all`) — if the chain really has
  a segment whose version is literally named `latest`, the literal version wins
  (running only through that segment, inclusive). `--all` and `--to` are mutually
  exclusive; `--to` given a version not on the chain → usage error, exit 2. Every
  intermediate state on the chain must be self-consistent against the current
  schema — re-validation always runs against the schema copy on disk.
- **Dry-run by default**: runs apply + re-validation in full and reports "which
  files would be written", without moving a single byte; only `--write` persists.
- **Write boundary**: writes only the JSON/YAML/CSV inside the project's local
  source roots (written back in place, grouped by each table's `source_file`; row
  fields render in schema declaration order; files with no byte change are skipped —
  a repeated `--write` is a no-op); Excel sources are **always report-only, never
  written** (the red line "no Excel editor" — edit manually per the report, then
  re-run migrate to verify convergence); registry entries and remote sources
  (registry:/mysql:/pg:/gsheet:/http) are read-only build inputs and are likewise
  report-only.

## migrate-draft (M series)

> **Lifecycle**: ①→② · migration chain drafting. See [The Lifecycle of Game Configuration](/en/lifecycle).

```bash
# 对比两份 schema 版本，起草一份迁移规则稿（write 到 stdout 或 -o 落盘）
cage migrate-draft old-schema.yaml new-schema.yaml --from 1.0.0 --to 2.0.0
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0 -o migrations/0001-draft.yaml
```

**Drafts** migration rules from schema evolution (design §46): `diff_schemas`
aligns the two schema versions by table name/field name, and mechanically safe
transforms become steps directly — `remove_field` (fields present in the old
version and absent in the new), `widen_type` (type changes within the
widening-direction table), `set_default` (a new field with a default, or required
flipping false→true with a default present; the default value takes the target
field's type family — UInt columns render in the canonical tagged form
`{type: UInt, value: 5}`, avoiding a bare scalar reading back as the Int family and
hitting E1101); whatever the structural diff cannot judge the intent of is left to
the author, listed at the file head as `# TODO` comments — field/table renames
(whether a rename is remove+add or a rename, only the author knows), non-widening
type changes, enum member additions/removals, required flips without a default,
deleted tables. **Renames are never guessed**: `rename_field`/`rename_table`/
`remap_values` appear only in hand-written revisions.

- **The output is a draft, not rules**: a draft with TODOs can be edited directly; a
  draft with no steps to propose renders `steps: []`, and `cage migrate` refuses to
  parse it with `E2001` — finish writing it all by hand before running. The
  render-readback round trip (render → `parse_spec`) is backed by unit tests:
  `set_default` values are always presented as safely quoted strings or the
  canonical form, so `yes`/`5` never flip type on readback.
- **Exit codes**: 0 always means success (including an all-TODO draft; the stdout
  summary reports step/TODO counts and, when steps are empty, gives the
  `steps: [] must be filled` hint); 1 means a schema file could not be read or
  written. The command does not run `validate_spec` (unlike `cage migrate`'s library
  path): `set_default` references the to-schema while `remove_field` references the
  from-schema, and neither version alone validates completely — the defense line is
  relayed by apply-time `E2003` and re-validation-time `E2004`.

## Quick start

```bash
cage init my-game                  # 交互式脚手架：cage.toml + schema + 示例数据（零改动即可 check/build）
cd my-game                        # 以下命令在项目根（cage.toml 所在目录）执行
cage check .                 # 只验证，不生成
cage build . --profile client  # 验证并生成目标产物
cage gen . --profile server    # 只生成代码绑定与定义文件（cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema/template）
cage inspect . Item          # 查看 Schema 与配置结构（项目路径必填，表名可选）
cage assets . --write assets.json   # 资产清单基线（blake3 逐文件盘点；--check 复核工作树）
cage diff build/a build/b          # 比较两个配置版本
cage graph . --format dot    # 表引用依赖图（text/dot/json，含拓扑序与环清单）
cage verify build/                 # 审计已生成产物（对照 manifest 账本逐文件重算 blake3）
cage snapshot . --profile client    # 构建 + 打包自校验快照（--verify 载入前校验，--list 列包）
cage web ./                        # 本地 Schema 编辑器与项目状态页服务（只绑 127.0.0.1）
cage lsp ./                        # stdio Language Server（IDE 诊断，与 check 同一校验管线）
cage registry publish . --registry reg/      # 配置包仓库：发布/列表/审计/GC/移除/导出/导入/直推/keygen（见 registry 节）
cage migrate .                     # 迁移预演（dry-run；--write 落盘并全栈回验，--all 全链）
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0   # 从两版 schema 起草迁移规则稿
cage schema-draft . mysql:Items    # 只读内省库表起草 schema（MySQL/PostgreSQL）
cage mcp --stdio                   # AI 助手 MCP server（校验/结构/构建预演做成只读 tools）
```
