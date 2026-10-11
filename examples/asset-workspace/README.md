# 资产体系完整示例（asset-workspace）

这是 Cage 资产体系（design §50）的最小完整工程：**四类资产俱全**，
配一个一键演练脚本 `examples/asset-workspace.sh`，把
「盘点 → 基线 → 校验 → 构建 → 快照 → 发布 → 签名分发 → 灾难恢复」
整条生命周期跑一遍。

```bash
bash examples/asset-workspace.sh
```

## 分类 ↔ 目录映射

| 资产类 | 目录 / 文件 | 回答的问题 | cage 能力 |
|--------|-------------|------------|-----------|
| 规则资产 | `schemas/` + `migrations/` | 数据应该长什么样（含校验规则、枚举、跨表约束） | `cage check`、迁移链 |
| 源数据资产 | `sources/` | 数据本身（策划填的表） | `cage build` 消费 |
| 构建规则 | `cage.toml` | 怎么构建（profiles / targets / registry） | `cage build --profile` |
| 产物与追溯 | `build/`、快照、`reg/` 条目 | 当时编译出了什么、能不能复现 | `cage snapshot`、registry、`cage diff` |

三个裁定（design §50）在本工程的落点：

- **rules/ 不设独立目录** —— 规则事实源就是 `schemas/` 文件集 +
  `migrations/` 链（校验规则、枚举、跨表约束写在 schema 里），
  拆目录等于维护两份账。
- **`assets.json` 基线随仓入库** —— `cage assets --write assets.json`
  的盘点快照是第一层防线的一部分，提交进 Git，`--check` 随时复核。
- **`build/` 与 `reg/` 不入版本控制** —— 产物可由源字节确定性重建，
  入库只会制造伪差异（`.gitignore` 已排除 `examples/*/build/`、
  `examples/*/reg/`）。

## 备份四层防线（每层独立成立，互不替代）

| 层 | 手段 | 本示例中的体现 | 防什么 |
|----|------|----------------|--------|
| 1. Git 版本管理 | 逐提交入库 | `sources/` `schemas/` `migrations/` `cage.toml` + `assets.json` 基线 | 改坏可回退、改了什么可追溯 |
| 2. 远程仓库异地 | push 到远端托管 | （部署事项）origin 即是 | 本机磁盘故障 |
| 3. 发布快照复现 | `cage snapshot` + `registry publish` + `export` | 脚本第 5-7 步：快照带 manifest（schema 哈希、源哈希、cage 版本），bundle 可离线分发 | 需要精确找回「当时用了哪版规则、编译出了什么」 |
| 4. 独立备份灾难恢复 | 异地冷备 + 恢复演练 | 脚本第 8 步：干净目录 import + verify + 重建字节 diff | 仓库托管整个不可用（GitHub 不是灾难恢复） |

注意：**备份 ≠ 版本管理 ≠ 可复现构建**。备份救丢失，版本救回退，
可复现救「为何与历史不一致」。第 3 层的快照能同时回答：

- 当时用了哪版 Schema 和校验规则？（manifest 的 schema 哈希 +
  规则资产逐文件 blake3，配合 `assets.json` 基线）
- 编译出了什么？（快照内 data/ + generated/ 全量产物）
- 为何与历史不一致？`cage diff`（快照 × 快照 / 目录 × 目录）给出
  逐文件差异。

## 发布工作流的 commit sidecar

`cage registry export --sign` 之外，发布流水线应把产线 commit 一并
封存：

```bash
git rev-parse HEAD > bundle.commit
```

为什么不进 manifest：manifest 记的是**内容**（同输入必出同字节的
确定性契约），commit 是**历史**（同内容可以在不同 commit 重现）。
把 commit 塞进 manifest 会破坏字节级确定性；sidecar 由工作流负责，
消费方按需对账。

## 恢复演练步骤（脚本第 8 步 automate 了这一切）

1. `cage registry keygen -o <seed-file>` —— 一次性生成签名密钥
   （seed 保密，public key 分发给消费方）。
2. 发布方：`registry publish` → `registry export --sign --key-env`
   → `git rev-parse HEAD > bundle.commit`，bundle + `.sig` +
   `.commit` 三件套送冷备。
3. 演练机（模拟灾难后）：全新空目录。
4. `cage registry import bundle.tar --verify-sig --key-env` ——
   签名先验，再过账本信任门，字节不符 E2107 拒入。
5. `cage registry verify` —— 逐条目账本复核（E1803）。
6. 消费方工程 `source_roots = "registry:asset-demo@0.1.0"` 连本地
   注册表构建，产物与灾前 `build/client/json/` **逐字节 diff** ——
   一致才算恢复成功。
