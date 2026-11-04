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
