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

配置的输入来源，例如 Excel、CSV、JSON、YAML，后续扩展 XML、TOML、SQLite、数据库、Remote API、Google Sheets 等。

Source 只负责：

> **把外部数据读取进 Cage。**

它不应该负责业务验证。详见 [Source 栏目](/source)。

### Schema

定义配置的结构和约束，例如：

```yaml
table: Item

primary_key: id

fields:
  id:
    type: uint32
    required: true

  name:
    type: string
    required: true

  price:
    type: uint32
    min: 0

  type:
    type: enum
    values:
      - Weapon
      - Armor
      - Consumable
```

Schema 定义：字段、类型、必填、默认值、范围、枚举、数组、对象、唯一性、引用、输出信息。详见 [Schema 栏目](/schema)。

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
| Dependency Graph | 表间引用的拓扑与增量规划 | `DependencyGraph` / `IncrementalPlanner`（cage-core::reference） | Reference（L5）生产 → 增量构建消费 | 图是编译结果不是运行时数据；未接线前不参与构建决策 |
| Profile | 面向消费端的裁剪视图 | `BuildProfile`（cage-core::manifest）+ profile 过滤 | 用户配置 → 裁剪 Schema + Document → Validation / Target | 过滤是起点不是终点：语义落地面见差距表 |
| Manifest | 构建产物的账本与输入指纹 | `BuildManifest` / `ArtifactInfo`（cage-core::manifest） | ManifestGenerator 生产 → verify / 增量 / 部署 / 回滚消费 | 只记账不生成；与产物一同落盘、随产物验证 |
| Snapshot | 可独立加载的配置快照（规划中） | —（未实装，见差距表） | 构建生产 → 服务器 / 客户端启动加载校验 | 快照自带校验信息，不依赖构建机现场 |

### 为什么 IR 不拆独立类型（v0.3 决策）

评审原型设想 IR ≈ `CompiledItemTable { schema_id, table_id, fields, values,
references, normalized_types, visibility, dependencies }`。对照现状：

- normalize（cage-core/src/normalize/mod.rs：`normalize_document` /
  `normalize_typed_value` / `normalize_value`）是 Canonical → Canonical 的纯
  变换，产出仍是 Document / TypedValue / Value 类型；
- 全部 10 个 target 的入口要么是 `(Schema, Document)`（数据 target），要么是
  `(Schema, schema_hash)`（代码 target——类型与元数据全来自 Schema，数据不参与
  代码生成）。target 面已经按 IR 视角消费，且结构性满足「不得读 Source」。

现阶段拆独立 IR 类型只会引入双份形状与无谓的转换/测试面。定界：IR 与
Canonical 同构，以（归一化 Document, Schema）表达。**何时再拆**：当校验面与
产物面形状开始分歧（per-profile 裁剪固化、visibility 折叠、产物需要编译期
预计算的派生形状）时，再以 Compiled IR 收敛——拆分点已预留，不做先行设计。

### 现状与差距（逐概念核对，2026-10）

| 概念 | 现状 | 差距 / 下一步 |
| --- | --- | --- |
| Schema | 已实装：19 种字段类型（含 Map）、引用、唯一约束、字段级 `targets` 可见性、19 错误码族 | 字段可见性冲突语义（E9006）仅码表、无调用点 |
| Canonical Model | 已实装：value.rs 全类型 + SourceLocation | — |
| IR | 定界完成（见上） | 派生形状需求出现时拆 Compiled IR |
| Validation Context | 已实装：schema/mod.rs（schema / current_table / current_row / current_field …） | — |
| Dependency Graph | 核心已实装：reference/mod.rs `DependencyGraph`（环检测 / 拓扑）+ `IncrementalPlanner`，测试先行 | **构建路径未接线**：cli 构造 ValidatedSchema 时 `dependency_graph` 恒 `DependencyGraph::default()` 占位；增量第二层（按依赖传播只重建受影响表）依赖此接线 |
| Profile | 已实装第一层：表 + 字段双层面板过滤（同时裁剪 Schema 与 Document，validation 前执行） | profile 不感知校验 / 安全语义：server-only 字段只有过滤落盘、无泄漏拦截；E9006 待实装 |
| Manifest | 已实装：7 顶层字段（project / profile / cage_version / schema_hash / source_hash / content_hash / artifacts）+ artifact 级 6 字段 | 缺 build_id / dependencies / generator_version（现以 cage_version 兼任）；ir_hash 随 IR 定界省略 |
| Snapshot | 未实装 | 规划：snapshot/ = manifest + schema + 数据 + 生成物 + 校验清单，服务器启动加载即校 |

### 评审对照修正（2026-10 外部评审）

| 评审项 | 评审评级 | 代码核对结论 |
| --- | --- | --- |
| IR | ⭐⭐⭐ | 定界后归入 Canonical，见「为什么 IR 不拆独立类型」 |
| Dependency Graph | ⭐⭐ | 核心已实装（环检测 / 拓扑 / 增量规划），差构建路径接线——应读作核心 4/5、接线 0/5 |
| Incremental Build | ⭐⭐ | 第一层（整轮哈希跳过）已实装；第二层（按依赖传播）待 DG 接线 |
| Snapshot | ⭐⭐ | 未实装（规划中） |
| Profile | ⭐⭐⭐⭐ | 过滤已实装；校验 / 安全 / 可见性冲突语义为下一步 |
| Manifest | — | 字段基本齐，差 build_id / dependencies 两个账本字段 |
| Diagnostics | ⭐⭐⭐⭐⭐ | 确认：L0-L7 全错误码族、行级定位 + hint、E1601 端到端 |
| Deterministic Build | ⭐⭐⭐⭐⭐ | 确认：同输入字节一致由 golden 测试锁定 |

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
│   │       ├── validation/      # L0-L7 验证流水线
│   │       ├── reference/       # 跨配置引用
│   │       ├── normalize/       # 归一化
│   │       └── manifest/        # Build Manifest
│   ├── cage-source-excel/   # Excel 输入源（calamine）
│   ├── cage-source-csv/     # CSV 输入源
│   ├── cage-source-json/    # JSON 输入源
│   ├── cage-source-yaml/    # YAML 输入源
│   ├── cage-target-json/    # JSON 产出
│   ├── cage-target-csv/     # CSV 产出
│   ├── cage-target-cs/      # C# 代码绑定
│   ├── cage-target-py/      # Python 代码绑定
│   ├── cage-target-lua/     # Lua 代码绑定
│   ├── cage-target-ts/      # TypeScript / JavaScript 代码绑定
│   ├── cage-target-cpp/     # C++ 代码绑定
│   ├── cage-target-go/      # Go 代码绑定
│   ├── cage-target-java/    # Java 代码绑定
│   └── cage-cli/            # cage 命令行（check / build / gen / inspect / diff）
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

所以 Cage 的价值不是「转换」，而是：

> **建立一条可靠的配置编译链。**

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

所以它不是「Excel 转换器」，而更像：

> **配置进入运行时世界之前的兑换与清算边界。**

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

## 结论

Cage 最应该建立的抽象不是：

```text
Excel -> JSON
```

而是：

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

**Excel、CSV、YAML、JSON 只是 Source；JSON、CSV、C#、Python、Lua、Protobuf 等只是 Target。**

真正属于 Cage Core 的，是中间这部分：

> **Schema + Canonical Model + Reference Graph + Validation Pipeline + Semantic Rules + Deterministic Build**
