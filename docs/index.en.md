---
layout: home

hero:
  name: Cage
  text: A configuration compiler and validator for games
  tagline: Reliably compile scattered, heterogeneous, human-authored configuration data into validated, deterministically built runtime configuration assets
  actions:
    - theme: brand
      text: Start with the architecture
      link: /en/architecture
    - theme: alt
      text: 中文
      link: /
    - theme: alt
      text: CLI at a glance
      link: /en/cli
    - theme: alt
      text: GitHub
      link: https://github.com/cuihairu/cage

features:
  - title: Source Adapter plugins
    details: Excel / CSV / JSON / YAML plus HTTP API / MySQL / PostgreSQL / Google Sheets remote sources. They only read and parse, and never perform business validation; XML and SQLite come later.
  - title: Schema decoupled from Source
    details: One Schema serves many configuration sources — types, required fields, defaults, ranges, enums, uniqueness and references are each defined independently.
  - title: Eight-level validation pipeline
    details: Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule run level by level; cage check --level validates by level.
  - title: Cross-configuration reference validation
    details: Existence plus compatibility checks — a referenced object that exists but has an invalid type is an error too; semantic predicate validation is reserved.
  - title: Diagnostics as a first-class citizen
    details: Error codes plus precise locations (file / Sheet / cell / field / value) plus fix hints, consumed directly by the CLI, the Web UI and the IDE (`cage lsp`).
  - title: Deterministic builds
    details: Same input → same artifacts, hashes and Manifest. Traceable, rollback-friendly and incremental.
  - title: Profile mechanism
    details: client / server profiles configure different targets and field visibility, so one configuration is reused on both ends instead of maintaining two sets of tables.
  - title: Target plugins
    details: Starting from JSON / CSV data artifacts, extended to MessagePack (deterministic binary), Protobuf `.proto` and FlatBuffers `.fbs` definition files, plus C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java code bindings and a Template Target (custom Tera templates); Binary / SQLite and the rest come later. Data serialization and code generation are kept separate.
  - title: Declarative data migration
    details: cage migrate steps through a versioned chain applying rule transforms (rename / fill defaults / drop fields / widen types / remap values) to validated configuration; it reports as a dry run by default, --write writes back in place and re-validates the whole stack.
---

[中文](/) | English


## Core flow

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline（L0 Parse ~ L7 Game Rules）
      → Normalize / IR → Target Generators → Manifest / Hash
```

Excel is only one kind of input source for Cage. Every input enters the unified Canonical Model, and once validation passes, deterministic assets are produced for the target platform — beyond the conversion itself, validation, references and normalization all happen on this same chain.

## Quick start

```bash
cage init my-game                    # 新项目：交互式脚手架（终端问答），零改动即可 check/build
cage check config/                   # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage gen config/ --profile server    # 只生成代码绑定与定义文件（cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema）
cage diff build/a build/b            # 比较两个配置版本
cage graph config/                   # 表引用依赖图（text/dot/json，含拓扑序与环清单）
cage verify build/                   # 审计已生成产物（逐文件重算 blake3 对照账本）
cage inspect config/ Item            # 查看 Schema 与配置结构（项目路径必填，表名可选）
cage assets config/ --write assets.json   # 资产清单基线（--check 复核缺失/篡改/多余）
cage snapshot config/ --profile client    # 构建 + 打包自校验快照（--verify 载入前校验，--list 列包）
cage registry publish config/        # 多版本配置仓库（发布/列表/审计/GC/移除/导出/导入/直推）
cage migrate config/                 # 数据迁移预演（dry-run；--write 落盘并全栈回验，--all 全链）
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0   # 从两版 schema 起草迁移规则稿
cage schema-draft . mysql:Items      # 只读内省库表起草 schema（MySQL/PostgreSQL）
cage web ./                          # 本地 Schema 编辑器与项目状态页（只绑 127.0.0.1）
cage lsp ./                          # stdio Language Server（IDE 诊断，与 check 同一校验管线）
cage mcp --stdio                     # AI 助手 MCP server（校验/结构/构建预演做成只读 tools）
```

## Documentation sections

- [Architecture](/en/architecture): positioning, core concepts, the plugin model and the project structure
- [Source](/en/source) / [Schema](/en/schema) / [Validation](/en/validation) / [Target](/en/target): the four stages of the data pipeline
- [CLI](/en/cli) / [Build](/en/build): the command line and deterministic builds
- [Web UI](/en/web): phase-three schema editor + project status surface (exchange model W1, local service W2, editor UI W3, samples and smoke W4, project status and build API W5 — all implemented)
- [UI front end](/en/ui): the decisions and roadmap for the web-enhanced shape — project and build status / validation panel (P0, U1 implemented), artifact browsing + editor target badges / profile-scoped views (P1, U2a/U2b implemented), snapshot / registry / layered diff (P2, U3 implemented); no Merkle, a semantic layered diff instead
- [IDE support](/en/cli): the `cage lsp` stdio Language Server (the I series of `docs/design.md §49` is implemented — a diagnostics-only LSP subset sharing the validation pipeline with check, a hand-written frame protocol with zero new dependencies) + a thin VSCode extension shell (the Problems panel is cage diagnostics, the VSIX ships with nightly)
- [Registry](/en/cli): the R series configuration registry of phase three (local publish/list + the `registry:` source and Schema resolution + `[dependencies]` version pinning + remote resolution + verify auditing the whole registry / gc rolling window / remove explicit removal, R1–R4 implemented; the A series bundle export/import + `registry push` straight to a remote + `--presign-map` presigned-URL push are also implemented)
- [Assets](/en/assets): three kinds of input assets — Schema / source data / build rules — plus asset management with artifact traceability (`docs/design.md §50`: the `cage assets` asset manifest and baseline re-check, four layers of backup defense, snapshot and registry versioning capabilities; [`examples/asset-workspace`](https://github.com/cuihairu/cage/tree/main/examples/asset-workspace) demonstrates the full lifecycle)
