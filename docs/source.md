# Source：输入源

Source Adapter 是 Cage 的输入插件。每种输入格式一个 Adapter，统一产出 [Canonical Model](/architecture#canonical-model)。

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

重要原则：

> **Source Adapter 只负责读取和解析，不负责游戏业务逻辑。**

类型猜测（例如 CSV 里 `100` 是数字还是字符串）交给 Schema 校准；Source 不做业务校验、不做转换决策。

## 第一阶段支持

```text
Excel
CSV
JSON
YAML
```

### Excel

Excel 是游戏行业非常常见的 Authoring Source，但它不是 Cage 的核心抽象。

例如：

```text
Item.xlsx
Monster.xlsx
Skill.xlsx
Quest.xlsx
```

Excel Adapter（基于 [calamine](https://crates.io/crates/calamine) 读取）可以处理：

- Workbook
- Worksheet
- Header
- Cell
- Row
- Column
- Merged Cells（合并单元格）
- Formula（公式缓存值，不重算）
- Comment
- Cell Type
- Sheet Metadata

错误定位精确到单元格（如 `monster.xlsx Sheet Monster Cell G27`）。

最终转换成：

```text
Document
  |
  +-- Table: Item
  +-- Table: Monster
  +-- Table: Skill
```

### CSV

- 以表头行定义列名，行为数据行
- 定位到表头名 + 行号
- 所有单元格初始按字符串读取，类型由 Schema 校准

### JSON / YAML

- 天然树形结构，直接映射到 Canonical Model
- 保留路径定位：文本格式错误（YAML 语法错误等）精确到行号（错误码 `E0001` 族）

### HTTP API（远程源，S1）

`[source_roots]` 直写 http(s) URL 即远程源：GET 响应字节原样落
`.cage-cache/source/<URL 指纹>/`，再走与本地 JSON 完全相同的解析
（`{表名: 行数组}` / 单对象 → `Root` / 行数组 → `Data`），L0-L7
全量校验、无旁路。连接类失败有界重试；取数失败报 `E1901`，
401/403 报 `E1902`，坏 JSON 报 `E0001`（与本地文件同一诊断）。
四源形态定稿见 design §45。

### MySQL / PostgreSQL（远程源，S2）

`[source_roots]` 写 `mysql:<表|具名查询>` / `pg:<表|具名查询>`：
具名查询取自 `[remote.<scheme>].queries`，否则名字必须是安全表名
并展开为 `SELECT * FROM <name>`。装载期双重只读保险——静态 SELECT
白名单（单条、SELECT 开头，拒分号 / 注释 / 行锁 / `INTO` / CTE，
`E1905`）先跑，之后才解析 `[remote.<scheme>].dsn_env` 指名的
环境变量（未声明 / 未设置报 `E1904`，DSN 永不进 cage.toml），
连接后会话钉只读（MySQL `SESSION TRANSACTION READ ONLY`、PG
`default_transaction_read_only`）。类型口径：DECIMAL / NUMERIC
文本保真不走 Float，NULL → Null，二进制列 base64，其余按列类型
映射为 JSON 标量。行集序列化成 canonical JSON 落
`.cage-cache/source/<scheme+DSN+SQL 指纹>/`，再走标准 JSON 解析
——与 HTTP 源同一缓存锚与解析链。行序：带 `ORDER BY` 尊重原序，
否则按行序列化形式排序（构建不依赖服务端返回顺序）。连接 /
语句失败报 `E1901`。

### Google Sheets（远程源，S3）

`[source_roots]` 写 `gsheet:<spreadsheet_id>/<tab>`，凭据是 API key，
从 `[remote.gsheets].credential_env` 指名的环境变量读（未声明 /
未设置报 `E1904`，key 永不进 cage.toml 也不落任何日志——错误诊断只
引 spec）。取数走 Sheets API v4 `values`，
`valueRenderOption=UNFORMATTED_VALUE`（公式缓存值，不重算，同 Excel
adapter 口径）。映射同 Excel 惯例：tab = 表、首行 = 表头（空表头
单元格退 `col<i>`）、空行跳过、短行补 null 对齐表头宽，tab 自然行序
= 作者承诺序原样保留。单元格初值为字符串（数字 / 布尔保留 JSON
文本），类型交 Schema 校准。响应形状门（非行集 / 空表头 /
majorDimension 非 ROWS）报 `E1903`；401/403 报 `E1902`，取数失败报
`E1901`。canonical JSON 落
`.cage-cache/source/<gsheets+id+tab 指纹>/`（API key 不进指纹——
它不改变字节语义），再走标准 JSON 解析。

### 离线回退与 `--no-cache`（远程源通用，S4）

三个远程源共享同一条离线语义：**传输类**取数失败（连接拒绝 / DNS /
超时 / 服务不可达）时，若上一份缓存副本还在
`.cage-cache/source/`，构建不失败——回退解析那份副本，stderr 打
`E1906` WARNING（`warning: E1906 remote source unreachable, serving
the previous cached copy: <spec> (cache: <路径>)`）。两类失败**永不
回退**：404（远端已删除）与 401/403（访问可能已被吊销）——旧字节会
静默出错，保持硬失败（`E1901` / `E1902`）。DB 源的连接 / 语句失败
按传输类回退；凭据（`E1904`）与查询白名单（`E1905`）在任何网络触达
之前发生，旧字节不掩盖配置错误。回退出的缓存文件走与在线路径同一条
标准 JSON 解析与全量校验，无旁路。

`--no-cache`（`check` / `build` / `gen` / `inspect` 四命令）关闭
回退：远端取不到即失败，即使有缓存副本。适合「必须以远端最新字节
构建」的发布前核对。

## 后续扩展（非首期）

```text
XML
TOML
SQLite
Custom Binary
```

以上格式不在首期交付范围内；首期四大远程源（HTTP / MySQL / PostgreSQL / Google Sheets）已随 S1–S5 全量交付，详见上文各节。

## Rust 落地

| 输入源 | crate | 主要依赖 |
| --- | --- | --- |
| Excel | `cage-source-excel` | calamine |
| CSV | `cage-source-csv` | csv |
| JSON | `cage-source-json` | serde_json |
| YAML | `cage-source-yaml` | serde_yaml |
| HTTP API | `cage-source-http` | `cage_core::remote`（取数 / 重试 / 缓存）+ cage-source-json |
| MySQL / PostgreSQL | `cage-source-db` | mysql / postgres（纯 Rust 协议客户端）+ `cage_core::remote`（缓存键）+ cage-source-json |
| Google Sheets | `cage-source-sheets` | `cage_core::remote`（取数 / 重试 / 缓存）+ cage-source-json |

统一产出 `cage-core` 的 Canonical Model：Null / Bool / Int / UInt / Float / String / Bytes / Array / Object，并携带 Source Location 与元数据。
