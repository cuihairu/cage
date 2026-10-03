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

每次构建生成 Manifest：

```json
{
  "project": "game",
  "profile": "client",
  "cage_version": "0.1.0",
  "schema_hash": "...",
  "source_hash": "...",
  "content_hash": "...",
  "artifacts": {
    "item.json": "...",
    "monster.json": "...",
    "skill.json": "..."
  }
}
```

用途：

- 版本追踪
- 部署
- 回滚
- 客户端/服务端版本匹配
- CI
- 缓存
- 增量构建

## 配置依赖图

Cage 应该建立配置依赖图（第二阶段）。例如：

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

- 引用检查
- 构建顺序
- 增量构建
- 变更影响分析
- 删除检查
- 循环依赖检测
- 调试

## 增量构建

`cage build --incremental` 已实装第一层：哈希比对跳过。构建时把当前
schema/source 哈希与上一次 `manifest.json` 记录的值比对，同 profile、同
哈希且产物都在磁盘上时直接跳过重新生成（校验仍然全量执行）；任一输入变
化或产物缺失则全量重建。注意：target 配置（cage.toml 的 targets）不参与
哈希，改完请跑一次全量构建。

基于 Dependency Graph 的第二层（变更影响分析，只重建受影响表）仍为第二阶段。

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
