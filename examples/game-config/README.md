# rpg-demo —— Cage 完整示例工程

贴近真实游戏的五张配置表，端到端演示 cage 的全部能力（含 CI 使用）。
**一键跑通（在仓库根目录）**：

```bash
bash examples/run.sh
```

14 步覆盖：check → L7 gamerule → 分环境 `--env` → 双 profile 构建 →
增量 → 快照打包与回验 → 代码生成 → diff → 声明式迁移 → 注册表发布 →
远程分发全链路（直推 / presigned / 消费方 resolve）→ 坏数据诊断 → inspect。

## 目录

```
game-config/
├── cage.toml          # 工程配置：client（13 target）/ server（3 target）双 profile
├── schemas/           # 五个 schema 文件（按名序合并加载，metadata 取自 character.yaml）
│   ├── character.yaml # 角色成长：枚举/唯一约束/正则/范围/默认值/保留字字段 class
│   │                  #   + hp：L6 语义规则 hp <= attack（E1501）
│   │                  #   + nickname：env_overrides（dev 放宽、prod 必填）
│   │                  #   + internal_note：targets ["server"]（字段级可见性）
│   ├── item.yaml      # 道具：整型枚举 + 字符串枚举 + 可选字段
│   ├── quest.yaml     # 任务：Excel 源、跨表引用、order_by 行排序
│   ├── shop.yaml      # 商店：MsgPack 源、env_overrides（prod 收紧折扣上限）
│   └── stage.yaml     # 关卡：数组/对象/跨表引用（含 cardinality: many 数组引用）/Map<string, Array<Int32>>
├── config/            # 五种源格式（同一 profile 一起加载）
│   ├── Character.csv  # CSV：文件名即表名
│   ├── Item.yaml      # YAML：根键 = 表名
│   ├── Quest.xlsx     # Excel：工作表名即表名（源适配自动发现）
│   ├── Shop.msgpack   # MsgPack：裸行对象数组，文件名即表名
│   └── Stage.json     # JSON：根键 = 表名
├── migrations/        # 声明式数据迁移规则（cage migrate dry-run）
│   └── 0001_rename_desc.yaml   # Item.desc → description（from 0.1.0 → 0.2.0）
├── ci/                # CI 参考片段（Jenkinsfile / github-actions.yml / gitlab-ci.yml）
└── bad/               # 坏数据：完整复制品，attack=500 + hp 超过 attack（E1501）+ 全行无 nickname
    ├── cage.toml      # schema_path = "../schemas"（共用同一份 schema）
    └── config/
```

## 演示点

| 演示 | 位置 |
| --- | --- |
| 多源异构（csv/yaml/json/xlsx/msgpack 五种源格式） | `config/` |
| 字段类型覆盖（Int/Bool/String/枚举/数组/对象/Map/引用/浮点） | `schemas/stage.yaml`、`quest.yaml`、`shop.yaml` |
| 整型枚举（带整数值） / 字符串枚举（无值） / 无值枚举 | `CharacterClass` / `Rarity` / `Difficulty` |
| 可选字段与默认值 | `Item.attack_bonus` 缺省、`Character.unlocked` 默认 true |
| 跨表引用（L5 校验） | `Stage.boss_item_id / Quest.reward_item_id / Shop.item_id → Item.id` |
| 数组引用 `cardinality: many`（逐元素 L5 校验） | `Stage.loot_item_ids → Item.id` |
| 保留字字段自动转义（`class`） | `Character.class` |
| Map<K,V> 字段（含嵌套，空 map 合法） | `Stage.drop_table: map<string, Array<Int32>>` |
| 行排序约束（E1304） | `Quest.order_by: [id]` |
| 分环境约束覆盖（`--env`，design §48） | `Character.nickname`（dev 放宽 max_length / prod 必填）、`Shop.discount`（prod 上限 0.5） |
| 字段级可见性（targets + E9006 安全） | `Character.internal_note` 仅 server 视图 |
| 声明式迁移（dry-run 不写盘，Excel 源只报告） | `migrations/0001_rename_desc.yaml` |
| 注册表发布（版本化自校验资产） | `run.sh` 第 11 步 |
| 远程分发（push 直推 / presigned 直推 / 消费方远端 resolve 构建，产物逐字节一致） | `run.sh` 第 12 步 |
| MessagePack 二进制（rmp 最小形，确定性字节） | `build/client/msgpack/` |
| Protobuf .proto3 定义（protoc 可编译） | `build/client/proto/` |
| 标准 JSON Schema 文档（draft-07，每表自包含） | `build/client/jsonschema/` |
| L6 语义规则（E1501，hp <= attack）：好数据通过 / 坏数据违例 | `run.sh` 第 1、13 步 |
| L7 Game Rule：好数据通过 / 坏数据 E1601 | `run.sh` 第 2、13 步 |
| CI 集成（Jenkins / GitHub Actions / GitLab CI） | `ci/` |
| Web 编辑器拒写目录 schema（409） | `examples/web-smoke.sh` 第 6 步 |

## 手动执行

```bash
cage check examples/game-config                    # L0-L6 校验（5 张表）
cage check examples/game-config --level gamerule   # 加上 L7 业务规则
cage check examples/game-config --env prod         # 分环境：nickname 必填、折扣 ≤ 0.5
cage build examples/game-config --profile client   # 13 个 target 全量构建（5 张表）
cage build examples/game-config --profile client --incremental  # up to date
cage build examples/game-config --profile server   # 服务端视图（internal_note 可见）
cage snapshot examples/game-config --profile client        # 自校验快照打包
cage snapshot examples/game-config/build/snapshot/client-*/ --verify  # 回验
cage gen   examples/game-config --profile client   # 只生成 9 种代码绑定（含 proto 定义）
cage diff  examples/game-config/build examples/game-config/build    # 确定性比对
cage migrate examples/game-config --to 0.2.0       # 迁移 dry-run（不写盘）
cage registry publish examples/game-config --registry /tmp/reg   # 发布到本地仓库
cage registry publish examples/game-config         # 发布进工程自带 [registry].path（reg/，push 源）
cage registry push examples/game-config --registry http://127.0.0.1:8080   # 直推远端（静态托管 / CI job 皆可）
cage check examples/game-config/bad                   # E1501（hp > attack），退出码 1
cage check examples/game-config/bad --level gamerule  # E1601，退出码 1
cage check examples/game-config/bad --env prod        # E1001（prod 必填 nickname），退出码 1
cage inspect examples/game-config                   # 表结构清单
```

产物在 `build/client/{json,csv,msgpack,proto,cs,py,lua,ts,js,cpp,go,java,jsonschema}/`
与 `build/server/{json,csv,cs}/`（已 gitignore，按需重新生成）。
