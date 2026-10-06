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
- [x] Registry（远程配置仓库 + 版本）——R1–R4 收官（2026-10，见下方
      「R 系列：Configuration Registry」：本地多版本仓库 / `[dependencies]`
      版本 pin / 远程 http(s) 只读解析 / verify 全册审计 + gc 滚动窗口 +
      remove 显式移除）；遗留的注册表鉴权与远程发布协议不在本项
- [ ] Remote Source（Google Sheets/MySQL/PostgreSQL/HTTP API）——已立项
      （2026-10），设计定稿 design §45，实现拆解见下方「S 系列」
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
- [x] R3 远程 Registry 只读解析：`[registry].path` 支持 http(s) 根；
      协议为匿名 GET 三资源（包 index / 条目账本 / 条目文件，条目
      字节不可变，鉴权留待后续独立立项）；解析 = 取 index → 与本地
      同一 `select_version` 选版本 → 按账本逐文件下载并逐一校验
      blake3（不符 E1803）→ 落 `.cage-cache/registry/<url 指纹>/`
      项目内缓存 → 过 `verify_snapshot` 信任门才交付；缓存复验干净
      离线复用（index 有本地副本兜底，不可达且无缓存 E1802）；
      `cage registry publish/list` 对远程根报 usage 错误（协议无包
      枚举，发布在本地注册表完成后托管）。验收达成：1 新 core 单测
      （select_version 纯函数决策表 + cache_key 确定性与隔离）+ 2 新
      CLI 集成测试（本地静态 HTTP 服务器托管真实注册表：pin→1.9.0、
      显式 @1.0.0、schema_path 走远程、杀服务器后离线复用、幽灵包/错
      版本/无解 pin E1802、篡改字节 E1803、死根 E1802、publish/list
      只读拒绝）全绿 + 文档（cli.md 远程章节/design.md §29 协议/
      validation.md/architecture.md/index.md）+ 本勾选
- [x] R4 回滚与清理：多版本共存下的 GC 策略、`cage registry verify`
      全册校验工具、条目移除/重新发布纪律——`verify_registry`
      （逐条目重过账本校验 + index 记录与条目账本 build_id/
      content_hash 交叉核对 + 无 index 记录的孤儿目录报告，E1803
      逐条列出、只读）；`gc_registry` 滚动窗口（每包保留最新
      `--keep N` 个版本、下限 1——窗口即回滚面，pin 旧版本的消费方
      仍解析、被挤出版本 E1802；孤儿一并清扫、有变化才重写 index、
      dry-run 同一清单不落笔）；`remove_entry` 显式行政移除（版本
      目录带 index 记录一并删、包 index 保留，同字节重新 publish
      干净入册，注册表自身绝不隐式改写历史）；`cage registry
      verify/gc/remove` 子命令（--dry-run，远程根 read-only 拒绝）。
      验收达成：1 新 core 单测（篡改/账本漂移/孤儿 + gc 窗口与
      dry-run/keep 下限/移除重发闭环）+ 2 新 CLI 集成测试（verify
      干净→篡改 E1803→index/账本漂移 E1803→孤儿；gc dry-run 不落
      笔→默认窗口→keep 1→挤出版本消费方 E1802→remove dry-run/
      实删/verify 空册/missing E1802→同字节重发入册）+ 远程只读
      拒绝扩 verify/gc/remove + 文档（cli.md R4 章节/design.md §29/
      validation.md/architecture.md/index.md）+ 本勾选

### S 系列：Remote Source（2026-10 立项，design §45；S1–S3 已交付，S4 起未开工）

形态定稿：四源（Google Sheets / MySQL / PostgreSQL / HTTP API）只读接入，
纪律对齐 R 系列——远端字节先落 `.cage-cache/source/<源指纹>/`，缓存
复用前重过校验门（未经校验不载入）；构建确定性锚在「取到的字节」上
（source_hash 覆盖解析后的 Canonical Model 内容，远端数据变化 →
build_id 旋转，manifest 里可见）；凭据只存环境变量名，不进 cage.toml；
查询只读（装载期 SELECT 白名单 + 运行期只读事务）。错误码 E19xx 族已
全族注册进 codes.rs 与 validation.md 并全部接线生效（E1901/E1902 随
S1、E1904/E1905 随 S2、E1903 随 S3）。

- [x] S0 设计定稿（design.md §45）：四源句法与映射表（table 形式：
      取数方式 / 行映射 / 类型口径，DECIMAL 走字符串）、确定性锚点与
      缓存布局、凭据经 env、只读双保险、断网回退语义、E19xx 五行规划、
      留待实现期清单（增量拉取 / OAuth / 连接池 / 分页 / 内省均不在
      首期）——本轮交付，勾选
- [x] S1 HTTP API 源（`cage-source-http`）：`[source_roots]` 直写
      http(s) URL，GET 响应字节落 `.cage-cache/source/<cache_key>/`
      后走标准 JSON 解析（形状与本地 JSON 完全一致，无新方言）；共享
      取数 / 重试 / 缓存键 helper 首落 `cage_core::remote`（cache_key /
      RetryPolicy / with_retries / http_get，R3 registry 同源复用，cli
      侧的重复实现摘除）；E1901/E1902 接线生效。验收达成：4 新 core
      单测（cache_key 确定与隔离、缓存目录派生、重试恢复与预算耗尽、
      服务端应答不重试）+ 6 新 crate 单测（取数解析并落缓存、401→
      E1902、404→E1901、死端口→E1901、坏 JSON→E0001 拒载、非 http
      URL 拒收）+ 2 新 CLI 集成测试（真实静态 HTTP 服务器：取数构建
      并断言缓存字节、同字节重建 manifest 逐字节一致、远端变更
      source_hash/build_id 旋转；401/404/死根/坏体的错误码与退出码）
      全绿 + 文档（design §45 实装状态与 source_hash 口径修正、
      validation.md E19xx、architecture.md 结构树、source.md HTTP API
      小节）+ 本勾选
- [x] S2 MySQL / PostgreSQL 源（`cage-source-db`，两后端同 crate 共享
      行集映射）：`mysql:<表|具名查询>` / `pg:<同>`，具名查询优先、
      表名展开 `SELECT *`；装载顺序 spec → 查询解析（E1905）→ SELECT
      白名单（单条、SELECT 开头，拒分号 / 注释 / FOR UPDATE|SHARE /
      INTO / CTE）→ dsn_env 解析（E1904）→ 连接——白名单与凭据校验
      都在任何网络触达之前；会话钉只读（MySQL `SESSION TRANSACTION
      READ ONLY`、PG `default_transaction_read_only`，双保险第二重）；
      类型口径 NULL → Null、DECIMAL/NUMERIC 文本保真不走 Float、
      二进制列 base64、日期时间文本渲染；行序确定：带 `ORDER BY`
      尊重原序，否则按行序列化形式排序（写实：主键在适配器侧不可知，
      序列化排序同样满足确定性）；行集 → canonical JSON（键序 = 列序，
      serde_json preserve_order 与全仓同口径）落 `.cage-cache/source/
      <scheme+DSN+SQL 指纹>/` → 标准 JSON 解析，与 S1 同一缓存锚与
      解析链。验收达成：8 新 crate 单测（spec 切分、白名单 13 拒 3 纳、
      具名查询优先与表名校验、dsn_env 三态、值映射含 u64::MAX 与
      DECIMAL 文本、行序两向 + pretty JSON 逐字节断言、materialize
      缓存落盘、load 报错顺序）+ 2 新 CLI 集成测试（无需真实 DB：
      未声明 / 未设 dsn_env → E1904、非 SELECT 具名查询与非法表名 →
      E1905 且先于凭据校验、mysql/pg 死端口与坏 DSN → E1901，退出码
      2）全绿 + 文档（design §45 实装状态与 `[remote.pg]` 句法对齐、
      validation.md E1904/E1905 转已接线、architecture.md 结构树、
      source.md MySQL / PostgreSQL 小节与 crate 表行）+ 本勾选
- [x] S3 Google Sheets 源（`cage-source-sheets`）：`gsheet:<id>/<tab>`
      走 Sheets API v4 `values` + UNFORMATTED_VALUE（公式缓存值），
      tab → 表、首行表头同 Excel 惯例（空表头退 col<i>）、空行跳过、
      短行补 null、tab 自然行序 = 作者承诺序保留；API key 经
      `[remote.gsheets].credential_env`（写实：service account 需
      OAuth JWT 交换，留待实现期，todo 原口径「service account / API
      key 经 env」首签只落 API key）；形状门 E1903 在此接线（非行集 /
      空表头 / majorDimension 非 ROWS）；API base 显式注入点供测试，
      CLI 恒走生产 endpoint。验收达成：10 新 crate 单测（spec 切分与
      id 严格校验、credential 三态、URL 编码、错误码映射且诊断不携
      key、canonical 映射含 pretty JSON 逐字节断言、形状门六拒、
      spec/凭据先于网络的 load 顺序、mock 服务器端到端取数映射缓存、
      403→E1902 / 404 与死端口→E1901 / 坏形状→E1903）+ 1 新 CLI
      集成测试（未声明 / 未设 credential_env → E1904、坏 spec 与
      traversal id → E1901，退出码 2）全绿 + 文档（design §45 实装
      状态与映射表 / E19xx 表 / service account 口径修正、
      validation.md E1903 转已接线与 Sheets 装载顺序、architecture.md
      结构树、source.md Google Sheets 小节与 crate 表行 + 后续扩展
      清单移除）+ 本勾选
- [ ] S4 确定性与离线语义收口：远端变更 → source_hash / build_id 旋转
      的端到端测试；断网缓存回退 + WARNING 诊断；`--no-cache` 严格模式
- [ ] S5 错误码接线收口：E1901–E1905 已于 S1 全族注册（`codes.rs` +
      validation.md）并已全部接线生效（E1901/E1902 随 S1、E1904/E1905
      随 S2、E1903 随 S3），本项核对全族「预留」标注清零、诊断渲染
      覆盖四源后收口
- [ ] S6 文档收口：source.md 后续扩展清单转正、cli.md 远程源章节、
      需求整理.md Remote Source 勾选、architecture.md 工程结构树补三 crate

### G 系列：Template Target（2026-10 立项，design §22；G1 随立项交付）

形态定稿：Tera（Jinja 风格，过滤器 / 继承 / 宏）统一官方与用户自定义的
代码生成面——IR（Schema）整体作模板变量（tables / fields / 类型 /
描述 / 默认值全部可引用，逐表模板另获当前 `table` 变量，顶层
`schema_hash`；迭代序 = schema 声明序）；模板文件名即输出文件名模板
（`{table}.py.tera` 按表名序每表一文件、不含 `{table}` 全局渲染一次，
沿用 `file_template` 的 `{table}` 占位符口径）；**模板内不写逻辑**——
命名约定与各语言类型映射 / 默认值字面量做成 Tera filter，决策留 Rust；
确定性 = 模板名序 × 表名序产物顺序 + Tera workspace 锁版 + 模板随源码
+ 现有 golden 测试逐字节不变为改写验收锚。与 §23 直渲染的关系：plan →
render → verify 三层与「决策留 Rust」不变，G2 只换 render 挂点（模板
文本），原「为什么不是模板引擎」论证在 §23 留档并附决策更新段。

- [x] G1 模板引擎接入（`cage-target-template`）：Tera 实例封装（递归
      收集 `*.tera`，模板名 = 相对路径，autoescape 关）、IR context
      桥（schema 全量序列化 + schema_hash + 逐表 table）、文件名映射
      两渲染形态（逐表 / 全局）、命名约定过滤器首落
      （snake_case / camelCase / PascalCase，含缩略词与数字边界
      words 分词）、from_config 读 `options.template_dir`；serde_json
      preserve_order 显式声明（fields / tables 迭代序 = schema 声明
      序，不随依赖图特征统一漂移）。验收：10 单测（逐表渲染两表序 +
      声明序断言、全局模板引用表 / 字段 / 类型 / 枚举、schema_hash
      暴露、三过滤器、描述与默认值可引用（default 过滤器兜底）、
      缺目录 / 空目录 / 语法错三路错误通道、from_config、words 边界
      四组）+ design §22 修订（Target Generator → Template Target，
      实装状态 G1 交付 / G2–G5 未开工）+ §23 决策更新段（论证留档 +
      反转理由写实）+ target.md 引用句同步。实现顺序后续四步：
      引擎接入（本轮）→ 官方模板改写 → 自定义模板加载 → 过滤器库
- [ ] G2 官方模板改写（推进中，Lua / C# 已落地；余 5 crate 见 G2b）：各语言
      （C#/Python/Lua/TS/JS/C++/Go/Java，7 crate）生成器 render 层改写
      为随包官方 `.tera` 模板（crate 内 `templates/` 目录可复制可改 +
      `include_str!` 编译进二进制保无文件系统时可用）；plan 层决策沉淀
      为 Rust 过滤器与 context 预计算；**现有测试逐字节不变为验收锚**
      （输出字节变 = 回滚信号）。引擎新增 `generate_official`：内存
      模板按传入序注册（artifacts = 传入序 × 表名序，与各生成器既有
      发射序一致），双 hook 分工——`setup` 一次性注册语言过滤器（纯
      文本转换，如 `lua_string`）、`extras` 逐 context 合并语言预计算
      （schema 感知映射 / 唯一名 / 字面量收集），模板只表达文本形状。
      Lua 首个落地：render_table / render_enums / header / path_for /
      sorted_tables 摘除，决策进 `lua_extras`（member_fields 名序 +
      unique_ident、type_label、doc、default_expr、primary_key_lit、
      emitted_enum_objects），`templates/table.lua.tera` +
      `enums.lua.tera` 随包，18 测试不改一字全绿（含 lua 运行时回验与
      确定性）。C# 第二签：render_table / render_enums / header /
      path_for / sorted_tables 摘除，决策进 `cs_extras`（namespace、
      summary_head+desc 转义、class_ident、members 名序含可选 `?` 与
      ref-init 解析、enum-vs-static-class 按全整型判定分流与 backing
      后缀选择），`templates/table.cs.tera` + `enums.cs.tera` 随包，
      18 测试不改一字全绿。Python 第三签：render_table / render_enums /
      header / path_for / sorted_tables 摘除，决策进 `py_extras`
      （import 面预计算——field/Any/enum 三路按需、dataclass 分组排序
      plain→defaulted 且成员行整体预拼、IntEnum-vs-plain-class 按全整型
      判定、IntEnum/字符串桶字面量），`templates/table.py.tera` +
      `enums.py.tera` 随包，18 测试不改一字全绿。TS/JS 第四签（双形态
      crate 一笔）：TS 与 JS 两模式 ×6 模板随包（table.{ts,js,d.ts} +
      enums.{ts,js,d.ts}.tera，.d.ts 走 `dts_path_for` 派生模板名），
      JS 模式每表 .js+.d.ts 成对发射——引擎「传入序 × 表名序」是模板主序，
      与旧逐表交错序不同，改为 main/dts 两次 generate_official 再按序
      zip（两遍走同一表集与 enums 条件，1:1 对齐），路径序测试不改一字
      仍锁定旧交错序；决策进 `ts_extras`（双 plan 预计算——Local 与
      InlineImport 两种枚举拼写同备、import type 整行、jsdoc 名称括号
      与扫描器安全分行判定、default_rows 与 factory_inline 同源、
      枚举桶字面量 + typedef 行），28 测试不改一字全绿。C++ 第五签：
      render_table / path_for 摘除（render_enums + header 留
      `#[cfg(test)]`——`test_render_enums_with_empty_slice` 直调空切片
      防御边界，测试不改一字），generate 保留共享名分配（结构名先、
      枚举名后跨文件一致）并预计算枚举头 context；决策进
      `Self::cpp_extras`（include 面 Needs 收集、`{}` 初始化与
      optional 分流、banner、双 context 分发），`templates/table.h.tera`
      + `enums.h.tera` 随包，19 测试不改一字全绿（含 g++ 编译回验）
- [ ] G2b 余量改写：Go → Java 逐 crate
      复制 Lua/C# 模式（每 crate 一笔：模板随包 + extras 提取 + 测试
      逐字节不变），全数落地后 G2 勾选收口
- [ ] G3 自定义模板加载：CLI 接线——target 配置 `template_dir`
      （`.cage/templates/` 惯例位置，相对项目根），`code_target_items`
      开 Result 分支承接模板渲染错误（现役口径不可失败），
      gen/build/diff 面板接输出；模板缺失 / 语法错诊断指到模板文件
- [ ] G4 过滤器库：G2 改写中沉淀的过滤器整理成库——各语言类型映射
      （py_type / cs_type / ts_type / …）、默认值字面量
      （*_literal）、语言字段排序（*_field_order）、命名约定扩展
      （kebab_case / SCREAMING_CASE 等），过滤器表进 target.md
- [ ] G5 文档收口：§22/§23 与实装对账复查、target.md 新增模板小节
      （模板变量表 / 过滤器表 / 自定义模板指南）、需求整理.md 状态行、
      architecture.md 结构树补 cage-target-template、README Target
      插件行更新

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
