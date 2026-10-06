# Cage

> Game Configuration Compiler, Validation & Transformation Framework

**Cage** 是一个面向游戏研发的通用配置编译、验证与转换框架。

它将各种**人类可维护的配置源（Authoring Sources）**解析为统一的中间配置模型，经过多层次验证、跨配置引用解析、业务语义校验与规范化处理后，再按照目标平台和运行时的需求生成不同格式的配置资产或代码。

Cage 不绑定 Excel，也不绑定 JSON。

Excel 只是 Cage 的一种输入源；JSON、YAML、CSV、数据库、表格服务以及未来其他配置源，都可以通过 Source Adapter 接入。

---

## 1. 定位

Cage 解决的问题不是：

> Excel 转 JSON

而是：

> **如何将游戏开发过程中分散、异构、面向人的配置数据，可靠地编译成经过验证、确定性构建、可供不同运行时直接消费的配置资产。**

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

---

# 2. 为什么需要 Cage

游戏项目中的配置通常同时服务于：

- 策划
- 程序
- 客户端
- 服务端
- 工具链
- CI/CD
- 测试环境
- 运营

不同角色需要不同的表达形式。

例如同一份 Item 配置：

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

---

# 3. 核心概念

Cage 将配置系统划分为六个核心概念：

```text
Source
Schema
Model
Validation
Transform
Artifact
```

## 3.1 Source

配置的输入来源。

例如：

```text
Excel
CSV
JSON
YAML
TOML
XML
SQLite
Database
Remote API
Google Sheets
Custom Format
```

Source 只负责：

> **把外部数据读取进 Cage。**

它不应该负责业务验证。

---

## 3.2 Schema

定义配置的结构和约束。

例如：

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

Schema 定义：

- 字段
- 类型
- 必填
- 默认值
- 范围
- 枚举
- 数组
- 对象
- 唯一性
- 引用
- 输出信息

---

## 3.3 Canonical Model

所有 Source 都应该进入统一的中间模型。

例如：

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

Canonical Model 不应该直接等同于 JSON。

推荐基本类型：

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

---

# 4. Source Adapter

Source Adapter 是 Cage 的输入插件。

架构：

```text
                  Cage Core
                     |
        +------------+-------------+
        |            |             |
     Excel         JSON          YAML
        |            |             |
        +------------+-------------+
                     |
              Canonical Model
```

第一阶段可以支持：

```text
Excel
CSV
JSON
YAML
```

以后扩展：

```text
XML
TOML
SQLite
MySQL            ← 已实装（§45，S2 cage-source-db）
PostgreSQL       ← 已实装（§45，S2 cage-source-db）
Google Sheets    ← 已实装（§45，S3 cage-source-sheets）
HTTP API         ← 已实装（§45，S1 cage-source-http）
Custom Binary
```

重要原则：

> Source Adapter 只负责读取和解析，不负责游戏业务逻辑。

---

# 5. Excel

Excel 是游戏行业非常常见的 Authoring Source，但它不是 Cage 的核心抽象。

例如：

```text
Item.xlsx
Monster.xlsx
Skill.xlsx
Quest.xlsx
```

Excel Adapter 可以处理：

- Workbook
- Worksheet
- Header
- Cell
- Row
- Column
- Merged Cells
- Formula
- Comment
- Cell Type
- Sheet Metadata

最终转换成：

```text
Document
  |
  +-- Table: Item
  +-- Table: Monster
  +-- Table: Skill
```

---

# 6. Schema 与 Source 解耦

不要把 Schema 固定写进 Excel。

例如：

```text
Excel
   |
   +---- Source Adapter
   |
   v
Canonical Model
   ^
   |
Schema
```

这样未来可以：

```text
Excel + Schema
JSON + Schema
YAML + Schema
CSV + Schema
```

同一个 Schema 可以用于多个 Source。

---

# 7. Validation Pipeline

Cage 的核心不是 Transform，而是 Validation。

推荐将验证拆成多个阶段。

```text
Parse
  |
  v
Schema
  |
  v
Type
  |
  v
Value
  |
  v
Table
  |
  v
Reference
  |
  v
Semantic
  |
  v
Game Rules
```

---

# 8. Syntax Validation

首先检查输入是否可以被正确解析。

例如：

```yaml
foo:
    - a
      - b
```

YAML 本身非法。

Cage：

```text
ERROR E0001

Source:
    config/test.yaml

Location:
    line 3

Invalid YAML syntax.
```

---

# 9. Schema Validation

检查配置结构是否符合 Schema。

例如缺少字段：

```text
ERROR E1001

Item[10001]

Missing required field:
    price
```

未知字段：

```text
ERROR E1002

Item[10001]

Unknown field:
    pric
```

---

# 10. Type Validation

例如：

```text
Level = "abc"
```

Schema：

```yaml
type: uint32
```

错误：

```text
ERROR E1101

Item[10001].Level

Expected:
    uint32

Actual:
    string

Value:
    "abc"
```

---

# 11. Value Validation

类型正确也不代表值合理。

例如：

```yaml
level:
  type: uint32
  min: 1
  max: 100
```

数据：

```text
level = 999
```

结果：

```text
ERROR E1201

Item[10001].level = 999

Allowed range:
    1..100
```

支持：

```text
min
max
min_length
max_length
regex
enum
unique
required
```

---

# 12. Table Validation

检查单张表内部的一致性。

例如：

```text
ID 必须唯一
```

数据：

```text
10001
10002
10001
```

结果：

```text
ERROR E1301

Duplicate primary key:

Item.ID = 10001

Rows:
    2
    4
```

还可以检查：

```text
组合唯一
字段组合约束
排序要求
空值规则
```

---

# 13. Reference Validation

游戏配置中最重要的能力之一。

例如：

### Item

```text
ID
10001
10002
10003
```

### Monster

```text
ID       DropItemID
20001    10001
20002    10002
20003    99999
```

Schema：

```yaml
DropItemID:
  type: uint32

  reference:
    table: Item
    field: ID
```

Cage：

```text
ERROR E1401

Monster[20003].DropItemID

Value:
    99999

Reference:
    Item.ID

Target does not exist.
```

---

# 14. Reference 不应该只验证“存在”

例如：

```text
Monster.DropItemID
        |
        v
Item.ID
```

Item 存在：

```text
Item[10001]
```

但：

```text
Item[10001].Type = QuestItem
```

而 Monster 掉落只允许：

```text
Weapon
Armor
Consumable
```

这时候应该继续报告：

```text
ERROR E1410

Monster[20001].DropItemID = 10001

Referenced Item exists,
but its type is not valid for Monster drops.

Actual:
    QuestItem

Allowed:
    Weapon
    Armor
    Consumable
```

因此 Reference 系统应该支持：

```text
Existence
Type
Predicate
Cardinality
Compatibility
```

---

# 15. Configuration Dependency Graph

Cage 应该建立配置依赖图。

例如：

```text
Monster
   |
   | DropTableID
   v
DropTable
   |
   | ItemID
   v
Item
```

最终：

```text
Monster
   |
   +----> DropTable
              |
              +----> Item
```

这个依赖图可以用于：

- 引用检查
- 构建顺序
- 增量构建
- 变更影响分析
- 删除检查
- 循环依赖检测
- 调试

---

# 16. Semantic Validation

Schema 解决的是结构问题。

Semantic Validation 解决：

> 数据组合起来有没有意义。

例如：

```text
MinLevel <= MaxLevel
```

```text
BuyPrice >= SellPrice
```

```text
Skill.Cost >= 0
```

```text
Monster.DropTableID 必须存在
```

可以定义表达式：

```yaml
rules:
  - name: level_range
    assert: min_level <= max_level

  - name: price_range
    assert: sell_price <= buy_price
```

---

# 17. Game Rule Validator

复杂的游戏业务逻辑不应该全部塞进 Schema DSL。

应该支持插件：

```text
Schema
Expression
Code Validator
```

例如：

```cpp
class Validator {
public:
    virtual void validate(
        const ConfigContext& context,
        const Document& document,
        Diagnostics& diagnostics
    ) = 0;
};
```

游戏项目可以实现：

```text
ItemValidator
SkillValidator
MonsterValidator
QuestValidator
DropTableValidator
MapValidator
```

这样 Cage Core 不需要理解具体游戏业务。

## 17.1 插件沙箱方案定稿

三种候选执行模型的对比与选择：

| 维度 | A. 进程内 trait | B. 动态库（C ABI shim） | C. 脚本沙箱（WASM/Rhai/Lua） |
| --- | --- | --- | --- |
| 类型安全 | 全 Rust 类型，零 ABI 层 | 需 C ABI shim（Rust trait 对象不能跨 dylib 边界），数据序列化过界 | 宿主 API 需专门投影，值来回转换 |
| 部署 | 换插件需重编宿主 | 编译期解耦，插件独立交付、按项目组合 | 随配置分发，无需编译 |
| 隔离性 | 无（可信代码） | 无（加载即信任） | 资源限额/能力裁剪，可跑不可信代码 |
| 性能 | 原生 | 原生 + 一次过界序列化 | 慢一个量级起 |
| 失败模式 | panic 直接暴露 | 边界捕获 panic/超时（E1603） | 沙箱超限可杀 |
| 实现成本 | 已具备 | 中（shim + 加载器 + 错误映射） | 高（运行时嵌入 + API 设计） |

决策（分层，不二选一）：

1. **现在：A（进程内 trait）**。trait `GameRuleValidator` + 注册表
   `GameRuleRegistry` 已实装，内建样例规则 `power_curve`
   （任一表同时含 `level`/`attack` 字段时校验
   `attack <= level * 100 + 50`）端到端跑通：
   `cage check --level gamerule` 加载 → 校验 → 输出 `E1601` 诊断
   （行级定位 + hint 带计算过程）。嵌入方（游戏服务器/工具链）同样
   以 `registry.register(...)` 注入自己的规则。
2. **第三方分发：B（动态库 + C ABI shim）**。游戏团队用自己的语言
   栈写插件，编译成 dylib；宿主经 shim 加载后适配回同一个 trait。
   错误码已预留：`E1602`（插件未找到）、`E1603`（panic/超时）、
   `E9904`（加载失败）。信任模型与架构页一致：**本地可信代码**。
3. **不可信代码（Registry/社区规则）：不执行**。真出现托管运行需求
   时再评估 C，优先 WASM（能力裁剪 + 资源限额成熟）；不进本阶段。

trait 即稳定契约——A/B/C 三层共用同一接口，从进程内迁移到动态库
不改业务插件代码，只换装箱方式。

---

# 18. Validation Level

建议定义：

```text
L0 Parse
L1 Schema
L2 Type
L3 Value
L4 Table
L5 Reference
L6 Semantic
L7 Game Rule
```

但这些不是必须全部执行。

可以：

```bash
cage check
```

执行完整验证。

也可以：

```bash
cage check --level schema
```

只做 Schema 层。

---

# 19. Diagnostics

Diagnostics 应该是一等公民。

每个错误至少包含：

```text
Code
Severity
Source
Location
Table
Row
Column
Field
Value
Message
Hint
```

例如：

```text
ERROR E1401

File:
    monster.xlsx

Sheet:
    Monster

Cell:
    G27

Row:
    Monster[20003]

Field:
    DropItemID

Value:
    99999

Reference:
    Item.ID

Message:
    Target does not exist.

Hint:
    Add Item[99999] or change DropItemID.
```

支持：

```text
ERROR
WARNING
INFO
```

这样可以直接被：

- CLI
- IDE
- CI
- Web UI

消费。

---

# 20. Normalize

验证通过后进入规范化。

例如：

```text
"100"
100
100.0
```

统一：

```text
UInt32(100)
```

布尔值：

```text
yes
YES
true
1
```

统一为：

```text
Bool(true)
```

Normalize 的目标：

> 相同语义的数据应该产生相同的 Canonical Representation。

---

# 21. Transform

Transform 负责：

> Canonical Model → Target Artifact

而不是重新验证数据。

架构：

```text
                  Valid Model
                       |
       +---------------+---------------+
       |               |               |
      CSV             JSON             Code
                                       |
                              +--------+--------+
                              |        |        |
                             C#      Python    Lua
```

---

# 22. Template Target

Target Generator 也是插件。数据与代码 target 清单不变：

第一阶段：

```text
CSV
JSON
C#
Python
Lua
```

以后：

```text
TypeScript
C++
Go
Protobuf
MessagePack
FlatBuffers
Binary
SQLite
```

## 模板化形态（G 系列，2026-10 立项）

Tera（Jinja 风格，过滤器 / 继承 / 宏）统一官方与用户自定义的代码生成
面（新 crate `cage-target-template`）：

- **IR 整体作模板变量**：Schema 全量序列化——tables / fields / 类型 /
  描述 / 默认值模板内全部可引用——外加顶层 `schema_hash`；逐表模板另获
  当前 `table` 变量。字段与表的迭代序 = schema 声明序（serde_json
  preserve_order 显式声明，与各语言官方生成器同口径，不随依赖图特征
  统一漂移）
- **模板文件名即输出文件名模板**：`{table}.py.tera` 按表名序每表一
  文件，不含 `{table}` 的模板全局渲染一次（输出名 = 文件名去 `.tera`）
  ——沿用 `file_template` 的 `{table}` 占位符口径
- **模板内不写逻辑**：命名约定（snake_case / camelCase / PascalCase）
  与各语言类型映射 / 默认值字面量做成 Tera filter，决策留在 Rust（原
  plan 层的活换了个挂点，不搬进模板）
- **确定性**：产物顺序 = 模板注册序（文件系统模式按路径名序、官方内存
  模板按传入序）× 表名序，Tera workspace 锁版，模板随源码——同 schema
  + 同模板逐字节一致；官方模板改写后现有测试逐字节不变为验收锚

实装状态：

- G1 模板引擎接入：已交付——`cage-target-template`（Tera 实例封装、
  IR context 桥、文件名映射两渲染形态、命名约定三过滤器首落、
  from_config 读 `options.template_dir`、10 单测）
- G2 官方模板改写：推进中——引擎新增 `generate_official`（内存模板 +
  双 hook：`setup` 注册语言过滤器、`extras` 按 context 合并语言预计算
  决策，产出顺序 = 传入序 × 表名序）；Lua / C# / Python 已改写
  （`templates/table.{lua,cs,py}.tera` + `enums.{lua,cs,py}.tera` 随包
  `include_str!`，各 18 测试逐字节不变），余
  TS/JS / C++ / Go / Java 未开工
- G3 自定义模板加载：未开工——CLI 接线（target 配置 `template_dir`，
  `.cage/templates/` 惯例位置）+ 模板渲染错误通道（现役
  `code_target_items` 是不可失败口径，需开 Result 分支）
- G4 过滤器库：未开工——类型映射 / 字面量 / 排序过滤器成库 + 文档表
- G5 文档收口：未开工——§22/§23 与实装对账复查、target.md 模板小节
  （模板变量表 / 过滤器表 / 自定义指南）、需求整理.md 状态

---

# 23. Data Serialization 与 Code Generation

这两类 Target 应明确区分。

## Data Targets

```text
JSON
CSV
YAML
MessagePack
Protobuf
FlatBuffers
Binary
```

## Code Targets

```text
C#
Python
Lua
TypeScript / JavaScript
C++
Go
Java
```

例如同一份数据：

```text
Canonical Model
      |
      +---- JSON
      |
      +---- CSV
      |
      +---- C#
      |
      +---- Python
      |
      +---- Lua
      |
      +---- TypeScript / JavaScript
      |
      +---- C++ / Go / Java
```

### 代码生成方式：plan → render → verify 直渲染（现役；G2 起官方迁移随包模板）

Code Target 的生成器不构建目标语言的 AST，也不依赖模板动态能力，
而是 **plan → render → verify** 三层的直接字符串渲染（模板化形态与
其分工见 §22 Template Target：G2 起官方生成器的 render 层改为随包
官方模板，决策仍在 Rust）：

```text
Schema
  |
  v
plan（mini-IR：一次算完所有决策——标识符、类型、import、默认值字面量）
  |
  v
render（String + writeln 逐行渲染，多处产物共享同一份 plan）
  |
  v
verify（dev-only：tsc / javac / g++ / gofmt 回验产物，不进 CI 依赖）
```

为什么不是各语言官方 AST 库 + printer：

- **构建依赖**：TypeScript compiler API / javac TreeMaker / go/ast 意味着
  一个纯 Rust workspace 得背上 Node / JDK / Go 工具链
- **确定性死穴**：AST printer 跟随版本演进，格式化器升级即输出字节变化，
  manifest 哈希 / golden 字节比对 / 增量构建跳过全部失效
- **API 错位**：这些 API 为改写已有代码（重构、rename）设计，手工拼声明
  反而更繁琐（javac JCTree 造一个字段声明远贵于写一行文本）

为什么不是模板引擎（Handlebars/Tera 一类）：声明式绑定的输出面固定
（约十种语句形状），模板的动态能力用不上，而转义与分支逻辑下沉到模板
里反而失去 Rust 类型检查；配置中的 `file_template = "{table}.ts"` 只是
文件名占位符，与代码模板无关。

> 决策更新（2026-10，G 系列，见 §22）：上面对「生成器内部结构」的
> 结论仍然成立——plan → render → verify 三层不动，决策（标识符 /
> 类型 / import / 默认值字面量）留在 Rust；但对「谁能改生成的文本」
> 不再成立：用户改模板不改代码的需求出现后，模板的动态能力（循环 /
> 分支 / 宏 / 继承）正是该场景要的。新口径是官方与自定义共用一个
> Tera 引擎（`cage-target-template`）：官方各语言绑定改写为随包官方
> 模板（G2），用户自定义模板经 `template_dir` 接入（G3），语言类型
> 映射与字面量做成 filter（G4），**模板内不写逻辑**——上面担心的
> 「分支逻辑下沉模板」用「决策留 Rust、模板只表达文本形状 + golden
> 逐字节锚定」挡住了。确定性锚不放松：Tera workspace 锁版 + 模板随
> 源码 + 现有 golden 测试字节不变。`file_template` 的占位符口径原样
> 沿用（模板文件名 `{table}.py.tera` 是同一套 `{table}` 替换）。

可靠性的失效模式分析：生成物是纯声明代码（字段、类型、默认值），没有
控制流——要么编译不过（当场暴露），要么就是对的；不存在「编译通过但
运行时悄悄出错」的中间态。因此语法正确性由 dev-only 的真编译器回验保证
（`tsc --strict`、`javac -Xlint:all -Werror`、`g++ -Wall -Wextra -Werror`、
`gofmt -l`，均在对抗性 schema——关键字字段名、`i64::MIN`、非 ASCII、
命名冲突——上实测通过），而非由构造层保证。业界同类先例：protoc 的
Java/C++ 生成器内部同样是字符串拼接。

升级信号：当某个 target 需要生成带逻辑的代码（内联校验函数、复杂
runtime 支撑码）或改写用户已有代码时，为该 target 单独引入 IR 层——
插件架构下这是局部决定，不影响其余 target。该信号与「用户改模板」
的需求合并，已落地为 Template Target（§22，G 系列）。

---

# 24. Frontend / Backend Profiles

不要把“客户端”和“服务端”写死在 Core。

可以定义 Build Profile：

```yaml
profile: client

targets:
  - json
  - csv
  - csharp
```

服务端：

```yaml
profile: server

targets:
  - json
  - csv
  - python
  - lua
```

然后：

```bash
cage build --profile client
cage build --profile server
```

---

# 25. Field Visibility

不同目标可能不需要全部字段。

例如：

```yaml
fields:

  id:
    type: uint32

  name:
    type: string

  admin_note:
    type: string
    targets:
      - server
```

客户端：

```text
id
name
```

服务端：

```text
id
name
admin_note
```

这样可以避免：

> 为客户端和服务器维护两套配置。

---

# 26. Build Manifest

每次构建生成 Manifest：

```json
{
  "project": "game",
  "profile": "client",
  "cage_version": "0.1.0",
  "schema_hash": "...",
  "source_hash": "...",
  "content_hash": "...",
  "artifacts": {
    "item.json": "...",
    "monster.json": "...",
    "skill.json": "..."
  }
}
```

用途：

- 版本追踪
- 部署
- 回滚
- 客户端/服务端版本匹配
- CI
- 缓存
- 增量构建

---

# 27. Deterministic Build

相同：

```text
Source
Schema
Cage Version
Profile
```

应该得到相同：

```text
Artifact
Hash
Manifest
```

即：

```text
Build(A) == Build(A)
```

避免：

- 时间戳导致文件变化
- 不确定 Map 顺序
- 随机 ID
- 非稳定排序

这是配置构建系统的重要基础。

---

# 28. Incremental Build

有了 Dependency Graph 后，可以支持增量构建。

例如：

```text
Item.xlsx 修改
```

影响：

```text
Item
 |
 +--> DropTable
 |
 +--> Monster
```

那么：

```text
Skill
Quest
Map
```

无需重新生成。

最终：

```text
Changed Sources
      |
      v
Dependency Graph
      |
      v
Affected Tables
      |
      v
Incremental Build
```

这属于后期能力。

---

# 29. Configuration Registry

后期可以增加远程配置仓库：

```text
Cage Registry
```

存储：

```text
Schema
Source Metadata
Build Manifest
Artifacts
Hash
Version
```

例如：

```text
game-config/
    v1.0.0/
    v1.1.0/
    v1.2.0/
```

但 Registry 不应该进入 MVP。

**实装（第三阶段 R 系列）**：R1 本地注册表已落地——`<registry>/<包>/<版本>/`
即一枚自校验 Configuration Snapshot（与 `cage snapshot` 同一打包产物、
同一 blake3 账本），包目录附确定性 `index.json`；`cage registry publish`
发布（同版本同字节幂等、异字节 E1801 冲突）、`cage registry list` 列表、
消费方 `source_roots: registry:<包>[@<版本>]` 解析（载入前账本校验，
E1802 未解析 / E1803 校验失败）。R2 Schema 侧解析与依赖 pin 已落地——
`schema_path: registry:<包>[@<版本>]` 直接读条目 `schema.json`（发布时的
profile 投影 schema），`[dependencies]` 为包声明版本 pin（比较符区间
AND、`^`/`~` 展开；省略 `@版本` 取满足区间的最高版本，显式 `@版本`
也必须落在 pin 内，违者 E1802）。R3 远程只读解析已落地——`[registry]
path` 支持 http(s) 根，协议为匿名 GET 三资源（包 index、条目账本、条目
文件），条目字节不可变；解析 = 取 index → 按同一 `select_version` 选版本
→ 按账本逐文件下载并逐字节校验 blake3 → 落 `.cage-cache/registry/
<url 指纹>/` 缓存 → 过 `verify_snapshot` 信任门（未经校验不载入）；
缓存复验干净则离线复用。远程根只读：publish/list 拒绝（协议无包枚举；
发布在本地注册表完成后由静态服务器托管）。鉴权（token/签名）留待后续
独立立项。R4 回滚与清理已落地——`cage registry verify` 全册审计
（逐条目重过账本校验 + index 记录与条目账本交叉核对 + 孤儿目录报告，
E1803 逐条列出）、`cage registry gc` 滚动窗口策略（每包保留最新
`--keep N` 个版本、下限 1——窗口即回滚面，pin 住旧版本的消费方仍可
解析，被窗口挤出的版本对消费方 E1802；孤儿目录一并清扫，--dry-run
报告同一清单不落笔）、`cage registry remove` 显式行政移除（版本目录
带 index 记录一并删、包 index 保留，同字节可重新 publish 干净入册；
注册表自身绝不隐式改写历史）。红线不变：Registry 不执行上传的
Validator（§35）。

---

# 30. CLI

推荐核心 CLI：

```bash
cage check
cage build
cage inspect
cage diff
cage verify
cage graph
```

## check

```bash
cage check config/
```

只验证，不生成 Runtime Artifact。

## build

```bash
cage build config/
```

验证并生成目标。

## inspect

```bash
cage inspect Item
```

查看 Schema 和配置结构。

## diff

```bash
cage diff build/a build/b
```

比较配置版本。

## verify

```bash
cage verify runtime/
```

验证已经生成的 Artifact。

## graph

```bash
cage graph
```

输出配置依赖图。

---

# 31. CI/CD

Cage 应该天然适合 CI。

```text
Git Push
    |
    v
CI
    |
    v
cage check
    |
    +---- Error ---> Build Failed
    |
    v
cage build
    |
    v
Artifacts
    |
    v
Package / Deploy
```

例如：

```bash
cage check --profile client
cage check --profile server

cage build --profile client
cage build --profile server
```

配置错误在 CI 里先失败，不带进游戏。

---

# 32. Warning Policy

建议支持：

```text
warning
error
```

并允许 CI 配置：

```yaml
ci:
  warnings_as_errors: false
```

生产构建：

```yaml
ci:
  warnings_as_errors: true
```

---

# 33. Project Structure

推荐：

```text
cage/
├── README.md
├── LICENSE
├── CMakeLists.txt
│
├── docs/
│   ├── architecture.md
│   ├── source.md
│   ├── schema.md
│   ├── validation.md
│   ├── transform.md
│   ├── targets.md
│   ├── cli.md
│   └── build.md
│
├── src/
│   ├── core/
│   │   ├── value/
│   │   ├── document/
│   │   ├── schema/
│   │   ├── diagnostics/
│   │   ├── validation/
│   │   ├── reference/
│   │   ├── dependency/
│   │   ├── normalize/
│   │   └── manifest/
│   │
│   ├── source/
│   │   ├── excel/
│   │   ├── csv/
│   │   ├── json/
│   │   └── yaml/
│   │
│   ├── target/
│   │   ├── csv/
│   │   ├── json/
│   │   ├── csharp/
│   │   ├── python/
│   │   └── lua/
│   │
│   ├── validator/
│   │   └── ...
│   │
│   └── cli/
│
├── schemas/
├── examples/
├── tests/
└── plugins/
```

---

# 34. Plugin Model

Cage 的核心应该尽量稳定。

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

---

# 35. Security / Isolation

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

---

# 36. MVP

第一版不要试图解决所有问题。

建议 MVP：

### Source

```text
Excel
JSON
YAML
CSV
```

### Schema

```text
type
required
default
enum
min
max
unique
reference
```

### Validation

```text
Syntax
Schema
Type
Value
Unique
Reference
Expression
```

### Target

```text
JSON
CSV
```

### CLI

```text
check
build
diff
inspect
```

### Build

```text
Deterministic
Manifest
Hash
```

这已经足够形成真正有价值的基础设施。

---

# 37. 第二阶段

加入：

```text
C#
Python
Lua
Protobuf
MessagePack
```

以及：

```text
Plugin SDK
Game Rule Validator
Dependency Graph
Incremental Build
CI Integration
```

---

# 38. 第三阶段

再考虑：

```text
Web UI                 ← 已实装（W 系列）
Schema Editor          ← 已实装（W 系列）
Configuration Registry ← 已实装（R1–R4）
Remote Source          ← 三源已实装（S1–S3：HTTP / DB / Sheets）（§45）
Google Sheets          ← 已实装（§45，S3 cage-source-sheets）
Database Source        ← 已实装（§45，S2 cage-source-db）
Migration
Artifact Distribution
```

---

# 39. 不应该做的事情

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

不绑定：

```text
Unity
Unreal
Godot
Cocos
```

它应该服务于所有游戏客户端和服务器。

---

# 40. 与普通配置转换器的区别

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

所以 Cage 的价值不是：

> **转换。**

而是：

> **建立一条可靠的配置编译链。**

---

# 41. Casino Cage 隐喻

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

所以它不是：

> “Excel 转换器”

而更像：

> **配置进入运行时世界之前的兑换与清算边界。**

赌场里的 Cage 不关心你最后玩：

```text
Poker
Blackjack
Baccarat
```

同样，Cage 不应该关心配置最终服务：

```text
Unity
Cocos
Unreal
Game Server
Tool
```

它只负责：

> **输入的数据必须合法、完整、可验证，然后才能兑换成运行时资产。**

---

# 42. 一句话定义

英文：

> **Cage is a configuration compiler and validation framework for game development. It transforms heterogeneous authoring data into validated, deterministic runtime artifacts.**

中文：

> **Cage 是一个面向游戏研发的配置编译与验证框架，将异构的配置源转换为经过验证、确定性构建的运行时配置资产。**

---

# 43. 核心设计原则总结

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

---

# 44. 最终架构

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
                      | Game Rules             |
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

---

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

这也是这个项目最值得做成独立开源基础设施的部分。


---

# 45. Remote Source

MVP 的四个 Source 都读本地文件。策划数据真正住在线上系统的场景有三个：
数值表放在 Google Sheets，运营与账号数据在业务库（MySQL /
PostgreSQL），工具链把一部分表开放成了 JSON 接口。Remote Source 把这
三类输入接进同一条编译链，连同 HTTP API 共四源。四源一律**只读**。

## 定位与红线

- 远端是数据源，不是可信源。取回的字节与本地文件走同一条
  Parse → Schema → … → Game Rule 流水线，L0-L7 全量执行，没有旁路。
  「未经校验不载入」对远端字节同样成立（与 R3 缓存复验同一口径）。
- 只读。不做写回、不跑 DDL/DML；配置数据的编辑仍在作者侧工具完成，
  `cage web` 的保存面不变。
- 不执行远端代码。不调用存储过程、不 eval 响应内容，与 §35 的
  Validator 红线同款。
- 凭据不入 cage.toml。配置文件只写环境变量名，连接串与密钥在运行时
  从 env 解析；env 未设置直接报错（E1904），不猜、不落日志。

## 源句法与映射

四源走 `[source_roots]` 的 scheme 声明，与 `registry:` 前缀同一风格。
`http(s) URL` 出现在 `[source_roots]` 是 HTTP API 源，出现在
`[registry].path` 是注册表根，两者不混用：

```toml
[source_roots]
items    = "https://api.example.com/v1/items.json"   # HTTP API：GET JSON
monsters = "mysql:Monsters"          # MySQL：表名或具名查询
drops    = "pg:drop_tables"          # PostgreSQL
levels   = "gsheet:1AbC...xz/Levels" # Sheets：spreadsheet_id / tab

[remote.mysql]
dsn_env = "CAGE_MYSQL_URL"           # 只存 env 名，DSN 不进仓库

[remote.pg]
dsn_env = "CAGE_PG_URL"

[remote.gsheets]
credential_env = "CAGE_SHEETS_CREDENTIAL"   # API key（service account 需
                                            # OAuth JWT 交换，留待实现期）
```

| 源 | 取数 | 表 / 行映射 | 类型口径 |
| --- | --- | --- | --- |
| HTTP API | 一次 GET，响应体即表 | 与本地 JSON 源同一形状：`{表名: 行数组}` / 单对象 → `Root` / 行数组 → `Data`；字段名 = 字段 | JSON 值类型直接映射 |
| MySQL / PostgreSQL | 只读 SELECT，表名展开为 `SELECT * FROM t`，具名查询放 `[remote.<scheme>.queries]` | 列名 = 字段，NULL = Null，行 = 记录 | DECIMAL / NUMERIC 渲染为字符串，不走 Float（浮点丢精度，怎么解释交 Schema） |
| Google Sheets | Sheets API v4 `values`（`gsheet:<spreadsheet_id>/<tab>`），`valueRenderOption=UNFORMATTED_VALUE`（公式缓存值，不重算，同 Excel adapter 口径）；API key 从 `[remote.gsheets].credential_env` 指名的 env 读（E1904），错误诊断只引 spec 不引 URL（key 不落日志） | tab = 表，首行 = 表头（空表头单元格退 `col<i>`，同 Excel 惯例），空行跳过、短行补 null 对齐表头宽，tab 自然行序 = 作者承诺序原样保留；非行集 / 缺表头拒载（E1903） | 初值为字符串（数字 / 布尔保留 JSON 文本），L2/L3 校验裁型 |

具名查询与表名在装载期做静态校验：只接受单条以 `SELECT` 开头的语句，
分号、注释、多语句、行锁子句（`FOR UPDATE` / `FOR SHARE`）与
`INTO` 一律拒（E1905）；运行期连接设为只读事务（PostgreSQL
`default_transaction_read_only`、MySQL `SESSION TRANSACTION READ
ONLY`），双保险。行序确定性：语句自带 `ORDER BY` 则尊重作者承诺的
顺序；否则行按其序列化形式排序——构建不依赖服务端返回顺序。行集
序列化成 canonical JSON 后落缓存（行对象键序 = SELECT 列序，与
CSV / Excel / 本地 JSON 同一顺序口径；DECIMAL / NUMERIC 文本保真、
二进制列 base64），再走标准 JSON 解析——与 HTTP 源同一缓存锚与
解析链。缓存键覆盖 scheme + DSN（单向哈希）+ 语句，不同
服务器 / 查询永不共槽。Sheets 源同锚同链：canonical JSON 落
`.cage-cache/source/<gsheets+id+tab 指纹>/`（API key 不进指纹——
它不改变字节语义），tab 自然行序即作者承诺序原样保留。

## 确定性与缓存

确定性构建的锚点是**取到的字节**，不是「远端的当前状态」。响应字节
原样落缓存，解析只吃这份字节，不存在「边取边算」：

```text
fetch
   |
   v
字节落 .cage-cache/source/<源指纹>/<指纹>.json
   |
   v
同一套 Parse / 校验流水线（无旁路）→ Canonical Model
   |
   v
内容进 source_hash → build_id → 产物
```

source_hash 覆盖的是解析后的 Canonical Model 内容（表名 / 行序 /
主键 / 字段，`manifest::hash_source` 同一把尺子）。远端数据变了，
解析出的内容就变，source_hash 跟着变，build_id 旋转——变化在
manifest 里看得见，不存在「悄悄换了数据」；同字节重复构建仍逐字节
一致，golden 契约不破。源指纹复用 R3 的 `cache_key` 口径（URL 的
blake3 前 12 hex），取数、重试、缓存路径抽一处共享 helper
（`cage_core::remote`，R3 registry 与各源适配器同源复用），不搞四份
实现。

断网语义与 R3 对齐：取不到远端时回退缓存并发 WARNING 诊断，缓存字节
同样先过校验门才可用；`--no-cache` 关闭回退，取不到即失败（回退在
S4 收口，S1 当前取不到即失败）。新鲜度上限（max_age）留待实现期。

## 错误码（E19xx 族）

| 代码 | 含义 | 状态 |
| --- | --- | --- |
| `E1901` | 远端取数失败（网络 / DNS / 超时重试用尽、404 等非认证错误状态、DB 连接 / 语句失败） | 已实装（HTTP / DB 源） |
| `E1902` | 认证 / 授权被拒（HTTP 401 / 403） | 已实装（HTTP / Sheets 源） |
| `E1903` | 响应形状不合法（非行集 / 缺表头） | 已实装（Sheets 源形状门） |
| `E1904` | 凭据缺失（env 未设置或凭据文件不可读） | 已实装（DB 源 dsn_env、Sheets 源 credential_env） |
| `E1905` | 查询非法（配置了非只读语句） | 已实装（DB 源：SELECT 白名单 + 表名校验） |

E1901–E1905 已全族注册进 `codes.rs` 与 validation.md 并全部接线生效
（E1901/E1902 随 S1、E1904/E1905 随 S2、E1903 随 S3）。HTTP 源的
坏 JSON 不走 E1903——它走与本地文件同一条 Parse 诊断（E0001 带行列
定位）；DB 源的行集由适配器自产 canonical JSON，形状不可能非法，
同样不经 E1903；E1903 由 Sheets 源的形状门消费（非行集 / 缺表头 /
majorDimension 非 ROWS）。

重试只覆盖连接类失败（有界次数 + 退避，口径同 ci.yml 的 curl 重试）；
4xx 不重试。

## 留待实现期（不在首期）

- 增量拉取：按 revision / updated_at 只取变更行
- OAuth 用户授权流、mTLS 与 service account 凭据（Sheets 首期只
  API key——service account 需 OAuth JWT 交换）
- 连接池、并发多源、大表游标分页
- Sheets 富文本与公式重算（首期只缓存值）
- HTTP 分页协议与限流协商
- 从库表内省自动生成 Schema 草稿

**实装状态**（2026-10）：S1 已交付——HTTP API 源实装
（`cage-source-http`，`[source_roots]` 直写 http(s) URL），共享取数 /
重试 / 缓存 helper 首落 `cage_core::remote`（R3 registry 同源复用）。
S2 已交付——MySQL / PostgreSQL 源实装（`cage-source-db`，两后端共享
行集映射：SELECT 白名单 E1905 → dsn_env 解析 E1904 → 连接 + 会话
只读 pin → 行集 canonical JSON 落缓存 → 标准 JSON 解析；CLI 错误码
E1901 / E1904 / E1905 接线生效）。S3 已交付——Google Sheets 源实装
（`cage-source-sheets`，`gsheet:<id>/<tab>`，UNFORMATTED_VALUE、
首行表头同 Excel 惯例、初值字符串口径、tab 自然行序保留，API key
经 credential_env，E1902 / E1903 / E1904 接线生效；service account
留待实现期）。缓存复用回退与 `--no-cache` 严格模式在 S4 收口（当前
取不到远端即失败）。S4–S6 未开工。
