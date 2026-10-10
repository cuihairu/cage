# 架构

Cage 解决的问题不是「Excel 转 JSON」，而是：**如何将游戏开发过程中分散、异构、面向人的配置数据，可靠地编译成经过验证、确定性构建、可供不同运行时直接消费的配置资产。**

核心流程：

```text
Authoring Sources
       |
       v
+------------------+
|  Source Adapters |
+--------+---------+
         |
         v
+------------------+
| Canonical Model  |
+--------+---------+
         |
         v
+--------------------------+
| Validation Pipeline      |
|                          |
| Syntax / Schema / Type   |
| Value / Table / Reference|
| Semantic / Game Rules    |
+------------+-------------+
             |
             v
+------------------+
| Normalize / IR   |
+--------+---------+
         |
         v
+------------------+
| Target Generators|
+--------+---------+
         |
    +----+----+--------+
    |    |    |        |
   JSON CSV  C#       Lua
             Python   Protobuf
             ...
```

图中 Protobuf 为规划项；当前已实装 JSON / CSV 数据产物、C# / Python /
Lua / TypeScript / JavaScript / C++ / Go / Java 代码绑定与 Template
Target（Tera 用户自定义模板）。

## 为什么需要 Cage

游戏项目中的配置通常同时服务于：

- 策划
- 程序
- 客户端
- 服务端
- 工具链
- CI/CD
- 测试环境
- 运营

不同角色需要不同的表达形式。例如同一份 Item 配置：

```text
策划：
    Excel

客户端：
    JSON / CSV / C#

服务端：
    JSON / CSV / Lua / Python

工具：
    JSON / SQLite / Binary
```

如果没有统一的配置构建系统，项目很容易演变成：

```text
Excel -> Python Script
Excel -> C# Script
Excel -> Lua Script
Excel -> JSON Script
Excel -> CSV Script
```

然后每个脚本：

- 自己解析
- 自己转换
- 自己校验
- 自己处理默认值
- 自己处理引用
- 自己报错

最终产生多个问题：

1. 不同输出可能产生不同结果。
2. 校验逻辑散落在不同脚本中。
3. 跨表引用无法统一验证。
4. 配置错误直到游戏运行时才暴露。
5. 修改一个字段需要修改多个转换器。
6. CI 无法得到统一的配置质量门禁。
7. 配置无法形成可追踪、可复现的构建产物。

Cage 的目标就是把这条链统一起来。

## 六个核心概念

Cage 将配置系统划分为六个核心概念：

```text
Source
Schema
Model
Validation
Transform
Artifact
```

### Source

配置的输入来源，例如 Excel、CSV、JSON、YAML，已扩展 MySQL / PostgreSQL、HTTP API、Google Sheets 远程源与 `registry:` 源根，后续扩展 XML、TOML、SQLite 等。

Source 只负责：

> **把外部数据读取进 Cage。**

它不应该负责业务验证。详见 [Source 栏目](/source)。

### Schema

定义配置的结构和约束，例如：

```yaml
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 }, required: true }
      title: { name: title, type: { kind: String }, required: true }
      price: { name: price, type: { kind: UInt32 }, min: 0 }
      type: { name: type, type: { kind: Enum, value: ItemType } }

enums:
  ItemType:
    name: ItemType
    values:
      - { name: Weapon, value: 1 }
      - { name: Armor, value: 2 }
      - { name: Consumable, value: 3 }
```

Schema 定义：字段、类型、必填、默认值、范围、枚举、数组、对象、唯一性、引用、输出信息（此为实际 wire 格式；完整 DSL 见 [Schema 栏目](/schema)）。

### Canonical Model

所有 Source 都应该进入统一的中间模型：

```text
Excel
JSON
YAML
CSV
SQLite
       |
       v
Canonical Model
```

Canonical Model 不应该直接等同于 JSON。推荐基本类型：

```text
Null
Bool
Int
UInt
Float
String
Bytes
Array
Object
```

并携带：

```text
Source Location
Schema Information
Type Information
Metadata
```

这样可以做到：

```text
monster.xlsx
Sheet: Monster
Row: 27
Column: DropItemID
```

在错误信息中精确定位。

## 编译器核心：八个概念的边界（v0.3 定稿）

共识：Cage 最大的风险不是功能不够，而是编译器核心模型的边界模糊——
`Schema / Canonical Model / IR / Validation Context / Dependency Graph /
Profile / Snapshot / Manifest` 八个概念一旦稳定，Excel/CSV/JSON/YAML 与
C#/Lua/C++/Python/Protobuf 都只是插件。本章把每个概念的归属与死线写死，
下面的插件模型与全部 Target 都在这张表之上工作。

### 概念边界表

| 概念 | 定义 | 承载类型 | 生产 → 消费 | 边界禁令 |
| --- | --- | --- | --- | --- |
| Schema | 配置的结构与约束定义（编译期输入） | `Schema` / `ValidatedSchema`（cage-core::schema） | 用户书写 → Validation、Target 消费 | 不读数据文件；不携带运行时值 |
| Canonical Model | 数据在 Cage 世界的语义模型 | `Value` / `TypedValue` / `Document`（cage-core::value，含 SourceLocation） | Source 生产 → Validation / Normalize / Target 消费 | 不直接等同任何文件格式；Source 只把它读懂，Target 只读它 |
| IR | 归一化后的 Canonical。v0.3 定界：与 Canonical 同构，不设独立类型 | 即 Canonical（归一化 `Document` + `Schema`） | Normalize 生产 → Target 消费 | Target 不得回读 Source 文件；IR 阶段不重新验证 |
| Validation Context | 验证执行期的游标与现场 | `ValidationContext`（current_table / current_row / current_field / schema…） | Validation 生产 → Diagnostics 消费 | 验证不产出产物、不修改数据；Target 不重复验证 |
| Dependency Graph | 表间引用的拓扑与增量规划 | `DependencyGraph` / `IncrementalPlanner`（cage-core::reference） | schema 引用声明建图（`from_schema`）→ L5 校验与增量构建共同消费 | 图是编译结果不是运行时数据 |
| Profile | 面向消费端的裁剪视图 | `BuildProfile`（cage-core::manifest）+ profile 过滤 | 用户配置 → 裁剪 Schema + Document → Validation / Target | 过滤是起点不是终点：语义落地面见差距表 |
| Manifest | 构建产物的账本与输入指纹 | `BuildManifest` / `ArtifactInfo`（cage-core::manifest） | ManifestGenerator 生产 → verify / 增量 / 部署 / 回滚消费 | 只记账不生成；与产物一同落盘、随产物验证 |
| Snapshot | 可独立加载的配置快照（D5 已实装） | `snapshot_files` / `verify_snapshot` / `load`（cage-core::snapshot） | 构建生产 → 服务器 / 客户端启动加载校验 | 快照自带校验信息，不依赖构建机现场 |

### 为什么 IR 不拆独立类型（v0.3 决策）

评审原型设想 IR ≈ `CompiledItemTable { schema_id, table_id, fields, values,
references, normalized_types, visibility, dependencies }`。对照现状：

- normalize（cage-core/src/normalize/mod.rs：`normalize_document` /
  `normalize_typed_value` / `normalize_value`）是 Canonical → Canonical 的纯
  变换，产出仍是 Document / TypedValue / Value 类型；
- 全部 12 个 target 的入口要么是 `(Schema, Document)`（数据 target），要么是
  `(Schema, schema_hash)`（代码 target——类型与元数据全来自 Schema，数据不参与
  代码生成）。target 面已经按 IR 视角消费，且结构性满足「不得读 Source」。

现阶段拆独立 IR 类型只会引入双份形状与无谓的转换/测试面。定界：IR 与
Canonical 同构，以（归一化 Document, Schema）表达。**何时再拆**：当校验面与
产物面形状开始分歧（per-profile 裁剪固化、visibility 折叠、产物需要编译期
预计算的派生形状）时，再以 Compiled IR 收敛——拆分点已预留，不做先行设计。

### 现状与差距（逐概念核对，2026-10）

| 概念 | 现状 | 差距 / 下一步 |
| --- | --- | --- |
| Schema | 已实装：19 种字段类型（含 Map）、引用、唯一约束、字段级 `targets` 可见性 | — |
| Canonical Model | 已实装：value.rs 全类型 + SourceLocation | — |
| IR | 定界完成（见上） | 派生形状需求出现时拆 Compiled IR |
| Validation Context | 已实装：validation/mod.rs（schema / document / diagnostics / max_level / profile / reference_cache / warnings_as_errors） | — |
| Dependency Graph | 已实装并接线：reference/mod.rs `DependencyGraph`（环检测 / 拓扑）+ `IncrementalPlanner`；cli 构建真实接线，增量第二层按表哈希 + 依赖传播只重建受影响表，manifest 落 `dependencies`/`table_hashes` 账（D2） | 增量删除表回退全量（不沿边传播删除语义） |
| Profile | 已实装（D3 语义化）：表 + 字段双层面板过滤裁剪 Schema 与 Document 产物视图；校验在**完整** schema/document 上执行（profile 只裁剪产物视图、不豁免数据校验）；ValidationContext 携带 profile，结构不可缺字段（required 无默认 / 主键 / 唯一约束 / 引用目标）被 profile 隐藏报 E9006 冲突而非静默过滤 | 可选字段裁剪保持合法视图语义；整表剔除是表级可见性语义 |
| Environment | 已实装（§48 分环境验证）：Schema 表级 `env_overrides` 声明「环境名 → 字段 → 约束补丁」（部分补丁、整值替换、类型不可覆盖），`--env` 在校验/生成前解析为一份普通 Schema（校验器与代码生成零改动）；环境名 Schema 自声明（declared_envs 并集），manifest 记 `environment`、schema_hash/build_id 随环境轮换，增量守卫按环境失配 | 非选中环境的坏覆盖 lint（需新 E 码）与 snapshot / registry publish 环境化出包留待 |
| Manifest | 已实装（D4 + 增量第三层 + 环境 §48）：13 顶层字段（project / profile / environment / cage_version / generator_version / build_id / schema_hash / source_hash / content_hash / dependencies / table_hashes / targets / artifacts；`environment` 基线构建缺省） | — |
| Snapshot | 已实装（D5）：`cage snapshot` 打包 profile 视图 + `--verify` 校验；`snapshot/<profile>-<build_id[..12]>` 确定性目录（manifest / schema.json / data / generated / HASHES 逐文件账本），core 提供 verify/load 服务器入口 | 删除/回滚策略（多快照共存管理）见 Registry 阶段——R1–R4 已实装（`cage registry` 本地发布/列表 + `registry:` 源与 schema_path 解析 + `[dependencies]` 版本 pin + 远程 http(s) 只读解析，条目即自校验快照 + verify 全册审计 / gc 滚动窗口 / remove 显式移除与重发）；§47 A 系列分发已实装（export 确定性 tar bundle / import 信任门入册 / push 直推 http(s) 远端根，E21xx 五码接线） |

### 评审对照修正（2026-10 外部评审）

| 评审项 | 评审评级（5 分制） | 代码核对结论 |
| --- | --- | --- |
| IR | 3 | 定界后归入 Canonical，见「为什么 IR 不拆独立类型」 |
| Dependency Graph | 2 | 已闭合（D2）：核心 + 构建接线 + 增量第二层 + manifest 账本（依赖/表哈希） |
| Incremental Build | 2 | 已闭合（D2）：第一层整轮跳过 + 第二层依赖传播只重建受影响表（携带字节与全量一致、manifest 收敛）；删除表回退全量 |
| Snapshot | 2 | 已闭合（D5）：`cage snapshot` 打包 + `--verify` 校验 + core verify/load 服务器入口；确定性目录名（build_id 指纹，弃日期命名） |
| Profile | 4 | 已闭合（D3）：过滤 + E9006 冲突语义 + profile 感知校验上下文；校验全库、profile 只裁剪产物视图 |
| Manifest | — | 建议评级（D4）：build_id / generator_version / dependencies / table_hashes 已落，前 24 位 blake3 确定性指纹替代时间戳 |
| Diagnostics | 5 | 确认：L0-L7 全错误码族、行级定位 + hint、E1601 端到端 |
| Deterministic Build | 5 | 确认：同输入字节一致由 golden 测试锁定 |

## 插件模型

Cage 的核心应该尽量稳定：

```text
                    Cage Core
                       |
       +---------------+----------------+
       |               |                |
 Source Plugins   Validator Plugins   Target Plugins
       |               |                |
     Excel          SkillValidator      JSON
     CSV            QuestValidator      CSV
     YAML           MonsterValidator    C#
     JSON                                Python
                                         Lua
```

这样第三方可以扩展 Cage，而无需修改 Core。

## 工程结构（Rust 落地）

Cage 使用 Rust 实现，按 Cargo workspace 组织为多个 crate；核心与各输入/输出插件彼此独立，依赖单向指向 `cage-core`：

```text
cage/
├── Cargo.toml               # workspace 根
├── crates/
│   ├── cage-core/           # Canonical Model / Schema / Diagnostics /
│   │   └── src/             # Validation / Reference / Normalize / Manifest
│   │       ├── value/           # Canonical Model（Value + Source Location）
│   │       ├── schema/          # Schema 定义与解析
│   │       ├── diagnostics/     # 诊断框架（错误码 / 定位 / 渲染）
│   │       ├── error/           # 错误码全表（E0xxx~E99xx 分族常量）
│   │       ├── validation/      # L0-L7 验证流水线
│   │       ├── reference/       # 跨配置引用 + DependencyGraph / IncrementalPlanner
│   │       ├── normalize/       # 归一化
│   │       ├── manifest/        # Build Manifest
│   │       ├── snapshot/        # Configuration Snapshot（打包 / 校验 / 加载）
│   │       ├── edit/            # Schema ↔ 编辑器交换模型（W1）
│   │       ├── registry.rs      # Configuration Registry（发布 / 解析 / 审计，R1-R4；bundle 导出导入 + http(s) 直推，§47 A 系列）
│   │       ├── migrate/         # 声明式数据迁移（规则模型 + 执行器 + 回验，M 系列，design §46）
│   │       └── remote.rs        # Remote Source 共享取数 / 重试 / 缓存键（§45；http_put 直推写通道，§47 A3）
│   ├── cage-source-excel/   # Excel 输入源（calamine）
│   ├── cage-source-csv/     # CSV 输入源
│   ├── cage-source-json/    # JSON 输入源
│   ├── cage-source-http/    # HTTP API 输入源（S1，design §45）
│   ├── cage-source-db/      # MySQL / PostgreSQL 输入源（S2，design §45：SELECT 白名单 + 会话只读 + 行集 → canonical JSON 落缓存）
│   ├── cage-source-sheets/  # Google Sheets 输入源（S3，design §45：values UNFORMATTED_VALUE + 首行表头 + 形状门 E1903）
│   ├── cage-source-yaml/    # YAML 输入源
│   ├── cage-source-msgpack/ # MessagePack 输入源（registry 回灌：msgpack target 线格式重消费）
│   ├── cage-target-json/    # JSON 产出
│   ├── cage-target-csv/     # CSV 产出
│   ├── cage-target-msgpack/ # MessagePack 产出（rmp 最小形，确定性二进制）
│   ├── cage-target-proto/   # Protobuf .proto3 定义文件（schema 驱动，代码绑定族）
│   ├── cage-target-cs/      # C# 代码绑定
│   ├── cage-target-py/      # Python 代码绑定
│   ├── cage-target-lua/     # Lua 代码绑定
│   ├── cage-target-ts/      # TypeScript / JavaScript 代码绑定
│   ├── cage-target-cpp/     # C++ 代码绑定
│   ├── cage-target-go/      # Go 代码绑定
│   ├── cage-target-java/    # Java 代码绑定
│   ├── cage-target-template/  # Tera 模板引擎：官方随包模板 + 用户自定义模板
│                            #   （format = "template"，G 系列，design §22）
│   └── cage-cli/            # cage 命令行（check / build / gen / inspect / diff /
│                            #   snapshot / web / registry / migrate，含远程注册表解析）
├── docs/                    # 本文档站（VitePress）
└── .github/workflows/       # CI / 每日构建 / 文档部署
```

## 安全与隔离

如果允许项目编写业务 Validator，需要注意：

```text
Validator
    |
    v
可能执行任意代码
```

因此：

- 本地开发可以使用 Native Plugin
- CI 可以考虑 Sandbox
- 不应默认执行不可信项目代码
- Remote Registry 不应该直接执行上传的 Validator

这是后期需要重点考虑的问题。

## 不应该做的事情

Cage 不应该变成：

### Excel 编辑器

Excel 本身已经是成熟的 Authoring Tool。

### 游戏数据库

Cage 构建配置，不负责成为游戏运行时数据库。

### Secret Manager

密码、Token、Key 不属于普通游戏配置。

### 游戏逻辑框架

Cage 可以验证业务规则，但不应该成为游戏服务器逻辑框架。

### 强绑定某一个游戏引擎

不绑定 Unity、Unreal、Godot、Cocos。它应该服务于所有游戏客户端和服务器。

## 与普通配置转换器的区别

普通 Converter：

```text
A -> B
```

Cage：

```text
Source
  |
  v
Parse
  |
  v
Canonical Model
  |
  +--> Schema
  |
  +--> Type
  |
  +--> Constraint
  |
  +--> Reference
  |
  +--> Semantic
  |
  +--> Game Rules
  |
  v
Validated Model
  |
  +--> JSON
  +--> CSV
  +--> C#
  +--> Python
  +--> Lua
  +--> Protobuf
  +--> ...
```

转换之外，Cage 建立的是一条配置编译链：校验、跨配置引用、规范化与确定性构建都在这条链上完成。

## 命名：Casino Cage 隐喻

Cage 的命名来自赌场中的 **Cage / Cashier's Cage**。

赌场中的 Cage 是：

```text
Cash
  |
  v
Cage
  |
  +--> Verify
  +--> Exchange
  +--> Record
  |
  v
Chips
```

Cage：

```text
Authoring Data
  |
  v
Cage
  |
  +--> Parse
  +--> Validate
  +--> Normalize
  +--> Transform
  +--> Audit
  |
  v
Runtime Assets
```

所以它更像配置进入运行时世界之前的兑换与清算边界：

赌场里的 Cage 不关心你最后玩 Poker、Blackjack 还是 Baccarat；同样，Cage 不应该关心配置最终服务 Unity、Cocos、Unreal、Game Server 还是工具。它只负责：

> **输入的数据必须合法、完整、可验证，然后才能兑换成运行时资产。**

## 一句话定义

英文：

> **Cage is a configuration compiler and validation framework for game development. It transforms heterogeneous authoring data into validated, deterministic runtime artifacts.**

中文：

> **Cage 是一个面向游戏研发的配置编译与验证框架，将异构的配置源转换为经过验证、确定性构建的运行时配置资产。**

## 核心设计原则

```text
 1. 不绑定 Excel
 2. Source 与 Target 解耦
 3. 所有输入进入统一 Canonical Model
 4. Validation 与 Transformation 分离
 5. Schema 负责结构
 6. Reference 负责跨配置关系
 7. Semantic Rule 负责组合逻辑
 8. Plugin Validator 负责复杂游戏业务
 9. Target Generator 负责输出
10. Build 必须可重复
11. Artifact 必须可追踪
12. Diagnostics 必须精确到 Source Location
13. Client / Server 使用 Profile，而不是硬编码
14. Core 不绑定具体游戏引擎
15. Excel 只是第一种 Source
```

## 最终架构

```text
                             CAGE
              Game Configuration Compiler
                                  |
       +--------------------------+--------------------------+
       |                          |                          |
       v                          v                          v
  Authoring Sources           Schema / Rules             Profiles
       |                          |                          |
  +----+----+----+                |                    +-----+-----+
  |    |    |    |                |                    |           |
Excel CSV JSON YAML               |                 Client       Server
  |    |    |    |                |                    |           |
  +----+----+----+----------------+--------------------+-----------+
                                  |
                                  v
                         +----------------+
                         | Canonical IR   |
                         +-------+--------+
                                 |
                                 v
                      +-----------------------+
                      | Validation Pipeline   |
                      |                       |
                      | Parse                 |
                      | Schema                |
                      | Type                  |
                      | Value                 |
                      | Table                 |
                      | Reference             |
                      | Semantic              |
                      | Game Rules            |
                      +-----------+-----------+
                                  |
                                  v
                         Validated Model
                                  |
                    +-------------+-------------+
                    |             |             |
                    v             v             v
                  JSON          CSV           Code
                                              |
                                      +-------+-------+
                                      |       |       |
                                     C#     Python   Lua
                    |
                    +-----------> Protobuf
                    |
                    +-----------> MsgPack
                    |
                    +-----------> Binary
                                  |
                                  v
                           Runtime Artifacts
                                  |
                                  v
                         Manifest / Hash / CI
```

图中 Protobuf / MsgPack / Binary 分支为规划项，不在当前 `format =` 支持
范围内（构建会以 `unsupported target format` 退出码 2 拒绝）。

## Source 与 Target 之外的 Core

Cage 的核心抽象不是某一对格式之间的转换：

```text
Excel -> JSON
```

而是中间这条链：

```text
                 Any Source
                     |
                     v
               Canonical Model
                     |
                     v
           Validate Everything
                     |
                     v
              Validated Model
                     |
                     v
               Any Target
```

Excel、CSV、YAML、JSON 只是 Source；JSON、CSV、C#、Python、Lua 只是
已实装的 Target，Protobuf 等为规划项。

真正属于 Cage Core 的，是中间这部分：

> **Schema + Canonical Model + Reference Graph + Validation Pipeline + Semantic Rules + Deterministic Build**
