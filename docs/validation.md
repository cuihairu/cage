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

未知字段（默认告警，`warnings_as_errors = true` 时升级为错误）：

```text
WARNING E1002

Item[10001]

Unknown field:
    pric
```

### E9006 字段可见性冲突（Profile 语义化）

`targets` 决定字段/表在哪些 profile 的视图里可见（空 = 全 profile；`"*"` 通配等价空）。
裁剪对视图是常规操作，但**结构上不可缺的字段被 profile 隐藏就是冲突**，
报 `E9006` 而不是静默过滤：

- `required` 且无 `default` 的字段被当前 profile 隐藏；
- 主键字段被隐藏；
- 唯一约束成员被隐藏；
- 可见表的引用字段，其目标表或目标字段被隐藏（投影后悬空引用）。

有 `default` 的 required 字段、非必需字段、整表被 `targets` 剔除，都是
合法视图，不报错。检查对完整 schema 执行（`profile` 只裁剪产物视图、不
豁免数据校验），`cage check/build --profile <p>` 与 `cage gen` 均按此
语义运行。`--env`（分环境约束覆盖，设计 §48）与 profile 正交且先于
profile 生效：环境补丁先抹到基线字段上（`required` 随之变化），E9006
按解析后的 `required` 判定——环境收紧后被子 profile 隐藏即冲突。示例：

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
      level:
        name: level
        type: { kind: UInt32 }
```

错误：

```text
ERROR E1101

Item[10001].Level

Expected:
    UInt32

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
        name: level
        type: { kind: UInt32 }
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
min / max               数值范围（E1201）
min_length / max_length 字符串长度（E1202）
pattern                 正则匹配（E1203）
enum_values             枚举取值域（E1204）。命名枚举（type: {kind: Enum, value: 名}）
                        的成员资格同在本层、同走 E1204——对顶层 enums: 声明的成员做
                        检查；只查字符串值，非字符串类型失配归 L2（E1101），
                        悬空枚举名归 L1（E1004）
min_items / max_items   数组元素个数（E1205）
```

`unique` 属 L4（E1301/E1302）、`required` 属 L1（E1001），不在本层。

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
组合唯一（unique_constraints，E1302）
排序要求（order_by，E1304）
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
      drop_item_id:
        name: drop_item_id
        type: { kind: UInt32 }
        reference:
          table: Item
          field: id
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

Item 存在（`Item[10001]`），但 `Item[10001].Type = QuestItem`，而 Monster 掉落只允许 `Weapon / Armor / Consumable`。用 `compatible_with` 声明目标字段须兼容的取值，违反报 `E1411`：

```text
ERROR E1411

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

Reference 的检查面分五类，实装状态：

```text
Existence        存在性          已实装（E1401）
Type             引用对象类型     已实装（E1411，compatible_with）
Predicate        谓词约束         已实装（E1410，断言在引用目标行上求值）
Cardinality      基数            已实装（E1404，源侧口径：one/optional 单值、many 数组逐元素）
Compatibility    兼容性          已实装（E1411，字段级约束）
```

### 表级引用环（E1403，已实装）

L5 在逐行引用检查之后做 schema 级循环检测：引用图
（`DependencyGraph::from_schema`）含环时每环报一条 `E1403` WARNING，
环路径写进诊断消息（最小表名起头、跨起点去重——两表互引只报一条）：

```text
WARNING E1403 — Circular Reference
  Source: schema | Table: Item

Circular reference: Item → Kit → Item
```

自引用（如解锁链 `Stage.next → Stage.id`）按引用图自身契约计环，同样
提示。环不阻断构建——生成是单遍的、增量传播对环安全——但会让库 API
的 `topological_sort` / `build_order` 失败，故以 WARNING 提示作者而
非报错（ERROR 会误杀合法互引 schema）；`warnings_as_errors = true`
时升级为错误（退出码 1）。

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
      level:
        name: level
        type: { kind: Int32 }
        rules:
          - name: level_range
            assert: min_level <= max_level
```

规则声明随 Schema 解析（`name`/`assert`/`message`/`warning_only`
齐全），求值器按**单比较断言** `operand OP operand` 实装（操作数为
字段引用——裸名或反引号名——与数字/字符串/布尔字面量；运算符
`== != <= >= < >`）。求值口径：

- 数值族（Int/UInt/Float）按数值比较，Int/UInt 走 i128 精确、混入
  Float 走 f64；字符串按字典序；布尔 false < true。
- 引用的字段在该行缺失或为 null（可选字段缺省）→ 规则对此行不表态，
  视为通过。
- 两侧类型不可比（如 string vs int、数组/对象/bytes）→ 记 `E1501`
  违例并在提示里点名两侧类型，绝不静默放过。
- 断言为假 → `E1501`（`message` 优先为提示，缺省渲染
  `Assertion '<assert>' failed`）；`warning_only: true` 降为 WARNING。
- **规则缺陷是 schema 错误**：断言解析失败或引用了表内未声明字段，
  以 `E1004` 每规则报一次（与行数无关，空表也报），该规则不对任何
  行求值。引用谓词（L5）的同类缺陷同码在 L5 报出。

更复杂的业务判定（跨行、算术、多条件组合）仍归 L7 业务规则插件。

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
cage check                      # 执行 L0–L6（默认 --level semantic）
cage check --level schema       # 只做 Schema 层
cage check --level reference    # 只做到引用层
cage check --level gamerule     # 加上 L7 业务规则插件
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

完整示例（实际渲染形态）：

```text
ERROR E1401 — Reference Target Missing
  Source: config/Monster.json | Sheet: Monster | Col: G27
  Table: Monster
  Row: 20003
  Field: DropItemID
  Value: 99999
  Message: reference target not found: Item.id = 99999
  Hint: Add Item[99999] or change DropItemID.
```

严重级别支持：

```text
ERROR
WARNING
INFO
```

CLI 与 Web UI 已直接消费这套诊断；IDE 插件留待后续接入。

## Warning Policy（已实装）

warning → error 升级策略是 `cage.toml` 的顶层开关（默认 `false`）：

```toml
# 日常开发：警告只提示，不阻断构建
warnings_as_errors = false

# 生产构建 / CI 门禁：警告升级为错误
warnings_as_errors = true
```

开启后同一处 warning 在 `cage build` 中按错误处理（退出码 1、不产
产物）；关闭时 `cage build` 对仅含 warning 的配置照常成功。CI 侧的
`warnings_as_errors` 门禁指 GitHub Actions 的 `-D warnings`（rustc
lint 层），与本配置项不同层，两者都在跑（见仓库 ci.yml）。

## 错误码表

错误码格式 `E{类别}{编号}`，类别对应验证层级；每码一文档说明，定义于
`cage-core::error::codes`。表为**全量定义注册表**；标注「预留」的码已
定义（常量 + 标题齐全）但当前代码路径尚未抛出——能力状态如实标注，
接线后去标。

### L0 Parse

| 代码 | 含义 |
| --- | --- |
| `E0001` | 语法错误（另：L1 对「schema 有 required 字段但文档缺整表」复用本码发 Warning） |
| `E0002` | 输入意外结束（预留） |
| `E0003` | 字符编码非法（预留） |
| `E0004` | 结构畸形（如 YAML 映射错误） |

### L1 Schema

| 代码 | 含义 |
| --- | --- |
| `E1001` | 缺少必填字段 |
| `E1002` | Schema 未定义的未知字段 |
| `E1003` | Schema 字段重复定义（预留） |
| `E1004` | Schema 自身引用悬空（pk/unique/枚举/引用指向不存在的字段、枚举或表；仅 Web API 装载路径执行，表间循环依赖在构建排序期以无码错误报告）；语义规则/引用谓词的断言畸形或引用未声明字段（L5/L6 求值期每规则报一次） |
| `E1005` | Schema 文件缺失或不可读（预留） |

### L2 Type

| 代码 | 含义 |
| --- | --- |
| `E1101` | 类型不匹配 |
| `E1102` | 无法强制转换到目标类型（预留） |
| `E1103` | 数值溢出 / 下溢（预留） |
| `E1104` | 枚举变体非法（预留） |

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
| `E1303` | 缺少必需行（预留） |
| `E1304` | 行序违规 |
| `E1305` | 必填表为空（预留） |

### L5 Reference

| 代码 | 含义 |
| --- | --- |
| `E1401` | 引用目标不存在 |
| `E1402` | 引用已删除实体（预留） |
| `E1403` | 循环引用（已实装：表级引用环 WARNING，warnings_as_errors 升级） |
| `E1404` | 基数违规（已实装，**源侧语义**，2026-10-10 拍板：`cardinality` 声明本字段如何引用目标——`one`（默认）与 `optional` 取单值，`optional` 容忍 null 而 `one` 下 null 违例；`many` 要求数组并逐元素走 E1401/E1410 校验；形状不符（数组配 `one`、标量配 `many`、null 配 `many`）报本码并点名改法；未知拼写报 `E1004`。目标侧 1:1 读法被否决——作默认 `one` 会误杀全部合法 many-to-one 配置） |
| `E1410` | 引用对象存在但语义谓词不满足（谓词在目标行上求值；目标行可选字段缺失视为不表态；两侧类型不可比同样报本码） |
| `E1411` | 引用对象字段约束违规 |

### L6 Semantic

| 代码 | 含义 |
| --- | --- |
| `E1501` | 断言表达式为假（或两侧类型不可比；可选字段缺失不表态；`warning_only` 降 WARNING） |
| `E1502` | 跨字段约束违规（预留） |
| `E1503` | 表达式求值错误（如除零）（预留） |

### L7 Game Rule

| 代码 | 含义 |
| --- | --- |
| `E1601` | 插件校验器报告错误 |
| `E1602` | 插件未找到（预留） |
| `E1603` | 插件执行失败（panic、超时等）（预留） |

### Build / Target（生成期）

| 代码 | 含义 |
| --- | --- |
| `E9001` | 找不到目标格式的生成器（预留） |
| `E9002` | 目标生成失败 |
| `E9003` | 确定性构建被破坏（输出不可复现） |
| `E9004` | Manifest 生成失败（预留） |
| `E9005` | Profile 不存在（预留） |
| `E9006` | 字段可见性冲突 |

### Editor（编辑器交换，第三阶段）

| 代码 | 含义 |
| --- | --- |
| `E1701` | 编辑态文档无法解析为 Schema（语法/形状/未知 kind），JSON 路径定位 |

### Registry（配置仓库，第三阶段 R 系列）

| 代码 | 含义 |
| --- | --- |
| `E1801` | 发布冲突：同版本重发但整包指纹不同（`build_id`/`content_hash` 任一不同——数据、schema、`--env` 环境变化都算；注册表不改写历史）、非法包/版本名、索引写入失败 |
| `E1802` | 注册表引用无法解析：`registry:` 规范非法、包/版本不存在、版本需求非法、显式版本不满足 `[dependencies]` pin、迁移链终点版本落在 pin 外（联动门，design §46）、区间内无满足版本、未接 `[registry].path`、索引损坏、远程根不可达（无缓存兜底） |
| `E1803` | 注册表条目账本校验失败（本地篡改/增删文件，或远程下载字节与账本不符、远程账本损坏）——发布与解析两侧都拒收；`registry verify` 全册审计同码报告条目损坏、index 记录与账本漂移、无 index 记录的孤儿目录 |

### Remote Source（远程数据源，S 系列）

| 代码 | 含义 |
| --- | --- |
| `E1901` | 远端取数失败：网络 / DNS / 超时（有界重试用尽）、404 或其他非认证错误状态、429 无 `Retry-After` / 不可解析 / 超过 30s 上限或限流重试用尽、分页契约违规（非数组页 / 回环 / 超 1000 页 / 坏 next 链接）、非规范 gsheet spec / 非法 spreadsheet id（HTTP / Sheets 源已接线）；DB 连接 / DSN 非法 / 会话只读 pin 失败 / 语句执行失败（DB 源已接线） |
| `E1902` | 远端认证 / 授权被拒（HTTP 401 / 403）（HTTP / Sheets 源已接线） |
| `E1903` | 远端响应形状不合法：Sheets 响应非行集 / 空表头 / majorDimension 非 ROWS（Sheets 源形状门已接线） |
| `E1904` | 凭据缺失：`[remote.<scheme>].dsn_env` / `[remote.gsheets].credential_env` 未声明，或声明的 env 未设置 / 为空（DB / Sheets 源已接线） |
| `E1905` | 远端查询非法：非 SELECT 开头、多语句（分号）、注释、行锁子句、`INTO`、具名查询外的非安全表名（DB 源已接线） |
| `E1906` | 远端不可达但已回退上一份缓存副本（离线回退 WARNING——非致命，stderr 提示；只对传输类失败发生，404 / 401/403 永不回退；`--no-cache` 关闭回退还原硬 `E1901`） |

HTTP 源（`cage-source-http`，design §45 S1）的坏 JSON 不走 E1903——它走与本地文件同一条 Parse 诊断（`E0001` 带行列定位）。DB 源（`cage-source-db`，§45 S2）的行集由适配器自产 canonical JSON，形状不可能非法，同样不经 E1903。DB 源装载顺序：解析 spec → 具名查询 / 表名解析（E1905）→ SELECT 白名单（E1905）→ DSN env 解析（E1904）→ 连接（E1901）——白名单与凭据校验都在任何网络触达之前。Sheets 源（`cage-source-sheets`，§45 S3）装载顺序：解析 spec 与 spreadsheet id（E1901）→ credential env 解析（E1904）→ 取数（401/403 → E1902，其余 → E1901）→ 形状门（E1903）——spec 与凭据校验同样都在网络触达之前，错误诊断只引 `gsheet:<id>/<tab>` spec，API key 不落日志。三源的传输类取数失败（连接 / DNS / 超时）在缓存副本存在时回退并发 `E1906` WARNING（`--no-cache` 关闭回退），404 与 401/403 永不回退——旧字节不得掩盖远端已删除或访问被吊销。

### Artifact Distribution（分发，A 系列）

| 代码 | 含义 |
| --- | --- |
| `E2101` | 分发传输失败：bundle 导出读不到条目（包 / 版本不存在、条目目录或账本缺失、非法包名 / 版本名）、写不出 bundle 文件；push 的传输失败（重试用尽）、状态探针不可读、远端 index 不可读；presigned push 的 map 畸形 / URL 非绝对 http(s) / 缺 `index.put` / 覆盖缺口（任何 PUT 之前失败关闭）（已接线） |
| `E2102` | 分发目标拒绝凭据：push 的状态探针或任一 PUT 收到 HTTP 401 / 403；token 只出现在请求头，错误文本与日志永不携带（已接线） |
| `E2103` | bundle 账本校验失败：导入侧在 `verify_snapshot` 信任门拒收未过验字节——结构问题（成员路径逃逸、index 畸形、缺账本）、信任门拒绝、index 摘录与账本漂移都在落册前拒绝（已接线） |
| `E2104` | 分发目标拒绝写入：push 的 PUT 收到 405 / 409 或其他明确 4xx——服务端无写通道，回退「本地 publish + 静态托管」（已接线） |
| `E2105` | 分发凭据缺失：`--auth-env` / `[registry].auth_env` 未声明或声明的 env 未设置——任何网络触达之前失败（已接线） |
| `E2106` | 签名密钥不可用：`--key-env` 命名的环境变量未设置 / 空 / 非 base64 / 非 32 字节 Ed25519 密钥，或库拒绝密钥材料、keygen 落盘与 sidecar 写出失败（已接线） |
| `E2107` | bundle 签名校验失败：`.sig` sidecar 缺失或畸形、算法名不认、或分离式 ed25519 签名在可信公钥下验不过——bundle 由别的密钥签署或字节被改动（已接线） |

### Migration（迁移，M 系列）

| 代码 | 含义 |
| --- | --- |
| `E2001` | 迁移规则文件非法：读不到、非合法 YAML、缺 `from` / `to` / `steps`、`from` 等于 `to`、steps 为空、未知变换名、payload 畸形（已接线） |
| `E2002` | 迁移规则引用不合法：step 引用的表 / 字段在 from-schema 不存在、rename 目标与现有表 / 字段撞名（已接线） |
| `E2003` | 迁移步骤对数据不满足：加宽方向不安全（收窄 / 跨族 / Int64·UInt64→Float64 精度丢失）、行值超出加宽目标值域、step 引用的表或行字段文档中不存在——整段失败不静默丢数据（已接线） |
| `E2004` | 迁移后回验失败：迁移产物在新 schema 下 L0–L6 全栈校验不通过，错误清单随 E2004 一并输出（已接线） |

### Internal（系统级）

| 代码 | 含义 |
| --- | --- |
| `E9901` | 内部不变量破坏（bug）（预留） |
| `E9902` | 源读取 I/O 错误 |
| `E9903` | 配置错误（CLI 参数、文件缺失）（预留） |
| `E9904` | 插件加载失败（预留） |
