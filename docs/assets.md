[English](assets.en.md) | 中文

# 资产：Schema 的管理与加载

配置编译的终点是**资产**——可以被运行时、工具链和其他系统直接消费的
确定性产物。Schema 不只是输入，它本身就是一种资产。本页讲清楚 cage
里「资产」包含什么、怎么管理（版本化、可信）、怎么加载（消费端接入）。

## 资产的层次

| 资产 | 是什么 | 谁产出 | 谁消费 |
| --- | --- | --- | --- |
| 配置数据（json/csv/msgpack） | 每张表的行集，确定性字节 | 构建 | 运行时加载 |
| 代码绑定（cs/py/lua/ts/js/cpp/go/java/proto） | schema 驱动的强类型定义 | `cage gen` | 服务端 / 客户端工程 |
| Schema（`schema.yaml` + 各源文件的表定义） | 结构、类型、约束的唯一事实源 | 作者 | 校验、生成、编辑器 |
| Snapshot（`manifest.json` + `schema.json` + 产物 + `HASHES.json`） | 一次构建的**自校验冻结视图**：profile 投影的 schema、全部产物、逐文件哈希账本 | `cage snapshot` | 分发、回滚、审计 |
| Registry 条目（`pkg@version`） | 版本化打包的快照——自校验、可寻址、可滚动回收 | `cage registry publish` | 消费端工程（跨仓/跨团队） |

## Schema 作为资产：管理

Schema 的演进走**显式版本链**，而不是「文件改了就是改了」：

1. **源头是文本**：schema 文件随代码仓走，评审、diff、回滚都按普通
   代码对待。`metadata.version` 是它对外自称的版本号。
2. **冻结进快照**：`cage snapshot <project>` 把**当前 profile 投影
   后的规范 schema** 落进 `snapshot/<profile>[-<env>]-<build_id[..12]>/schema.json`
   （构建所依据的形态，不是源文件本身；`--env` 时即该环境的 resolved
   形态，目录名带 env 段）。快照自带 `HASHES.json` 逐文件
   账本与 `--verify` 回验——资产完整性不靠承诺，靠哈希。
3. **发布进注册表**：`cage registry publish` 把快照打成
   `rpg-demo/0.1.0` 式条目（自校验 bundle）入册。注册表（R3）是资产
   的权威目录：`list` 查得到、`verify` 全册审计、`gc` 滚动窗口、
   `remove` 显式移除与重发。

一句话：**源码评审管 schema，快照哈希管构建，注册表目录管分发**——
三份账各司其职，同一字节链。

## 消费端：怎么加载

消费端（另一个仓库的构建、工具链、CI）不拉源文件，拉**注册表条目**：

```toml
# 消费端 cage.toml
schema_path = "game:0.1.0/schema.json"   # 版本化的规范 schema

[source_roots]
game = "registry:0.1.0"                 # 数据条目（自校验快照）

[dependencies]
game = "0.1.0"                            # 版本 pin：构建期解析并校验
```

`registry:<version>` 在构建时解析为注册表内的条目目录（本地多版本仓
库或 http(s) 远程根），先 `verify_snapshot` 信任门（签名 / 哈希），再
以同一份 schema + 数据跑完整 L0–L7 流水线——**未经校验不入册，未经校
验不载入**，对远端字节同样成立。

本地 HTTP API 源（`http(s) URL`）与 MySQL/PostgreSQL / Google Sheets
是另一条「在线取数」路径（design §45），适合把尚未入库的系统数据接
进编译链；两者的校验口径完全一致。

## 资产分类与备份策略（design §50）

按内容划分，cage 工程的资产分三类「输入资产」加一类「产物资产」：

| 类 | 是什么 | 落点 | `cage assets` 类别 |
| --- | --- | --- | --- |
| 规则资产 | 数据应该长什么样：校验规则、枚举、跨表约束、迁移链 | `schemas/` + `migrations/` | `rule` |
| 源数据资产 | 数据本身：策划填的表 | `[source_roots]` 指向的文件集 | `source` |
| 构建规则 | 怎么构建：profiles / targets / registry | `cage.toml` | `build` |
| 产物与追溯 | 当时编译出了什么、能不能复现 | `build/`、快照、registry 条目、bundle | （产物不入清单——可由前三类确定性重建） |

两个裁定（design §50）：**rules/ 不设独立目录**——规则事实源就是
schema 文件集 + 迁移链，拆目录等于维护两份账；**产物不入版本控制**——
同输入必出同字节，入库只会制造伪差异（`.gitignore` 掉 `build/`）。

### `cage assets`：资产清单

```bash
cage assets <project>                    # 文本清单：路径 / 类别 / blake3 / 大小
cage assets <project> --json             # 同形 JSON
cage assets <project> --write assets.json  # 基线随仓入库（提交进 Git）
cage assets <project> --check assets.json  # 复核：缺失 / 篡改 / 多余逐条点名，退出码 1
```

三类输入资产逐文件 blake3、按路径确定性排序；远程 spec
（`registry:` / `gsheet:` / `mysql:` / `pg:` / `http(s)://`）不盘点
——它们是版本 pin 或带自有契约的远端，由 manifest 的 `dependencies` +
`source_hash` 覆盖。`--check` 是发布前 / 恢复后的完整性门。

### 备份四层防线

| 层 | 手段 | 防什么 |
| --- | --- | --- |
| 1. Git 版本管理 | 源资产 + `assets.json` 基线逐提交入库 | 改坏可回退、改了什么可追溯 |
| 2. 远程仓库异地 | push 到远端托管 | 本机磁盘故障 |
| 3. 发布快照复现 | `cage snapshot` + `registry publish` + `registry export`（签名 bundle + `.commit` sidecar） | 需要精确找回「当时用了哪版规则、编译出了什么」 |
| 4. 独立备份灾难恢复 | bundle 三件套送冷备 + 定期恢复演练 | 仓库托管整体不可用——**GitHub 不是灾难恢复** |

**备份 ≠ 版本管理 ≠ 可复现构建**：备份救丢失，版本救回退，可复现救
「为何与历史不一致」。commit 不进 manifest（确定性契约：同内容可在
不同 commit 重现，commit 是历史不是内容），由发布工作流记
`<bundle>.commit` sidecar。

完整可跑的示范工程（盘点 → 基线 → 篡改复核 → 双 profile 构建 → 快照
→ 发布 → 签名导出 → 干净目录恢复演练）见
[`examples/asset-workspace/`](https://github.com/cuihairu/cage/tree/main/examples/asset-workspace)
，一键跑通：`bash examples/asset-workspace.sh`。

## JSON Schema（标准）形态

cage 的 schema 是自有 DSL（YAML，见 [Schema 文档](/schema)）——它是
规则事实源，不是 JSON Schema 标准文档。给非 cage 工具链（编辑器、ajv、
前端表单等）消费的标准 JSON Schema 形态由 **`format = "jsonschema"`
target** 产出：每表一个自包含 `{table}.schema.json`（draft-07 默认、
2020-12 可选，枚举内联、闭形对象），schema 驱动、确定性字节，随源
schema 演进——详见 [Target · JSON Schema](/target#json-schema-target-已实装)。
快照里的 `schema.json`（profile 投影的规范结构）是 cage 自有形态，
编辑器 / 前端同形消费；两者各司其职：自有形态管编译链内消费，标准
形态管链外工具。
