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

## 后续扩展

```text
XML
TOML
SQLite
MySQL            ← 已立项（design §45 Remote Source，2026-10 设计定稿）
PostgreSQL       ← 已立项（design §45）
Google Sheets    ← 已立项（design §45）
HTTP API         ← 已立项（design §45）
Custom Binary
```

四源形态定稿（句法、映射、确定性锚点、E19xx 错误码规划）见仓库设计稿
`docs/design.md` §45；实现拆解见仓库根 `todo.md` 的 S 系列。

## Rust 落地

| 输入源 | crate | 主要依赖 |
| --- | --- | --- |
| Excel | `cage-source-excel` | calamine |
| CSV | `cage-source-csv` | csv |
| JSON | `cage-source-json` | serde_json |
| YAML | `cage-source-yaml` | serde_yaml |

统一产出 `cage-core` 的 Canonical Model：Null / Bool / Int / UInt / Float / String / Bytes / Array / Object，并携带 Source Location 与元数据。
