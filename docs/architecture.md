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
│   └── cage-cli/            # cage 命令行（check / build / inspect / diff）
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
