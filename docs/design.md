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
- **模板内不写逻辑**：命名约定（snake_case / camelCase / PascalCase /
  kebab_case / SCREAMING_CASE）+ `field_order` 字段名序与各语言类型映射
  / 默认值字面量做成 Tera filter（约定层恒注册，语言层经
  `options.lang_filters` 挂载），决策留在 Rust（原 plan 层的活换了个
  挂点，不搬进模板）
- **确定性**：产物顺序 = 模板注册序（文件系统模式按路径名序、官方内存
  模板按传入序）× 表名序，Tera workspace 锁版，模板随源码——同 schema
  + 同模板逐字节一致；官方模板改写后现有测试逐字节不变为验收锚

实装状态：

- G1 模板引擎接入：已交付——`cage-target-template`（Tera 实例封装、
  IR context 桥、文件名映射两渲染形态、命名约定三过滤器首落、
  from_config 读 `options.template_dir`、10 单测）
- G2 官方模板改写：已交付——引擎新增 `generate_official`（内存模板 +
  双 hook：`setup` 注册语言过滤器、`extras` 按 context 合并语言预计算
  决策，产出顺序 = 传入序 × 表名序）；7 crate 全数改写
  （`templates/table.{lua,cs,py}.tera` + `enums.{lua,cs,py}.tera`
  随包 `include_str!`，各 18 测试逐字节不变；TS/JS 一 crate 双形态
  6 模板 `table.{ts,js,d.ts}.tera` + `enums.{ts,js,d.ts}.tera`，28 测试
  逐字节不变，.js+.d.ts 交错序由两次调用按序 zip 复刻；C++
  `table.h.tera` + `enums.h.tera`，19 测试逐字节不变；Go
  `table.go.tera` + `enums.go.tera`，20 测试逐字节不变——gofmt 对齐
  数学（tabwriter 移植）留 Rust，extras 以对齐后的最终行供给模板；
  Java `table.java.tera` + `enums.java.tera`，20 测试逐字节不变 +
  javac 真机回验——路径按 class ident 重映射（file stem = class
  ident，引擎按 schema name 替换 `{table}`，产物按发射序 zip 回写））
- G3 自定义模板加载：已交付——`format = "template"` 进 CLI：
  `code_target_items` 开 Result 分支（bundled 七语言 `Some(Ok(…))`
  不可失败口径原样保留，template 分支承接 `TemplateTargetGenerator::
  from_config(target).generate(…)` 的可失败结果），build 面板模板错
  走 `BuildFailure::Io`、gen 面板 eprintln + 退码 2；`template_dir`
  默认 `.cage/templates/`，相对路径按项目根（build/gen 的 path 参数）
  解析、`options.template_dir` 可覆盖；产物 path 含 `output_dir` 前缀
  （引擎 `render_registered` 统一 `join_output`，build/gen/diff 三面板
  经 manifest 天然覆盖——diff 只比 manifest 无需接线）。错误通道增强：
  Tera 顶层 Display 只有 `Failed to render '<name>'`，crate 新增
  `tera_err` 展平 cause 链（未知变量/过滤器参数缺失等真实故障直达），
  文件系统模式 `add_template_file` 错误带模板全路径。CLI 测试 5 新增
  （惯例目录渲染 + `options.template_dir` 覆盖 + build/manifest 记录
  format "template" + 缺目录/空目录两形态 + 语法错指到模板文件 +
  两跑逐字节一致）；from_config 默认值随惯例位置收敛
  （`templates` → `.cage/templates`）
- G4 过滤器库：已交付——决策函数成库，模板里不写逻辑。语言无关层
  （`cage-target-template`，恒注册）：约定过滤器补 `kebab_case` /
  `SCREAMING_CASE`，新增 `field_order`（`fields` 映射 → 字段名序数组，
  官方各语言统一的发射序）；`generate_with_setup` 开出 setup 钩子
  （`generate` = 无钩子特例）。语言层（各 `cage-target-*` crate 暴露
  `pub fn register_filters(tera, schema[, holder])`）：七语言
  `{lang}_type`（字段对象 → 官方同款类型文本，枚举引用带分配后标识符；
  lua 的类型面是标签）+ 七语言 `{lang}_default`（字段对象 → 官方同款
  字面量，无默认 / 不可
  渲染 → null）；CLI `options.lang_filters`
  （逗号分隔，未知键渲染前报错退码 2）挂载，java 的 holder 从
  `options.enums_file` stem 推导（与 from_config 同源）。枚举分配
  重构为零行为变化提取：go `allocate_names` / cpp `allocate_names`
  （包级共享集，generate 与过滤器同源），java 过滤器侧
  `filter_enum_idents`（holder 种子集），ts 复用 `enum_exports`。
  输入约定统一字段对象（序列化 `FieldSchema`），crate 公共助手
  `field_parts` / `field_default`。测试 11 新增（七 crate
  type/default smoke + 约定过滤器与 field_order + setup 钩子 +
  CLI 挂载与未知键两形态），workspace 545 全绿；过滤器表进
  target.md Template 小节
- G5 文档收口：已交付——target.md Template 小节（配置样例 /
  输出名与渲染序约定 / IR 变量表 / 约定+语言过滤器两表 / 错误口径，
  G3/G4 随签落地本签复查）；需求整理.md 状态行 + 生成器清单 +
  crate 落地口径补 cage-target-template；architecture.md 结构树补
  cage-target-template；README Target 插件行补 Template Target；
  §22/§23 与实装对账复查（本面板 G1–G4 全数已交付，命名约定/过滤器
  清单同步 G4 实况）

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
JSON Schema（标准文档，2026-10 实装）
```

### JSON Schema Target 决策记录（2026-10）

**定了什么**：标准 JSON Schema 形态的 schema 导出走 `format =
"jsonschema"` target（`cage-target-jsonschema`，代码绑定族面板）——
每表一个**自包含** `{table}.schema.json`，draft-07 默认、2020-12 经
`options.draft` 切换；枚举一律内联 `enum` 数组（**实例恒为成员名**——
L2 把 enum 字段类型定为 string，整型字面量只是代码生成元数据，故整数
枚举也发字符串枚举）；类型化 Object 与表文档都发
`additionalProperties: false`（与校验器「未知键拒绝」同口径）；
`map<int, V>` 的键发 `propertyNames.pattern` 数字串约束（数据模型里
int 键是数字字符串）；bytes 在 draft-07 发 `contentEncoding: base64`
（2020-12 已删该关键字，不发）；文档不带自定义扩展键，跨表引用与语义
规则无标准拼写不进文档（仍由 cage 流水线在事实源执行）。确定性 =
表名序 × 固定键插入序（serde_json preserve_order），两跑逐字节一致。
**为什么**：让非 cage 工具链（编辑器、ajv、前端表单）零运行时消费
schema——这是「Schema 即资产」的链外形态；自包含单文件避免 `$ref`
解析负担；draft-07 兼容面最宽，2020-12 留升级口。**备选**：带
`$defs` + `$ref` 的合并单文档（消费方要做引用解析，弃）；共享枚举
文件（JSON Schema 无跨文件 import 标准机制，`$ref` 相对路径在分发
场景易碎，弃）；默认 2020-12（工具支持面窄于 07，只留 options）。

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
Migration              ← 已立项（2026-10 拍板，M 系列，§46）
Artifact Distribution  ← 已立项（2026-10 拍板，A 系列，§47）
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
主键 / 字段，`ManifestGenerator::input_hashes` → 私有
`hash_source` 同一把尺子）。远端数据变了，
解析出的内容就变，source_hash 跟着变，build_id 旋转——变化在
manifest 里看得见，不存在「悄悄换了数据」；同字节重复构建仍逐字节
一致，golden 契约不破。源指纹复用 R3 的 `cache_key` 口径（URL 的
blake3 前 12 hex），取数、重试、缓存路径抽一处共享 helper
（`cage_core::remote`，R3 registry 与各源适配器同源复用），不搞四份
实现。

断网语义（S4 已收口）：只有传输类失败（连接 / DNS / 超时——连接拒绝、
断网、服务不可达）回退缓存并发 `E1906` WARNING；404（远端已删除）与
401/403（访问可能已被吊销）不回退——旧字节会静默出错，保持硬失败。
DB 源的取数失败全部按传输类处理（凭据 E1904 / 查询白名单 E1905 在
任何网络触达之前发生，旧字节不掩盖配置错误）。回退出的缓存文件走与
在线路径同一条标准 JSON 解析（缓存字节同样过校验门）。三源共享
`cage_core::remote::source_cache_fallback` 一个门；WARNING 只带
spec / 表名（http 为 URL），凭据永不落日志。`--no-cache`（check /
build / gen / inspect 四命令）关闭回退，取不到即失败。新鲜度上限
（max_age）留待实现期。

## 错误码（E19xx 族）

| 代码 | 含义 | 状态 |
| --- | --- | --- |
| `E1901` | 远端取数失败（网络 / DNS / 超时重试用尽、404 等非认证错误状态、DB 连接 / 语句失败） | 已实装（HTTP / DB 源） |
| `E1902` | 认证 / 授权被拒（HTTP 401 / 403） | 已实装（HTTP / Sheets 源） |
| `E1903` | 响应形状不合法（非行集 / 缺表头） | 已实装（Sheets 源形状门） |
| `E1904` | 凭据缺失（env 未设置或凭据文件不可读） | 已实装（DB 源 dsn_env、Sheets 源 credential_env） |
| `E1905` | 查询非法（配置了非只读语句） | 已实装（DB 源：SELECT 白名单 + 表名校验） |
| `E1906` | 远端不可达但已回退上一份缓存副本（离线回退 WARNING，非致命；`--no-cache` 关闭回退还原为硬 E1901） | 已实装（三源共享 `source_cache_fallback` 门） |

E1901–E1906 已全族注册进 `codes.rs` 与 validation.md 并全部接线生效
（E1901/E1902 随 S1、E1904/E1905 随 S2、E1903 随 S3、E1906 随 S4）。
HTTP 源的
坏 JSON 不走 E1903——它走与本地文件同一条 Parse 诊断（E0001 带行列
定位）；DB 源的行集由适配器自产 canonical JSON，形状不可能非法，
同样不经 E1903；E1903 由 Sheets 源的形状门消费（非行集 / 缺表头 /
majorDimension 非 ROWS）。

重试只覆盖连接类失败（有界次数 + 退避，口径同 ci.yml 的 curl 重试）；
4xx 不重试。

## 留待实现期（不在首期）

- 增量拉取（行级）：按 revision / updated_at 只取变更行——文档级增量
  已实装（S8 条件 GET，见下方决策记录），行级增量需要各源的数据契约
- OAuth 用户授权流、mTLS 与 service account 凭据（Sheets 首期只
  API key——service account 需 OAuth JWT 交换）
- 连接池、并发多源、大表游标分页
- Sheets 富文本与公式重算（首期只缓存值）
- ~~从库表内省自动生成 Schema 草稿~~（已实装，S9，见下方决策记录）

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
留待实现期）。S4 已交付——离线语义收口：三适配器传输类取数失败回退
上一份缓存副本（E1906 WARNING，404 / 401/403 永不回退），`--no-cache`
（check / build / gen / inspect）关闭回退还原硬失败；远端变更 →
source_hash / build_id 旋转的端到端测试随 S1 已锚定（identical
rebuild manifest 逐字节一致 + 数据变更双哈希旋转），本签补三适配器
与 CLI 进程级回退 / 严格两形态测试。S5 已交付——错误码接线收口对账：
全仓（codes.rs / validation.md / design.md）E19xx 族「预留 / reserved」
标注清零，四源错误路径的码覆盖系统盘点在案——HTTP：E1901（传输 /
404 / 其他状态 / 非 http URL）+ E1902（401/403）+ E0001（坏 JSON 走
本地同款 parse 诊断）+ E9902（缓存写）+ E1906（回退 WARNING）；
DB：E1905（spec / 白名单 / 表名）+ E1904（dsn_env）+ E1901（DSN
非法 / 连接 / 会话只读 pin / prepare / 查询——mysql.rs 与 pg.rs 全
带码）+ E9902；Sheets：E1901（spec / id / 传输）+ E1904
（credential_env）+ E1902 + E1903（形状门）+ E9902 + E1906——每条
路径既存测试逐码断言（crate 级 assert contains + CLI 进程级 stderr
断言），诊断渲染经 `load_project` Err → `error: {e}` 全覆盖。S6
已交付——文档收口（source.md 后续扩展清单转正、cli.md 新增 Remote
Source 章节、需求整理.md Remote Source 行勾选、architecture.md 结构树
对账），S 系列 S1–S6 全数交付。S7 已交付——HTTP 分页协议与限流协商
（原留待实现期项，见下方决策记录）。S8 已交付——HTTP 条件 GET
增量拉取（文档级：ETag/Last-Modified revalidation，304 复用缓存
字节；行级增量仍在留待实现期清单，见下方决策记录）。S9 已交付——
库表内省 Schema 草稿（原留待实现期项，`cage schema-draft`，见下方
决策记录）。

### S7 决策记录（HTTP 分页与限流协商）

**分页协议 = RFC 8288 `Link: <...>; rel="next"` 响应头。** 定了什么：
HTTP 源 GET 成功后若响应带 `rel="next"` 的 `Link` 头则沿链取页——每页
体必须是 JSON 行数组，页序拼接成一份合并文档（裸数组 → `Data` 表）；
相对 next 目标按 RFC 3986 对当前页 URL 解析；已访问集去重（回环 →
定错）；硬上限 `MAX_PAGES` = 1000 页。**为什么**：Link 头是标准协议、
与响应体形状无关——不需要猜每家 API 的 `next`/`next_page`/`cursor`
键名，猜键名正是 cage 一贯避免的隐式魔法；相对解析、回环与页数边界
都是本地可控的确定性语义。**备选**：响应体游标约定（`{"next": url}`）
——每家命名不同，弃；`X-Next-Page` 类自定义头——非标准，弃。
**兼容**：首响应无 next 时字节原样返回——单 GET 源的既有契约（任意
可接受形状）逐字节不变，分页契约只在 next 出现时生效。**不参与离线
回退**：分页契约违规（非数组页 / 回环 / 越界 / 坏链接）是「服务器已
应答」，与 404/401/403 同列，永不回退缓存——只有传输类失败（服务器
不可达）才回退（S4 口径不变）。

**限流协商 = 429 + `Retry-After`，有上限地照办。** 定了什么：429 带
可解析的 `Retry-After`（delta-seconds 或 RFC 7231 IMF-fixdate
HTTP-date）且等待 ≤ `MAX_RATE_LIMIT_WAIT`（30s）时，按服务器要求的确
切时长退避后重试（`FetchFailure::RateLimited`，与传输类共用同一
`attempts` 预算，`RetryClassify::cooldown` 携带协商等待）；缺失 /
不可解析 / 超上限一律即时定错（`Status(429)` → E1901），不猜、不睡
长等。**为什么**：限流是服务器明确的「何时回来」协商，照办即正确
重试；上限保护构建不被一小时限流拖死。**备选**：忽略 `Retry-After`
一律固定 backoff——不尊重服务器节奏，弃；429 一律不重试——对短时
限流过于脆弱，弃。**HTTP-date 解析**：仅 IMF-fixdate（服务器实践中
唯一形态），两种 obsolete 格式不实现；civil 日期 → 纪元秒用 Howard
Hinnant 的 `days_from_civil`，不引入日期库。**回退语义**：429 重试用
尽后是硬 E1901，不回退缓存——服务器可达（只是限流），与「服务器
不可达才回退」口径一致。

**验收**：cage-core 单测 ×5（`Retry-After` 双形态与垃圾值表、
HTTP-date 已知纪元锚 1970/2015/2020 闰日/2030、冷却映射与可重试
分类、限流消耗同一预算）+ cage-source-http 单测 ×7 新增（双页合并
与缓存合并字节、非数组页定错点名页 URL、回环定错、页界 enforce
（小界注入）、限流退避后成功且计数断言、超上限即时定错不入睡、
无头定错）+ RFC 8288 解析单测（引号/多成员/多 token/大小写/目标内
逗号）+ CLI 集成 ×1（分页源端到端构建 → Data.json 含两页行 → 缓存
= 合并数组 → 同态重建 manifest 逐字节一致）。

---

# 46. Migration（M 系列，2026-10 立项拍板）

**实装状态**：M1 迁移规则模型与解析（`parse_spec` / `validate_spec` /
`parse_migration_dir`，E2001/E2002）、M2 迁移执行器与回验（`apply` 六类
变换语义 + `reverify` L0–L6 回验，E2003/E2004）、M3 CLI 与落盘
（`cage migrate` dry-run/`--write`/`--all`/`--to`，本地文本源原路写回）
均已交付，E20xx 四码全数接线（validation.md E20xx 表为准）；追加交付：
`--to latest`（解析为整链、同 `--all`；链上字面版本 `latest` 优先）与
Excel 报告的单元格级定位（`StepReport.affected_locations`——Excel 表
受影响行的 `Sheet`/`Row` 定位随报告输出，文本源不采集）；再追加：
`cage migrate-draft`（`migrate::diff` 模块——两版 schema 的结构性
diff → 迁移规则**草稿**：机械安全变换成步、歧义项留 `# TODO`，见下方
规则草稿决策记录）；下方接口块
与签名以实装为准——`apply` 实带 `from_schema` 参数（`widen_type` 方向
检查用），`reverify(doc, schema)` 为独立入口。

**实装决策记录（M3，随码补充）**：

- **widen 双路径**：CLI 载入的 schema 恒是**新版**（工作流是先改
  schema 再写规则迁数据），`widen_type` 的 `to` 与 schema 当前类型
  相同——若仍走方向表会被 `X → X` 非「安全加宽」误拒。定案：`current
  == to` 时跳过方向表、只跑逐行值域检查（值域是安全网，两条路径都
  跑）；`current != to` 才按方向表（库调用方持旧 schema 的场景不变）。
- **CLI 不跑 `validate_spec`**：其 from-schema 引用校验需要**旧版
  schema**，而磁盘上只有一份新 schema（历史 schema 不入库）。定案：
  from-schema 校验留作库 API 能力（编辑器/工具链持有两份 schema 时
  用）；CLI 路径的坏引用由 apply 层防线接力——rename 目标已被占用
  （行内同存 from/to、表名同存 from/to 都直接 `E2003` 拒绝，绝不合并
  两个值）、引用的表文档里没有（`E2003`）、字段级残留在回验 `E2004`
  兜底。失败即失败，无静默通道。
- **幂等**：`apply` 的 `rows_changed` 只计字节真正改变的行——重跑
  已迁移文档每步都是 no-op（改名目标已就位 / 默认值已补 / 值已映射 /
  表已改名），`--write` 前逐文件字节比对，相同跳过。第二次 `migrate
  --write` = 0 行变更 + 全部 unchanged。
- **落盘渲染序**：JSON 源读取不保文件内键序，原序不可作为再渲染依据；
  定案按 **schema 字段声明序**渲染行（契约序、稳定），schema 外字段
  按 load 序尾插。首次 `--write` 是一次性格式规范化（报告为 wrote），
  自第二轮起字节恒定。
- **多段链回验**：回验永远对磁盘上那一份（最终版）schema 跑——链上
  每个中间态必须自洽满足当前 schema；跨到中间态停住的链（中间态不满足
  最终 schema）应 `--all` 一次到位。
- **latest 符号与字面版本**：`--to latest` 默认解析为整链（与 `--all`
  同选段）；链上若真有段 `to: "latest"`，按字面版本优先、只取到该段为
  止——具体目标赢过符号，避免链自带 latest 版本时无法只迁到它。
- **Excel 单元格级定位**：Excel 源永不写回，报告是其唯一变更可见面——
  `apply` 对 Excel 表（.xlsx/.xls 后缀判定）采集受影响行的
  `Row.location`（`file | Sheet | Row` 渲染），文本源不采集（落盘 diff
  即变更记录）；CLI 每步骤行下按源序渲染、超 8 行折叠
  `… +K more row(s)`（确定性封顶，无时间戳无随机序）。

**实装决策记录（规则草稿 `migrate-draft`，随码补充）**：

- **草稿不是自动推断**（备选①弃的是「自动推断直接成规则」）：结构性
  diff 只提**机械安全**变换——`remove_field`（旧有新无）、`widen_type`
  （方向表内）、`set_default`（新字段带默认 / required false→true 且有
  默认）；歧义项一律 `# TODO`（字段与表改名——rename 还是删旧增新只有
  作者知道、非加宽换型、枚举成员增删、required 无默认翻转、删表与
  新表），**改名从不猜**。全 TODO 稿渲染 `steps: []`，`parse_spec` 按
  E2001 拒绝——草稿永不直接运行。
- **UInt 默认值的带标签形态**：裸 YAML 标量无符号性（`yaml_to_value`
  先试 i64），`5` 回读是 `Int(5)`——UInt 列的默认值若走裸标量，回验
  必撞 E1101。定案：`set_default` 值解析对 mapping 先试 canonical
  邻接标签形态（`{type: UInt, value: 5}`），失败回落 plain 读取；草稿
  渲染端凡值内含 `UInt` 一律发标签形态。字符串值恒单引号（`yes`/数字
  形态不回读翻型）。render → `parse_spec` 往返有单测兜底。
- **草稿命令不跑 `validate_spec`**（同 M3 CLI 先例的因由更深一层）：
  `set_default` 引用的是 to-schema 的新字段、`remove_field` 引用的是
  from-schema 的旧字段——单独哪一版 schema 都验不全。防线由 apply 的
  E2003 与回验 E2004 接力。

**决策记录（2026-10 拍板，依「待拍板项按建议方案自行定 + 记录，用户
后续审核再调」授权）**

- **定了什么**：Migration = Schema 演进下对存量 Source 数据的**声明式
  迁移**——迁移规则（`migrations/` 目录，文件名序即版本步进链：`0001-…`、
  `0002-…`）显式声明每段变换（字段改名 / 补默认值 / 删字段 / 安全类型
  加宽 / 枚举值映射 / 表改名），`cage migrate` 对 Canonical Model 执行
  变换、逐表逐行出报告、迁移后在新 schema 下回验（check 全绿才算完成）；
  默认只读报告，`--write` 才落盘。
- **为什么**：schema 变更后存量数据怎么办是编译器的演进刚需——手工改不可
  审计、自动推断不可靠；显式规则 + 回验 = 可审计（规则进版本库）、确定性
  （同规则 + 同数据 = 同产物，与全仓确定性契约同构）、失败即失败（不静默
  丢数据）。
- **备选**：① 从 schema diff 自动推断迁移步骤（弃——歧义处静默走错方向：
  字段改名 vs 删旧增新无法机械区分）；② 只出报告不改数据（弱——没闭环，
  用户仍手改）；③ 运行时兼容层（弃——越界，红线「不做游戏运行时数据库 /
  游戏逻辑框架」）。
- **实现顺序**：A 系列（§47）先行——复用 R 系列在册设施、零新概念；M 系列
  随后（新模块爬坡）。

**能力边界**：

- 做的：Canonical Model 层的声明式变换；可文本改写的源（JSON / YAML /
  CSV）支持 `--write` 落盘；Excel 源只出报告不落盘（红线「不做 Excel
  编辑器」——按报告手工改，改完重跑 migrate 校验收敛）；迁移后的注册表
  条目 = 重跑 build + `cage registry publish`（复用 R 系列，不新造通道）。
- 不做的：自动推断（规则必须显式）；schema 自身的演进（schema 编辑走
  Schema Editor / 手编，迁移只管数据）；段选择按需取（`--all` 全链、
  `--to <ver>` 前缀、`--to latest` 整链）。

**接口**：

```text
cage-core::migrate
├── MigrationSpec      # 一段迁移：from / to / steps[]
├── Step               # rename_field / set_default / remove_field /
│                      # widen_type / remap_values / rename_table
├── parse_spec(...)    # 迁移文件 → MigrationSpec（E2001 解析失败）
├── validate_spec(...) # 规则引用对 schema 校验（E2002——表/字段存在性、
│                      # rename 撞名；库 API，持 from-schema 的调用方用）
├── parse_migration_dir(...)  # 目录 → 文件名序版本步进链
├── apply(spec, doc, from_schema)  # Canonical Model 原位变换 + 逐步骤
│                      # 行数报告（E2003 变换不满足：方向不安全、值超
│                      # 域、表/行字段缺失、rename 目标撞名；幂等——
│                      # 重跑已迁移文档 0 行变更）
└── reverify(doc, schema)  # 迁移产物对新 schema L0–L6 回验（E2004 附诊断）

migrate::diff
├── diff_schemas(from, to)    # 表名/字段名对齐的结构 diff（FieldDelta 五类）
├── draft_migration(diff)     # 机械安全变换成 Step、歧义项成 TODO 注释
└── render_draft(draft)       # 规则稿文本（TODO 块 + steps:；空稿 `[]`）

cage migrate [--all | --to <ver|latest>] [--write]   # 默认 dry-run 只报告
└── 载入工程 → 按文件名序逐段应用 → 当前 schema 回验（E2004 迁移后
    校验失败不落盘）→ 报告（每步骤行数 / 每源文件处置）；
    --write 对本地 JSON/YAML/CSV 源按 source_file 原路写回（schema
    字段序渲染、字节未变跳过），Excel 源与 registry/远程源恒报告

cage migrate-draft <from-schema> <to-schema> --from <ver> --to <ver> [-o file]
└── 两版 schema 的规则**草稿**：机械安全变换成步、歧义项 `# TODO`；
    默认 stdout，`-o` 落盘（父目录自动创建）；空稿 `steps: []` 需手写
    步骤后才能被 cage migrate 解析
```

**错误码（E20xx 族）**：E2001 迁移规则文件解析失败 / E2002 规则引用
不合法 / E2003 变换不满足 / E2004 迁移后回验失败。全族随 M1 进
`codes.rs` 的 `error::codes::migration` 模块 + validation.md，每码一
doc（预留标注到 M1 接线清零，与 S 系列同纪律）。

**留待实现期**：~~迁移规则与
`[dependencies]` 版本 pin 的联动校验~~（已实装，见下方决策记录）。
（`--to latest` 与 Excel 报告的
单元格级定位已随 M3.1 补齐；schema diff 辅助生成规则草稿已随
`cage migrate-draft` 补齐——见实装状态与规则草稿决策记录。）

---

# 47. Artifact Distribution（A 系列，2026-10 拍板）

**实装状态**：A1 bundle 导出（`cage registry export`，确定性 tar）、A2
bundle 导入（`cage registry import`，信任门入册）、A3 直推（`cage
registry push`，探针 + 逐文件 PUT + index 远端合并）均已交付，E21xx
五码全数接线（validation.md E21xx 表为准）；A1 的延迟项压缩容器已补齐——
`export_bundle` 增 `compression` 参数（CLI `--compress zstd`，固定级别 19
单 zstd 帧，同 tar + 同级别 + 同 zstd 库版本 → 同容器字节），`import_bundle`
按帧魔数嗅探容器（不看扩展名，账本哈希解压后内容，两种形态信任门同判）；
push 不变（推单文件不推 bundle）；A6 签名账本已补齐——`cage registry
keygen`（种子只落用户指定文件、公钥上 stdout）、`export --sign
--key-env`（`<bundle>.sig` 分离式 ed25519 签名）、`import --verify-sig
--key-env`（账本信任门前的签名门），E2106/E2107 接线，共七码（下方
接口块与签名以实装为准）。

**决策记录（2026-10 拍板，同 §46 授权口径）**

- **定了什么**：注册表条目的两条分发路——① **离线 bundle**：
  `cage registry export <包>[@<版本>] -o <file>` 产出确定性 tar（条目全部
  文件 + 账本 + 包 index 摘录；mtime/uid/gid 归零、成员按名序——同条目 =
  同字节），`cage registry import <file>` 先过同一账本信任门
  （`verify_snapshot`，未经校验不入册）再入册（同字节幂等 / 异字节
  E1801 冲突，坏账本 E2103）；② **直推**：`cage registry push <project>
  [包[@版本]] --registry <远端根>` 从项目 `[registry].path` 声明的本地
  注册表读条目（镜像 publish 的项目路径加载；包名缺省 `project.name`、
  版本缺省点分序最新），对 http(s) 远端根逐文件 PUT，条目文件全部成功
  后最后写包 `index.json`（条目字节不可变纪律不变）；`--registry` 只收
  http(s) 根（本地目标是 publish 的领地）；鉴权 `Authorization: Bearer
  $TOKEN`，token 经 `--auth-env`（覆盖）或 `[registry].auth_env` 的环境
  变量名解析（E1904 同口径：凭据永不进 cage.toml、永不落日志与错误
  文本、网络触达前先验缺失——E2105）；推送前匿名 GET 远端包 index 作
  状态探针（R3 读协议不变，404 = 远端尚无此包 → 全量上传；同 content_hash
  → 幂等零 PUT；异 content_hash → E1801 拒推——远端已有同版本不同字节的
  条目），index PUT 与远端现有条目合并后整体重写（远端历史条目永不
  改写或删除）。
- **为什么**：R3 已闭环「消费方怎么取」（匿名 GET 三资源只读解析），缺的
  是「怎么到远端、怎么走非网络渠道」——bundle 覆盖离线 / 审计 / 对象存储
  渠道（产物本就是自校验快照，拷贝即分发），push 覆盖自动化发布（CI 出包
  直进远端注册表）；两条路复用同一账本信任门，不引入第二套信任模型。
- **备选**：① 自建 registry 服务端进程（弃——红线不造服务端产品；协议只定
  客户端写形态，服务端任何能收 PUT 的静态网关 / nginx WebDAV / CI job
  皆可）；② git 作分发后端（弃——隐式改写历史与注册表「绝不隐式改写
  历史」纪律冲突）；③ gzip/zstd 压缩容器（初版弃——压缩帧时间戳 / 字典态
  顾虑后被证实不适用于 bulk 单帧编码：帧头无时间戳、无字典态；已实装——
  `--compress zstd` 固定级别 19 单帧包裹确定性 tar，容器字节同库版本内
  逐字节可复现，导入按帧魔数嗅探，见实装状态）。
- **实装决策记录（签名账本 A6，随码补充）**：

- **签名挂在 bundle 字节层，不在账本文件层**：HASHES.json 账本依然是
  完整性锚（导入信任门不变），ed25519 分离式签名覆盖 bundle **落盘
  精确字节**（纯 tar / zstd 容器同判）——签名对象包含账本，账本本身
  被篡改也会连带验签失败，不引入第二份可信清单。
- **sidecar 的 public_key 不是信任锚**：`<bundle>.sig` 里的公钥只为
  人类核对与传输便利；验证永远钉死在消费方 `--key-env` 带来的钥匙上
  ——「谁可信」由消费方决定，签名只回答「谁签的」。没有可信钥就不
  跑（`--verify-sig` 强制 `--key-env`，clap `requires`），不做「信
  sidecar 自身公钥」的静默降级——那等于只验完整性，把抗抵赖卖掉。
- **密钥纪律**：密钥只从环境变量读（base64 32 字节种子/公钥），不进
  cage.toml、不进日志；`keygen` 的种子只写 `-o` 指定文件（unix 0600，
  尽力而为不阻断），stdout 永不出种子。E2106（密钥材料不可用）与
  E2107（验签不过 / sidecar 缺失畸形 / 算法不认）分码——拿不到钥匙与
  钥匙对不上是两类失败。
- **签名门在账本门之前**：不可归因的 bundle 先拒绝，再谈完整性——
  顺序决定失败信息：E2107 先于 E2103/E1801 出现。

**服务端约定（文档化，不实现）**：GET `<root>/<包>/index.json` 匿名
  作状态探针（404 = 包不存在）；PUT `<root>/<包>/<版本>/<文件>` 逐文件
  上传，条目全部成功后 PUT `<root>/<包>/index.json`（服务端应整体替换
  该文件——客户端已合并远端现有条目）；405/501 = 服务端未实现写通道
  （E2104，提示回退「本地 publish + 静态托管」）；401/403 = 凭据被拒
  （E2102，探针与 PUT 同映射）；无鉴权部署照旧可用（R3 读路径不变，
  push 可选 auth_env 缺省不带头）。

**能力边界**：export / import / push 只动「已入册条目」——不重新构建
（发布仍是 `cage registry publish`，本地根专属）；读路径保持 R3 匿名
只读；不做服务端实现、不做增量 delta。签名已实装（A6 ed25519 分离式
签名，见实装状态）——blake3 账本仍是完整性锚，签名在其上加抗抵赖。

**接口**：

```text
cage-core::registry 增
├── export_bundle(root, package, version, compression, out)  # E2101 条目读取失败
│    # 条目全文件 + HASHES.json + 包 index 摘录 → 确定性 tar
│    # compression = Plain | Zstd（固定级别 19 单帧）
├── import_bundle(root, file, dry_run)          # verify_snapshot 信任门
│    # → 入册（坏账本 E2103；同字节幂等、异字节 E1801）
├── push_entry(source_root, remote_root, package, version, auth_env,
│              dry_run)
│    # resolve_entry + 账本重建 index 记录 → 匿名探针 GET 远端包 index
│    #   （同 hash 幂等早退 / 异 hash E1801 / 404 空远端 / 401·403 E2102）
└── 签名账本（A6）：generate_signing_key + key_material（keygen，种子只落
     用户指定文件）、signing_key_from_env / verifying_key_from_env（E2106
     密钥材料不可用）、sign_bundle_bytes / verify_bundle_bytes（E2107
     验签不过）、read/write_bundle_signature（`<bundle>.sig` sidecar）
     # → 逐文件 PUT（remote::http_put，RetryPolicy 与 http_get 同口径，
     #   Bearer 仅随 PUT）→ index 合并远端条目最后 PUT
     # E2101 传输 / 远端 index 不可读 / 非 http(s) 根
     # E2102 鉴权被拒（401/403，探针与 PUT 同映射）
     # E2104 服务端拒写（405/501 等明确 4xx）
     # E2105 凭据缺失（env 未设或空，网络触达前失败）
     # dry_run 走完整本地读 + 状态探针，零 PUT

cage registry export <pkg>[@<ver>] -o <file> [--compress zstd]
    [--sign --key-env <VAR>] [--registry <local-root>]
cage registry import <file> [--verify-sig --key-env <VAR>] [--registry <local-root>] [--dry-run]
cage registry keygen -o <file>
cage registry push <project> [pkg[@ver]] --registry <remote-root>
    [--auth-env <VAR>] [--dry-run]
```

**错误码（E21xx 族）**：E2101 分发传输 / 条目读取失败 / E2102 鉴权被拒
（HTTP 401/403）/ E2103 bundle 账本校验失败 / E2104 服务端拒写（405 /
409 / 明确 4xx）/ E2105 push 凭据缺失（auth_env 未设）/ E2106 签名密钥
不可用（A6）/ E2107 bundle 签名校验失败（A6）。全族随 A1 起逐签进
`codes.rs` 的 `error::codes::distribution` 模块 + validation.md，每码
一 doc；A6 收口时七码全部转已接线。

**留待实现期**：增量 delta 分发、S3 presigned 直推、pull-through 缓存
代理。（压缩容器已实装——固定级别 19 的 bulk 编码本就不依赖字典态，
「zstd 确定性字典」变体随之失效；签名账本已随 A6 实装——见实装状态与
A6 决策记录。）

---

# 48. Environment Validation（分环境验证，2026-10 拍板）

游戏的配置是分环境的：同一张表，dev 环境允许 `hp` 缺省，prod 环境不但
必填还要求下限 100。Schema 是唯一的规则载体，所以环境语义落在
Schema 上——每个环境是同一套字段形状的一档「严格度旋钮」，不是另一
张 Schema。

**拍板：约束覆盖（方案 1）。** 2026-10 用户拍板取「约束覆盖」方向
（分环境 = 对既有约束的覆盖，而非「每环境独立 Schema」或「数据分叉」
两案），并授权巡检按决策记录口径代定实现细节。**授权口径**：实现期
发现方案 1 与既有约束机制冲突 → 记录冲突与理由、停下上报；未发现冲
突，开批交付。

**线格式（实现期裁定，同语义下的载体偏离）**：采用**表级侧表**
`env_overrides`（TableSchema 新字段：环境名 → 字段名 → 约束补丁
`FieldOverride`），而非拍板预览里的「逐键内联 `{default, envs}`」
形态。**为什么偏离**：内联形态要求把 `FieldSchema` 的每个约束字段改
造成包装结构，波及全仓 12 个 crate（含 7 个 golden 锁定的代码生成
crate）约百处 `.required` / 约束读取点，机械改动量大且触golden；
侧表形态语义完全一致（每环境一组约束 delta），但零波及——校验器、
代码生成、编辑器往返全部不变。**分类**：既定方向内的实现载体选择，
非机制冲突（约束覆盖与 `rules`/`reference`/profile 投影互不侵占：
覆盖只调同形状的严格度，不增删字段、不改类型），故不停批，记录在
案。

**语义**：`FieldOverride` 是字段的**部分补丁**——只列某环境要动
的约束键（required/min/max/min_length/max_length/pattern/
enum_values/min_items/max_items），未列的键保持基线值；列出的键是
**整值替换**（如 `enum_values` 覆盖为生产白名单，不是与基线并集）。
**类型不在补丁里**：环境只调严格度，永不重塑字段形状。空补丁（一个
键都没设）在 schema 结构检查时定错。

**解析点**：`Schema::resolve_env(env)` 在 load 之后、profile 投影与
校验之前一次性执行——克隆 Schema、把该环境的补丁抹到普通字段上，交
付给下游的永远是「一份普通 Schema」。全部校验器（L0–L7、profile 语
义、E9006）、代码生成、哈希都消费 resolved 结果，**零改动**；基线
Schema 从不被修改。E9006（投影剥掉结构必需字段）按 resolved 的
`required` 判定——环境先收紧、投影再触发冲突，次序自然正确。

**环境名来源**：Schema 自声明——`env_overrides` 各表键名的并集
（`declared_envs`），不在 cage.toml 里登记第二份名单（单一事实源）。
`--env` 未声明 → 用法错误 exit 2，报错列出全部已声明环境；Schema
完全无 `env_overrides` 时 `--env` → exit 2「declares no
environments」。**不新增 E 码**：环境名错误与 unknown profile 同类
（CLI 用法错误，非文档校验失败）；`--env` 路径上对全部环境的覆盖做
结构检查（字段存在、非空补丁，报错带 table/env/field 定位）——坏覆
盖在选中该环境时必被拦下。非选中环境的坏覆盖告警（无 `--env` 的
lint）已实装（2026-10）：`Schema::lint_env_overrides` 收集全部结构性
问题，基线路径（不选环境）逐条 `warning:` 提示不阻断，`--env` 路径
维持硬错误——同一结构缺陷两种口径，与 unknown field 的
告警/升级同构；沿用「不新增 E 码」的既有裁定（装载期纯文本诊断，
与 `--env` 结构错误的呈现一致）。

**构建账本**：`schema_hash` 对 resolved Schema 全量哈希（环境改变
→ 字段约束变 → schema_hash 变 → build_id 变）；manifest 新增
`environment: Option<String>`（serde default，基线构建不写该键），
显式记录构建所处的环境；L1 / layer-2 增量守卫在 profile 之外再比对
environment——**换环境永不复用上一环境的构建产物**。同环境确定性
不变：同输入同环境 → manifest 逐字节一致。
`generator_version` 1.1.0 → 1.2.0（布局演进：新增 `environment`
字段）。~~`cage snapshot` / `registry publish` 暂以基线构建（无
`--env`）出包；环境化出包留待后续。~~ **环境化出包已实装
（2026-10，见下批决策记录）。**

**验收**：cage-core 单测（declared_envs 并集 / 结构检查拦未知字段与
空补丁 / resolve_env 抹补丁且基线不被修改 / 无覆盖环境解析等于基
线）+ CLI 集成 ×4（同表三套规则：基线 E1001、dev 放行、prod
E1201/E1204 换档；未声明环境与无覆盖 Schema 均 exit 2 并列出已声明
集；manifest 记录 `environment` 且换环境轮换 build_id、增量守卫按
环境失配；dev → prod 增量构建与全量构建逐字节收敛）。

### 环境化出包决策记录（2026-10）

**一版本一包，整包指纹判同。** 定了什么：`cage snapshot` 与
`cage registry publish` 都挂 `--env`（与 check/build/gen 同语义：
`env_overrides` 先解析进 schema 再走全量验证与打包）；快照目录名带
env 段 `<profile>-<env>-<build_id[..12]>`（基线不带 env 段），包内
`manifest.json` 的 `environment` 字段与 resolved 形态的 `schema.json`
随包走；publish 沿用既有流程，环境化条目入册时 manifest 即声明环境。

**E1801 从「同 content_hash」放宽判定面到整包指纹**：幂等/冲突判定
改为 `build_id + content_hash` 双比对（publish 与 import 预检同步）。
为什么：content_hash 只盖产物字节——环境化重打包时产物可能逐字节相同
（数据没变、变的只是规则），只看 content_hash 会把新包误判成
「identical, no-op」，prod 字节永远落不了地。`build_id` 盖
profile+schema_hash+source_hash+content_hash，schema 一变它必变——
同版本换环境（或任何仅 schema 变化）从此正确报 E1801，提示换版本号
（如 `0.1.0-prod`，版本词法本就允许 label 段）。

备选与取舍：给同版本挂多环境子目录（`<version>/<env>/`）弃——破坏
「一版本一包」的可寻址性与 bundle/export 形态，消费端 pin 语义复杂化；
resolve/verify/gc/remove/push 保持按字节与账本工作，不感知环境——
环境只是打包时的一条构建参数，入了册就是普通字节。`--env` 与
`snapshot --verify` 互斥（回验只看字节，不看环境）。

**验收**：core 单测（同产物异 build_id 冲突且报文点 build_id、同
build_id+content 幂等 no-op）+ CLI 集成 ×2（`snapshot --env`：目录名
env 段、包内 manifest 记环境、schema.json 带 resolved 约束、回验通过、
基线快照不受影响；`publish --env`：同版本换环境 E1801、新版本入册且
manifest 记环境、幂等重发 no-op、registry verify 全绿）+ 示例 run.sh
步骤 6/11 扩展（环境化快照与 E1801 实机演示）。

### S8 决策记录（HTTP 条件 GET 增量拉取）

**文档级增量 = RFC 7232 revalidation，按 URL 缓存验证器。** 定了什么：
HTTP 源的每份缓存字节旁落一枚 sidecar meta（`.cage-cache/source/<key>/<key>.meta.json`，
`{etag, last_modified}`，键取自响应头）；同 URL 再次取数时把验证器
作为 `If-None-Match` / `If-Modified-Since` 发出去，304 → 缓存字节原样
复用（连 meta 都不重写），200 → 新字节替换旧字节、新验证器替换旧
meta。**为什么只做文档级**：行级增量（按 revision/updated_at 只取
变更行）需要每家源暴露数据契约（revision 键、变更行集语义），HTTP
API 没有通用口径——验证器是响应级的、与响应体形状无关，是唯一能
不改各源契约就拿到收益的层级；行级留待实现期。

**只有首页 GET 带条件**：分页源里只有首页的验证器描述整个集合状态；
后续页是首页字节里的相对 URL，带自己的链接级语义，条件化没有可
靠依据——首页 304 时整个分页链不必重走。

**meta 无伴生 payload 即视为无缓存**：meta 与 payload 必须同时存在才
允许 revalidation——半份缓存发给服务器的断言无从证实，老实地按无
缓存处理（走普通 GET 重建两份），不新增状态、不静默复用。

**`--no-cache`（strict）依然 revalidate**：strict 的语义是「缓存字节
不充当答案」，不是「不与服务器说话」——304 是服务器确认缓存，属于
合法回答；strict 与普通模式共用同一条 revalidation 路径，只差传输
失败时是否回退（见 S4）。

**与离线回退的关系**：304 不经过传输失败分类，不是回退候选也不触
发 E1906；只有真传输错误才落 S4 的语义。验证器写入失败（E9902）
不影响 payload——下一轮无 meta 即走普通 GET。

**验收**：cage-source-http 单测 ×2（旋转服务器五步：首取无条件
且落 meta ETag；二取带 INM 且 304 字节不动；轮换后 INM 旧值 200
落新字节新 meta；下轮带新 ETag；strict 仍 revalidate——服务器计数
逐次断言；meta 删除后按无缓存重建）+ 全绿 gates（fmt / clippy -D
warnings / 689 测试 / run.sh 实机 exit 0）。

### 迁移与依赖 pin 联动校验决策记录（2026-10，design §46 遗留清账）

**链终点必须落在 pin 内。** 定了什么：`cage migrate` 在选段之后、
应用任何规则之前做一道联动门——当 `schema_path` 来自
`registry:<包>[@<版本>]` 且 `[dependencies]` 为该包声明了 pin 时，
选中链的终点版本（最后一段的 `to`，默认单段 / `--to` / `--all` 同
判）必须满足 pin，否则 `E1802` 硬错误（exit 2，dry-run 与 `--write`
同判，报文点名版本 / pin / 包名）。**为什么**：链终点与 pin 是同一
件事（「数据要迁到哪个 schema 版本」）的两份独立声明——pin 管这个
工程认哪些 schema 版本，迁移链管数据实际走到哪；数据迁到 pin 之外
的版本，回验与后续构建都无法在工程可解析的任何 schema 下成立。这两
份声明此前的唯一交集是装载时的版本解析，迁移目标本身无人把关。

**备选与取舍**：只告警不拦截（同基线 env lint 口径）弃——迁移是
改写源数据的动作，dry-run 报告本身就承担「提醒」职能，放行一个
注定回验无门的版本只会把失败推迟到 reverify（E2004）或下一次
build；挂到 E20xx 新码弃——这是 pin 违规，不是规则引用/变换/回验
问题，E1802 的既有语义（「显式版本必须落在 pin 内」）原样覆盖。

**边界**：本地 schema（非 `registry:`）无 pin 可联，门静默跳过；
`registry:` 无 pin 的规范由显式版本单独管辖（R2 既有语义）。迁移
链自身的连续性（from 衔接）仍是 `parse_migration_dir` 的职责，本门
只看终点。

**验收**：CLI 集成 ×1（发布 schema 条目 → 消费方 `registry:cfg@1.0.0`
+ pin `>=1.0.0, <1.1.0` → `migrate --to 1.1.0` exit 2 且报文带
E1802/版本/pin/包名 → 放宽 pin 后同链 dry-run 通过）——本轮交付，
勾选。

### S9 决策记录（库表内省 Schema 草稿）

**草稿 = 结构机械、语义留人。** 定了什么：`cage schema-draft
<project> <spec>... [-o <file>]`——每个 `mysql:<表>` / `pg:<表>`
spec（与 source_roots 同句法，`schema.表` 解析到具名 schema）经只读
会话内省（绑定参数的 `information_schema` SELECT，表名永不拼接进
SQL），渲染成一份确定性 YAML 草稿：表序 = spec 序、列序 = 声明序、
主键来自约束、`NOT NULL → required: true`；DECIMAL / 日期时间 /
JSON / 未知类型在行内注释里点出「这列要作者拍板」；无主键表以空
`primary_key: []` 落盘并注释提醒。**为什么结构机械**：内省能回答的
只有形状（列名 / 类型 / 可空 / 主键）——min/max/pattern/enum_values/
references 是业务语义，DB 不知道 cage 的校验域；替作者拍板正是草稿
的反面。类型映射取「最小忠实口径」：整型按宽度落 Int8–Int64 /
UInt8–UInt64（unsigned 后缀提升家族）、`tinyint(1)` 按 MySQL 惯例落
Bool、浮点按精度落 Float32/64；DECIMAL / temporal / JSON / 文本一律
String——**与适配器落进缓存 JSON 的实际形态一致**，草稿必须能在自
家管线上校验通过，而不是描述服务器的世界观。

**凭据与只读纪律全数沿用 S2**：DSN 从 `[remote.<scheme>].dsn_env`
指名的环境变量（E1904，永不进 cage.toml）、会话先钉只读、自产的
内省 SQL 必须过自家 `validate_select` 白名单（单测自检）——网线上
跑的东西只有一套纪律。表找不到（零列返回）是 E1901 查找失败。不新
增 E 码：spec/DSN/连接三档全部落在 E1905 / E1904 / E1901 既有语义。

**确定性**：渲染字节只依赖内省形状——头部注释不带时间戳（与全仓
「同输入同字节」纪律一致），同一次内省永远渲染同一段 YAML。`-o`
父目录自动创建（与 migrate-draft 同口径）；默认 stdout。

**验收**：cage-source-db 单测 ×15（draft 7 + mysql 4 + pg 4：MySQL COLUMN_TYPE 分类表 30 档、
PG data_type 分类 + varchar/numeric 尺寸重组、内省 SQL 过白名单 +
const 不可拼接自检、渲染逐字节形状、**渲染 YAML 解析回 Schema 并断
言表/主键/required 形状**、异形名引号往返、无 PK 空列表、注释仅在
需要拍板的档位出现）+ CLI 集成 ×2（无 [remote] / env 未设 / 非 db
spec / mysql+pg 不可达五档错误码 + exit 2；缺 cage.toml 与缺 spec
两档用法错误）——本轮交付，勾选。
