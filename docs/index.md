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
    details: Excel / CSV / JSON / YAML plus HTTP API / MySQL / PostgreSQL / Google Sheets remote sources. They only read and parse, and never perform business validation; XML and SQLite come later.
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
    details: Starting from JSON / CSV data artifacts, extended to MessagePack (deterministic binary), Protobuf `.proto` and FlatBuffers `.fbs` definition files, plus C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java code bindings and a Template Target (custom Tera templates); Binary / SQLite and the rest come later. Data serialization and code generation are kept separate.
  - title: Declarative data migration
    details: cage migrate steps through a versioned chain applying rule transforms (rename / fill defaults / drop fields / widen types / remap values) to validated configuration; it reports as a dry run by default, --write writes back in place and re-validates the whole stack.
---

<script setup lang="ts">
// 首次访问按浏览器语言选择文档语言（vitepress 无此能力，GitHub Pages 不支持 _redirects）
// English is the site default locale (root); Chinese-speaking visitors are redirected to /zh/;
// priority: manual localStorage choice > navigator.language
import { onBeforeMount } from 'vue'
const KEY = 'cage-lang-auto-picked'
const BASE = '/cage/'
const ROOT_PATH = BASE
const ZH_PATH = BASE + 'zh/'
onBeforeMount(() => {{
  if (typeof window === 'undefined') return
  const p = window.location.pathname
  if (p !== ROOT_PATH && p !== ROOT_PATH.slice(0, -1)) return
  const picked = localStorage.getItem(KEY)
  if (picked) {{ if (picked !== p) window.location.replace(picked); return }}
  const langs = (navigator.languages?.length ? navigator.languages : [navigator.language]).map(l => l.toLowerCase())
  if (langs.some(l => l.startsWith('zh'))) window.location.replace(ZH_PATH)
}})
if (typeof window !== 'undefined') {{
  window.addEventListener('click', (e) => {{
    const a = (e.target as HTMLElement)?.closest?.('a') as HTMLAnchorElement | null
    const href = a?.getAttribute('href') || ''
    if (href.startsWith(ZH_PATH)) localStorage.setItem(KEY, ZH_PATH)
    else if (href === ROOT_PATH || href === ROOT_PATH.slice(0, -1)) localStorage.setItem(KEY, ROOT_PATH)
  }}, true)
}}
</script>


[中文](/zh/) | English


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
cage init my-game                    # 新项目：交互式脚手架（终端问答），零改动即可 check/build
cd my-game                          # 以下命令在项目根（cage.toml 所在目录）执行
cage check .                   # 只验证，不生成
cage build . --profile client  # 验证并生成目标产物
cage gen . --profile server    # 只生成代码绑定与定义文件（cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema/template）
cage diff build/a build/b            # 比较两个配置版本
cage graph .                   # 表引用依赖图（text/dot/json，含拓扑序与环清单）
cage verify build/                   # 审计已生成产物（逐文件重算 blake3 对照账本）
cage inspect . Item            # 查看 Schema 与配置结构（项目路径必填，表名可选）
cage assets . --write assets.json   # 资产清单基线（--check 复核缺失/篡改/多余）
cage snapshot . --profile client    # 构建 + 打包自校验快照（--verify 载入前校验，--list 列包）
cage registry publish . --registry reg/        # 多版本配置仓库（发布/列表/审计/GC/移除/导出/导入/直推）
cage migrate .                 # 数据迁移预演（dry-run；--write 落盘并全栈回验，--all 全链）
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0   # 从两版 schema 起草迁移规则稿
cage schema-draft . mysql:Items      # 只读内省库表起草 schema（MySQL/PostgreSQL）
cage web ./                          # 本地 Schema 编辑器与项目状态页（只绑 127.0.0.1）
cage lsp ./                          # stdio Language Server（IDE 诊断，与 check 同一校验管线）
cage mcp --stdio                     # AI 助手 MCP server（校验/结构/构建预演做成只读 tools）
```

## Documentation sections

- Assets: three kinds of input assets — Schema / source data / build rules — plus asset management with artifact traceability (`docs/design.md §50`: the `cage assets` asset manifest and baseline re-check, four layers of backup defense, snapshot and registry versioning capabilities; [`examples/asset-workspace`](https://github.com/cuihairu/cage/tree/main/examples/asset-workspace) demonstrates the full lifecycle)
