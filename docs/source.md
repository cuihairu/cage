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
四源形态定稿见 design §45；Google Sheets 在 S3。

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

## 后续扩展

```text
XML
TOML
SQLite
Google Sheets    ← 已立项（design §45，S3）
Custom Binary
```

## Rust 落地

| 输入源 | crate | 主要依赖 |
| --- | --- | --- |
| Excel | `cage-source-excel` | calamine |
| CSV | `cage-source-csv` | csv |
| JSON | `cage-source-json` | serde_json |
| YAML | `cage-source-yaml` | serde_yaml |
| HTTP API | `cage-source-http` | `cage_core::remote`（取数 / 重试 / 缓存）+ cage-source-json |
| MySQL / PostgreSQL | `cage-source-db` | mysql / postgres（纯 Rust 协议客户端）+ `cage_core::remote`（缓存键）+ cage-source-json |

统一产出 `cage-core` 的 Canonical Model：Null / Bool / Int / UInt / Float / String / Bytes / Array / Object，并携带 Source Location 与元数据。
