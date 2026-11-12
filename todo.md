# Cage TODO

> 依据 docs/design.md + docs/requirements.md。每项可独立验收，完成即勾。

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
- [x] T3.7 命名枚举成员资格校验
### T4 Normalize 与 Target

- [x] T4.1 Normalize（数值/布尔归一，确定性 Canonical Representation）
- [x] T4.2 JSON Target（稳定键序、确定性字节输出）
- [x] T4.3 CSV Target
- [x] T4.4 Profile 机制（client/server 配置 + 字段级 targets 可见性过滤）
### T5 Build 与 CLI

- [x] T5.1 确定性构建 + Build Manifest（schema_hash/source_hash/content_hash/artifacts）
- [x] T5.2 CLI
- [x] T5.3 golden 测试
## 第二阶段

- [x] 代码生成器
- [x] MsgPack 生成器
  （workspace 615 全绿；8 单测；1 集成）
- [x] Protobuf 生成器
  （workspace 630 全绿；14 单测；1 集成）
- [x] JSON Schema 生成器
  （docs：docs/assets.md）
- [x] Plugin SDK + Game Rule Validator 实装（沙箱方案定稿于 design §17.1：进程内 trait 现已实装、动态库 C ABI shim 为第三方分发路线、不可信代码不执行；GameRuleValidator trait + GameRuleRegistry + 内建 power_curve 样例端到端，`cage check --level gamerule` 输出 E1601 行级诊断；动态库装载与插件市场不在本期）
- [x] Dependency Graph（cage graph：引用图/构建顺序/循环检测）
- [x] E1403 循环检测接线进校验面
- [x] msgpack source adapter
- [x] 增量构建
  （workspace 649 全绿；docs：docs/build.md）
- [x] CI 集成（warnings_as_errors、GitHub Actions 模板）
## 第三阶段

### W 系列：Web UI / Schema Editor

- [x] W1 编辑器交换模型（core）
- [x] W2 `cage web` 本地服务（HTTP API）
- [x] W3 Schema 编辑器前端
- [x] W4 文档与示例
  （docs：docs/web.md）
### 其余

- [x] Registry
- [x] Remote Source（Google Sheets/MySQL/PostgreSQL/HTTP API）
- [x] 分环境验证 `--env`
- [x] Artifact 分发与迁移
### R 系列：Configuration Registry

- [x] R1 本地注册表 + 源解析
  （3 集成）
- [x] R2 Schema 解析与依赖声明
  （1 集成）
- [x] R3 远程 Registry 只读解析
  （2 集成）
- [x] R4 回滚与清理
  （2 集成）
### S 系列：Remote Source

- [x] S0 设计定稿
- [x] S1 HTTP API 源（`cage-source-http`）
  （2 集成）
- [x] S2 MySQL / PostgreSQL 源
  （2 集成）
- [x] S3 Google Sheets 源（`cage-source-sheets`）
  （1 集成）
- [x] S4 确定性与离线语义收口
- [x] S5 错误码接线收口
- [x] S6 文档收口
- [x] S7 增量交付
- [x] S9 库表内省 Schema 草稿
### 分环境验证

- [x] 分环境约束覆盖 `--env`
- [x] 环境化出包
- [x] 未选中环境坏覆盖 lint
- [x] S8 条件 GET 增量拉取
### A 系列：Artifact Distribution

- [x] A0 设计定稿
- [x] A1 bundle 导出（`cage registry export`）
- [x] A2 bundle 导入（`cage registry import`）
- [x] A3 直推（`cage registry push` + auth_env）
- [x] A4 文档收口
- [x] A5 压缩容器
- [x] A6 签名账本（§47 留待实现期：ed25519 抗抵赖签名）
- [x] A7 §47 留待实现期三项闭账
### M 系列：Migration

- [x] M0 设计定稿
- [x] M1 规则模型与解析
- [x] M2 执行器与报告
- [x] M3 CLI 与文档收口
- [x] M3.1 增量交付
- [x] M3.2 增量交付（§46 留待实现期：schema diff 规则草稿）
- [x] 迁移与依赖 pin 联动校验
### G 系列：Template Target

- [x] G1 模板引擎接入
  （10 单测）
- [x] G2 官方模板改写
- [x] G2b 余量改写
- [x] G3 自定义模板加载
  （workspace 534 全绿）
- [x] G4 过滤器库
  （workspace 545 全绿）
- [x] G5 文档收口
## 第四阶段

- [x] D1 边界定稿
- [x] D2 Dependency Graph 接线
- [x] D3 Profile 语义化
- [x] D4 Manifest 强化
- [x] D5 Configuration Snapshot
### 验收打磨

- [x] 错误码全量走查
  （docs：docs/validation.md）
- [x] presign 拒绝路径测试补齐
- [x] CLI presign-map 错误路径端到端
### L6 表达式引擎实装

- [x] `validation/expr.rs`
- [x] L6 接线
- [x] L5 接线
- [x] 测试：expr 解析/求值 5 例 + L6 违例/可选缺失/warning/不可比
- [x] 文档：validation.md L6 段落与 E1004/E1410/E1501 行、codes.rs
### 外部/拍板卡点登记

- [x] E1404 基数校验接线
- [x] cage-cli web API 单测补齐（覆盖率 0% → 71.6%）
### R3 远端完整性防线覆盖

- [x] registry_remote 新增完整性失败路径测试
- [x] remote.rs 收尾
- [x] web.rs handle 接线覆盖（行覆盖 69.79% → 90.57%）
### migrate/diff.rs 覆盖收口

- [x] diff.rs 未覆盖行 84.01% → 96.58%
### cage-cli main.rs 覆盖收口

- [x] main.rs 未覆盖行 84.90% → 94.36% → 98.50%
- [x] 顺带修复：pack_snapshot 自校验由 debug_assert! 改为硬错误
- [x] 登记 57 行 → 收口至 49 行（98.5% → 98.7%，全目标口径）
### cage-core registry.rs 覆盖收口

- [x] registry.rs 覆盖率 96.2% → 99.33%
- [x] 剩余 18 行收口至 9 行（262 lib 测试全绿）
### cage-core migrate/mod.rs 覆盖收口

- [x] migrate/mod.rs 覆盖率 95.1% → 100%（64 → 0 行未覆盖，790 测试全绿）
### 路线图收尾巡检

- [x] 环境化出包闭环核对
  （commit 4e1d4fe73f89）
- [x] 待办清零：todo 全册无开放功能项
### cage-target-java 覆盖收口

- [x] cage-target-java 覆盖率 95.65% → 99.36%
### cage-target-cpp 覆盖收口

- [x] cage-target-cpp 覆盖率 52 行未覆盖 → 2 行
### cage-source-http 覆盖收口

- [x] cage-source-http 覆盖率 99.24%
### validation/mod.rs 覆盖收口

- [x] validation/mod.rs 37 行未覆盖 → 3 行（269 lib 测试全绿，workspace 812）
### cage-cli web.rs 覆盖收口

- [x] web.rs 32 行未覆盖 → 14 行（96.53%，390/404，54 bin 测试全绿）
- [x] 覆盖率收尾登记册全清
### 覆盖率 98% 门槛（2026-11-09）

- [x] 实测：codecov 口径=CI 无 DB env=97.18%（50587/52053）；开 DB live 测试=98.03%（51030/52053）。缺口 0.16% 不是"几个分支"，而是 cage-source-db live 测试在 CI 从未运行（443 行 DB 代码仅活库可覆盖）。
- [x] 处置：补 FieldType value-first replay 全覆盖测试（`schema/mod.rs` buf_content_replay_tests + `tests/field_type_depth.rs`，395 lib + 14 集成全绿）+ CI coverage job 加 postgres:16/mysql:8.0 service 并注入 DSN（`ci.yml`，live 测试如实参与统计）。
- [x] 证据：commit `51faf98`（test）、`24147a6`（ci），本地 DB-enabled 98.03%，剩余未覆盖集中在 web.rs 阻塞服务器/fs 竞态臂、main.rs 错误臂、remote.rs 网络错误臂等难达分支；codecov project 状态为 informational（`target: auto`，1% 阈值），不构成合入门禁。
- [x] CI 确认：run `42604610186` 全绿；4 个 live 测试（`live_pg_pooled_fetch_round_trips_pins_and_reuses`/`live_pg_bulk_fetch_round_trips`/`live_mysql_*`）在 CI 真实运行通过（service 生效）；codecov/project **98.71%**（+1.55% vs `4419e97`），codecov/patch 100.00%，达标。
### 路线图四项拍板

- [x] 四项逐条裁定完毕
- [x] I 系列：IDE 支持
- [x] F 系列：FlatBuffers Target
- [x] L 系列：注册表远程包枚举
- [x] §45 外部依赖三项 → 卡点登记
### B 系列：资产体系

- [x] B0 决策记录
- [x] B1 `cage assets` 资产清单命令
  （6 单测）
- [x] B2 完整示例 `examples/asset-workspace/`
- [x] B3 文档收口
  （docs：docs/assets.md）
### D 系列：文档站图表 Mermaid 化

- [x] D0 architecture.md 图示 Mermaid 化
### §45 连接池 / 大表取行落地

- [x] 进程级按 DSN 连接池（`cage-source-db::pool`）
- [x] 大表取行：mysql `query` 整体缓冲改 `query_iter` 逐行流式
- [x] 测试先行 + live 验证
- [x] live 测试钓出真 bug
- [x] 并发多源裁定不立项
- [x] 文档：design §45 卡点登记补落地附记、「留待实现期」划线
### 示例四套环境补齐

- [x] ① schemas 以 `env_overrides` 键声明四套环境
- [x] ② examples/run.sh 第 3 步改四套 check + 四套 build
- [x] ③ 文档实测输出同步
  （commit df808dfaa1cd；docs：docs/example.md）
- [x] ④ 实测证据
  （commit 6271d907b39e）
### 两个功能单

- [x] 功能 A：`cage init` 交互式脚手架
  （docs：docs/cli.md）
- [x] 功能 B：`cage mcp` MCP server（stdio）
  （12 单测；docs：docs/cli.md）
- [x] 文档 + UI 单
  （docs：docs/lifecycle.md）
- [x] 文档与实际子命令脱节
  （docs：docs/cli.md）
- [x] VSCode 插件支持
  （docs：docs/cli.md）
## 文档双语

- [x] D1 docs 根目录双语配对
- [x] D2 design.md（3244 行）处理口径
- [x] D2b design.md 完整英文版
  （docs：docs/design.en.md）
- [x] D3 站点英文导航
- [x] D4 docs/design/ 两个 HTML 原型（init-wizard / validation-result）界面文案
  （docs：docs/design-prototypes.md）
- [x] D4b 子目录缺口核查
- [x] D5 README.md 双语收口
  （docs：docs/requirements.md）
- [x] 审核整改
- [x] 表一#1 增量删除越界
- [x] 表一#2 Web 边界
- [x] 表一#3 + 表二#6 快照语义
- [x] 表二#1-#5 文档对齐（以代码为准，未改业务行为）
- [x] 附带修复：luna-review 两报告加入 srcExclude
- [x] 表一#4 迁移写回事务性
- [x] 表三评估与分期
  （docs：docs/design.md）
- [x] 文档时效性清理（2026-10-18 用户令：很多文档已过时）
  （docs：docs/index.md）
## 规则配置界面

- [x] P0 一键校验界面
  （docs：docs/web.md）
- [x] P1 规则表单化编辑
  （docs：docs/web.md）
- [x] P1 一键导出
  （docs：docs/ui.md）
- [x] P2 版本对比
  （docs：docs/ui.md）
- [x] P2 模板库
  （docs：docs/web.md）
## 外部评审：Luban 对标

- [x] P0 三源一致性对拍套件
- [x] P0 可声明游戏规则系统
  （commit 12c8038）
- [x] P1 Luban 同题对照五案例测试
  （commit 6782fbe）
- [x] 回流-1 Excel 空单元格默认值
  （commit f889b12）
- [x] 回流-2 nullable 类型字段
  （commit 2061110；docs：docs/schema.md）
- [x] 回流-3 单元格分隔列表
  （commit e3d49b8；docs：docs/schema.md）
- [x] 回流-4 数值塔统一
  （commit 397d098；docs：docs/schema.md）
- [x] 回流-5 多级表头/嵌套 bean/多行记录/多态
  （commit e5aacee；docs：docs/design.md）
- [x] 回流-6 tagged union 类型
  （commit cd2858c；docs：docs/schema.md）
## Excel 元行规则承载

- [x] X0 约定解析架构定稿
  （docs：docs/design.md）
- [x] X1 约定解析层（内置预设 + 脚本适配器）
  （12 单测；docs：docs/design.md）
- [x] X1 跟进
- [x] X2 schema-draft Excel 内省 + 漂移核对
  （commit 50bcfe5）
- [x] X3 与三源对拍合流
  （commit 9174f28）
- [x] X4 文档与示例
  （commit f613ac6；docs：docs/design.md）
- [x] Excel 日期单元格数据面缺陷
  （commit 347b690）
## 代码深审待办（2026-11-01 批次）

- [x] P2-1 远端 HTTP 响应体读取设字节上限（remote.rs:326,490，现 read_to_end 无上限）
  （已修：remote.rs 新增 MAX_RESPONSE_BODY_BYTES=64MiB + read_bounded()，GET/PUT 两路共用，FetchFailure/PutFailure 增 TooLarge；cage-core 338 单测全绿，cli/sheets/http 集成全绿）
- [x] P2-2 flatbuffers `(key)` 复查 flatc 类型资格（lib.rs:294-300，非 string/整型单字段主键会产出非法 .fbs）
  （已修：`key_eligible()` 过滤单字段主键——仅 string/整型/枚举标量保留 `(key)`，其余类型静默丢弃该属性；新增 `test_key_attribute_respects_flatc_type_eligibility`，cage-target-flatbuffers 7 测试全绿）
- [x] P3-1 schema/FieldType 递归深度显式护栏（schema/mod.rs:288-371）
  （已修：新增 `MAX_FIELD_TYPE_DEPTH=64` + `field_type_depth`，`Schema::validate()` 对超深 Array/Map/Object/TaggedUnion 报 E1004；新增 `schema_rejects_field_type_deeper_than_the_max` 覆盖超限拒绝与恰好达限接受）
- [x] P3-2 valid_component 收紧（registry.rs:61-68，拒单 `.` 作包/版本名）
  （已修：`valid_component()` 要求至少一个字母/数字，单 `.`、`-`、`_` 均拒；扩展 `publish_rejects_bad_names_unverifiable_packs_and_blocked_targets` 覆盖单 `.` 包名/版本名与单 `-` 包名）

## 审核发现

- [x] 审核发现：X2 未兑现 §51 Q2/Q5 脚本指纹契约——脚本字节 blake3/解析路径/script_config/契约版本从未汇入 source_hash 或 build_id，改脚本但输出不变时 build_id 不变，违背设计 Q2「编辑脚本或其 config 必改 build_id」承诺（commit 101d581）
- [x] 审核发现：drift 对结构形状漂移漏报——嵌套 bean/multi_row 列自身隐含的 Object/Array 类型从不与 adopted 比较，子列递归仅在双方 properties 均 Some 时发生且从不下钻 items，故工作簿声明 bean/multi_row 而 schema 写标量或数组标量时静默通过 E2208（commit 101d581）
- [x] 审核发现：派生 schema 丢弃 bean/multi_row 列自身的 required/default/targets——field_schema/bean_schema 对含子列字段统一走 ..empty_field()，无 YAML 派生链路下 ##required true 声明被静默降级为 false（commit 101d581）
- [x] 审核发现：sheets 脚本适配器违反共享契约网格类型——request.grid 对所有单元格调 .text() 致数字/布尔/空格全部序列化为字符串（空格变 "" 而非 null），与契约 Q2「number/string/boolean/null」及 Excel 侧 cell_to_json 的类型保真不一致（commit 101d581）
- [x] 审核发现：Web 状态页仍以 source_extra=None 重算 source_hash，声明脚本适配器时修改脚本或 script_config 会被误报为 fresh（commit 101d581）
- [x] 审核发现：drift 递归仍只在 adopted properties 存在时下钻，仅用 type: Object / Array<Object> 的常规 schema 会跳过全部子列存在性与类型检查（commit 101d581）
- [x] 审核发现：脚本指纹把未规范化的 root.join(script) 路径写入 source_hash，同一工程用相对路径与绝对路径构建会得到不同 build_id，破坏同输入同字节承诺（commit 101d581）
- [x] 审核发现：script_config 指纹未做 canonical 键序规范化，仅交换同值配置项顺序就会改变 build_id，与“canonical form”及确定性承诺不符（commit 101d581）
- [x] 审核发现：无 convention 时同名表头靠 fields.insert 直接覆盖,后列静默覆盖前列,前列值丢失（commit 101d581）
- [x] 审核发现：空 used-range 的 sheet 在 range.is_empty() 处被无诊断静默跳过,与读失败会发 E0001 警告不一致（commit 101d581）
- [x] 审核发现：重复表头查重只覆盖扁平路径，任一结构化列使 header_groups 存在后，重复顶级表头仍会静默覆盖并丢失前列值（commit 101d581）
- [x] 审核发现：结构化路径查重对空 `##var` 单元格（未被合并区覆盖时 name 为空串）也参与查重，两个及以上空顶级表头触发 E0001「Duplicate column name ''」硬失败；而扁平路径 headers_at 对空表头合成唯一 col<N> 名（commit 101d581）
- [x] 审核发现：`docs/review/2026-10-30-decisions.md` 多处行号引用按写入时基线（commit 4acaf9c）
- [x] 审核发现：`docs/review/2026-10-30-decisions.md:16` 称引用「`docs/ui.md`『策划规则界面』节」，但 `docs/ui.md` 为英文文档，该节实际标题为「Planner Rule Interface」（commit 4acaf9c）
- [x] 审核发现：`docs/review/2026-10-30-decisions.md:11` 修复时把 `docs/design-prototypes.md:64-71` 改为 `82-88`，但该文件 HEAD 仅 87 行、行 88 不存在，引用越界；§III 三条 Open decisions（原始问题）实际在 82-87，应引 `82-87`（commit 101d581）
- [x] 审核发现：todo.md 新增「跨语言标题」审核发现记录称 "Planner Rule Interface" 节位于 `docs/ui.md:269`，实际该节标题在 `docs/ui.md:268`，行号偏差 1 行（commit 101d581）
- [x] 审核发现：新增架构图 SVG（中英两份）称 CLI 子命令含 `compile`（`cage check --level / compile / migrate / lsp`），但 `cage` CLI 实际子命令为 check/build/gen/inspect/assets 等，无 `compile` 子命令，与源码不符（commit 101d581）
- [x] 审核发现：SVG 中 `id="cli-cross"` 的 `data-from="cli"` 引用不存在的节点（图中无 `id="cli"` 元素），而其余箭头的 `data-from` 均指向真实节点，内部元数据引用悬空（commit 101d581）
- [x] 审核发现：新增记录称 commit 0bfc962「仅把上一轮记录勾选为 [x]」，但该提交实际新增了一条审核记录且未改动上一行，陈述与仓库历史不符（commit 0bfc962）
- [x] 审核发现：英文 README.md 顶部一级标题 `# Cage` 与项目简介被移除，现无任何标题/标语，与基线及中文版（仍有 `# Cage`）不一致（commit 101d581）
- [x] 审核发现：README.md:4 存在两个无内容的空 `<a href="https://cuihairu.github.io/cage//">`（其一 URL 含双斜杠 `cage//`）锚点，为损坏/残留标记（commit 101d581）
- [x] 审核发现：README.md:5 两个 shield 徽标未套链接且标签错误（`docs-docs-blue`、`docs-blue`，alt 重复为 `docs`），应为单个可点击的 `docs-latest-blue` 文档徽标（commit 101d581）
- [x] 审核发现：README.zh.md:9 文档徽标重复——既有 `[Docs]` 指向 `/cage/zh/`，又新增 `文档` 徽标同样指向 `/cage/zh/`，冗余且与英文版单徽标设计不一致（commit 101d581）
- [x] 审核发现：README.zh.md 文件末尾缺少换行符（基线以 `\n` 结尾，现 `[Apache-2.0](LICENSE)` 后无 `\n`）（commit 101d581）
- [x] 审核发现：`docs/source.md:100-109`（中文版 `docs/zh/source.md` 同）的 Excel 元行「Vocabulary」表漏掉实现已支持的 `##unique`、`##targets`、`##description` 三个关键字（`crates/cage-core/src/convention/mod.rs:871-874` 的 slot_is_set 分支均接受），表题为“Vocabulary”且 design §51 明示 `##description` 为注释行，作者只读 source.md 会以为这三个关键字不存在/被拒，词汇表不完整（commit 32789d4；已补入 docs/source.md:104-107 + docs/zh/source.md:105-108）
- [x] 审核发现：`docs/review/2026-11-01-code-review.md:50` 称回归测试写入「300 字节『爆』stderr」，实际测试为 `'爆' * 300`（300 个 CJK 字符 = 900 字节），把字符数误记为字节数；触发机制仍成立（字节 400 落在多字节字符内），但报告对测试载荷的描述不准确（commit 32789d4；已改为 `'爆' * 300` 即 900 字节）
- [x] 审核发现：回归测试 `test_script_failure_truncates_cjk_stderr_at_char_boundary` 以 `d.message.len() < 700` 间接验证截断，未精确断言 stderr 段 ≤400 字节且在字符边界；若修复后续退化把上限从 400 放宽到 500 字节（仍满足 <700）测试不会失败，400 字节截断契约缺少精确的回归断言（commit 32789d4；已改为精确统计尾随 `爆` 段并断言 ≤400 字节）
- [x] 审核发现：docs/source.md:105（docs/zh/source.md:106）新增的 `##unique`「Unique column」词汇行是过度承诺——`ColumnMeta.unique`（crates/cage-core/src/convention/mod.rs:126）被 apply() 写入（mod.rs:906）却从未被读取：derive.rs::field_schema()（crates/cage-core/src/convention/derive.rs:136-148）拷贝了 description/required/default/min/max/enum/reference/targets 但唯独没有 unique，`FieldSchema` 无逐字段唯一槽（唯一性仅在表级 unique_constraints，crates/cage-core/src/schema/mod.rs:144），declared_mismatch() 漂移检查（derive.rs:219-264）也不比较 unique，故 Excel 元行写 `##unique true` 被词汇门控接受（不报 E2201）后静默丢弃，不产生唯一约束、不参与漂移/校验；先前发现「实现已支持的三个关键字」仅凭 mod.rs:871-874 slot_is_set 均接受而成立，对 `##unique` 实为「接受但丢弃」（已修：`derive.rs::to_table_schema` 把 `##unique true` 映射为单字段表级 `unique_constraints`（`uniq_<field>`），`drift` 比较声明的唯一列集合；新增 `derive_unique_column_becomes_table_unique_constraint` + `drift_unique_mismatch_is_e2208`，cage-core convention 39 测试全绿）
- [x] 审核发现：docs/source.md:110（docs/zh/source.md:111）`##targets`「(empty = all)」与实现矛盾——parse_list_text()（crates/cage-core/src/convention/mod.rs:941-953）对空列表返回 E2202「is not a value list」，故 `##targets `（关键字后带尾随空格，own 解析为空串）会报错而非「全部 profile」；「empty = all」仅对完全省略该关键字成立（value 为 None、apply 不调用、targets 保持空 Vec），文档措辞会把「留空=全部」误导成「空值=全部」（已修：docs/source.md:110 + docs/zh/source.md:111 改为「省略该关键字 = 全部」）
- [x] 审核发现：TooLarge 映射接线无测试覆盖——`read_response` 的 `BodyReadError::TooLarge → FetchFailure::TooLarge`（crates/cage-core/src/remote.rs:365-366）与 `http_put_once` 的 `→ PutFailure::TooLarge`（remote.rs:537-539），以及全部消费端（cage-cli/src/remote.rs:120-124→E2101、registry.rs:1314-1316→E2101、sheets failure_brief lib.rs:281、http 源 lib.rs:101-103 兜底→E1901）均无任何测试；且这两函数直接取 `ureq::Response`（不可在单测构造）、`read_bounded` 调用处硬编码 `MAX_RESPONSE_BODY_BYTES`，无法用小上限测该映射，只能靠 ≥64 MiB 集成 fixture——新增单测仅覆盖 `read_bounded` 原语与 Display（remote.rs:748-796），TooLarge 从「检测」到「用户可见错误」的整条接线是盲区（已修：抽出纯函数 `map_fetch_body_err`/`map_put_body_err`（remote.rs:359-373），`read_response`/`http_put_once` 经其映射，新增 `body_err_maps_to_fetch_and_put_failures` 覆盖 TooLarge 与 Io→Transport 两路；消费端文案统一改用 cap 措辞，cli/registry/sheets 集成全绿）
- [x] 审核发现：`read_bounded`（crates/cage-core/src/remote.rs:345-356）超限时 `bytes.len()` 恒等于 `cap+1`（`take(cap+1)` 读满即停），故 `TooLarge(n)` 携带的 `n` 永远是 `MAX_RESPONSE_BODY_BYTES+1`（67108865），并非真实 body 大小——10 GB body 与 64 MiB+1 body 都报 "response body too large: 67108865 bytes"（remote.rs:105-107、501-503）；注释 "the observed length is carried" 的「observed length」实为已读长度而非 body 长度，消息会误导用户把恒定 cap+1 读成真实大小；huge 测试（remote.rs:772-776）显式断言 1024 字节 body 报 TooLarge(9)，说明行为有意但对外语义欠佳（已修：`read_bounded` 超限改传 `cap` 而非 `cap+1`，`FetchFailure::TooLarge`/`PutFailure::TooLarge` Display 与各消费端统一报「exceeds the N byte cap」，不再谎报真实 body 大小；`read_bounded_accepts_at_cap_and_refuses_over` 断言 `TooLarge(8)`）
- [x] 审核发现：`read_bounded_accepts_at_cap_and_refuses_over`（crates/cage-core/src/remote.rs:748-775）注释称 "fails fast (reads cap+1, not the whole stream)"，但断言仅验证返回 `TooLarge(9)`，未用计数字节 reader 验证实际只读 `cap+1` 字节；若未来实现退化为「先读整个流再判定」，失去 fail-fast 防御，该测试仍通过——「读取量受限于 cap+1」这一核心防护属性缺少断言（已修：新增 `read_bounded_refuses_over_cap_after_cap_plus_one_bytes`（remote.rs:787），用计数字节 reader 断言恰好消费 `cap+1` 字节）
- [x] 审核发现：commit 1adfd8c 修复了 P2-1（todo.md 已标 `[x]`「已修」），但未同步 `docs/review/2026-11-01-code-review.md`：该文 P2-1 条目（§52-63）仍为「建议…登记待办，不改码」，§⑤ 汇总（§113「已修 1，剩余 4（P2×2、P3×2）」）仍把 P2-1 计为未修，与 todo.md 状态不一致；若该文为冻结快照则无碍，否则两处追踪器漂移（不属实：`docs/review/2026-11-01-code-review.md` 是 9f7292f 一次成文、32789d4 仅修正 P1-1 措辞的批次审核快照，非 live 追踪器；P2-1 待办状态由 todo.md 权威记录，报告保留原文保持历史快照准确）
- [x] 审核发现：`field_type_depth`（crates/cage-core/src/schema/mod.rs:712）自身为无界递归——新护栏注释称「a maliciously deep schema must not overflow the stack」，但该函数沿 Array/Map/Object/TaggedUnion 递归求深时无深度上限、无提前退出，`Schema::validate()`（schema/mod.rs:607-619）须等整个递归返回后才在 :611 判 `depth > MAX_FIELD_TYPE_DEPTH` 报 E1004；故对嵌套深度超过栈容量的 FieldType（程序构造或深 YAML 幸存反序列化），溢出发生在 validate 的深度探测本身，先于 E1004 触发。护栏只保证 validate 之后的 normalize/generator 遍历有界（深≤64），未覆盖探测自身的栈溢出——探测应改为迭代或用深度上限短路（如超 MAX 即返回 Some(MAX+1)）（已修：`field_type_depth` 内部 `walk` 超过 `MAX_FIELD_TYPE_DEPTH` 即短路返回哨兵 `MAX+1`，不再随输入深度递归（schema/mod.rs:712-745）；新增 `schema_depth_probe_short_circuits_on_a_huge_type` 构造 200k 层嵌套验证探测不溢出且报 E1004（schema/mod.rs:1485）；`schema::` 20 定向测试全绿）
- [x] 审核发现：枚举单字段主键 `(key)` 路径无测试覆盖——`key_eligible`（crates/cage-target-flatbuffers/src/lib.rs:654）显式允许 `FieldType::Enum(_)`，注释（lib.rs:648-653）称「integer-backed 时是枚举、否则降级 string，两种映射均 key-valid」，但 `test_key_attribute_respects_flatc_type_eligibility`（lib.rs:895）只覆盖 float/bool/vector/string 四例，枚举分支零覆盖；「flatc 接受 enum 字段的 (key)」这一关键前提未经验证，若 flatc 实际拒绝，P2-2 的非法 .fbs 缺陷对枚举主键仍存在且无回归测试拦截（属实但行为本身正确：flatc `IsScalar` 对整型枚举 underlying type 成立（idl_parser.cpp:1168-1179、idl.h:134-135）；已在 `test_key_attribute_respects_flatc_type_eligibility` 补枚举分支：整型枚举 pk 保留 `(key)` 且 include 枚举文件（lib.rs:967-990））
- [x] 审核发现：唯一约束漂移检查会在 `##unique false` 声明下误报且文案误导——`declares_unique`（crates/cage-core/src/convention/derive.rs:292）对 `Some(false)` 也返回 true（parse_bool_text 接受 `##unique false`，convention/mod.rs:906），而 `to_table_schema` 只对 `== Some(true)` 产出约束；故仅显式声明 `##unique false` 的工作簿（derive.rs:401 进入唯一漂移分支）其 derived.unique_constraints 为空 `[]`，若 adopted 模式对工作簿从未声明唯一的列持有唯一约束，E2208 报「declares unique columns [] but the schema has [...]」——左端空集、且属误报（工作簿从未声称该列唯一）。该行为与既有 primary_key 漂移（declares_primary_key 亦用 is_some()，derive.rs:286）同构，属新逻辑对既有模式的复制，但空左端文案会误导（已修：`declares_primary_key`/`declares_unique` 改为仅 `== Some(true)` 视为声明，`##primary_key false`/`##unique false` 是作者显式放弃而非声明，不进入漂移分支（derive.rs:290-309）；新增 `drift_unique_false_is_not_a_declaration`/`drift_primary_key_false_is_not_a_declaration` 覆盖两侧 opt-out（convention/tests.rs:846,887）；`convention::` 41 定向测试全绿）

## 漏洞扫描遗留（2026-11-03 全舰队扫描，源 scratch/vuln-scan-*.md）

- [x] VULN-CAGE-001 [高危 9.1/7.5/7.3/—·RUSTSEC-2026-0369/0429/0367/0368] zip 8.6.0→9.0.0【跨大版本需改代码】：patched 仅 ≥9.0.0，涉及 zip API 变更需适配（calamine 链路引入）；验收：cargo audit 上述 4 个 ID 清零 + cargo test 绿（已修：calamine 0.36→0.38 使 zip 升至 9.3.0，Cargo.toml:60；cargo audit 4 个目标 ID 清零，cargo test --workspace 全绿）

- [x] 审核发现：`docs/source.md:100-109`（中文版 `docs/zh/source.md` 同）的 Excel 元行「Vocabulary」表漏掉实现已支持的 `##unique`、`##targets`、`##description` 三个关键字（`crates/cage-core/src/convention/mod.rs:871-874` 的 slot_is_set 分支均接受），表题为“Vocabulary”且 design §51 明示 `##description` 为注释行，作者只读 source.md 会以为这三个关键字不存在/被拒，词汇表不完整（docs/source.md:100-109；已补入 docs/source.md:104-107 + docs/zh/source.md:105-108）
- [x] 审核发现：`docs/review/2026-11-01-code-review.md:50` 称回归测试写入「300 字节『爆』stderr」，实际测试为 `'爆' * 300`（300 个 CJK 字符 = 900 字节），把字符数误记为字节数；触发机制仍成立（字节 400 落在多字节字符内），但报告对测试载荷的描述不准确（docs/review/2026-11-01-code-review.md:50；已改为 `'爆' * 300` 即 900 字节）
- [x] 审核发现：回归测试 `test_script_failure_truncates_cjk_stderr_at_char_boundary` 以 `d.message.len() < 700` 间接验证截断，未精确断言 stderr 段 ≤400 字节且在字符边界；若修复后续退化把上限从 400 放宽到 500 字节（仍满足 <700）测试不会失败，400 字节截断契约缺少精确的回归断言（crates/cage-source-excel/src/lib.rs:3239-3243；已改为精确统计尾随 `爆` 段并断言 ≤400 字节）

- [x] 审核发现：深度探测修复不完整——`schema_depth_probe_short_circuits_on_a_huge_type` 用 `std::mem::forget(schema)`（schema/mod.rs:1499）泄漏 20 万层嵌套类型以回避其递归 `Drop`（`FieldType::Array(Box<FieldType>)`，schema/mod.rs:318）栈溢出（已实测：debug 200000 层 + 2MiB 测试线程栈 → 栈溢出 exit 134）；护栏仅限住探测自身递归，未限住类型递归 `Drop`/serde 反序列化，真实恶意深 schema 在 validate 报 E1004 后仍会于释放时崩溃进程，测试仅因泄漏才通过，未验证「拒绝而非崩溃」覆盖类型完整生命周期（已修：`FieldType` 新增迭代式 `Drop`（crates/cage-core/src/schema/mod.rs:426）以显式 worklist 逐节点剥离容器脊柱，200k 层 schema 经 validate 报 E1004 后真实 `drop(schema)` 不再栈溢出；测试移除 `std::mem::forget` 改为真实 drop（crates/cage-core/src/schema/mod.rs:1548,1574）；cargo test -p cage-core 346 全绿、cargo test --workspace 全绿）
- [x] 审核发现：E1004 的 `found {depth}` 对深度 >65 的类型一律报哨兵 65（MAX_FIELD_TYPE_DEPTH+1）而非真实深度（schema/mod.rs:617），20 万层类型报「found 65」，误导排查；改动前未溢出深度报真实值，改动后探测短路使消息失真，应报真实深度或改为「超过上限」措辞（已修：E1004 消息改为 `exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}`，不再携带会误导的哨兵深度（crates/cage-core/src/schema/mod.rs:680）；测试断言消息不含「found 65」且点名上限（crates/cage-core/src/schema/mod.rs:1565-1571）；`field_type_depth` 返回值不再进入消息，仅作超限判定；cargo test -p cage-core 346 全绿）

- [x] 审核发现：E1004 消息去掉 `found {depth}` 后对恰超上限一层（深度 65）不再报真实深度，操作员无法区分「超一层」与「20 万层恶意」（已修：`field_type_depth` 返回 `DepthMeasure{depth,saturated}`，MAX+1/MAX+2 保留真实深度，MAX+3 起饱和省略数值；E1004 对非饱和超限报 `has nesting depth {N}`、饱和路径仅报上限；测试覆盖 MAX+1/MAX+2 真实深度与 200k 饱和无深度（crates/cage-core/src/schema/mod.rs:1291-1440,2098-2136）；cargo test -p cage-core --lib 348 全绿）
- [x] 审核发现：`field_type_depth` 返回饱和哨兵 MAX+1（65）而非真实深度，doc 注释仍称「The nesting depth of a FieldType」，实际契约已变为「是否超限」指示（已修：契约改为 `DepthMeasure{depth,saturated}`——`depth` 为真实深度、超限饱和为哨兵且 `saturated=true` 标记不可当真实深度；doc 注释与实现一致（crates/cage-core/src/schema/mod.rs:1284-1300）；cargo test -p cage-core --lib 348 全绿）
- [x] 审核发现：深度硬化不完整——`FieldType` 派生的 `Clone`/`PartialEq`/`Debug`/`Serialize` 对程序构造深类型无界递归（已修：`Clone`（schema/mod.rs:475-577）、`Debug`（schema/mod.rs:579-672）、`PartialEq`（schema/mod.rs:674-749）迭代实现，`Serialize` 对超限类型返回上限错误而非递归（schema/mod.rs:772-793）；新增 200k 层回归 `huge_field_type_survives_clone_eq_debug_and_serialize`（schema/mod.rs:2212-2234）；实测修复前 clone/eq/debug/serialize 200k 层均栈溢出 exit 134，修复后全绿；cargo test -p cage-core --lib 348 全绿）
- [x] 审核发现：显式 `##unique false` 或 `##primary_key false` 时跳过整个漂移检查，即使同一列在 adopted schema 中被声明为唯一列或主键也不报 E2208（crates/cage-core/src/convention/derive.rs:290） — 不属实：`##false` 是作者显式 opt-out 的既定设计语义（derive.rs:284-303 注释），仅跳过主键/唯一两个子检查，字段级存在性/形状漂移循环（derive.rs:462-481）仍执行，且 `convention/tests.rs:846,887` 已有两侧 opt-out 回归
- [x] 审核发现：为公开的 `FieldType` 实现 `Drop` 后，下游按值匹配并移出容器变体内容会因 Rust E0509 无法编译（已修：保留迭代式 Drop 以维持深度安全，新增 `FieldType::take` 作为整体移出迁移路径（schema/mod.rs:962-970）；新增回归 `field_type_take_is_the_migration_path_for_e0509` 固定 E0509 限制与 take 用法（schema/mod.rs:2236-2246）；cargo test -p cage-core --lib 348 全绿）

- [x] 审核发现：正文宽度规则的选择器优先级低于 VitePress 默认组件的 scoped 样式，带侧栏的正文仍被限制为 43rem，无侧栏页面也仍受默认 47rem/49rem 限制，未实现预期的拓宽（docs/.vitepress/theme/custom.css:162） — 不属实：HTTP 渲染探针（dist 构建产物按 base `/cage/` 起服后取 computed style）实测 `.VPDoc.has-aside .content-container` 与 `.VPDoc .container > .content` 均为 960px，`!important` 已压过默认 scoped 规则（先前误报源于 file:// 探针未加载 base 前缀的资产）（docs/.vitepress/theme/custom.css:157-164）
- [x] 审核发现：三条正文宽度规则（960px !important）与既有第 117-118 行 `.VPDoc.has-aside .content-container { max-width: 1100px }` 目的相同但取值冲突，960px 因 !important 生效使既有 1100px 规则永久失效成死代码，且其注释「放宽到 1100px」与实际渲染 960px 矛盾（docs/.vitepress/theme/custom.css:157-164） — 属实：已删除死规则并把注释改指下方 fleet-wide 960px 规则，复构建探针仍 960px（docs/.vitepress/theme/custom.css:115-117）
- [x] 审核发现：新增 `pre code` 白名单选择器冗余——`white-space`/`word-break` 可继承，默认主题对 `[class*='language-'] code` 未重置这两个属性，仅 `pre` 即可；且该注释标题 "unwrap long code lines" 与实际行为（wrap 换行）相反（docs/.vitepress/theme/custom.css:151-154） — 不属实（定性纠正）：选择器冗余成立但保留作防御（默认主题版本更迭可能对 code 显式重置）；注释标题实为 "wrap long code lines"，与 pre-wrap 行为一致，无矛盾（docs/.vitepress/theme/custom.css:151-154）
- [x] 审核发现：commit 信息 "unwrap long code lines"（放宽）与 CSS 实际行为 `white-space: pre-wrap`（收窄为换行）语义相反，会误导后续维护者（e325c82 commit message） — 不修：已推送的历史 commit 信息不改写，语义以本条登记为准
- [x] 审核发现：`.vp-doc div[class*='language-'] pre` 的 `pre-wrap` 在默认主题 `overflow-x: auto`（横向滚动）仍在的情况下移除了横向滚动的实际用途；宽表格靠横向滚动（168-171 行），宽代码却换行，两种宽内容策略不一致且未说明取舍（docs/.vitepress/theme/custom.css:151-156） — 属实（未说明取舍部分）：已在表格规则注释补明取舍——代码换行保逐行可读、表格滚动保列结构（docs/.vitepress/theme/custom.css:118-120）

- [x] 审核发现：`.vp-doc table { display:block; overflow-x:auto; }` 在 custom.css 存在两份完全相同的定义（120-123 与 164-167），第二份属冗余清理漏网（已修：删除 fleet-wide 块的重复表格规则，保留首个带取舍注释的规则（docs/.vitepress/theme/custom.css:118-123）；已删除原 164-167 重复块）
- [x] 审核发现：todo.md:373 新增条目称显式 `##unique false`/`##primary_key false` 时「跳过整个漂移检查」且行号仅引 `declares_primary_key`，表述双重不准 — 不属实：原条目已裁定为语义不成立并删除，纠错对象不存在；其指出的「仅门控主键/唯一子检查、字段级循环仍执行」已作为原条目不属实理由登记（crates/cage-core/src/convention/derive.rs:284-303,384,410,462-481）

- [x] 审核发现：新迭代式 `Debug` 与派生 `Debug` 输出不一致——`Object`/`TaggedUnion` 经 `fmt_map` 渲染为 `{ "a": ...}`（`{ ` 带前导空格、`}` 前无空格，schema/mod.rs:602,611），而派生 `IndexMap` 的 `debug_map` 输出为 `{"a": ...}`；空 map 渲染为 `{ }` 而非 `{}`；且 `{:#?}` 交替/美化标志被整体忽略（始终单行，schema/mod.rs:680 只 `write_str`），doc 注释「writes the same shape as the derive」与实测不符——任何依赖 `{:?}`/`{:#?}` 精确串的下游快照或诊断输出都会变化（实测：`Object({"a": Int32, "b": Array(String)})` 派生 vs `Object({ "a": Int32, "b": Array(String)})` 手动）（crates/cage-core/src/schema/mod.rs:595-672,680） — 属实：已改为按深度分派——普通类型用标准 `debug_tuple`/`debug_map` builder 精确复现派生 `{:?}`/`{:#?}`（含紧凑 `{"a": ...}`、空 map `{}`、pretty 交替缩进），仅超限类型走迭代 `fmt_deep`（有界栈）；新增回归 `debug_matches_the_derived_shape_for_compact_and_pretty`（crates/cage-core/src/schema/mod.rs:723,2430）
- [x] 审核发现：手动 `Serialize` 改为先构建 `serde_json::Value` 树再交给 serde，其 `Object`/`TaggedUnion` 键序现依赖 serde_json 的 `preserve_order` 特性——cage-core 自身未开启（`serde_json = { workspace = true }`，crates/cage-core/Cargo.toml:14，根 `serde_json = "1.0"` 亦无特性），当前仅因其它 workspace crate（cage-source-db/sheets、cage-target-json/jsonschema/template 各自显式 `preserve_order`）传递统一才生效；若 cage-core 独立编译或依赖图无该特性，`serde_json::Map` 退化为 BTreeMap，`TaggedUnion` 的 value 键序从派生的 `tag`→`branches` 变为字母序 `branches`→`tag`，wire 格式与修复前不一致（实测：无 preserve_order 时手动输出 `{"branches":...,"tag":...}` vs 派生 `{"tag":...,"branches":...}`）（crates/cage-core/src/schema/mod.rs:916-918,930-933） — 属实：cage-core 显式开启 `serde_json/preserve_order`（crates/cage-core/Cargo.toml:14），键序不再依赖其它 crate 的传递统一；新增回归 `serialize_object_and_union_keys_stay_in_declaration_order` 断言声明序 wire（crates/cage-core/src/schema/mod.rs:2483）
- [x] 审核发现：`SerializeFrame::FinishObject`/`FinishUnion` 对每个子 `serde_json::Value` 执行 `child.clone()` 深拷贝后立即丢弃 `children` Vec（schema/mod.rs:917,931），本可 move 而非 clone，属纯浪费且随对象宽度线性放大（crates/cage-core/src/schema/mod.rs:907-938） — 属实：`FinishObject`/`FinishUnion` 改用 `children.into_iter()` 逐值 move，不再深拷贝（crates/cage-core/src/schema/mod.rs:1088-1112）；由 `serialize_object_and_union_keys_stay_in_declaration_order` 覆盖
- [x] 审核发现：手动 `Serialize` 对 `field_type_depth > MAX`（深度 >64）的类型改为直接报错（schema/mod.rs:851-856），而派生 `Deserialize` 仍是递归无深度上限、仅受 serde 自身 ~128 递归限制（实测：深度 65 反序列化成功、深度 200 才报「recursion limit exceeded」），形成「能 from_str 却不能 to_string」的往返不对称——深度 65..~127 的类型可反序列化却被序列化拒绝（crates/cage-core/src/schema/mod.rs:834-857） — 属实：为 `FieldType` 手写有界 `Deserialize`（经 `FieldTypeDe` 忠实派生语义，`from_de` 强制 ≤`MAX_FIELD_TYPE_DEPTH`，超限报同一上限错误），`Serialize`/`Deserialize` 现对称（crates/cage-core/src/schema/mod.rs:415-505）；新增回归 `deserialize_refuses_a_type_deeper_than_the_cap`（crates/cage-core/src/schema/mod.rs:2522）
- [x] 审核发现：`FieldType::take` 的文档声称「先调用本方法，再按值 match 返回的 FieldType 或 clone 其子节点」，但 take() 返回的仍是实现了自定义 Drop 的 `FieldType`，按值 match 并移出子节点依旧触发 E0509（E0509 取决于类型是否实现 Drop，而非值来源），实际只能整体移走该值或 clone 子节点——文档对「按值 match 移出子节点」的指引不可达，属误导（crates/cage-core/src/schema/mod.rs:949-956） — 属实：`take` doc 改为「整体移走该值；返回的仍实现自定义 Drop，按值 match 移出子节点仍 E0509，应 clone 子节点或对已排空 spine 的 `&mut` 匹配」；既有 `field_type_take_is_the_migration_path_for_e0509` 固定 E0509 限制（crates/cage-core/src/schema/mod.rs:1131-1141）
- [x] 审核发现：`DepthMeasure` 结构体文档称深度「只测到超过 MAX 的第一层（即 MAX+1）就饱和并报告 MAX+1」，但代码实测对 MAX+1 与 MAX+2 层仍精确测量（内联注释 1292-1297 明确「MAX+1/MAX+2 仍精确」，饱和判定为 `ancestors > MAX+1`，即仅 MAX+3 及以上才饱和），结构体 doc 与内联注释矛盾（crates/cage-core/src/schema/mod.rs:1270-1276 vs 1292-1298） — 属实：`DepthMeasure` doc 改为「MAX+1/MAX+2 仍精确测量；仅 MAX+3 及以上饱和并报告 MAX+1」与内联注释一致（crates/cage-core/src/schema/mod.rs:1455-1463）
- [x] 审核发现：本轮复审 f7f96995..HEAD 未发现新增问题——既有 6 条开放发现均复现属实（Debug 空白/`{:#?}` 忽略 todo.md:385、Serialize 键序依赖 preserve_order :386、FinishObject/Union 子值 clone 浪费 :387、超限序列化拒绝致往返不对称 :388、take() 文档误导 :389、DepthMeasure 文档矛盾 :390）；另逐帧推演新迭代式 Clone/PartialEq/Eq（crates/cage-core/src/schema/mod.rs:475-770）正确无缺陷，E1004 消息分支与 custom.css 去重亦无问题 — 属实：上述 6 条均已修复并勾选；cargo test --workspace 全绿（cage-core lib 351 项）
- [x] 审核发现：深度上限在递归反序列化完整的 FieldTypeDe 之后才检查，通过 serde_json::from_value 传入 10000 层嵌套时会先栈溢出并终止进程，无法安全拒绝超限输入（crates/cage-core/src/schema/mod.rs:432） — 属实：反序列化改为在每层 `BoundedFieldTypeDe` 入口限深（`MAX_FIELD_TYPE_DEPTH`），超限即拒绝，不再先解析完整 `FieldTypeDe`；`from_str` 10000 层与借用 `Value` 10000 层均安全返回深度错误（tests/field_type_depth.rs 4/4 全绿）
- [ ] 审核发现：`serde_json::from_value` 按值消费 10000 层 `Value` 时，serde_json `Value` 的递归 Drop 仍会先爆栈，cage-core 无法在 `from_value` 内部拦截已消费 `Value` 的析构 — BLOCKED（复核 2026-11-10：机制属实、上游条件仍缺——crates.io 最新仍 1.0.154，serde-rs/json#440 自 2019-01-11 open 至今无动静；同时已盘仓内全部生产 `serde_json::from_value` 调用点，确认 cage 自有面无此暴露：target-* 模板过滤器只作用于 cage 校验过的 `FieldType` 序列化产物（深度 ≤ MAX_FIELD_TYPE_DEPTH=64，构造即有界）、convention 列元数据来自 Excel 单元格 JSON（浅层）、文本/字节进入只有 `from_str`/`from_slice`（serde_json 解析期 128 递归上限，产出的 Value ≤128 层 Drop 安全）、schema/mod.rs 与 cage-cli 的其余 `from_value` 均在 `#[cfg(test)]` 内——即 10000 层按值 `from_value` 只可能发生在 cage 之外的调用方自建 `Value` 场景，上游落地迭代式 Drop 前无 cage 侧可修点；仓内 `tests/field_type_depth.rs` 的 `drop_iteratively` 辅助正是测试侧绕开该递归析构的先例）
- [x] 审核发现：`value` 位于 `kind` 之前时，serde 在调用限深包装器前递归缓冲整个字段值，借用 10000 层 `serde_json::Value` 反序列化会栈溢出并终止进程，限深护栏可被合法键序绕过（crates/cage-core/src/schema/mod.rs:386） — 属实：实测复现——借用 10000 层 value-first `Value` 反序列化 SIGABRT 爆栈，kind-first 同深度则被安全拒绝（serde_derive 1.0.219 de.rs:1672 对 content-first 键序 `next_value::<Content>()` 无界缓冲，先于任何限深检查）。修复：为 `FieldType` 手写有界 adjacently-tagged 反序列化——content-first 先缓冲进深度封顶的 `BufContent`（`BUF_CONTENT_DEPTH_CAP` = MAX×8 = 512，crates/cage-core/src/schema/mod.rs:461），再按 serde `ContentDeserializer` 语义重放，两键序解析语义一致；tag-first 直接派发且逐层过 `BoundedFieldTypeDe` 限深（`FieldTypeDeVisitor` mod.rs:1291、`dispatch_variant` mod.rs:1112）。unknown variant 错误改为在读取 `kind` 值时即产生（`VariantNameSeed` mod.rs:1204），与 serde 派生一致地把错误归到 `.type.kind`（serde_path_to_error 路径归属，edit::tests::e1701 仍绿）。新增回归 5 项（crates/cage-core/tests/field_type_depth.rs:103-165：10000 层借用拒绝、10000 层文本拒绝、MAX+1 拒绝、容量处两键序等价+往返、各容器族两键序等价）；cage-core 385 lib + 9 集成全绿，clippy -D warnings 与 fmt 干净

## 源码分层分析 2026-11-07

### 抽象层清单

- [x] APP-001 应用接口层：把 CLI/HTTP/MCP/LSP 请求分发给共享编译管线，不重写校验或生成逻辑（`crates/cage-cli/src/main.rs:575`） — 属实（已满足，无需改动）：校验唯一入口 `validate_project`（crates/cage-cli/src/main.rs:1304，内部调 cage_core::validation::validate_with_profile）被四端共用——CLI check（main.rs:1333）、HTTP（web.rs:733、web.rs:1264）、MCP（mcp.rs:278、mcp.rs:422）、LSP（lsp.rs:110）；生成唯一入口 `generate_target_items`（main.rs:3727）被 CLI build（main.rs:2452）与 MCP（mcp.rs:465）共用；web.rs/lsp.rs 不含生成路径，`validate_with_profile` 在 cage-cli 内仅 main.rs 三处（1319、2226、3557，均为共享管线内部），无一端重写校验或生成逻辑
- [x] APP-002 工程装载层：解析 `cage.toml`、schema、本地/远程源与 registry pin，产出编译输入（`crates/cage-cli/src/main.rs:787`） — 属实（已满足，无需改动）：装载唯一入口 `load_project`→`load_project_impl`（crates/cage-cli/src/main.rs:784/793），产出 `Project{config, schema, document}` 编译输入；`cage.toml/cage.yaml/cage.yml/cage.json` 解析唯一在 `load_project_config`（main.rs:883），schema 装载唯一在 `load_schema_for_config`（main.rs:997，schema_path 也可指 registry pin——`load_schema_from_entry` main.rs:1003）；本地/远程源分派唯一在 `load_project_impl` 内（main.rs:806-836）：`registry:` → `registry_entry`（main.rs:938，ledger 校验 + 显式 `@version` 须满足 `[dependencies]` pin，E1802）、`mysql:`/`pg:` → `DbSourceAdapter::load`、`gsheet:` → `SheetsSourceAdapter::load`、http(s) → `HttpSourceAdapter::load`、其余本地 → `load_sources_with_meta`（main.rs:1020）——这些装配件在生产代码中无第二调用点（仅测试引用）；消费端全部走此入口：CLI check main.rs:1312、build 2196、migrate 1405（fill=false 原始源）、gen 3522/3837、MCP mcp.rs:308/337/371/441、web web.rs:795/1010（env 覆盖走 `project_with_env`，不重解析源）、LSP 无自有装载器（经 `validate_project` 共享管线）
- [x] APP-003 核心模型层：用格式无关的 `Schema`、`Value`、`Document` 承载结构化配置与位置元数据（`crates/cage-core/src/schema/mod.rs:9`、`crates/cage-core/src/value/mod.rs:134`）。已完善：`Value::from_json`/`Value::to_json` 格式无关 JSON 通道（`crates/cage-core/src/value/mod.rs:270`、`crates/cage-core/src/value/mod.rs:305`，Bytes→base64、非有限浮点→null 与 JSON target 约定一致）；`Schema::get_enum` 共享枚举查找（`crates/cage-core/src/schema/mod.rs:2370`）；修复 `Document::add_table` 重复登记 source_file（`crates/cage-core/src/value/mod.rs:460`，此前 source-json/source-csv 文档里同一文件出现两次）；新增 `Document::table_names`（`crates/cage-core/src/value/mod.rs:468`）。
- [x] APP-004 归一化层：把源数据转换成 schema 声明的类型视图，处理默认值和单元格数组（`crates/cage-core/src/normalize/mod.rs:116`） — 属实（已满足，无需改动）：`normalize_document`/`normalize_value`/`normalize_typed_value`（normalize/mod.rs:69/9/60）做类型收敛，`split_cell_arrays` 拆单元格数组、`fill_field_defaults`（normalize/mod.rs:116，回流-1：缺失键与 Excel 空单元格同样落 schema 默认值）；三件套经 cage-cli 唯一装载入口消费（main.rs:872-873 在 `load_project_impl` 内先拆数组再填默认），migrate/gen 的重归一化走 `normalize_document`（main.rs:2240、3576），无旁路实现
- [x] APP-005 校验层：按 L0-L7 执行结构、类型、约束、引用、语义和游戏规则检查（`crates/cage-core/src/validation/mod.rs:168`） — 属实（已满足，无需改动）：validation/mod.rs 首行即「L0 through L7 validation levels」，`ValidationLevel` 从 L0 Parse 到 L7 Game rule plugins（mod.rs:23-37，L7 委托 rules 插件 mod.rs:1400），唯一入口 `validate`（mod.rs:157）/`validate_with_profile`（mod.rs:168）被四端共用（CLI check main.rs:1333、HTTP web.rs:733/1264、MCP mcp.rs:278/422、LSP lsp.rs:110，见 APP-001 证据），无一端自写校验
- [x] APP-006 构建与分发层：哈希/增量重建、写 manifest、打包自校验快照、发布 registry 和远端 PUT（`crates/cage-cli/src/main.rs:2184`、`crates/cage-core/src/registry.rs:528`） — 属实（已满足，无需改动）：`build_project`（main.rs:2184）以 `ManifestGenerator::input_hashes`（schema+document+source_extra）做增量第 1 层跳过（main.rs:2245 起），产物写 manifest；打包链强制 `snapshot::verify_snapshot` 自校验（registry.rs:549 拉取校验、971 staging 校验、871 注释明示 ledger 必过 verify_snapshot）；`registry::publish`（registry.rs:528，zstd bundle 字节确定性有测试）；远端 PUT 走 presigned/index_put（registry.rs:1193 起）
- [x] APP-007 目标生成层：把 schema/document 分发到数据、代码绑定、契约和模板生成器（`crates/cage-cli/src/main.rs:3727`） — 属实（已满足，无需改动）：唯一分发入口 `generate_target_items`（main.rs:3727，gen_schema/gen_doc 入参）——先 `code_target_items`（main.rs:3640s：cs/py/lua/ts/cpp/go/java 代码绑定、proto/flatbuffers 契约、jsonschema、template 模板引擎含按语言 `register_filters`），未命中再落数据格式 json/csv/msgpack（main.rs:3738-3746）；build 与 MCP 共用此入口（APP-001 证据）
- [x] APP-008 源接入层：把文件、数据库行集、HTTP payload 和 Sheets 行集解析成 canonical document（`crates/cage-source-excel/src/lib.rs:54`） — 属实（已满足，无需改动）：文件走 `ExcelSourceAdapter`（source-excel lib.rs:54），db 行集经 `canonical_json`（source-db lib.rs:218）落「canonical JSON → cache file → standard JSON parse」共享路径（source-db lib.rs:242，sheets/http 同路——`load_project_impl` 内四源统一分派，APP-002 证据），各 adapter 产出同构 `Document`，无格式私有旁路
- [x] APP-009 治理与诊断层：资产基线、声明式迁移/schema diff 和统一诊断模型跨接口复用（`crates/cage-core/src/assets/mod.rs:86`、`crates/cage-core/src/diagnostics/mod.rs:13`） — 属实（已满足，无需改动）：`assets::inventory`（assets/mod.rs:86，fail-closed 基线：不可读文件即错误）、声明式迁移与 schema diff 在 migrate/{mod,diff}.rs（step 链 + `schema_default`）；统一 `Diagnostic{code,severity,...}`（diagnostics/mod.rs:13）与 `Diagnostics` 被 web.rs:42 引入、mcp/lsp 共用（跨接口复用 8+ 处引用），CLI 渲染与 exit code 同源

### 超大文件候选

- [x] LAY-001 [P1] 拆分 `crates/cage-cli/src/main.rs`（7572 行）——CLI 定义、项目装载、校验、构建、快照、注册表、迁移、目标分发和测试挤在同一入口（`crates/cage-cli/src/main.rs:83`、`crates/cage-cli/src/main.rs:2184`）。已拆出 6 个模块：loading（Project+装载层）、migrate（迁移+草稿+CSV 重写）、build（构建+快照）、registry（本地注册表 CLI）、gen（codegen+manifest）、inspect（inspect/diff/graph/assets/verify），commit 0d6ee4f 5db27a6 887b4d1 7215e80 6811ce7 b6bb035；main.rs 7717→4131 行（其余为 clap 定义、main 分发、run_check 与 ~2700 行单测）。每步全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-002 [P1] 拆分 `crates/cage-core/src/registry.rs`（4208 行）——包模型、版本语义、发布/解析、bundle/签名和 push 路由集中在一个文件（`crates/cage-core/src/registry.rs:39`、`crates/cage-core/src/registry.rs:528`）。已转为目录模块 registry/{mod.rs,model.rs,version.rs,store.rs,push.rs,verify.rs,signing.rs,tests.rs}，commit 82ea1c0；公开 API 经 mod.rs `pub use` 原样再导出（cage-cli 等外部调用路径不变），私有助手 `pub(crate) use` 桥接。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-003 [P1] 拆分 `crates/cage-core/src/validation/mod.rs`（4105 行）——级别枚举、八个校验器、共享规则和测试同文件（`crates/cage-core/src/validation/mod.rs:22`、`crates/cage-core/src/validation/mod.rs:157`）。已按验证级别拆出 7 个 pass 模块（l0_parse/l1_schema/l2_types/l3_values/l4_tables/l5_refs/l6_semantic）+ tests.rs，mod.rs 仅剩 246 行（级别枚举、上下文、入口、分发），commit 1d5c648；公开 API `pub use` 原样再导出，校验器间共享 `pub(crate) use` 桥接。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-004 [P1] 拆分 `crates/cage-cli/src/web.rs`（4613 行）——HTTP 循环、路由、安全边界、API handler、嵌入 UI 和测试集中（`crates/cage-cli/src/web.rs:61`、`crates/cage-cli/src/web.rs:49`）。已转为目录模块 web/{mod.rs,handlers.rs,tests.rs}，commit a580cb8：mod.rs 留 HTTP 循环、路由表、响应助手与本地安全边界（456 行），handlers.rs 收 14 个 api_* handler 与上传/表格助手（1454 行），tests.rs 收全部集成测试；include_str 嵌入路径随目录深度修正为 `../../../../docs/...`，`mod tests` 加 `#[cfg(test)]` 使 dev-deps 不进普通 bin 构建。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-005 [P1] 拆分 `crates/cage-source-excel/src/lib.rs`（3491 行）——工作簿打开、meta 约定、单元格/类型转换和适配器在同一 lib（`crates/cage-source-excel/src/lib.rs:54`、`crates/cage-source-excel/src/lib.rs:95`）。已拆为 lib.rs（适配器结构体+Default+SourceAdapter trait/impl，117 行）+ meta.rs（工作簿打开与 meta 约定流水线，966 行）+ cells.rs（网格/单元格转换助手，264 行）+ tests.rs（单测，2202 行），commit 38f798a；跨模块助手 `pub(crate)`，HeaderGroup 提升为 `pub(crate)`。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-006 [P1] 拆分 `crates/cage-core/src/schema/mod.rs`（5132 行）——schema/table/field 模型、解析、定界和测试同文件（`crates/cage-core/src/schema/mod.rs:9`、`crates/cage-core/src/schema/mod.rs:133`）。已拆为 mod.rs（模型类型+Schema::validate，547 行）+ field_type.rs（FieldType 枚举、固有 impl、wire 序列化与深度测量，1003 行）+ field_type_serde.rs（FieldTypeDe 镜像与有界缓冲反序列化+白盒回放测试，1643 行）+ tests.rs（主单测，1961 行），commit 3678ff0；公开 API 经 `pub use` 原样再导出，field_type_depth/DepthMeasure `pub(crate)` 供校验层复用。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-007 [P1] 拆分 `crates/cage-core/src/migrate/mod.rs`（2275 行）——spec/step 模型、解析校验、执行器、报告回验和测试集中（`crates/cage-core/src/migrate/mod.rs:35`、`crates/cage-core/src/migrate/mod.rs:513`）。已拆为 mod.rs（MigrationSpec/Step 模型与 Deserialize，313 行）+ parse.rs（parse_spec/parse_migration_dir/validate_spec/validate_step，157 行）+ apply.rs（apply/报告/widen 助手/reverify，440 行）+ tests.rs（单测，1389 行），commit 301daaf；公开 API 经 `pub use` 原样再导出，diff.rs 改引 apply::is_widening。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-008 [P2] 拆分 `crates/cage-target-java/src/lib.rs`（2191 行）——Java 类型映射、成员生成、保留字策略、模板填充和测试集中（`crates/cage-target-java/src/lib.rs:71`）。已拆为 lib.rs（生成器结构体+表格/枚举渲染 impl，511 行）+ types.rs（FieldType→Java 类型映射，125 行）+ literals.rs（默认值/字面量渲染+字段 Javadoc，319 行）+ idents.rs（保留字表与标识符清洗，128 行）+ filters.rs（Tera 过滤器与 register_filters，99 行）+ tests.rs（单测，1044 行），commit a51cba9；跨模块助手 `pub(crate)`，register_filters 经 `pub use` 保持公开路径。门禁：全仓 75 套件全绿 + clippy -D warnings + fmt 过。
- [x] LAY-009 [P2] 拆分 `crates/cage-target-go/src/lib.rs`（2164 行）——Go 类型/默认值/包名/导出规则与生成器测试集中（`crates/cage-target-go/src/lib.rs:71`）。
  - 2026-11-10 落地：拆为 docfmt/idents/types/literals/filters 五个模块 + 扁平化 tests.rs，lib.rs 剩 ~549 行；crate 根 `pub use`/`pub(crate) use` 保外部路径不变。fmt+clippy(-D warnings)+75 套件全绿（go 30 tests: 29 passed, 1 ignored）。commit 7a97e07。
- [x] LAY-010 [P2] 拆分 `crates/cage-target-ts/src/lib.rs`（2114 行）——TS/JS 语法、类型映射、运行时形态与生成器测试集中（`crates/cage-target-ts/src/lib.rs:82`）。
  - 2026-11-10 落地：拆为 docfmt/idents/types/literals/plan/filters 六个模块 + 扁平化 tests.rs，lib.rs 剩 ~314 行；crate 根 `pub use register_filters` 保外部路径不变。fmt+clippy(-D warnings)+全量套件全绿。commit a68ee56。
- [x] LAY-011 [P2] 拆分 `crates/cage-target-cpp/src/lib.rs`（2041 行）——C++ 类型映射、命名、头文件渲染和测试集中（`crates/cage-target-cpp/src/lib.rs:69`）。
  - 2026-11-10 落地：拆为 idents/types/literals/filters 四个模块 + 扁平化 tests.rs，lib.rs 剩 ~417 行；crate 根 `pub use register_filters` 保外部路径不变。fmt+clippy(-D warnings)+75 套件全绿。commit 36a59e4。
- [x] LAY-012 [P2] 拆分 `crates/cage-core/src/validation/rules.rs`（1738 行）——声明式游戏规则实现与规则测试挤进校验 crate 的 rules 文件（`crates/cage-core/src/validation/rules.rs`）。
  - 2026-11-10 落地：rules.rs 拆出 `rules/declarative.rs`（五个 run_* + 共享 helpers，~492 行）+ 扁平化 `rules/tests.rs`，rules.rs 剩 ~350 行；`run_rule` 留在 rules.rs 做分发。fmt+clippy(-D warnings)+75 套件全绿。commit 3cb035e。
- [x] LAY-013 [P2] 拆分 `crates/cage-core/src/manifest/mod.rs`（1609 行）——BuildManifest、增量记录、cage.toml schema 和生成器在一个文件（`crates/cage-core/src/manifest/mod.rs:9`、`crates/cage-core/src/manifest/mod.rs:523`）。
  - 2026-11-10 落地：拆出 `manifest/config.rs`（BuildProfile/TargetConfig/ProjectConfig/SourceConfig/RegistryConfig/RemoteSourceConfig/ProjectInfo + Default，~208 行）+ 扁平化 `manifest/tests.rs`；mod.rs 剩 ~500 行，crate 根 `pub use config::{...}` 保外部路径不变。fmt+clippy(-D warnings)+75 套件全绿。commit dbab666。
- [x] LAY-014 [P2] 拆分 `crates/cage-source-sheets/src/lib.rs`（1540 行）——Sheets API 调用、分页、值转换、约定和错误集中（`crates/cage-source-sheets/src/lib.rs:298`）。
  - 2026-11-10 落地：拆出 `rows.rs`（网格/元数据/规范转换，~317 行）+ 扁平化 `tests.rs`（含原 1540-1641 全部用例）；lib.rs 剩 ~389 行。fmt+clippy(-D warnings)+75 套件全绿。commit a03d508。
- [x] LAY-015 [P2] 拆分 `crates/cage-core/src/normalize/mod.rs`（1459 行）——递归规范、类型收敛、默认值/数组拆分与测试集中（`crates/cage-core/src/normalize/mod.rs:116`）。拆出 `normalize/fields.rs`（286 行：fill_field_defaults / split_cell_arrays / coerce_to_type，`pub use` 保持原路径）+ `normalize/tests.rs`（1073 行）；mod.rs 剩 107 行。clippy -D warnings 绿，75 套件全绿，commit 2efdef0。
- [x] LAY-016 [P2] 拆分 `crates/cage-core/src/convention/tests.rs`（1438 行）——Excel/Sheets 约定行为测试集中成单测试文件（`crates/cage-core/src/convention/tests.rs`）。实际 2084 行，拆成 `tests/grammar.rs`（629：解码/词汇/A1/cell 文本）+ `tests/derive.rs`（1081：derive/drift E2208，沿用原 X2 分节）+ `tests/script.rs`（358：脚本指纹/响应门/python3 运行器），共享 `grid`/`meta_of` 留在 tests.rs。clippy -D warnings 绿，75 套件全绿，commit 3b6737a。
- [x] LAY-017 [P2] 拆分 `crates/cage-core/src/convention/mod.rs`（1445 行）——meta 解码、类型文本、脚本执行和响应协议集中（`crates/cage-core/src/convention/mod.rs:165`、`crates/cage-core/src/convention/mod.rs:1144`）。拆出 `meta.rs`（536：convention_territory/decode/apply）+ `types.rs`（151：parse_*_text 文本解析）+ `script.rs`（370：script_fingerprint/resolve_script/run_script/parse_response），mod.rs 剩 412（约定配置/网格原语/A1），`pub use` 保持原路径。clippy -D warnings 绿，75 套件全绿，commit 2309f7e。
- [x] LAY-018 [P3] 拆分 `crates/cage-cli/tests/cli_paths.rs`（1385 行）——CLI 路径/安全场景共享常量与多组集成测试集中（`crates/cage-cli/tests/cli_paths.rs:37`）。改为 `tests/cli_paths/main.rs`（88：共享 run_cage/常量/write_project）+ 按 CLI 子命令分模块 check（281）/build（329）/gen（101）/inspect（152）/diff（195）/scenario（242，e9006/snapshot/msgpack 源）。49 个测试数不变全绿，clippy -D warnings 绿，75 套件全绿。
- [x] LAY-019 [P2] 拆分 `crates/cage-target-cs/src/lib.rs`（1355 行）——C# 类型、默认值、命名与生成器测试集中（`crates/cage-target-cs/src/lib.rs:53`）。拆出 `idents.rs`（126：CS_KEYWORDS/标识符）+ `types.rs`（59：cs_type_inner/cs_map_key_type/cs_type）+ `literals.rs`（194：默认值/字符串与浮点字面量）+ `docfmt.rs`（58：xml_escape/field_doc）+ `filters.rs`（64：cs_type/cs_default 过滤器）+ `tests.rs`（588）；lib.rs 剩 307，`pub use` 保持原路径。clippy -D warnings 绿，75 套件全绿。
- [x] LAY-020 [P2] 拆分 `crates/cage-target-py/src/lib.rs`（1321 行）——Python 类型、默认值、过滤器与生成器测试集中（`crates/cage-target-py/src/lib.rs:56`）。拆出 `idents.rs`（PY_KEYWORDS/标识符）+ `types.rs`（py_type/map_key_annotation/annotation_*）+ `literals.rs`（默认值/数组与映射字面量/字符串浮点）+ `docfmt.rs`（field_doc）+ `filters.rs`（py_type/py_default）+ `tests.rs`；lib.rs 剩 ~350，`pub use` 保持原路径。clippy -D warnings 绿，75 套件全绿。
- [x] LAY-021 [P2] 拆分 `crates/cage-source-http/src/lib.rs`（1302 行）——分页、重试、内容类型、缓存 JSON 与测试集中（`crates/cage-source-http/src/lib.rs:45`、`crates/cage-source-http/src/lib.rs:133`）。拆出 `paging.rs`（187：MAX_PAGES/PageFailure/PageFetch/条件 GET 分页游走/Link next 解析）+ `meta.rs`（60：validators sidecar 读写）+ `tests.rs`（928）；lib.rs 剩 148（HttpSourceAdapter 管线）。clippy -D warnings 绿，75 套件全绿。
- [x] LAY-022 [P2] 拆分 `crates/cage-target-lua/src/lib.rs`（1295 行）——Lua 类型/默认值/转义和生成器测试集中（`crates/cage-target-lua/src/lib.rs:59`）。（完成：lib 259 + idents/types/literals/docfmt/filters + tests 695；9671956）
- [x] LAY-023 [P3] 拆分 `crates/cage-cli/tests/registry.rs`（1265 行）——注册表集成场景、辅助 fixture 与多个 push/import 用例集中（`crates/cage-cli/tests/registry.rs:61`）。（完成：tests/registry/{main,publish,resolve,export,import}.rs；916a432）
- [x] LAY-024 [P2] 拆分 `crates/cage-core/src/reference/mod.rs`（1247 行）——引用解析、依赖图、增量规划和测试集中（`crates/cage-core/src/reference/mod.rs:9`、`crates/cage-core/src/reference/mod.rs:240`）。（完成：mod 242 + graph 244 + tests 775；8c77d2a）
- [x] LAY-025 [P2] 拆分 `crates/cage-core/src/migrate/diff.rs`（实为 1274 行，todo 记 1220）——schema diff、规则起草和测试集中（`crates/cage-core/src/migrate/diff.rs:88`、`crates/cage-core/src/migrate/diff.rs:110`）。（完成：diff 218 + diff/{draft,values,render,tests} 191+166+82+647；98870db）
- [x] LAY-026 [P2] 拆分 `crates/cage-cli/src/mcp.rs`（实为 1615 行）——工具 schema、执行体、错误解释和测试集中（`crates/cage-cli/src/mcp.rs:135`、`crates/cage-cli/src/mcp.rs:270`）。（完成：mcp 174 + mcp/{params,tools,views,tests} 89+312+369+711；2f69c42）
- [x] LAY-027 [P3] 拆分 `docs/public/editor/app.js`（实为 1156 行）——schema 状态、渲染、校验、构建/产物/快照/注册表 UI 事件集中（`docs/public/editor/app.js:37`）。（完成：app 221 入口 + constants/state/dom/api/types/forms 27+18+184+52+180+513，web/mod.rs 增路由，web.rs 断言跟模块走；28e6a55）
- [x] LAY-028 [P2] 拆分 `crates/cage-cli/src/init.rs`（实为 1432 行）——交互问答、项目 scaffold 和配置生成集中（`crates/cage-cli/src/init.rs`）。（完成：init 340 + init/{dialog,scaffold,tests} 365+192+556；0c02729）
- [x] LAY-029 [P3] 拆分 `crates/cage-cli/tests/web.rs`（实为 1127 行）——Web API 集成 fixture、HTTP 驱动和多面用例集中（`crates/cage-cli/tests/web.rs:174`）。（完成：tests/web/{main,editor,status,data,snapshots}.rs 255+406+118+240+125；ec3a703）
- [x] LAY-030 [P3] 拆分 `crates/cage-target-flatbuffers/src/lib.rs`（1100 行）——fbs 类型/服务声明、模板渲染和测试集中（`crates/cage-target-flatbuffers/src/lib.rs:145`）。lib.rs 74 + render.rs 679 + tests.rs 492（commit 7770f5f，75 suites / 1249 tests 绿）。
- [x] LAY-031 [P3] 拆分 `crates/cage-target-proto/src/lib.rs`（1015 行）——proto 类型映射、消息渲染和测试集中（`crates/cage-target-proto/src/lib.rs:128`）。lib.rs 71 + render.rs 592 + tests.rs 365（commit c4f21d2，75 suites / 1249 tests 绿）。

### 目录/分类不合理处

- [x] CAT-001 `crates/cage-cli/src/main.rs` 中的 `load_project_impl`、`build_project`、`generate_target_items` 和 registry CLI 流程属于可独立编译职责，但与 clap 定义混在 bin crate 入口（`crates/cage-cli/src/main.rs:787`、`crates/cage-cli/src/main.rs:2184`）。已随 LAY-001 拆出：load_project_impl→loading.rs:77、build_project→build.rs:97、generate_target_items→gen.rs:229、run_registry_push→registry.rs:479（commit 0d6ee4f 5db27a6 887b4d1 7215e80 6811ce7 b6bb035）；main.rs 4131 行仅剩 clap 定义/分发/单测。
- [x] CAT-002 `crates/cage-core/src/registry.rs` 兼有本地文件 registry、bundle 传输、签名校验和 HTTP push；bundle/push 是分发传输语义，registry 是包解析语义（`crates/cage-core/src/registry.rs:777`、`crates/cage-core/src/registry.rs:1078`）。已随 LAY-002 拆为 registry/{mod,model,version,store,push,verify,signing,tests}.rs：push.rs 独立收 push_entry:45/push_entry_presigned:179（commit 82ea1c0）。
- [x] CAT-003 `crates/cage-core/src/manifest/mod.rs` 同时定义运行账本和 `ProjectConfig`/`TargetConfig` 配置 schema；两者的变更周期与消费者不同（`crates/cage-core/src/manifest/mod.rs:9`、`crates/cage-core/src/manifest/mod.rs:523`）。已随 LAY-013 拆出 config.rs：BuildProfile:9/TargetConfig:24/ProjectConfig:39（commit dbab666）；mod.rs 499 行仅剩账本/生成器。
- [x] CAT-004 `crates/cage-core/src/convention/tests.rs` 与 `convention/mod.rs` 都是超大文件；生产逻辑和跨格式约定测试可按 Excel/Sheets 场景分组（`crates/cage-core/src/convention/mod.rs:165`、`crates/cage-core/src/convention/tests.rs`）。已随 LAY-016/017 拆出 tests/{grammar,derive,script}.rs（629+1081+358，commit 3b6737a）与 mod.rs 剩 412 + meta.rs/types.rs/script.rs（commit 2309f7e）。
- [x] CAT-005 `docs/public/editor/app.js` 同时承担文档站示例资源和 CLI 编译期内嵌编辑器 UI，状态/视图/API 更新缺少源码级模块边界（`docs/public/editor/app.js:37`、`crates/cage-cli/src/web.rs:49`）。已随 LAY-027 拆出 editor/{constants,state,dom,api,types,forms}.js 27+18+184+52+180+513，web/mod.rs 增 EDITOR_*_JS include_str 路由（commit 28e6a55）。

- [x] 审核发现：序列形式的单元类型会忽略并接受非法内容，例如 `["Int32",true]` 现在被解析为 Int32，而原派生解析器会拒绝（crates/cage-core/src/schema/mod.rs:1366） — 属实：独立探针复现派生原语义——`["Int32"]`→invalid length 1（内容元素必需）、`["Int32",null]`→OK、`["Int32",true]`/`["Int32",1]`→invalid type、`[]`→invalid length 0、`["Enum"]`→invalid length 1、`["Int32",null,1]`→trailing characters；修复 `FieldTypeDeVisitor::visit_seq`（schema/mod.rs:1357）与派生逐一对齐——tag 缺失改报 `invalid_length(0)`、单元变体内容元素改 `next_element::<()>()`（仅收 unit 形状，`["Int32",true]` 拒绝）、内容缺失报 `invalid_length(1)`、多余元素不再自检（交由格式层报 trailing characters，与派生一致）；回归更新 `tests/field_type_depth.rs::seq_form_matches_the_map_form`（含新增 `["Int32",true]`/`["Int32",1]` 拒绝断言）与 `non_unit_variant_without_value_is_refused`；cage-core 400 lib + 14 集成全绿，workspace clippy -D warnings 0 错、fmt 干净（顺带修绿 cb49999 遗留的 3 处 clippy lint）

- [x] 覆盖率实测与缺口补齐（work-runner 任务）：workspace 行覆盖 95.70%→95.81%（missed lines 1367→1334）、region 94.73%→94.85%（2741→2681）、function 91.02%→91.06%（251→250）——纯测试新增 9 个行为测试、0 行生产代码改动：`normalize/fields.rs` 86.40%→99.20%（对象→TaggedUnion 逐分支强制转换 + split_cell_arrays 未知分支/非数组带分隔符/缺席与空单元格/外部表守卫）、`validation/expr.rs` 94.60%→96.37%（尾随空白词法、空反引号引用、未终止字符串字面量、rhs 后多余 field/literal/operator 的定位报错）、`assets/mod.rs` 91.78%→97.95%（AssetClass Display 三标签、无项目配置 fail-closed、sources 内 symlink 触发 unsupported file type）、`convention/script.rs` 91.46%→92.53%（stderr>400 字节按字符边界截断，CJK 不 panic）；`cage-source-db/pg.rs` 41.72% 与 `mysql.rs` 52.15% 为活动 DB 连接路径（外部 I/O），如实记录不硬凑；cargo test --workspace 全绿、clippy -D warnings 0 错、fmt 干净（commit 1e4ac4c）

- [x] 审核发现：本轮 diff 未见问题（cb49999a..HEAD）— 属实，复核通过：77 个提交 = 74 个 `refactor:`/`style:` 模块拆分 + 2 个行为改动（276f6e9 序列字段类型、467a413 Windows 端口性）+ 1 个 rustfmt 重排（a69e399）。逐提交对 *.rs 做行多重集比对（剔除 `mod`/`use` 声明、纯括号行与注释），74 个拆分提交**零非管道漂移**，即纯移动不改语义；有改动的仅 3 个非拆分提交（1e4ac4c 加测试、467a413 平台门控、a69e399 导入重排），均已单独审过。467a413 的 `MAIN_SEPARATOR`→`/` 归一化**覆盖面完整**：cpp/cs/go/lua/py/ts 六个代码生成目标不自建产物键，全部委托 `cage-target-template` 的 `join_output`（crates/cage-target-template/src/lib.rs:224-229，内含 `.replace(MAIN_SEPARATOR,"/")`），14 个 target crate 的 artifact key 均经归一化路径构造（json:159/223、csv:217、java:279/289、jsonschema:170、msgpack:162、flatbuffers/render.rs:185、proto/render.rs:171），不存在 Windows 下 `/` 与 `\` 混用的键。gate：fmt 干净、clippy -D warnings 0 错、75 套件 0 失败（1257 passed）。
