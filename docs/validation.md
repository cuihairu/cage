# Validation：验证流水线

Cage 的核心不是 Transform，而是 **Validation**。

验证拆成多个阶段，逐级执行：

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

每一级对应一个验证层级与一族错误码：

| 层级 | 名称 | 检查内容 | 错误码族 |
| --- | --- | --- | --- |
| L0 | Parse | 输入是否可以正确解析 | `E0001` 族 |
| L1 | Schema | 结构是否符合 Schema | `E1001` 族 |
| L2 | Type | 类型是否匹配 | `E1101` 族 |
| L3 | Value | 值是否在约束内 | `E1201` 族 |
| L4 | Table | 单表内部一致性 | `E1301` 族 |
| L5 | Reference | 跨配置引用 | `E1401` 族 |
| L6 | Semantic | 表达式/组合逻辑 | `E1501` 族 |
| L7 | Game Rule | 业务插件校验 | `E1601` 族 |

## L0 Syntax（语法）

首先检查输入是否可以被正确解析。例如：

```yaml
foo:
    - a
      - b
```

YAML 本身非法。Cage：

```text
ERROR E0001

Source:
    config/test.yaml

Location:
    line 3

Invalid YAML syntax.
```

文本类 Source（JSON/YAML/CSV）的语法错误精确到行号。

## L1 Schema（结构）

检查配置结构是否符合 [Schema](/schema)。

缺少字段：

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

### E9006 字段可见性冲突（Profile 语义化）

`targets` 决定字段/表在哪些 profile 的视图里可见（空 = 全 profile）。
裁剪对视图是常规操作，但**结构上不可缺的字段被 profile 隐藏就是冲突**，
报 `E9006` 而不是静默过滤：

- `required` 且无 `default` 的字段被当前 profile 隐藏；
- 主键字段被隐藏；
- 唯一约束成员被隐藏；
- 可见表的引用字段，其目标表或目标字段被隐藏（投影后悬空引用）。

有 `default` 的 required 字段、非必需字段、整表被 `targets` 剔除，都是
合法视图，不报错。检查对完整 schema 执行（`profile` 只裁剪产物视图、不
豁免数据校验），`cage check/build --profile <p>` 与 `cage gen` 均按此
语义运行。示例：

```text
ERROR E9006

Account.secret

Required field hidden by profile (targets: ["server"], profile: "client")
```

## L2 Type（类型）

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

Map 字段的键值逐项校验：键按 `key_type` 门控（string 键恒合法；int 键
必须是可 `parse::<i64>` 的数字字符串），值按 `value_type` 递归校验，
不匹配同样走 `E1101`（提示带 `map<key, value>` 拼写的期望类型）；空 map
恒合法。

## L3 Value（值）

类型正确也不代表值合理。Schema：

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

支持的值约束：

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

## L4 Table（表级）

检查单张表内部的一致性。例如 ID 必须唯一，数据：

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

## L5 Reference（引用）

游戏配置中最重要的能力之一。例如：

**Item**

```text
ID
10001
10002
10003
```

**Monster**

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

### 引用不应只验证「存在」

```text
Monster.DropItemID
        |
        v
Item.ID
```

Item 存在（`Item[10001]`），但 `Item[10001].Type = QuestItem`，而 Monster 掉落只允许 `Weapon / Armor / Consumable`。这时候应该继续报告：

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
Existence        存在性
Type             引用对象类型
Predicate        谓词约束
Cardinality      基数
Compatibility    兼容性
```

## L6 Semantic（语义）

Schema 解决的是结构问题；Semantic Validation 解决：**数据组合起来有没有意义。**

例如：

```text
MinLevel <= MaxLevel
BuyPrice >= SellPrice
Skill.Cost >= 0
Monster.DropTableID 必须存在
```

可以定义表达式规则：

```yaml
rules:
  - name: level_range
    assert: min_level <= max_level

  - name: price_range
    assert: sell_price <= buy_price
```

违反断言报 `E1501`，定位到行级。

## L7 Game Rule（业务插件，已实装）

复杂的游戏业务逻辑不应该全部塞进 Schema DSL。层次应该是：

```text
Schema
Expression
Code Validator
```

业务校验做成插件（Rust trait，进程内注册）：

```rust
pub trait GameRuleValidator: Send + Sync {
    fn name(&self) -> &'static str;
    fn validate(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic>;
}

// cage check --level gamerule 运行内建注册表；嵌入方亦可自行组装：
let mut registry = GameRuleRegistry::with_builtins();
registry.register(Box::new(DropTableValidator));
let diagnostics = registry.run(&schema, &document);
```

内建样例规则 `power_curve` 端到端可用：任一表同时含 `level`/`attack`
字段时校验 `attack <= level * 100 + 50`，违者报 `E1601`（行级定位，
hint 带计算过程）：

```text
ERROR E1601 — Game Rule Validation Failed
  Source: config/monster.json | Row: 1
  Table: Monster
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)
```

这样 Cage Core 不需要理解具体游戏业务。执行模型的选型（进程内
trait → 动态库 → 沙箱）与信任边界见
[架构：安全与隔离](/architecture#安全与隔离)；完整对比表与分层决策
见仓库设计稿 `docs/design.md` §17.1「插件沙箱方案定稿」。

## 分级执行

这些层级不是必须全部执行：

```bash
cage check                      # 执行完整验证
cage check --level schema       # 只做 Schema 层
cage check --level reference    # 只做到引用层
```

详见 [CLI](/cli)。

## Diagnostics：一等公民

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

完整示例：

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

严重级别支持：

```text
ERROR
WARNING
INFO
```

这样诊断可以直接被 CLI、IDE、CI、Web UI 消费。

## Warning Policy

建议支持 warning / error 升级策略，允许 CI 配置：

```yaml
ci:
  warnings_as_errors: false    # 日常开发
```

生产构建：

```yaml
ci:
  warnings_as_errors: true     # 门禁收紧
```

## 错误码表

错误码格式 `E{类别}{编号}`，类别对应验证层级；每码一文档说明，定义于 `cage-core`。

### L0 Parse

| 代码 | 含义 |
| --- | --- |
| `E0001` | 语法错误 |
| `E0002` | 输入意外结束 |
| `E0003` | 字符编码非法 |
| `E0004` | 结构畸形（如 YAML 映射错误） |

### L1 Schema

| 代码 | 含义 |
| --- | --- |
| `E1001` | 缺少必填字段 |
| `E1002` | Schema 未定义的未知字段 |
| `E1003` | Schema 字段重复定义 |
| `E1004` | Schema 定义非法（如循环引用） |
| `E1005` | Schema 文件缺失或不可读 |

### L2 Type

| 代码 | 含义 |
| --- | --- |
| `E1101` | 类型不匹配 |
| `E1102` | 无法强制转换到目标类型 |
| `E1103` | 数值溢出 / 下溢 |
| `E1104` | 枚举变体非法 |

### L3 Value

| 代码 | 含义 |
| --- | --- |
| `E1201` | 数值超出范围（min/max） |
| `E1202` | 字符串长度越界 |
| `E1203` | 正则不匹配 |
| `E1204` | 枚举值不允许 |
| `E1205` | 数组长度越界 |

### L4 Table

| 代码 | 含义 |
| --- | --- |
| `E1301` | 主键重复 |
| `E1302` | 组合唯一键重复 |
| `E1303` | 缺少必需行 |
| `E1304` | 行序违规 |
| `E1305` | 必填表为空 |

### L5 Reference

| 代码 | 含义 |
| --- | --- |
| `E1401` | 引用目标不存在 |
| `E1402` | 引用已删除实体 |
| `E1403` | 循环引用 |
| `E1404` | 基数违规 |
| `E1410` | 引用对象存在但语义谓词不满足 |
| `E1411` | 引用对象字段约束违规 |

### L6 Semantic

| 代码 | 含义 |
| --- | --- |
| `E1501` | 断言表达式为假 |
| `E1502` | 跨字段约束违规 |
| `E1503` | 表达式求值错误（如除零） |

### L7 Game Rule

| 代码 | 含义 |
| --- | --- |
| `E1601` | 插件校验器报告错误 |
| `E1602` | 插件未找到 |
| `E1603` | 插件执行失败（panic、超时等） |

### Build / Target（生成期）

| 代码 | 含义 |
| --- | --- |
| `E9001` | 找不到目标格式的生成器 |
| `E9002` | 目标生成失败 |
| `E9003` | 确定性构建被破坏（输出不可复现） |
| `E9004` | Manifest 生成失败 |
| `E9005` | Profile 不存在 |
| `E9006` | 字段可见性冲突 |

### Editor（编辑器交换，第三阶段）

| 代码 | 含义 |
| --- | --- |
| `E1701` | 编辑态文档无法解析为 Schema（语法/形状/未知 kind），JSON 路径定位 |

### Registry（配置仓库，第三阶段 R 系列）

| 代码 | 含义 |
| --- | --- |
| `E1801` | 发布冲突：同版本重发但字节不同（注册表不改写历史）、非法包/版本名、索引写入失败 |
| `E1802` | 注册表引用无法解析：`registry:` 规范非法、包/版本不存在、版本需求非法、显式版本不满足 `[dependencies]` pin、区间内无满足版本、未接 `[registry].path`、索引损坏 |
| `E1803` | 注册表条目账本校验失败（篡改/增删文件）——发布与解析两侧都拒收 |

### Internal（系统级）

| 代码 | 含义 |
| --- | --- |
| `E9901` | 内部不变量破坏（bug） |
| `E9902` | 源读取 I/O 错误 |
| `E9903` | 配置错误（CLI 参数、文件缺失） |
| `E9904` | 插件加载失败 |
