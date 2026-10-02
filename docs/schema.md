# Schema：结构定义

Schema 定义配置的结构和约束：字段、类型、必填、默认值、范围、枚举、数组、对象、唯一性、引用、输出信息。它是独立于输入源的 DSL 文件（YAML），不依附于任何一种 Source。

## Schema 与 Source 解耦

不要把 Schema 固定写进 Excel：

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

**同一个 Schema 可以用于多个 Source。**策划在 Excel 里维护数值，程序在 YAML 里写工具配置，两者可以共享同一套结构与校验规则。

## DSL 示例

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

字段间可以声明引用关系：

```yaml
DropItemID:
  type: uint32

  reference:
    table: Item
    field: ID
```

## 字段约束清单（MVP）

| 约束 | 说明 |
| --- | --- |
| `type` | 字段类型：`uint32` / `int64` / `float` / `string` / `bool` / `enum` / `array` / `object` 等 |
| `required` | 必填（缺失报 `E1001`） |
| `default` | 默认值（缺省时填充） |
| `enum` / `values` | 枚举取值域 |
| `min` / `max` | 数值范围 |
| `min_length` / `max_length` | 字符串长度范围 |
| `regex` | 正则匹配 |
| `unique` | 唯一性 |
| `primary_key` | 主键（表级） |
| `reference` | 跨表引用（`table` + `field`） |

Schema 校验由 [Validation 流水线](/validation) 的 L1-L3 层执行。
