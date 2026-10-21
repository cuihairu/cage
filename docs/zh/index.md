---
layout: home

hero:
  name: Cage
  text: 游戏配置编译与验证框架
  tagline: 把分散、异构、面向人的配置数据，可靠地编译成经过验证、确定性构建的运行时配置资产
  actions:
    - theme: brand
      text: 从架构开始
      link: /zh/architecture
    - theme: alt
      text: English
      link: /
    - theme: alt
      text: CLI 速览
      link: /zh/cli
    - theme: alt
      text: GitHub
      link: https://github.com/cuihairu/cage

features:
  - title: Source Adapter 插件
    details: Excel / CSV / JSON / YAML 与 HTTP API / MySQL / PostgreSQL / Google Sheets 远程源，只负责读取解析，不做业务校验；后续扩展 XML、SQLite。
  - title: Schema 与 Source 解耦
    details: 同一份 Schema 可用于多个配置源：类型、必填、默认值、范围、枚举、唯一、引用独立定义。
  - title: 八级验证流水线
    details: Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule 逐级执行，cage check --level 可分级验证。
  - title: 跨配置引用校验
    details: 存在性 + 兼容性检查，引用对象存在但类型不合法同样报错；引用谓词按语义求值，不满足即报 E1410。
  - title: Diagnostics 一等公民
    details: 错误码 + 精确定位（文件 / Sheet / 单元格 / 字段 / 值）+ 修复提示，CLI、Web UI 与 IDE（`cage lsp`）直接消费。
  - title: 确定性构建
    details: 相同输入 → 相同产物、哈希与 Manifest，可追踪、可回滚、可增量。
  - title: Profile 机制
    details: client / server 配置不同 targets 与字段可见性，一份配置两端复用，不维护两套表。
  - title: Target 插件
    details: JSON / CSV 数据产物起步，已扩展 MessagePack（确定性二进制）、Protobuf `.proto` 与 FlatBuffers `.fbs` 定义文件，以及 C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java 代码绑定与 Template Target（Tera 自定义模板）（Binary / SQLite 等留待后续）；数据序列化与代码生成分离。
  - title: 声明式数据迁移
    details: cage migrate 按版本步进链对已验证配置做规则变换（改名 / 补默认 / 删字段 / 类型加宽 / 值重映射），默认 dry-run 报告，--write 原路写回并全栈回验。
---

<script setup lang="ts">
// 记录经 English 链接离开时的手动语言选择，英文首页的首次访问重定向就不会把用户弹回来。
const KEY = 'cage-lang-auto-picked'
const BASE = '/cage/'
const ROOT_PATH = BASE
const ZH_PATH = BASE + 'zh/'
if (typeof window !== 'undefined') {{
  window.addEventListener('click', (e) => {{
    const a = (e.target as HTMLElement)?.closest?.('a') as HTMLAnchorElement | null
    const href = a?.getAttribute('href') || ''
    if (href.startsWith(ZH_PATH)) localStorage.setItem(KEY, ZH_PATH)
    else if (href === ROOT_PATH || href === ROOT_PATH.slice(0, -1)) localStorage.setItem(KEY, ROOT_PATH)
  }}, true)
}}
</script>


[English](/) | 中文


## 核心链路

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline（L0 Parse ~ L7 Game Rules）
      → Normalize / IR → Target Generators → Manifest / Hash
```

Excel 只是 Cage 的一种输入源。所有输入进入统一 Canonical Model，验证通过后按目标平台产出确定性资产，转换之外校验、引用、规范化都在同一条链上完成。

## 界面原型

`cage init` —— 新项目脚手架的终端问答向导（9 题，逐题单行输入，可回退、留空取默认值）：

![cage init 交互式问答向导](/design/init-wizard.png)

`cage check` —— 校验结果视图：错误列表，含错误码、源文件定位与修复建议：

![cage check 校验结果（错误列表）](/design/validation-result.png)

`cage check` —— 环境差异视图：dev/test/staging/prod 四套 `env_overrides` 的字段级约束对比：

![cage check 校验结果（环境差异）](/design/validation-result-env.png)

`cage check` —— 结构树视图：schema 与字段的层级、类型、约束与 profile 可见性：

![cage check 校验结果（结构树）](/design/validation-result-tree.png)

## 快速开始

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

## 文档栏目

- 资产：Schema/源数据/构建规则三类输入资产 + 产物追溯的资产管理（design §50：`cage assets` 资产清单与基线复核、备份四层防线、快照与注册表的版本化能力；[`examples/asset-workspace`](https://github.com/cuihairu/cage/tree/main/examples/asset-workspace) 全生命周期示范）
