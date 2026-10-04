# Cage TODO

> 依据 docs/design.md + docs/需求整理.md。每项可独立验收，完成即勾。

## 第一期 MVP

### T1 工程骨架
- [x] T1.1 Cargo workspace 起（cage-core/source-*/target-*/cli 多 crate + CI）
- [x] T1.2 Canonical Model（value.rs：Null/Bool/Int/UInt/Float/String/Bytes/Array/Object + Source Location）
- [x] T1.3 Diagnostics 框架（错误码表 + 全字段诊断 + 多级 Severity + 文本渲染器）

### T2 Source
- [x] T2.1 JSON Source Adapter（serde 读入 → Canonical Model，保留路径定位）
- [x] T2.2 YAML Source Adapter（语法错误 E0001 精确到行）
- [x] T2.3 CSV Source Adapter（表头/行号定位）
- [x] T2.4 Excel Source Adapter（calamine：sheet/合并单元格/缓存值；错误定位到单元格 G27 式）

### T3 Schema 与校验
- [x] T3.1 Schema DSL（YAML 定义：type/required/default/enum/min/max/unique/primary_key/reference）
- [x] T3.2 L1 Schema + L2 Type + L3 Value 校验（E1001/E1002/E1101/E1201 族）
- [x] T3.3 L4 Table（主键唯一 E1301、组合唯一）
- [x] T3.4 L5 Reference（E1401 存在性；E1410 谓词：引用对象字段约束）
- [x] T3.5 L6 Semantic 表达式规则（assert: min_level <= max_level，行级定位）
- [x] T3.6 `--level` 分级执行

### T4 Normalize 与 Target
- [x] T4.1 Normalize（数值/布尔归一，确定性 Canonical Representation）
- [x] T4.2 JSON Target（稳定键序、确定性字节输出）
- [x] T4.3 CSV Target
- [x] T4.4 Profile 机制（client/server 配置 + 字段级 targets 可见性过滤）

### T5 Build 与 CLI
- [x] T5.1 确定性构建 + Build Manifest（schema_hash/source_hash/content_hash/artifacts）
- [x] T5.2 CLI：cage check / build / inspect / diff（clap）
- [x] T5.3 golden 测试：同输入两次构建字节级一致

## 第二阶段（预排）
- [x] 代码生成器：C#/Python/Lua/TypeScript/JavaScript/C++/Go/Java
- [x] Plugin SDK + Game Rule Validator 实装（沙箱方案定稿于 design §17.1：进程内 trait 现已实装、动态库 C ABI shim 为第三方分发路线、不可信代码不执行；GameRuleValidator trait + GameRuleRegistry + 内建 power_curve 样例端到端，`cage check --level gamerule` 输出 E1601 行级诊断；动态库装载与插件市场不在本期）
- [x] Dependency Graph（cage graph：引用图/构建顺序/循环检测）
- [x] 增量构建（--incremental 按 manifest 的 schema/source 哈希跳过未变更的整轮重建；按 target 的变更影响传播未做，target 配置变更不参与哈希、需全量）
- [x] CI 集成（warnings_as_errors、GitHub Actions 模板）

## 第三阶段（预排）
- [ ] Web UI / Schema Editor
- [ ] Registry（远程配置仓库 + 版本）
- [ ] Remote Source（Google Sheets/MySQL/PostgreSQL/HTTP API）
- [ ] Artifact 分发与迁移

## 第四阶段（核心模型边界定稿，2026-10 评审驱动）

方向共识：不堆功能，先把「Schema / Canonical Model / IR / Validation
Context / Dependency Graph / Profile / Snapshot / Manifest」八个概念的边界
收敛；边界定义（含 IR 不拆独立类型的决策与逐概念现状·差距核对）落
docs/architecture.md「编译器核心：八个概念的边界」、清单落 docs/build.md。

- [x] D1 边界定稿（架构文档 v0.3 概念表 + 差距核对 + 评审对照修正；build.md
      Manifest 字段表 / 依赖图现状 / 增量分层 / Snapshot 规划）
- [x] D2 Dependency Graph 接线：L5 校验产出真实依赖图（cli 构建
      ValidatedSchema 时以 `reference::DependencyGraph::from_schema` 实装，
      不再是空占位），增量第二层按 `table_hashes` 比对 + 依赖图传播只重建
      受影响表（未受影响表从磁盘携带，manifest 与全量构建逐字节收敛，验证
      见 tests/incremental.rs layer2 用例）；manifest 落 `dependencies` 账
- [x] D3 Profile 语义化：校验在完整 schema/document 上执行（profile 只
      裁剪产物视图）、ValidationContext 携带 profile；E9006 冲突实装——
      结构不可缺字段（required 无默认/主键/唯一约束/引用目标表或字段）
      被 profile 隐藏报冲突码而非静默过滤，可选字段与整表剔除仍为合法
      视图；`cage check/build --profile` 与 `cage gen` 均按此语义
- [x] D4 Manifest 强化：`build_id`（确定性指纹 blake3(profile+三哈希)
      前 24 位——时间戳方案会破坏确定性构建契约，弃）与
      `generator_version`（"1.0.0"，布局演进自增）落地；旧 manifest
      缺字段经 serde(default) 兼容反序列化；确定性/轮换/兼容三测试
- [ ] D5 Configuration Snapshot：snapshot 构建（manifest + schema + 数据 +
      生成物 + 校验清单）、服务器启动加载校验入口
