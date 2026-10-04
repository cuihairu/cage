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

落地口径：内部一律 `BTreeMap` / 稳定排序，产物字节级可复现（golden 测试：同输入两次构建字节级一致），禁止时间戳与随机 ID 进入产物。

## Build Manifest

每次构建生成 Manifest（cage-core::manifest，`BuildManifest` / `ArtifactInfo`）：

```json
{
  "project": "game",
  "profile": "client",
  "cage_version": "0.1.0",
  "schema_hash": "...",
  "source_hash": "...",
  "content_hash": "...",
  "artifacts": {
    "item.json": { "path": "item.json", "hash": "...", "size": 123, "format": "json", "table": "Item", "encoding": "utf-8" }
  }
}
```

当前字段与账本目标对照（「编译器核心：八个概念的边界」中 Manifest 的差距落地面）：

| 字段 | 现状 | 说明 |
| --- | --- | --- |
| `project` / `profile` | 已实装 | 构建身份 |
| `cage_version` | 已实装 | 兼任 generator_version / 编译器版本 |
| `schema_hash` | 已实装 | Blake3，覆盖 schema 全量 |
| `source_hash` | 已实装 | Blake3，覆盖全部源内容 |
| `content_hash` | 已实装 | Blake3，覆盖全部产物字节 |
| `artifacts` | 已实装 | 每产物 path / hash / size / format / table / encoding |
| `build_id` | 待补 | 唯一构建标识（时间戳 + 短哈希），支撑服务器版本匹配与回滚点 |
| `dependencies` | 待补 | 表间依赖清单（依赖图接线后由 ManifestGenerator 落账） |
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

- 引用检查（L5 校验产出引用拓扑）
- 构建顺序
- 增量构建（第二层：变更影响传播）
- 变更影响分析
- 删除检查
- 循环依赖检测
- 调试

现状：核心类型与算法已实装并有测试（reference/mod.rs：`DependencyGraph` /
`IncrementalPlanner`、环检测、拓扑、增量规划），但**构建路径尚未接线**——
cli 构建时传给验证器的依赖图仍是空占位，真实图未参与构建决策。接线任务见
「增量构建」第二层与 todo 第四阶段。

## 增量构建

`cage build --incremental` 已实装第一层：哈希比对跳过。构建时把当前
schema/source 哈希与上一次 `manifest.json` 记录的值比对，同 profile、同
哈希且产物都在磁盘上时直接跳过重新生成（校验仍然全量执行）；任一输入变
化或产物缺失则全量重建。注意：target 配置（cage.toml 的 targets）不参与
哈希，改完请跑一次全量构建。

第二层（基于 Dependency Graph 的变更影响分析，只重建受影响表）未实装：
核心算法（环检测 / 拓扑 / 增量规划）已在 cage-core 就位，工程上是把
`ValidatedSchema` 里的空占位图换成 L5 真实构建的图，再沿边传播影响。

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
Dependency Graph
      |
      v
Affected Tables
      |
      v
Incremental Build
```

## Configuration Snapshot（规划）

Cage 的最终产物不只是零散的 JSON 文件，而应是可独立加载的
**Configuration Snapshot**（未实装，见架构文档「八个概念的边界」）：

```text
snapshot-2026-10-04-001/
├── manifest.json      # 构建账本（profile / schema_hash / source_hash / artifacts）
├── schema/            # 导出 schema 副本
├── data/              # 数据类产物（json / csv）
├── generated/         # 代码类产物
└── hashes/            # 逐文件校验清单
```

服务器启动流程：

```text
server
   ↓
load snapshot
   ↓
verify manifest（hash 与清单逐项比对）
   ↓
load configuration
```

快照自带校验信息、不依赖构建机现场——与 runtime 配置体系（见
[Configuration Registry](#configuration-registry)）互为正反两面：前者是产物
形态，后者是运行时遥测与发布面。

## Configuration Registry

后期可以增加远程配置仓库（第三阶段）：

```text
Cage Registry
```

存储：

```text
Schema
Source Metadata
Build Manifest
Artifacts
Hash
Version
```

例如：

```text
game-config/
    v1.0.0/
    v1.1.0/
    v1.2.0/
```

Registry 不进入 MVP。

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

任何配置错误都在进入游戏之前失败。本仓库自身由 CI 门禁（全测试绿）与每日构建守护，`warnings_as_errors` 策略见 [Validation：Warning Policy](/validation#warning-policy)。
