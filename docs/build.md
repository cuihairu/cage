# Build：确定性构建

确定性是配置构建系统的硬要求：**相同输入必须得到相同产物**。

## 确定性构建

相同：

```text
Source
Schema
Cage Version
Profile
```

应该得到相同：

```text
Artifact
Hash
Manifest
```

即：

```text
Build(A) == Build(A)
```

避免：

- 时间戳导致文件变化
- 不确定 Map 顺序
- 随机 ID
- 非稳定排序

落地口径：`IndexMap` 保序 + 显式稳定排序（对象键经 `BTreeMap` 规范化），产物字节级可复现（golden 测试：同输入两次构建字节级一致），禁止时间戳与随机 ID 进入产物。

## Build Manifest

每次构建生成 Manifest（cage-core::manifest，`BuildManifest` / `ArtifactInfo`）：

```json
{
  "project": "game",
  "profile": "client",
  "cage_version": "0.1.0",
  "generator_version": "1.0.0",
  "build_id": "...",
  "schema_hash": "...",
  "source_hash": "...",
  "content_hash": "...",
  "dependencies": { "Monster": ["DropTable"], "DropTable": ["Item"] },
  "table_hashes": { "Item": "...", "DropTable": "...", "Monster": "..." },
  "artifacts": {
    "item.json": { "path": "item.json", "hash": "...", "size": 123, "format": "json", "table": "Item", "encoding": "utf-8" }
  }
}
```

当前字段与账本目标对照（「编译器核心：八个概念的边界」中 Manifest 的差距落地面）：

| 字段 | 现状 | 说明 |
| --- | --- | --- |
| `project` / `profile` | 已实装 | 构建身份 |
| `cage_version` | 已实装 | cage 编译器版本 |
| `generator_version` | 已实装 | Manifest 结构版本（"1.0.0"；布局演进时自增，独立于 cage 版本） |
| `build_id` | 已实装 | **确定性指纹**：blake3(profile + schema_hash + source_hash + content_hash) 前 24 位。同输入同 ID（可复现/匹配/回滚），语义变更即旋转。不用时间戳——那会破坏「相同输入 → 相同 Manifest 字节」的确定性契约 |
| `schema_hash` | 已实装 | Blake3，覆盖 schema 全量 |
| `source_hash` | 已实装 | Blake3，覆盖全部源内容 |
| `content_hash` | 已实装 | Blake3，覆盖全部产物字节 |
| `artifacts` | 已实装 | 每产物 path / hash / size / format / table / encoding |
| `dependencies` | 已实装 | 表间引用账：`表 -> 引用表名有序列表`（L5 依赖图接线后由 ManifestGenerator 落账，D2；旧 manifest 缺字段经 serde(default) 兼容） |
| `table_hashes` | 已实装 | 每表行级指纹（增量第二层变更检测） |
| `ir_hash` | 随 IR 定界省略 | IR 与 Canonical 同构（见架构文档），以 schema_hash + source_hash 覆盖 |

用途：

- 版本追踪
- 部署
- 回滚
- 客户端/服务端版本匹配
- CI
- 缓存
- 增量构建

## 配置依赖图

表间引用构成配置依赖图（cage-core::reference 已实装 `DependencyGraph`，含
环检测与拓扑排序）。例如：

```text
Monster
   |
   | DropTableID
   v
DropTable
   |
   | ItemID
   v
Item
```

最终：

```text
Monster
   |
   +----> DropTable
              |
              +----> Item
```

这个依赖图可以用于：

- 引用检查（L5 校验消费引用拓扑）
- 构建顺序
- 增量构建（第二层：变更影响传播）
- 变更影响分析
- 删除检查
- 循环依赖检测
- 调试

现状：核心类型与算法已实装并有测试（reference/mod.rs：`DependencyGraph` /
`IncrementalPlanner`、环检测、拓扑、增量规划），构建路径已接线——引用图由
schema 引用声明经 `DependencyGraph::from_schema` 构建（先于且独立于 L5
校验运行），L5 校验与 cli 增量第二层共同消费它，manifest 落
`dependencies` 引用账。核心与 cli 的接线覆盖在
tests/incremental.rs（含第二层端到端：变更传播 + 携带 + manifest 收敛）。

## 增量构建

`cage build --incremental` 分两层。

**第一层：哈希比对跳过。** 构建时把当前 schema/source 哈希与上一次
`manifest.json` 记录的值比对，同 profile、同哈希且产物都在磁盘上时直接
跳过重新生成（校验仍然全量执行）；任一输入变化或产物缺失则进入第二层或
全量重建。注意：target 配置（cage.toml 的 targets）不参与哈希，改完请跑
一次全量构建。

**第二层：依赖图传播（v0.3 实装）。** 当 schema 哈希未变（schema 驱动
code-target 形态，schema 变则整体回退全量）、prev 与当前 profile 相同、
`table_hashes` 非空且没有
删除表时，按表哈希找出变更表，经依赖图
（`IncrementalPlanner::compute_affected`）传播出受影响表集合，只重建这
些表：

- 生成输入裁剪到受影响表（code-target 保留完整枚举集：共享 enums 单元
  全量重生成，从不携带）；
- 未受影响表、且在磁盘上完好的产物从磁盘携带（generator 确定性 ⇒ 携带
  字节与全量构建完全一致），产物顺序按路径排序规范化，合并后的 manifest
  与全量构建逐字节一致（确定性契约）。

回退全量的条件：prev 无 `table_hashes`（旧版 manifest）、删除过表（prev
表集合 ⊄ 当前 —— 避免把过期产物错误携带）、携带产物读失败（warn 后
回退全量）。携带产物缺文件不算回退：缺文件的表在计算受影响集合前已
视为变更，随受影响表定向重生成。

例如 `Item.xlsx` 修改，影响：

```text
Item
 |
 +--> DropTable
 |
 +--> Monster
```

那么 `Skill / Quest / Map` 无需重新生成：

```text
Changed Sources
      |
      v
table_hashes 比对 → 变更表
      |
      v
Dependency Graph 传播 → 受影响表
      |
      v
只重建受影响表 + 携带其余（字节一致）
```

`drop-table → full fallback` 语义：`table_hashes` 的键是上一次构建的全部
表，当前 schema 若少了任何表（删除场景），第二层整体回退全量重建，避免
把过期产物错误携带。

## Configuration Snapshot（v0.3 实装）

构建产物不止零散 JSON：`cage snapshot` 把 profile 视图打包成可独立
加载、自带校验的 **Configuration Snapshot**（八个概念 #7，格式定义在
cage_core::snapshot）：

```text
build/snapshot/<profile>-<build_id[..12]>/
├── manifest.json      # 构建账本（profile / hashes / artifacts，与 build 逐字节一致）
├── schema.json        # profile 视图的规范 schema（构建所依据的形态）
├── data/…             # 数据类产物（json / csv / msgpack）
├── generated/…        # 代码类产物
└── HASHES.json        # 逐文件 blake3 账本（trust root，不自我哈希）+ build_id/content_hash
```

与规划草稿的差异：目录名用 **profile + build_id 指纹**而非日期——时间戳
命名会破坏确定性构建契约（同输入 → 同快照字节、同名目录，重建即覆盖）。
数据/代码产物按 target format 分区，路径剥掉 `output_dir` 前缀、保留目标
子目录（`build/client/Item.json` → `data/client/Item.json`）。

使用：

```text
cage snapshot <project> --profile <p>   # 构建 + 打包 + 自校验
cage snapshot <snapshot-dir> --verify   # 载入前校验（内核入口同服务器）
```

服务器启动流程（cage_core::snapshot::{verify_snapshot, load}）：

```text
server
   ↓
verify HASHES.json（逐文件重哈希比对，缺/错/多文件均报）
   ↓
load（manifest + schema + data/generated 产物全量读入映射）
   ↓
load configuration
```

严格性说明：逐文件账本比对的是「文件即账本、账本即文件」的完整一致性；
`content_hash` 已涵盖产物整体指纹，作为账本中的交叉字段带上。整目录的
值传递：篡改任一文件、增删任一文件都会在校验时暴露。

快照自带校验信息、不依赖构建机现场。它与 runtime 侧的
[Configuration Registry](#configuration-registry) 分工明确：前者管产物
形态，后者管多版本分发与回滚。

## Configuration Registry（R1–R4 已实装，design §29）

注册表把上面的可独立加载快照升级为**多版本配置仓库**：同一包可共存多
个版本的[自校验快照](/cli#registry)（`<registry>/<包>/<版本>/` 即一枚
Snapshot，逐文件 blake3 账本），包目录附确定性 `index.json`（版本序 / 
build_id / content_hash / 文件数），消费方经 `registry:<包>[@<版本>]`
载入条目——「未经校验不入册、未经校验不载入」。

首签 R1 本地发布/解析后，R 系列持续推进：

- R2 `[dependencies]` 版本 pin（比较符区间 / `^` / `~`）与
  `schema_path: registry:` 解析；
- R3 远程 http(s) 根只读解析（匿名 GET 三资源 + 项目内
  `.cage-cache` 缓存、离线复用）；
- R4 全册 `verify` 审计、滚动窗口 `gc`、显式 `remove` 与同字节重发。

命令、配置句法与回滚纪律见 [CLI：registry](/cli#registry)；
协议与错误码见 [design §29](https://github.com/cuihairu/cage/blob/main/docs/design.md#29-configuration-registry) 与
[validation.md E180x](/validation#registry配置仓库第三阶段-r-系列)。

## CI/CD

Cage 应该天然适合 CI：

```text
Git Push
    |
    v
CI
    |
    v
cage check
    |
    +---- Error ---> Build Failed
    |
    v
cage build
    |
    v
Artifacts
    |
    v
Package / Deploy
```

例如：

```bash
cage check --profile client
cage check --profile server

cage build --profile client
cage build --profile server
```

配置错误在 CI 里先失败，不带进游戏。本仓库自身由 CI 门禁（全测试绿）与每日构建守护，`warnings_as_errors` 策略见 [Validation：Warning Policy](/validation#warning-policy)。
