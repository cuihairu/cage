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

文本类源在解析期做基础类型推断（如 CSV 里 `100` 推断为整数、`true` 推断为布尔），Schema 随后在 L2 按目标类型**严格**校验（族不匹配即 `E1101`，不做字符串↔数字重校准——数值文本写进 String 列会报类型错误）；Source 不做业务校验、不做转换决策。

## 第一阶段支持

```text
Excel
CSV
JSON
YAML
MessagePack
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

Excel Adapter（基于 [calamine](https://crates.io/crates/calamine) 读取 `.xlsx`/`.xlsm`/`.ods`）可以处理：

- Workbook
- Worksheet
- Header
- Cell
- Row
- Column
- Merged Cells（合并单元格回填；仅 `.xlsx`，`.ods` 不做）
- Formula（公式缓存值，不重算）
- Cell Type

单元格注释（Comment）与表级元数据（Sheet Metadata）未实装。

错误定位精确到单元格（诊断 `Source` 字段渲染形如
`monster.xlsx | Sheet: Monster | Row: 27 | Col: G`）。

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
- 解析期做类型推断：`true`/`false`/`yes`/`no` → Bool，整数与浮点文本 →
  Int/UInt/Float，空单元格 → Null，其余 → String；Schema 在 L2 按目标
  类型严格校验（数值文本进 String 列会报 `E1101`，引号是 CSV 语法、
  不改变内容类型）

### JSON / YAML

- 天然树形结构，直接映射到 Canonical Model
- YAML 根形态：顶层 Sequence（元素为 Mapping）按多表读，顶层 Mapping
  按单行表读，null 跳过；支持 `<<: *anchor` 合并键展开
- 保留路径定位：文本格式错误（YAML 语法错误等）精确到行号（错误码 `E0001` 族）

### MessagePack

- 文件形态即 msgpack target 的线格式：裸行对象数组
  `[{"id": 1, "name": "Sword"}, …]`；二进制不带表名，文件名 stem
  命名表（CSV 惯例）
- 浮点按位还原、`bin` 载荷还原为 `Bytes`、超出 `i64::MAX` 的整数保留
  `UInt` 族——JSON 往返无法表达的三类值；非负整数归一为 `Int`
  （与 JSON 源一致：线格式的正数标记不携带符号性）
- 空数组产出空表（不跳过），保证 target → source 往返完整
- 形状违规（根非数组、行非对象、尾随字节、截断帧）按 `E0001` /
  `E0004` 报错

### 源目录装载范围

`[source_roots]` 目录发现收集 `json` / `yaml` / `yml` / `csv` /
`xlsx` / `xls` / `msgpack` 扩展名；其中 `.xls` 旧格式被收集后按 `E0001` 拒绝，
`.xlsm`/`.ods` 不在目录收集清单内（目录模式下被静默忽略——单文件
`schema_path`/`source` 显式指定时适配器本身支持 `.xlsx`/`.xlsm`/`.ods`）。
远程源根（`registry:`，见 [CLI · registry](/cli#registry)）同样可作
`[source_roots]` 取值。

### HTTP API（远程源，S1）

`[source_roots]` 直写 http(s) URL 即远程源：GET 响应字节原样落
`.cage-cache/source/<URL 指纹>/`，再走与本地 JSON 完全相同的解析
（`{表名: 行数组}` / 单对象 → `Root` / 行数组 → `Data`），L0-L7
全量校验、无旁路。连接类失败有界重试；取数失败报 `E1901`，
401/403 报 `E1902`，坏 JSON 报 `E0001`（与本地文件同一诊断）。
四源形态定稿见 design §45。

分页（S7）：响应带 RFC 8288 `Link: <...>; rel="next"` 头时沿链取页
——每页必须是 JSON 行数组，页序拼接成一份合并文档（裸数组 →
`Data` 表）落缓存；相对 next 目标对页 URL 解析，回环与超过 1000 页
均定错（`E1901`）。无 next 头的响应保持单 GET 契约逐字节不变。
限流协商：429 带 `Retry-After`（秒数或 HTTP-date）且 ≤ 30s 时按其
退避重试（与传输重试共用预算），缺失 / 不可解析 / 超上限即时
`E1901` 定错。分页契约违规与限流定错不参与离线回退——只有传输类
失败（服务器不可达）才回退缓存（`E1906` WARNING）。

条件 GET 增量拉取（S8，文档级）：响应带 `ETag` / `Last-Modified`
头时，验证器随字节一起存进缓存旁的 sidecar meta
（`.cage-cache/source/<URL 指纹>/<URL 指纹>.meta.json`）；同 URL
再次取数时先带 `If-None-Match` / `If-Modified-Since` 发条件请求，
服务器回 **304 则缓存字节原样复用**（连 meta 都不重写），200 则新
字节新验证器双双替换。meta 与 payload 必须同时存在才做
revalidation——缺一半即按无缓存走普通 GET 重建。只有首页 GET 带
条件（首页的验证器描述整个集合，304 时分页链不必重走）；
`--no-cache`（strict）依然 revalidate——304 是服务器确认缓存，
不是缓存自答。传输失败仍走 S4 离线回退语义，304 不参与。行级
增量（按 revision / updated_at 取变更行）仍留待实现期。

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
| MessagePack | `cage-source-msgpack` | rmpv |
| HTTP API | `cage-source-http` | `cage_core::remote`（取数 / 重试 / 缓存）+ cage-source-json |
| MySQL / PostgreSQL | `cage-source-db` | mysql / postgres（纯 Rust 协议客户端）+ `cage_core::remote`（缓存键）+ cage-source-json |
| Google Sheets | `cage-source-sheets` | `cage_core::remote`（取数 / 重试 / 缓存）+ cage-source-json |

统一产出 `cage-core` 的 Canonical Model：Null / Bool / Int / UInt / Float / String / Bytes / Array / Object，并携带 Source Location 与元数据。
