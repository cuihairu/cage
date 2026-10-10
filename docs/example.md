# 完整示例

仓库内常驻一个端到端示例工程 [`examples/game-config/`](https://github.com/cuihairu/cage/tree/main/examples/game-config)——「角色成长 + 道具 + 任务 + 商店 + 关卡」五张表、五种源格式（CSV / YAML / JSON / Excel / MsgPack）、一份覆盖常用字段类型与约束用法的 schema、双 profile（client 13 个 target / server 3 个 target）、分环境约束覆盖（`--env`）、快照 / diff / 迁移 / 注册表，外加一套触发 L7 Game Rule 与分环境必填的坏数据。

本页所有命令与输出都是**真实执行结果**（在仓库根目录运行），不是伪代码。

## 一键跑通

```bash
bash examples/run.sh
```

脚本从 `cage check` 到 `cage build` 到 `cage gen` 到 `cage migrate` 到 `cage registry`（含坏数据 E1601 / E1001 断言与 `cage inspect`）13 步一条命令跑完，每步打印真实命令；任一步失败立即退出，CI 里同款执行（示例烂了 CI 就红）。

<details>
<summary>examples/run.sh 完整输出（点击展开）</summary>

```text
==> 构建 cage 可执行文件

============================================================
==> [1/13] cage check examples/game-config   —— L0-L6 全量校验（好数据应通过）
============================================================
$ cage check examples/game-config
cage check: OK (5 tables, 0 warnings, level <= semantic)

============================================================
==> [2/13] cage check --level gamerule examples/game-config   —— L7 业务规则（好数据应通过）
============================================================
$ cage check examples/game-config --level gamerule
cage check: OK (5 tables, 0 warnings, level <= gamerule)

============================================================
==> [3/13] 分环境验证 —— --env dev/prod 放行、未知环境 exit 2
============================================================
$ cage check examples/game-config --env dev
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env prod
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env staging   # 期望退出码 2
error: unknown environment 'staging' (declared: dev, prod)

============================================================
==> [4/13] cage build --profile client   —— 验证 + 生成全部 13 个 target（5 张表）
============================================================
$ cage build examples/game-config --profile client
cage build: OK (profile 'client', 80 artifacts, manifest examples/game-config/build/manifest.json)

============================================================
==> [5/13] cage build --profile client --incremental   —— 指纹一致 → 跳过重建
============================================================
$ cage build examples/game-config --profile client --incremental
cage build: up to date (profile 'client', 80 artifacts, manifest examples/game-config/build/manifest.json)

============================================================
==> [6/13] cage snapshot --profile client + --verify   —— 自校验快照打包与回验
============================================================
$ cage snapshot examples/game-config --profile client
cage snapshot: OK (profile 'client', 83 files, 80 artifacts, verified, examples/game-config/build/snapshot/client-3e0930ac97c2)
$ cage snapshot <dir> --verify
cage snapshot: verified (examples/game-config/build/snapshot/client-3e0930ac97c2/ — 82 files checked)

============================================================
==> [7/13] cage gen --profile client   —— 只生成代码绑定与 JSON Schema（含 proto 定义）
============================================================
$ cage gen examples/game-config --profile client
cage gen: OK (profile 'client', 65 artifacts, manifest examples/game-config/build/manifest.json)

============================================================
==> [8/13] cage diff build build   —— 同一构建前后比对（确定性：全不变）
============================================================
$ cage diff examples/game-config/build examples/game-config/build
cage diff: 0 added, 0 removed, 0 changed, 65 unchanged

============================================================
==> [9/13] cage build --profile server   —— 服务端视图（server-only 字段 internal_note 可见，client 视图剔除）
============================================================
$ cage build examples/game-config --profile server
cage build: OK (profile 'server', 16 artifacts, manifest examples/game-config/build/manifest.json)

============================================================
==> [10/13] cage migrate --to 0.2.0   —— 声明式迁移 dry-run（不写盘）
============================================================
$ cage migrate examples/game-config --to 0.2.0
cage migrate: segment 0001_rename_desc.yaml (0.1.0 → 0.2.0)
  rename_field(Item.desc → description): 3 row(s)
  unchanged examples/game-config/config/Character.csv
  would write examples/game-config/config/Item.yaml (1 table(s))
  skip examples/game-config/config/Quest.xlsx (Excel source: report only — 1 table(s), 3 row(s))
  skip examples/game-config/config/Shop.msgpack (not a text source)
  would write examples/game-config/config/Stage.json (1 table(s))
cage migrate: OK (1 segment(s), 3 row(s) migrated, schema 0.1.0) — dry run, nothing written

============================================================
==> [11/13] cage registry publish + list   —— 版本化自校验资产发布到本地仓库
============================================================
$ cage registry publish examples/game-config --registry /tmp/tmp.IpsC5EIsbU
cage registry: published rpg-demo/0.1.0 (profile 'client', 83 files, build_id 3e0930ac97c2, content_hash c21d96d7e142)
$ cage registry list --registry /tmp/tmp.IpsC5EIsbU
cage registry: 1 package(s) in /tmp/tmp.IpsC5EIsbU
rpg-demo
  0.1.0        build 3e0930ac97c2  content c21d96d7e142  83 files

============================================================
==> [12/13] 坏数据诊断   —— E1601 越过 power curve + --env prod E1001 缺昵称
============================================================
$ cage check examples/game-config/bad --level gamerule   # 期望 E1601 + 退出码 1
ERROR E1601 — Game Rule Validation Failed
  Source: examples/game-config/bad/config/Character.csv | Row: 1
  Table: Character
  Row: 0
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)


cage check: FAILED (1 errors, 0 warnings)
$ cage check examples/game-config/bad --env prod   # prod 收紧 nickname 必填 → E1001
ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 1 | Field: nickname
  Table: Character
  Row: 0
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 2 | Field: nickname
  Table: Character
  Row: 1
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 3 | Field: nickname
  Table: Character
  Row: 2
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 4 | Field: nickname
  Table: Character
  Row: 3
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row


cage check: FAILED (4 errors, 0 warnings)

============================================================
==> [13/13] cage inspect examples/game-config   —— 查看表结构
============================================================
$ cage inspect examples/game-config
project: rpg-demo
version: 0.1.0
tables:
  Character (4 rows)
  Item (5 rows)
  Quest (3 rows)
  Shop (3 rows)
  Stage (3 rows)
schemas:
  Character (10 fields, primary key: id)
  Item (7 fields, primary key: id)
  Quest (4 fields, primary key: id)
  Shop (3 fields, primary key: id)
  Stage (9 fields, primary key: id)

全部通过。
产物目录：examples/game-config/build/{client,server}/{json,csv,msgpack,proto,cs,py,lua,ts,js,cpp,go,java,jsonschema}
快照目录：examples/game-config/build/snapshot/
```

</details>

## 工程结构

```text
examples/game-config/
├── cage.toml          # 工程配置：client（13 target）/ server（3 target）双 profile
├── schemas/           # 五个 schema 文件（按名序合并加载，metadata 取自 character.yaml）
│   ├── character.yaml # 角色成长：枚举/唯一约束/正则/范围/默认值/保留字字段 class
│   │                  #   + nickname：env_overrides（dev 放宽、prod 必填）
│   │                  #   + internal_note：targets ["server"]（字段级可见性）
│   ├── item.yaml      # 道具：整型枚举 + 字符串枚举 + 可选字段
│   ├── quest.yaml     # 任务：Excel 源、跨表引用、order_by 行排序
│   ├── shop.yaml      # 商店：MsgPack 源、env_overrides（prod 收紧折扣上限）
│   └── stage.yaml     # 关卡：数组/对象/跨表引用/Map<string, Array<Int32>>
├── config/            # 五种源格式（同一 profile 一起加载）
│   ├── Character.csv  # CSV：文件名即表名
│   ├── Item.yaml      # YAML：根键 = 表名
│   ├── Quest.xlsx     # Excel：工作表名即表名（源适配自动发现）
│   ├── Shop.msgpack   # MsgPack：裸行对象数组，文件名即表名
│   └── Stage.json     # JSON：根键 = 表名
├── migrations/        # 声明式数据迁移规则（cage migrate dry-run）
│   └── 0001_rename_desc.yaml   # Item.desc → description（from 0.1.0 → 0.2.0）
├── ci/                # CI 参考片段（Jenkinsfile / github-actions.yml / gitlab-ci.yml）
└── bad/               # 坏数据：完整复制品，attack=500 + 全行无 nickname
    ├── cage.toml      # schema_path = "../schemas"（共用同一份 schema）
    └── config/
```

## 逐条命令

### cage check —— 验证不生成

```console
$ cage check examples/game-config
cage check: OK (5 tables, 0 warnings, level <= semantic)
```

`--level` 分级执行验证流水线，`gamerule` 加上 L7 业务规则：

```console
$ cage check examples/game-config --level gamerule
cage check: OK (5 tables, 0 warnings, level <= gamerule)
```

### cage check --env —— 分环境约束覆盖

schema 的 `env_overrides` 按环境换约束（design §48）：dev 把 `Character.nickname` 的 `max_length` 放宽到 32，prod 把它改成必填、`Shop.discount` 上限收紧到 0.5。`--env` 在验证前把覆盖解析进 schema，校验/生成/哈希全部走解析后的形态：

```console
$ cage check examples/game-config --env dev
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env prod
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env staging
error: unknown environment 'staging' (declared: dev, prod)
$ echo $?
2
```

未知环境是用法错误（exit 2），不新增 E 码；环境名进 manifest 的 `environment` 字段并参与增量指纹——换环境即换构建。

### cage build —— 验证 + 全量构建

```console
$ cage build examples/game-config --profile client
cage build: OK (profile 'client', 80 artifacts, manifest examples/game-config/build/manifest.json)
```

80 个产物 = 数据 15 份（5 表 × json/csv/msgpack）+ proto 6 份（5 表 + 共享 `cage_enums.proto`）+ 代码 54 份（8 种语言 × 各 5 表 + 1 枚举单元，JS 形态额外配对 `.d.ts`）+ JSON Schema 5 份（5 表 × `{table}.schema.json`）；manifest 在 `build/manifest.json`，不在 `client/` 计数内：

```text
build/client/
├── json/    Character.json  Item.json  Quest.json  Shop.json  Stage.json
├── csv/     Character.csv   Item.csv   Quest.csv   Shop.csv   Stage.csv
├── msgpack/ Character.msgpack  Item.msgpack  Quest.msgpack  Shop.msgpack  Stage.msgpack
├── proto/   Character.proto Item.proto Quest.proto Shop.proto Stage.proto cage_enums.proto
├── cs/      Character.cs    Item.cs    Quest.cs    Shop.cs    Stage.cs    CageEnums.cs
├── py/      Character.py    Item.py    Quest.py    Shop.py    Stage.py    cage_enums.py
├── lua/     Character.lua   Item.lua   Quest.lua   Shop.lua   Stage.lua   cage_enums.lua
├── ts/      Character.ts    Item.ts    Quest.ts    Shop.ts    Stage.ts    cage_enums.ts
├── js/      Character.js/.d.ts  Item.js/.d.ts  Quest.js/.d.ts  Shop.js/.d.ts  Stage.js/.d.ts  cage_enums.js/.d.ts
├── cpp/     Character.h     Item.h     Quest.h     Shop.h     Stage.h     cage_enums.h
├── go/      Character.go    Item.go    Quest.go    Shop.go    Stage.go    cage_enums.go
├── java/    Character.java  Item.java  Quest.java  Shop.java  Stage.java  CageEnums.java
├── jsonschema/ Character.schema.json  Item.schema.json  Quest.schema.json  Shop.schema.json  Stage.schema.json
└── manifest.json
```

确定性抽查：全量重建后 msgpack / proto 逐字节一致（sha256 相同）。

### cage build --incremental —— 指纹一致跳过重建

```console
$ cage build examples/game-config --profile client --incremental
cage build: up to date (profile 'client', 80 artifacts, manifest examples/game-config/build/manifest.json)
```

L1（schema + 源指纹）与 L2（产物指纹）双层守卫；`environment` 参与指纹，换 `--env` 会触发重建。

### cage snapshot —— 自校验快照打包与回验

```console
$ cage snapshot examples/game-config --profile client
cage snapshot: OK (profile 'client', 83 files, 80 artifacts, verified, examples/game-config/build/snapshot/client-3e0930ac97c2)
$ cage snapshot examples/game-config/build/snapshot/client-3e0930ac97c2/ --verify
cage snapshot: verified (examples/game-config/build/snapshot/client-3e0930ac97c2/ — 82 files checked)
```

快照 = profile 投影的规范 schema（`schema.json`）+ 全部产物 + 逐文件哈希账本（`HASHES.json`），自校验、可回滚、可审计。

### cage gen —— 只生成代码绑定

```console
$ cage gen examples/game-config --profile client
cage gen: OK (profile 'client', 65 artifacts, manifest examples/game-config/build/manifest.json)
```

跳过 json/csv/msgpack 数据 target，代码类产物（含 proto 定义）与 JSON Schema 文档与 build 完全一致。

### cage diff —— 确定性比对

```console
$ cage diff examples/game-config/build examples/game-config/build
cage diff: 0 added, 0 removed, 0 changed, 65 unchanged
```

同一构建前后比对全不变——确定性是资产可信的前提。

### cage build --profile server —— 服务端视图

```console
$ cage build examples/game-config --profile server
cage build: OK (profile 'server', 16 artifacts, manifest examples/game-config/build/manifest.json)
```

`Character.internal_note` 声明 `targets: ["server"]`：server 视图 4 行全带该字段，client 视图 0 处出现（字段级可见性，E9006 顺序安全）。

### cage migrate —— 声明式迁移 dry-run

```console
$ cage migrate examples/game-config --to 0.2.0
cage migrate: segment 0001_rename_desc.yaml (0.1.0 → 0.2.0)
  rename_field(Item.desc → description): 3 row(s)
  unchanged examples/game-config/config/Character.csv
  would write examples/game-config/config/Item.yaml (1 table(s))
  skip examples/game-config/config/Quest.xlsx (Excel source: report only — 1 table(s), 3 row(s))
  skip examples/game-config/config/Shop.msgpack (not a text source)
  would write examples/game-config/config/Stage.json (1 table(s))
cage migrate: OK (1 segment(s), 3 row(s) migrated, schema 0.1.0) — dry run, nothing written
```

迁移规则是版本链上的声明式 segment；dry-run 只报告不写盘，Excel 源只报告（二进制源不参与文本迁移）。

### cage registry —— 版本化自校验资产发布

```console
$ cage registry publish examples/game-config --registry /tmp/reg
cage registry: published rpg-demo/0.1.0 (profile 'client', 83 files, build_id 3e0930ac97c2, content_hash c21d96d7e142)
$ cage registry list --registry /tmp/reg
cage registry: 1 package(s) in /tmp/reg
rpg-demo
  0.1.0        build 3e0930ac97c2  content c21d96d7e142  83 files
```

快照打成 `rpg-demo/0.1.0` 条目入册：自校验、可寻址、可滚动回收。Schema 作为资产的管理与消费端加载见[资产页](/assets)。

### cage check（坏数据）—— E1601 行级诊断 + 分环境 E1001

`bad/` 与好数据两处差别：`Character.csv` 首行 `attack` 从 120 改成 500，且全行没有 `nickname` 列。内置 power_curve 规则要求 `attack <= level * 100 + 50`，level=1 时上限 150：

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

同一份坏数据在 prod 环境下换一种烂法——`nickname` 必填后 4 行全缺：

```console
$ cage check examples/game-config/bad --env prod
ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 1 | Field: nickname
  Table: Character
  Row: 0
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 2 | Field: nickname
  Table: Character
  Row: 1
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 3 | Field: nickname
  Table: Character
  Row: 2
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 4 | Field: nickname
  Table: Character
  Row: 3
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

cage check: FAILED (4 errors, 0 warnings)
```

诊断带文件/行/表/行号定位，hint 给出计算过程；退出码 1 可直接接入 CI。

### cage inspect —— 查看表结构

```console
$ cage inspect examples/game-config
project: rpg-demo
version: 0.1.0
tables:
  Character (4 rows)
  Item (5 rows)
  Quest (3 rows)
  Shop (3 rows)
  Stage (3 rows)
schemas:
  Character (10 fields, primary key: id)
  Item (7 fields, primary key: id)
  Quest (4 fields, primary key: id)
  Shop (3 fields, primary key: id)
  Stage (9 fields, primary key: id)
```

## Schema 要点（本示例覆盖的用法）

| 用法 | 位置 |
| --- | --- |
| 多源异构：csv / yaml / json / xlsx / msgpack 五种源格式 | `config/` |
| 整型枚举（带整数值） | `CharacterClass`、`ItemType` |
| 字符串枚举（无值） | `Rarity` |
| 无值枚举（成员名即回退值） | `Difficulty` |
| 数值范围 min/max、字符串长度与 pattern | `Character.level/name` |
| 组合唯一约束 | `Character` 的 `name_unique` |
| 跨表引用（L5 校验） | `Stage.boss_item_id`、`Quest.reward_item_id`、`Shop.item_id → Item.id` |
| 可选字段与 Schema 默认值 | `Item.attack_bonus/desc`、`Character.unlocked` |
| 可选对象组（部分行缺省） | `Stage.boss` |
| 数组字段 min_items/max_items | `Stage.tags` |
| Map<K,V>（含嵌套，空 map 合法） | `Stage.drop_table: map<string, Array<Int32>>` |
| 行排序约束（E1304） | `Quest.order_by: [id]` |
| 分环境约束覆盖（`--env`） | `Character.nickname`（dev 放宽 max_length / prod 必填）、`Shop.discount`（prod 上限 0.5） |
| 字段级可见性（targets + E9006 安全） | `Character.internal_note` 仅 server 视图 |
| 保留字字段自动转义（`class`） | `Character.class` → cs `@class`、py/java `class_`、go `Class`；lua/ts/js 不转义（消费端按键名取值） |
| 标准 JSON Schema 文档（draft-07，枚举内联、闭形对象） | `build/client/jsonschema/` |

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

分环境覆盖的 schema 写法（`schemas/character.yaml` / `schemas/shop.yaml`）：

```yaml
env_overrides:
  dev:
    nickname: { max_length: 32 }   # 放宽
  prod:
    nickname: { required: true }    # 收紧成必填
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

TypeScript（`build/client/ts/Quest.ts`，节选——Excel 源 + 跨表引用 + 行排序）：

```ts
export interface Quest {
  /** required, min: 1 */
  id: number;
  /** required, min: 1, max: 100 */
  min_level: number;
  /** required, min_length: 1, max_length: 64 */
  name: string;
  /** 完成奖励道具 —— 引用 Item.id（L5 跨表引用校验）, required, → Item.id */
  reward_item_id: number;
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

数据产物（`build/client/json/Shop.json`，节选——MsgPack 源表，字段按 schema 名序输出）：

```json
[
  { "discount": 0.25, "id": 1, "item_id": 1001 },
  { "discount": 0.4, "id": 2, "item_id": 2001 },
  { "discount": 0.15, "id": 3, "item_id": 3001 }
]
```

标准 JSON Schema（`build/client/jsonschema/Item.schema.json`，节选——枚举内联成员名、约束映射 `minimum`/`maxLength`，非 cage 工具链直接消费）：

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Item",
  "description": "道具表（YAML 源；attack_bonus/desc 可选 —— 部分行缺省，演示可空字段）",
  "type": "object",
  "properties": {
    "id": {
      "type": "integer",
      "minimum": 1
    },
    "type": {
      "type": "string",
      "enum": [
        "Weapon",
        "Armor",
        "Consumable"
      ]
    },
    "rarity": {
      "type": "string",
      "enum": [
        "common",
        "rare",
        "epic",
        "legendary"
      ]
    },
    "price": {
      "type": "integer",
      "minimum": 0,
      "maximum": 999999
    }
  },
  "required": [
    "id",
    "name",
    "type",
    "rarity",
    "price"
  ],
  "additionalProperties": false
}
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

`examples/game-config/ci/` 是三份可直接落地的 CI 参考片段，都是「装 Rust → `bash examples/run.sh` → 归档产物」：

| 文件 | 平台 | 要点 |
| --- | --- | --- |
| `github-actions.yml` | GitHub Actions | `dtolnay/rust-toolchain` 装链，`upload-artifact` 归档 `build/` |
| `Jenkinsfile` | Jenkins（声明式） | `rustup` 引导工具链，`archiveArtifacts` 归档 |
| `gitlab-ci.yml` | GitLab CI | `rust:1-bookworm` 镜像，`artifacts` 上报 |

三份片段与 `run.sh` 的 13 步一一对应：check（含 `--env` 与坏数据断言）→ 双 profile 构建 → 增量 → 快照回验 → gen → diff → migrate → registry，任何一步红都是真烂。

## 资产：Schema 的管理与加载

示例的终点不是产物目录，而是**可分发、可回验的资产**：`cage snapshot` 冻结一次构建（schema + 产物 + 哈希账本），`cage registry publish` 把它打成 `rpg-demo/0.1.0` 版本化条目。Schema 本身也是资产——源码评审管 schema，快照哈希管构建，注册表目录管分发。

消费端（另一个仓库的构建 / 工具链 / CI）不拉源文件，拉注册表条目：`schema_path = "game:0.1.0/schema.json"` + `[source_roots] game = "registry:0.1.0"` + `[dependencies]` 版本 pin，构建期先过信任门再跑完整流水线。标准 JSON Schema 形态由 `format = "jsonschema"` target 产出（`build/client/jsonschema/`），给非 cage 工具链消费。

详见[资产页](/assets)。
