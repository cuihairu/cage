# 完整示例

仓库内常驻一个端到端示例工程 [`examples/game-config/`](https://github.com/cuihairu/cage/tree/main/examples/game-config)——「角色成长 + 道具 + 关卡」三张表、三种源格式（CSV / YAML / JSON）、一份覆盖常用字段类型与约束用法的 schema、一个 profile 构建全部 10 个 target（json/csv 数据 + 8 种语言代码绑定），外加一套触发 L7 Game Rule 的坏数据。

本页所有命令与输出都是**真实执行结果**（在仓库根目录运行），不是伪代码。

## 一键跑通

```bash
bash examples/run.sh
```

脚本从 `cage check` 到 `cage build` 到 `cage gen`（含坏数据 E1601 断言与 `cage inspect`）一条命令跑完，每步打印真实命令；任一步失败立即退出，CI 里同款执行（示例烂了 CI 就红）。

<details>
<summary>examples/run.sh 完整输出（点击展开）</summary>

```text
==> [1/5] cage check examples/game-config   —— L0-L6 全量校验（好数据应通过）
$ cage check examples/game-config
cage check: OK (3 tables, 0 warnings, level <= semantic)

==> [2/5] cage check --level gamerule examples/game-config   —— L7 业务规则（好数据应通过）
$ cage check examples/game-config --level gamerule
cage check: OK (3 tables, 0 warnings, level <= gamerule)

==> [3/5] cage build --profile client   —— 验证 + 生成全部 10 个 target
$ cage build examples/game-config --profile client
cage build: OK (profile 'client', 42 artifacts, manifest examples/game-config/build/manifest.json)

==> [4/5] cage gen --profile client   —— 只生成 8 种代码绑定（跳过数据 target）
$ cage gen examples/game-config --profile client
cage gen: OK (profile 'client', 36 artifacts, manifest examples/game-config/build/manifest.json)

==> [5/5] 坏数据：attack=500 越过 power curve（level=1 上限 150）→ 期望 E1601 + 退出码 1
$ cage check examples/game-config/bad --level gamerule   # 期望退出码 1
ERROR E1601 — Game Rule Validation Failed
  Source: examples/game-config/bad/config/Character.csv | Row: 1
  Table: Character
  Row: 0
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)

cage check: FAILED (1 errors, 0 warnings)

==> 附：cage inspect examples/game-config   —— 查看表结构
$ cage inspect examples/game-config
project: rpg-demo
version: 0.1.0
tables:
  Character (4 rows)
  Item (5 rows)
  Stage (3 rows)
schemas:
  Character (8 fields, primary key: id)
  Item (7 fields, primary key: id)
  Stage (9 fields, primary key: id)

全部通过。
```

</details>

## 工程结构

```text
examples/game-config/
├── cage.toml          # 工程配置：一个 profile 构建全部 10 个 target
├── schemas/           # 三个 schema 文件（按名序合并加载，metadata 取自 character.yaml）
│   ├── character.yaml # 角色成长：枚举/唯一约束/正则/范围/默认值/保留字字段 class
│   ├── item.yaml      # 道具：整型枚举 + 字符串枚举 + 可选字段
│   └── stage.yaml     # 关卡：数组/对象/跨表引用/Map<string, Array<Int32>>
├── config/            # 三种源格式（同一 profile 一起加载）
│   ├── Character.csv  # CSV：文件名即表名
│   ├── Item.yaml      # YAML：根键 = 表名
│   └── Stage.json     # JSON：根键 = 表名
└── bad/               # 坏数据：完整复制品，只把首行 attack 改成 500
    ├── cage.toml      # schema_path = "../schemas"（共用同一份 schema）
    └── config/
```

## 逐条命令

### cage check —— 验证不生成

```console
$ cage check examples/game-config
cage check: OK (3 tables, 0 warnings, level <= semantic)
```

`--level` 分级执行验证流水线，`gamerule` 加上 L7 业务规则：

```console
$ cage check examples/game-config --level gamerule
cage check: OK (3 tables, 0 warnings, level <= gamerule)
```

### cage build —— 验证 + 全量构建

```console
$ cage build examples/game-config --profile client
cage build: OK (profile 'client', 42 artifacts, manifest examples/game-config/build/manifest.json)
```

42 个产物 = 数据 6 份（3 表 × json/csv）+ 代码 36 份（8 种语言 × 各 3 表 + 1 枚举单元，JS 形态额外配对 `.d.ts`）；manifest 在 `build/manifest.json`，不在 `client/` 计数内：

```text
build/client/
├── json/    Character.json  Item.json  Stage.json
├── csv/     Character.csv   Item.csv   Stage.csv
├── cs/      Character.cs    Item.cs    Stage.cs    CageEnums.cs
├── py/      Character.py    Item.py    Stage.py    cage_enums.py
├── lua/     Character.lua   Item.lua   Stage.lua   cage_enums.lua
├── ts/      Character.ts    Item.ts    Stage.ts    cage_enums.ts
├── js/      Character.js/.d.ts  Item.js/.d.ts  Stage.js/.d.ts  cage_enums.js/.d.ts
├── cpp/     Character.h     Item.h     Stage.h     cage_enums.h
├── go/      Character.go    Item.go    Stage.go    cage_enums.go
├── java/    Character.java  Item.java  Stage.java  CageEnums.java
└── manifest.json
```

### cage gen —— 只生成代码绑定

```console
$ cage gen examples/game-config --profile client
cage gen: OK (profile 'client', 36 artifacts, manifest examples/game-config/build/manifest.json)
```

跳过 json/csv 数据 target，代码类产物与 build 完全一致。

### cage check（坏数据）—— E1601 行级诊断

`bad/` 与好数据唯一的差别：`Character.csv` 首行 `attack` 从 120 改成 500。内置 power_curve 规则要求 `attack <= level * 100 + 50`，level=1 时上限 150：

```console
$ cage check examples/game-config/bad --level gamerule
ERROR E1601 — Game Rule Validation Failed
  Source: examples/game-config/bad/config/Character.csv | Row: 1
  Table: Character
  Row: 0
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)

cage check: FAILED (1 errors, 0 warnings)
$ echo $?
1
```

诊断带文件/行/表/行号定位，hint 给出计算过程；退出码 1 可直接接入 CI。

### cage inspect —— 查看表结构

```console
$ cage inspect examples/game-config Item
table: Item
description: 道具表（YAML 源；attack_bonus/desc 可选 —— 部分行缺省，演示可空字段）
primary key: id
rows: 5
fields:
  id: Int32 (required)
  name: String (required)
  type: Enum("ItemType") (required)
  rarity: Enum("Rarity") (required)
  price: Int32 (required)
  attack_bonus: Int32
  desc: String
```

## Schema 要点（本示例覆盖的用法）

| 用法 | 位置 |
| --- | --- |
| 多源异构：csv / yaml / json 三种源格式 | `config/` |
| 整型枚举（带整数值） | `CharacterClass`、`ItemType` |
| 字符串枚举（无值） | `Rarity` |
| 无值枚举（成员名即回退值） | `Difficulty` |
| 数值范围 min/max、字符串长度与 pattern | `Character.level/name` |
| 组合唯一约束 | `Character` 的 `name_unique` |
| 跨表引用（L5 校验） | `Stage.boss_item_id → Item.id` |
| 可选字段与 Schema 默认值 | `Item.attack_bonus/desc`、`Character.unlocked` |
| 可选对象组（部分行缺省） | `Stage.boss` |
| 数组字段 min_items/max_items | `Stage.tags` |
| Map<K,V>（含嵌套，空 map 合法） | `Stage.drop_table: map<string, Array<Int32>>` |
| 保留字字段自动转义（`class`） | `Character.class` → cs `@class`、py/java `class_`、go `Class`；lua/ts/js 不转义（消费端按键名取值） |

Map 字段的 schema 写法（`schemas/stage.yaml`）：

```yaml
drop_table:
  name: drop_table
  type:
    kind: Map
    value:
      key_type: string            # string | int（int 键在数据里是数字字符串）
      value_type:
        kind: Array               # 值类型任意，可嵌套
        value: { kind: Int32 }
  required: true
```

对应数据（`config/Stage.json`）：

```json
"drop_table": { "common": [1001, 2001], "rare": [1002], "epic": [1002, 2002] }
```

## 生成物样例

TypeScript（`build/client/ts/Stage.ts`，节选）：

```ts
export interface Stage {
  /** 关卡 Boss（可选对象组：部分行缺省） */
  boss?: Record<string, unknown>;
  /** 稀有度 → 掉落道具 id 列表（map<string, Array<Int32>>；空 map 合法）, required */
  drop_table: Map<string, number[]>;
  /** 关卡标签（Array<String>，1-4 个）, required */
  tags: string[];
}
```

Go（`build/client/go/Stage.go`，节选）：

```go
DropTable map[string][]int32 `json:"drop_table"`
```

C#（`build/client/cs/Stage.cs`，节选）：

```csharp
public Dictionary<string, IReadOnlyList<int>> drop_table { get; init; } = new();
```

Lua（`build/client/lua/Stage.lua`，`M.fields` 元数据节选——Map 与 Array 都是 table，用类型标签区分）：

```lua
-- 稀有度 → 掉落道具 id 列表…, required, 键值表（hash part），与 Array 的数组 table（sequence part）不同
{ name = "drop_table", key = "drop_table", type = "map<string, array<integer>>", required = true },
```

Python（`build/client/py/Item.py`，节选——可选字段的 `T | None` 与枚举导入）：

```python
from cage_enums import ItemType, Rarity

@dataclass(frozen=True)
class Item:
    id: int
    name: str
    rarity: Rarity
    type: ItemType
    attack_bonus: int | None = None
    desc: str | None = None
```

## 19 种字段类型 × 8 语言映射总表

代码 target 的字段类型映射（Map 为 v0.2 新增的第 19 种类型）：

| FieldType | C# | Python | Lua | TS / JS | C++ | Go | Java |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Null | `object?` | `Any` | `any` | `unknown` | `std::any` | `any` | `Object` |
| Bool | `bool` | `bool` | `boolean` | `boolean` | `bool` | `bool` | `boolean` / `Boolean` |
| Int8 | `sbyte` | `int` | `integer` | `number` | `std::int8_t` | `int8` | `byte` / `Byte` |
| Int16 | `short` | `int` | `integer` | `number` | `std::int16_t` | `int16` | `short` / `Short` |
| Int32 | `int` | `int` | `integer` | `number` | `std::int32_t` | `int32` | `int` / `Integer` |
| Int64 | `long` | `int` | `integer` | `number` | `std::int64_t` | `int64` | `long` / `Long` |
| UInt8 | `byte` | `int` | `integer` | `number` | `std::uint8_t` | `uint8` | `short` / `Short` |
| UInt16 | `ushort` | `int` | `integer` | `number` | `std::uint16_t` | `uint16` | `int` / `Integer` |
| UInt32 | `uint` | `int` | `integer` | `number` | `std::uint32_t` | `uint32` | `long` / `Long` |
| UInt64 | `ulong` | `int` | `integer` | `number` | `std::uint64_t` | `uint64` | `long` / `Long` |
| Float32 | `float` | `float` | `number` | `number` | `float` | `float32` | `float` / `Float` |
| Float64 | `double` | `float` | `number` | `number` | `double` | `float64` | `double` / `Double` |
| String | `string` | `str` | `string` | `string` | `std::string` | `string` | `String` |
| Bytes | `byte[]` | `bytes` | `bytes` | `Uint8Array` | `std::vector<std::uint8_t>` | `[]byte` | `byte[]` |
| Array\<T\> | `IReadOnlyList<T>` | `list[T]` | `array<T>` | `T[]` | `std::vector<T>` | `[]T` | `List<T>` |
| Object | `IReadOnlyDictionary<string, object?>` | `dict[str, Any]` | `table` | `Record<string, unknown>` | `std::map<std::string, std::any>` | `map[string]any` | `Map<String, Object>` |
| **Map\<K,V\>** | `Dictionary<K, V>` | `dict[K, V]` | `table`（类型标签 `map<K, V>`） | `Map<K, V>` | `std::unordered_map<K, V>` | `map[K]V` | `HashMap<K, V>` |
| Enum | 枚举名（未解析回退 `string`） | 枚举名 | 枚举名 | 枚举名 | 枚举名（字符串桶 `std::string_view`） | 枚举名 | 枚举名 |
| Any | `object?` | `Any` | `any` | `unknown` | `std::any` | `any` | `Object` |

Map 的键类型 K 映射：

| key_type | C# | Python | Lua | TS / JS | C++ | Go | Java |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `string` | `string` | `str` | `string` | `string` | `std::string` | `string` | `String` |
| `int` | `long` | `int` | `int` | `number` | `std::int64_t` | `int64` | `Long` |

Java 列的 `primitive / wrapper` 取值取决于字段可空性（必填用 primitive，非必填升包装）；C++ 的 `Map` 走 `std::unordered_map`（与 Object 的 `std::map` 区分），需要时由生成器自动注入 `#include <unordered_map>`；Lua 中 Map 与 Array 都是 `table`，生成物的类型标签与字段注释写明 hash part / sequence part 之别。

## CI 防烂

`.github/workflows/ci.yml` 有独立的 `example` job：检出后执行 `bash examples/run.sh`。示例工程的数据、schema、run.sh 任何一处烂掉，CI 直接红。
