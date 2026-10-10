# Schema：结构定义

Schema 定义配置的结构和约束：字段、类型、必填、默认值、范围、枚举、数组、对象、唯一性、引用、输出信息。它是独立于输入源的 DSL 文件（YAML 或 JSON，`schema_path` 可为单文件或目录——目录下按文件名序合并加载全部 `yaml`/`yml`/`json`），不依附于任何一种 Source。

> 本文示例均为实际 wire 格式，可直接装载。可运行的完整工程见
> [`examples/game-config/schemas/`](https://github.com/cuihairu/cage/blob/main/examples/game-config/schemas)。

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

Schema 文件的根键是 `tables` / `enums` / `metadata`。每张表必须带
`name` 与 `primary_key`，每个字段必须带 `name`：

```yaml
tables:
  Item:
    name: Item
    description: 道具表
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 }, required: true, min: 1 }
      title: { name: title, type: { kind: String }, required: true, max_length: 64 }
      price: { name: price, type: { kind: UInt32 }, min: 0 }
      type:
        name: type
        type: { kind: Enum, value: ItemType }
        required: true

enums:
  ItemType:
    name: ItemType
    values:
      - { name: Weapon, value: 1 }
      - { name: Armor, value: 2 }
      - { name: Consumable, value: 3 }
```

字段间可以声明引用关系：

```yaml
      drop_item_id:
        name: drop_item_id
        type: { kind: UInt32 }
        reference:
          table: Item
          field: id
```

`reference` 还有三个可选子键：`predicate`（引用对象须满足的语义谓词，
求值器当前为占位、恒通过）、`cardinality`（`one`/`many`/`optional`，
默认 `one`，基数校验预留未接线）、`compatible_with`（引用对象类型
兼容性检查，产出 `E1411`）。

## 字段类型（19 种）

类型一律写成 adjacent-tag 形态 `type: { kind: X }`（带参数的类型在
`value` 里传参）；小写标量形态（`type: uint32`）**无法解析**：

| kind | 说明 |
| --- | --- |
| `Null` / `Bool` | 空值 / 布尔 |
| `Int8` `Int16` `Int32` `Int64` | 有符号整数 |
| `UInt8` `UInt16` `UInt32` `UInt64` | 无符号整数 |
| `Float32` `Float64` | 浮点数 |
| `String` / `Bytes` | UTF-8 字符串 / 原始字节 |
| `Array` | 数组，`value` 为元素类型：`{ kind: Array, value: { kind: Int32 } }` |
| `Object` | 对象，`value` 为属性表：`{ kind: Object, value: { x: { kind: Int32 } } }` |
| `Map` | 映射 `map<K,V>`，`value: { key_type: string\|int, value_type: { kind: … } }`，可嵌套 |
| `Enum` | 命名枚举引用，`value` 为枚举名（须在顶层 `enums:` 注册） |
| `Any` | 任意值 |

## 字段约束清单

| 约束 | 说明 |
| --- | --- |
| `type` | 字段类型（见上表，`{ kind: X }` 形态） |
| `required` | 必填（缺失报 `E1001`） |
| `default` | 声明缺省值；当前用于 `E9006` 可见性豁免与生成代码的成员初始值，**数据行缺失该字段仍按 `E1001` 报错**（不做数据回填） |
| `min` / `max` | 数值范围（`E1201`） |
| `min_length` / `max_length` | 字符串长度范围（`E1201`） |
| `pattern` | 字符串须匹配的正则（`E1203`）。注意：写 `regex:` 会被当作自定义元数据**静默忽略** |
| `enum_values` | 内联枚举取值域（`E1204`）。注意：写 `values:` 会被静默忽略；命名枚举走顶层 `enums:` + `{ kind: Enum, value: 名 }` |
| `min_items` / `max_items` | 数组元素个数范围（`E1205`） |
| `items` | 数组元素的字段 schema（递归） |
| `properties` / `additional_properties` | 对象属性 schema 与未知键开关 |
| `reference` | 跨表引用（`table` + `field`，`E1401`；见上文子键说明） |
| `targets` | 字段对哪些 profile 可见（空 = 全部；`"*"` 等价空；被隐藏的结构必需字段报 `E9006`） |
| `rules` | 字段级语义规则（`name`/`assert`/`message`/`warning_only`；表达式求值器当前为占位、恒通过） |

## 表级约束

| 约束 | 说明 |
| --- | --- |
| `name` / `description` | 表名（必填）与描述 |
| `primary_key` | 主键字段列表（必填；重复报 `E1301`） |
| `unique_constraints` | 组合唯一约束列表：`- { name: name_unique, fields: [name] }`（`E1302`）。注意：字段上没有 `unique:` 键，写了会被静默忽略 |
| `order_by` | 行排序要求：字段名列表（`E1304`） |
| `targets` | 表对哪些 profile 可见（空 = 全部；整表剔除或隐藏必需表报 `E9006`） |

未知键不会报错——经 `#[serde(flatten)]` 落入自定义元数据。写错约束键
名（如 `regex`、`values`、字段级 `unique`）因此**静默无效**，校验时
该约束形同不存在。

Schema 校验由 [Validation 流水线](/validation) 分级执行：required 在
L1、类型在 L2、值域（min/max/length/pattern/enum/items）在 L3、唯一性
与主键在 L4、跨表引用在 L5。

## 分环境约束覆盖（`env_overrides`，设计 §48）

同一张表在不同环境用不同严格度——dev 放宽、prod 收紧。环境是同一字
段形状的一档「严格度旋钮」：表级侧表 `env_overrides` 声明
**环境名 → 字段名 → 约束补丁**，补丁只列该环境要动的约束键
（`required` / `min` / `max` / `min_length` / `max_length` /
`pattern` / `enum_values` / `min_items` / `max_items`），未列的键保
持基线值，列出的键整值替换；**类型不在补丁里**（环境只调严格度，不
重塑形状），空补丁定错，补丁里的字段名必须存在于本表。

```yaml
tables:
  Hero:
    # … 基线字段 …
    env_overrides:
      dev:
        hp: { required: false }          # 放宽
      prod:
        hp: { required: true, min: 100 } # 收紧
        rarity: { enum_values: [common, rare, epic] }
```

环境名来自各表 `env_overrides` 键的并集（Schema 自声明，不在
cage.toml 登记）。`cage check` / `build` / `gen` 以 `--env <名>` 选
环境：解析发生在校验与生成之前（基线字段被抹成该环境的值，基线本身
不被修改），所有校验器与代码生成看到的都是一份普通 Schema；未声明
的环境名与无 `env_overrides` 的 Schema 传 `--env` 都是用法错误
（exit 2）。构建账本随环境轮换：`schema_hash` 对解析后 Schema 计
算，manifest 记录 `environment` 字段，换环境不复用上一环境的构建产
物。结构性坏覆盖（指向不存在字段 / 空补丁）两道守卫：不带 `--env`
时装载期逐条 `warning:` 提示（不阻断，坏覆盖不会潜伏到有人选环境才
暴露）；带 `--env` 时硬错误（exit 2）。`cage snapshot` /
`registry publish` 同样支持 `--env` 出环境化包（见
[build](/build#configuration-snapshot) 与 [cli](/cli#snapshot)）。

