# rpg-demo —— Cage 完整示例工程

贴近真实游戏的三张配置表，端到端演示 cage 的全部能力。
**一键跑通（在仓库根目录）**：

```bash
bash examples/run.sh
```

## 目录

```
game-config/
├── cage.toml          # 工程配置：一个 profile 构建全部 12 个 target
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

## 演示点

| 演示 | 位置 |
| --- | --- |
| 多源异构（csv/yaml/json 三种源格式） | `config/` |
| 字段类型覆盖（Int/Bool/String/枚举/数组/对象/Map/引用） | `schemas/stage.yaml` |
| 整型枚举（带整数值） / 字符串枚举（无值） / 无值枚举 | `CharacterClass` / `Rarity` / `Difficulty` |
| 可选字段与默认值 | `Item.attack_bonus` 缺省、`Character.unlocked` 默认 true |
| 跨表引用（L5 校验） | `Stage.boss_item_id → Item.id` |
| 保留字字段自动转义（`class`） | `Character.class` |
| Map<K,V> 字段（含嵌套，空 map 合法） | `Stage.drop_table: map<string, Array<Int32>>` |
| MessagePack 二进制（rmp 最小形，确定性字节） | `build/client/msgpack/` |
| Protobuf .proto3 定义（protoc 可编译） | `build/client/proto/` |
| L7 Game Rule：好数据通过 / 坏数据 E1601 | `run.sh` 第 2、5 步 |
| Web 编辑器拒写目录 schema（409） | `examples/web-smoke.sh` 第 6 步

## 手动执行

```bash
cage check examples/game-config                    # L0-L6 校验
cage check examples/game-config --level gamerule   # 加上 L7 业务规则
cage build examples/game-config --profile client   # 12 个 target 全量构建
cage gen   examples/game-config --profile client   # 只生成 9 种代码绑定（含 proto 定义）
cage check examples/game-config/bad --level gamerule  # E1601，退出码 1
cage inspect examples/game-config                   # 表结构清单
```

产物在 `build/client/{json,csv,msgpack,proto,cs,py,lua,ts,js,cpp,go,java}/`（已 gitignore，按需重新生成）。
