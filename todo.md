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

### W 系列：Web UI / Schema Editor（2026-10 开工，预排落 docs/web.md）
- [x] W1 编辑器交换模型（core）：`cage-core::edit`——Schema ↔ 编辑器
      JSON（to_editor_json/from_editor_json，解析失败报 E1701 并铺
      JSON 路径定位）与确定性 canonical YAML 保存写回/回读
      （to_canonical_yaml/from_canonical_yaml，同 Schema 同字节、
      回环幂等）；round-trip 与 E1701 路径测试；与 snapshot
      schema.json 同一形状；错误码表入 validation.md
- [x] W2 `cage web` 本地服务（HTTP API）：项目装载（复用 cli 加载链）、
      GET /api/schema（编辑器 JSON）、POST /api/validate（编辑态校验）、
      POST /api/schema（canonical YAML 写回）；本地工具不鉴权；
      集成测试 + README 入口
- [x] W3 Schema 编辑器前端（docs/public/editor 静态单页，无 node 构建链，
      编译期嵌入 cage 二进制）：表/枚举树、19 字段类型（Array/Object/
      Map/Enum 递归）、约束编辑、E 码诊断面板（点诊断定位字段）、保存
- [x] W4 文档与示例：编辑流程定稿、示例工程 web 冒烟（编辑→校验→
      保存→cage build 复现）、CI 集成（examples/web-demo + web-smoke.sh
      + ci.yml web-smoke job + docs/web.md 定稿）

### 其余（预排）
- [ ] Registry（远程配置仓库 + 版本）——已开工，拆解为下方「R 系列：
      Configuration Registry」（2026-10）；Remote Source 与 Artifact 分发
      仍留待后续独立立项
- [ ] Remote Source（Google Sheets/MySQL/PostgreSQL/HTTP API）
- [ ] Artifact 分发与迁移

### R 系列：Configuration Registry（2026-10 开工，design §29）

形态定稿：注册表根 `<registry>/<包>/<版本>/` 为**自校验 snapshot 入口**
（manifest/schema.json/data/generated/HASHES 逐文件 blake3 账本，与
`cage snapshot` 同一格式、同一次打包产物即同一字节），包目录另附
确定性 `index.json`（包名 + 版本序列表：版本/build_id/content_hash/文件数）。
发布与解析都先过账本校验——「未经校验不入册、未经校验不载入」。
版本序：点分数字序（1.9 < 1.10）；包/版本名只许 `[A-Za-z0-9._-]`，
路径穿越类输入直接拒绝。源解析语法 `registry:<包>[@<版本>]`
（省略版本 = 最高序），消费方在 cage.toml `[registry].path` 声明
注册表根（相对项目根）；解析成功 = 校验通过 + 解出条目 data/ 目录
作为源根。注册表字节全路径确定性：同输入 → 同条目 → 同 index。

- [x] R1 本地注册表 + 源解析：
      `cage-core::registry`（publish/resolve + 确定性 index.json +
      点分版本序 + 包/版本名合法性校验）；`cage registry publish`
      （全量构建 → 快照打包 → 账本自校验 → 入册；package 默认
      project.name、version 默认 project.version；同版本同字节重发
      幂等 OK，同版本异字节 E1801 版本冲突拒写）；`cage registry
      list`（包/版本/build_id/content_hash/文件数，确定性序）；
      source_roots 支持 `registry:` 前缀解析（未接 `[registry].path`
      、包不存在、版本不存在 → E1802 无法解析；账本校验不过 →
      E1803 条目校验失败；条目 data/ 按最高保真格式载入 json >
      yaml > csv > excel，表名以条目 manifest artifact 记录为准）；
      schema_path 本轮只走文件系统；E1801/E1802/E1803 入错误码表。
      验收达成：9 core 单测 + 3 CLI 集成测试（发布→列表→latest/
      pin 解析→构建复现→确定性字节→篡改 E1803→冲突 E1801→错误
      路径）全绿 + 文档（cli.md/validation.md/architecture.md/
      design.md §29/index.md）+ 本勾选
- [x] R2 Schema 解析与依赖声明：`schema_path: registry:<包>[@<版本>]`
      直接读条目 `schema.json`（发布时的 profile 投影 schema，serde
      同形回读；缺失/损坏 E1802），`cage web` 对 registry schema 工程
      拒绝保存（409，schema 归发布方所有）；`[dependencies]` 版本 pin
      ——比较符 `=` `>` `>=` `<` `<=` 逗号 AND、`^` caret（左起首个
      非零分量 +1 其后归零）/ `~` tilde（锁定次末位）展开、比较逐分量
      补零（`>=1.2` 不排除 `1.2.0`）；省略 `@版本` 取满足区间最高版本，
      显式 `@版本` 必须落在 pin 内，区间无解/越界/非法 pin 均 E1802。
      验收达成：4 新 core 单测（区间展开/补零满足性/非法规范/pin 解析
      与门禁）+ 1 新 CLI 集成测试（三版本 pin→1.9.0、caret、@版本
      冲突 pin、无解区间、非法 pin）+ 1 新 web 集成测试（GET 走注册
      表解析、POST 409 拒存）全绿 + 文档（cli.md 依赖章节/validation.md
      /architecture.md/design.md §29/index.md）+ 本勾选
- [ ] R3 远程 Registry 只读解析：HTTP(S) 注册表根（resolve 走网络，
      发布仍限本地；需鉴权/协议版本设计先行）
- [ ] R4 回滚与清理：多版本共存下的 GC 策略、`cage registry verify`
      全册校验工具、条目移除/重新发布纪律

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
- [x] D5 Configuration Snapshot：`cage snapshot` 打包（manifest/schema.json/
      data/generated/HASHES 逐文件 blake3 账本）至确定性目录
      `snapshot/<profile>-<build_id[..12]>`（弃日期命名——破坏确定性契约），
      构建后自校验；`cage snapshot <dir> --verify` 载入前校验；core 提供
      verify_snapshot/load 服务器入口（篡改/增删文件均暴露）
