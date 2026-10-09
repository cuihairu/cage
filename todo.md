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
- [x] T3.7 命名枚举成员资格校验（审计遗留闭环，审计-文档一致性.md 留档第 1 项）：
      L3 `validate_field_constraints` 对 `type: {kind: Enum, value: 名}` 字段检查
      顶层 `enums:` 声明的成员资格（同走 E1204，提示 Allowed values）；只查字符串值
      ——非字符串类型失配归 L2（E1101），悬空枚举名归 L1（E1004），不双重诊断；
      顺手顺修审计留档第 2 项（schema/mod.rs compatible_with 注释 E1410→E1411）；
      验收已过：l3_enforces_named_enum_membership 单测（越界成员 E1204 +
      合法成员通过 + 非字符串仅 E1101）——本轮交付，勾选

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
- [x] MsgPack 生成器（2026-10，第二阶段唯一遗留收尾之一，design §23
      Data Targets）：新 crate `cage-target-msgpack`——Canonical Model →
      MessagePack 二进制，`format = msgpack`/`messagepack` 进 `cage
      build` 数据面（json/csv 同面板，`unsupported target format` 对其
      收口）；编码走 `rmp` 最小形（定值定字节），确定性契约 = 行字段
      `sort_keys` 名序（默认 true，false 保留源序）+ 嵌套对象键
      normalize BTreeMap 序 + 非有限浮点编码 nil（NaN 位型跨平台不稳，
      对齐 JSON target 非有限 → null 规则）+ Bytes 原生 bin 不做
      base64；options 仅 `sort_keys`；验收：8 单测（手算 golden 字节、
      两跑逐字节一致、全 Value 族逐字段字节断言、sort_keys 双向、
      profile 过滤、from_config 三态、trait 面）+ 1 CLI 集成测试
      （构建产物 golden 字节 + 全量重建同字节 + manifest 记录），
      workspace 615 全绿
- [x] Protobuf 生成器（2026-10，第二阶段唯一遗留收尾之二，design §23；
      与 MsgPack 同批收官）：设计稿无 wire 编码明文口径，拍板 = 输出
      .proto3 定义文件（schema 驱动、与 C#/Go 代码绑定同族，数据不
      参与——消费方 protoc + 自有 runtime）；新 crate
      `cage-target-proto` 进 `cage build`/`cage gen` 代码面
      （`format = proto`/`protobuf`，`unsupported target format` 对其
      收口）。映射口径（target.md 存表）：窄整折叠 int32/uint32、
      int64/uint64、float32/float64 → float/double；object/null/any →
      google.protobuf.Struct/Value（自由形状，与 C++ std::any 同族）；
      map 键 int → int64（go/cpp/java 同族）；repeated 元素 / map 值位
      的 array/map 包 `{字段}Value` 嵌套 message（深度叠后缀）；整数
      枚举（成员全整数且在 int32 域）进共享 cage_enums.proto + 按需
      import，缺 0 前置合成 `{Enum}_UNSPECIFIED = 0`、值重复
      allow_alias，否则字段回退 string 并注释标注；确定性 = 表/字段
      名序 + 字段号 1..n 名序分配 + 标识符共享去重集合（表先枚举后、
      冲突追 `_`）+ 文件头锤 schema 哈希；options：package（默认
      cage.generated）/ enums_file（默认 cage_enums.proto，import 路径
      随之）。验收：14 单测（产物路径名序、头块、全类型映射、
      well-known import 按需、枚举三态合成、int32 域外回退、嵌套包装
      三形态、标识符 sanitize/去重/保留字、确定性两跑、schema 哈希
      可选、空表、from_config、enums_file 驱动 import）+ 1 CLI 集成
      测试（gen 产物形状 + 两跑逐字节 + build 同 lane + manifest 记
      录），workspace 630 全绿；文档 target.md/cli.md/architecture.md/
      README/需求整理.md 同步。至此第二阶段全部交付
- [x] Plugin SDK + Game Rule Validator 实装（沙箱方案定稿于 design §17.1：进程内 trait 现已实装、动态库 C ABI shim 为第三方分发路线、不可信代码不执行；GameRuleValidator trait + GameRuleRegistry + 内建 power_curve 样例端到端，`cage check --level gamerule` 输出 E1601 行级诊断；动态库装载与插件市场不在本期）
- [x] Dependency Graph（cage graph：引用图/构建顺序/循环检测）
- [x] E1403 循环检测接线进校验面（2026-10 点火：此前图 API 的
      find_cycles 只服务增量/库侧，校验面不报）：L5 逐行引用检查后做
      schema 级循环检测——`DependencyGraph::from_schema` + `find_cycles`，
      每环旋转最小表名起头去重（两表互引 A↔B 不再按起点双报）后逐环
      报一条 `E1403` WARNING（`Circular reference: Item → Kit → Item`），
      自引用按图契约计环同报；语义拍板 = WARNING 而非 ERROR——环不阻断
      构建（生成单遍、增量传播 visited 集安全），但断库 API 拓扑序
      （`topological_sort`/`build_order` 返 Err），ERROR 会误杀合法互引
      schema（掉落表 ↔ 怪物表）；`warnings_as_errors = true` 升级。
      验收：core 单测×4（两表环 canonical 消息与 table/source 断言/
      三表环单告/自环 + 菱形无环静默/升级）+ CLI 集成×1（check 退码 0
      带 E1403 与 canonical 环路径、warnings_as_errors 退码 1）+
      validation.md L5 表级引用环节与码表、需求整理.md:67 转已接线
      ——本轮交付，勾选
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
- [x] Remote Source（Google Sheets/MySQL/PostgreSQL/HTTP API）——S1–S6
      收官（2026-10，见下方「S 系列：Remote Source」：S1 HTTP API 源 /
      S2 MySQL / PostgreSQL 源 / S3 Google Sheets 源 / S4 确定性与
      离线语义收口 / S5 错误码接线收口 / S6 文档收口）
- [x] Artifact 分发与迁移——全数交付（2026-10 拍板：A 系列注册表分发
      A1–A4 收官 + M 系列声明式数据迁移 M0–M3 收官，见下方两节；设计
      定稿 design §46/§47，决策记录随稿——定了什么 / 为什么 / 备选）

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

### S 系列：Remote Source（2026-10 立项，design §45；S1–S6 全数交付）

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
- [x] S4 确定性与离线语义收口：远端变更 → source_hash / build_id 旋转
      的端到端测试（`remote_source_fetch_build_and_rotate` 随 S1 已锚定
      ——identical rebuild manifest 逐字节一致 + 数据变更双哈希旋转）；
      断网缓存回退 + WARNING 诊断（E1906 新码：三适配器传输类失败 →
      `cage_core::remote::source_cache_fallback` 共享门 → 缓存副本存在
      则 eprintln warning 并解析副本，404 / 401/403 永不回退——旧字节
      不掩盖远端删除或访问吊销，DB 的 E1904/E1905 在网络触达前发生同样
      不回退；回退出的缓存文件走与在线路径同一条标准 JSON 解析无旁路；
      WARNING 只带 spec / 表名 / URL，凭据永不落日志）；`--no-cache`
      严格模式（check / build / gen / inspect 四子命令，load_project
      透传 strict，回退关闭还原硬 E1901）。测试 6 新增——http crate
      回退 / strict / 404+401 不回退（2）、db materialize 闭包注入
      回退 / strict / 无缓存原错误（2）、sheets mock server 回退三态
      （1）、CLI 进程级 build 落缓存 → 杀 server → 离线 build E1906
      WARNING → `--no-cache` 退码 2 E1901（1），现有测试不改语义全绿，
      workspace 551（545 + 6）。文档对账：design §45 断网语义段转已
      收口（仅传输类回退 / E1906 / DB 全传输类口径）、E19xx 表补
      E1906、实装状态补 S4；validation.md E1906 行 + 三源段落补回退
      口径；source.md 新增「离线回退与 --no-cache」小节；cli.md 新增
      `--no-cache` 小节；需求整理.md Remote Source 行更新 S4 已交付
      + 本勾选
- [x] S5 错误码接线收口：E1901–E1905 已于 S1 全族注册（`codes.rs` +
      validation.md）并已全部接线生效（E1901/E1902 随 S1、E1904/E1905
      随 S2、E1903 随 S3），本项核对全族「预留」标注清零、诊断渲染
      覆盖四源后收口——已交付：codes.rs E1903/E1904 doc 尾「reserved
      until」措辞清零（E1906 随 S4 入族时模块头已转全族 wired，本签
      收尾两处常量注释），全仓 grep E19xx×预留/reserved 零残留；
      四源错误路径码覆盖系统盘点——HTTP：E1901（传输 / 404 / 其他
      状态 / 非 http URL）+ E1902 + E0001（坏 JSON 本地同款 parse
      诊断）+ E9902 + E1906；DB：E1905（spec / 白名单 / 表名）+
      E1904 + E1901（DSN 非法 / 连接 / 只读 pin / prepare / 查询，
      mysql.rs / pg.rs 全带码）+ E9902；Sheets：E1901 + E1904 +
      E1902 + E1903（形状门）+ E9902 + E1906；每条路径既存测试逐码
      断言（crate 级 err.contains + CLI 进程级 stderr contains），
      诊断渲染经 load_project Err → `error: {e}` 全覆盖，无吞码路径
      ——零代码行为变化，纯措辞与文档签。design §45 实装状态补 S5
      对账详述、todo 勾选、需求整理.md 行同步
- [x] S6 文档收口：source.md 后续扩展清单转正（注明首期四大远程源已交付）、cli.md 新增 Remote Source 章节（四源语法/特性/错误码对照表 + 离线回退引用）、需求整理.md Remote Source 行同步 S6 已交付、architecture.md 工程结构树三 crate（cage-source-http/db/sheets）早已在列

### A 系列：Artifact Distribution（2026-10 立项，design §47）

形态定稿：注册表条目的分发走两条路，复用同一账本信任门（未经校验不分发、
不入册）——① **离线 bundle**：`cage registry export` 产出确定性 tar
（条目全文件 + HASHES.json + 包 index 摘录，mtime/uid/gid 归零、成员名序，
同条目 = 同字节），`cage registry import` 先 `verify_snapshot` 再入册
（坏账本 E2103，同字节幂等 / 异字节 E1801）；② **直推**：
`cage registry push` 对 http(s) 根逐文件 PUT、条目全成后最后写包 index
（字节不可变纪律不变），`Authorization: Bearer $TOKEN` 经
`[registry].auth_env` 环境变量名解析（E1904 同口径：凭据不进 cage.toml、
不落日志、网络触达前先验缺失）。服务端只文档化约定（任何能收 PUT 的静态
网关 / nginx WebDAV / CI job 皆可），cage 不实现服务端。决策记录（定了
什么 / 为什么 / 备选）见 design §47。实现顺序：A 先 M 后（A 复用 R 系列
在册设施零新概念，M 新模块爬坡）。

- [x] A0 设计定稿（design.md §47）：两条分发路（bundle / 直推）与接口
      草案（export_bundle / import_bundle / push_entry + 三子命令）、
      E21xx 五码规划（E2101 传输 / E2102 鉴权 / E2103 账本 / E2104
      拒写 / E2105 凭据缺失）、能力边界（只动已入册条目、不做服务端 /
      delta / 签名）、留待实现期清单——本轮交付，勾选
- [x] A1 bundle 导出（`cage registry export`）：`export_bundle` 确定性
      tar 打包（成员名序 + mtime/uid/gid 归零 + 0o644，同条目两跑逐字节
      断言锁定）+ E2101 接线 + codes.rs `distribution` 五码模块（E2102–
      E2105 预留标注至 A2/A3 接线清零）+ validation.md E21xx 新表节；
      验收已过：单测（确定性两跑一致 + tar 头归零 + index.json 摘录
      单条目 / 包·版本·路径逃逸·缺账本四路 E2101）+ CLI 集成测试
      （publish → export 两包逐字节一致 → `tar::Archive` 成员清单可检 →
      缺包缺版本 E2101 退出码 1）+ cli.md「bundle 导出」小节——本轮交付，
      勾选
- [x] A2 bundle 导入（`cage registry import`）：import_bundle 解包进
      临时暂存区 → verify_snapshot 信任门 → index 摘录与账本交叉核对
      （build_id/content_hash/文件数）→ publish 路径入册；--dry-run
      跑完整门只报告；结构问题/信任门拒绝/摘录漂移一律 E2103（拒绝
      字节永不接触目标根）、异字节 E1801 复用；tempfile 提升为 core
      正式依赖（暂存区 RAII 清理）；验收已过：单测×2（干净导入 →
      verify_registry 全绿 + resolve 可解析 + 字节一致、重导幂等；
      篡改 E2103 且目标根零字节、假摘要 E2103、异字节 E1801、dry-run
      不落笔）+ CLI 集成（dry-run 报告不写 → 导入 → 消费方含条目
      schema 构建通过 → 重导 no-op → 篡改 E2103 → 异字节 E1801）+
      codes.rs E2103 doc 转接线 + validation.md 转已接线 + cli.md
      「bundle 导入」小节——本轮交付，勾选
- [x] A3 直推（`cage registry push` + auth_env）：`remote::http_put`
      （RetryPolicy 与 http_get 同口径，`send_bytes` 定长发送——`send`
      走 chunked 静态主机不收；RetryClassify trait 泛化 with_retries，
      GET/PUT 共用一政策各留失败种类）+ 逐文件 PUT、index 合并远端
      条目收尾（远端历史永不改写）+ `RegistryConfig.auth_env`（serde
      default 可选，只存变量名）+ E2102/E2104/E2105 接线；推送语义：
      `push <project> [pkg[@ver]] --registry <远端根> [--auth-env]`，
      源 = `[registry].path`（包名缺省 project.name、版本缺省最新），
      匿名探针 GET（404=全量上传/同 hash 幂等零 PUT/异 hash E1801），
      Bearer 只随 PUT；验收已过：core 单测×3（顺序断言 GET 匿名+PUT
      骑 Bearer+index 最后、合并序点分、重推幂等零 PUT；401 E2102
      fail-fast 且 token 不入错误文本、405 E2104、未设/空 env E2105
      闭合端口零网络；dry-run 零存零 PUT、异字节 E1801 远端 index
      未被改写）+ CLI 集成×2（push → 远端根 resolve 复现消费方构建
      通过 → 重推 no-op → dry-run 零 PUT → E2105 网络前失败；401
      E2102 / 405 E2104 / 本地路径拒 exit 2）+ design §47 接口块按
      实装细化 + cli.md「直推远端」小节 + validation.md/codes.rs
      三码转已接线——本轮交付，勾选
- [x] A4 文档收口：design §47 加实装状态段（A1–A3 交付、E21xx 五码
      全接线、接口以实装为准）、validation.md E21xx 全族转已接线
      （E2101 补 push 传输语义、E2102/E2104/E2105 本系前签转正）、
      architecture.md 工程树 registry.rs/remote.rs 注释与 Snapshot 行
      补 A 系列分发、需求整理.md 状态行与 Artifact 分发行转已交付
      ——本轮交付，A 系列收官，勾选
- [x] A5 压缩容器（§47 延迟项补齐）：`export_bundle` 增 compression 参数
      （`BundleCompression::Plain | Zstd`），CLI `--compress zstd`（clap
      value_parser 只收字面量 zstd，异值参数错误 exit 2）把确定性 tar 原样
      包进单个 zstd 帧（`zstd::stream::encode_all`，`BUNDLE_ZSTD_LEVEL`
      钉死级别 19——bundle 是小配置产物比率优先于速度；同 tar+同级别+
      同库版本 → 同容器字节，两跑逐字节断言锁定）；`import_bundle` 按文件
      头帧魔数（28 B5 2F FD）嗅探容器、不看扩展名（改名 .tar/.zst 均导入），
      压缩只在容器层——HASHES.json 账本哈希解压后内容，两种形态过同一
      信任门；push 不变（推单文件不推 bundle）；依赖 zstd = "0.13" 进
      workspace.dependencies（cage-core 引用，离线缓存即解）；验收：core
      单测×2（zstd 导出两跑逐字节一致且解包载荷 == 纯 tar 导出；zstd
      bundle 导入 verify_registry 全绿、条目字节同源、dry-run 不落笔、
      改名双向嗅探、截断帧 E2103）+ CLI 集成×2（--compress zstd 两跑一致
      + 头四字节魔数 + 重导 no-op；--compress gzip exit 2 零落笔）
      ——本轮交付，勾选

- [x] A6 签名账本（§47 留待实现期：ed25519 抗抵赖签名）：
      `cage registry keygen -o <file>`（种子只落 `-o` 文件 unix 0600、
      永不上 stdout；公钥 + 环境变量用法上 stdout）+ `cage registry
      export --sign --key-env <VAR>`（bundle 落盘字节整体 ed25519 签名 →
      `<bundle>.sig` 分离式 sidecar，JSON algorithm/public_key/signature）
      + `cage registry import --verify-sig --key-env <VAR>`（账本信任门
      之前的签名门——sidecar 随行公钥非信任锚，验证只钉消费方 env 钥匙，
      缺钥匙不跑、无静默降级）；密钥只走环境变量（base64 32 字节），不进
      cage.toml、不进日志；E2106（密钥材料不可用）/ E2107（sidecar 缺失
      畸形、算法不认、验签不过）接线进 codes.rs + validation.md；
      ed25519-dalek =2.2.0（rand_core feature，离线缓存全链可解）+
      rand_core 0.6.4（getrandom）+ base64 0.22；验收已过：core 单测×3
      （签验往返/篡改与他钥拒绝/坏密钥材料 E2106/sidecar 往返与 0600）
      + CLI 集成×1（keygen→export --sign→import --verify-sig 全流程 +
      篡改字节/缺 sidecar/他钥 bundle/坏 env 四类拒绝）+ parse 单测
      （--sign/--verify-sig 与 --key-env clap requires 方向）——本轮
      交付，勾选

### M 系列：Migration（2026-10 立项，design §46）

形态定稿：Schema 演进下的声明式数据迁移——`migrations/` 目录文件名序即
版本步进链，每段显式声明变换（rename_field / set_default / remove_field /
widen_type / remap_values / rename_table），`cage migrate` 对 Canonical
Model 执行、逐表逐行报告、新 schema 回验（check 全绿才算完成）；默认
dry-run 只报告，`--write` 才对可文本源（JSON/YAML/CSV）落盘，Excel 源
恒报告不落盘（红线「不做 Excel 编辑器」）；迁移后的注册表条目 = 重跑
build + publish，不新造通道。决策记录（定了什么 / 为什么 / 备选：diff
自动推断弃、只报告弃、运行时兼容层弃）见 design §46。

- [x] M0 设计定稿（design.md §46）：能力边界（Canonical Model 层显式
      变换 / Excel 只报告 / 条目迁移复用 publish）/ 接口草案
      （MigrationSpec / Step / parse_spec / apply + cage migrate）/
      E20xx 四码规划（E2001 解析 / E2002 引用 / E2003 变换 / E2004
      回验）/ 留待实现期清单——本轮交付，勾选
- [x] M1 规则模型与解析：`cage-core::migrate`（MigrationSpec / Step /
      parse_spec，版本步进链文件名序）+ E2001/E2002 注册与 validation.md
      预留标注；实装细化：Step 六变体 wire 格式 = 每步单键映射
      （`- rename_field: {…}`，未知变换名解析即拒）、serde_yaml 无
      externally-tagged 枚举支持故 Step 手写 Deserialize（单键映射
      手工分派，payload 经 serde_yaml::from_value；set_default 的值走
      yaml_to_value 裸 YAML 转换——canonical Value 是 adjacently
      tagged serde_yaml 解不了，Bytes/自定义 tag 不支持）；结构自检
      （from/to 非空且不等、steps 非空）；validate_spec 对 from-schema
      校验引用（表/字段存在性 + rename 目标撞名）；验收已过：单测×4
      （六类 Step 全解析顺序保持、目录文件名序链 + 非迁移文件忽略、
      坏文件六路 E2001——读不到/坏 YAML/空 steps/未知变换/缺 to/
      from==to、引用校验——表不存在/字段不存在/rename 撞字段/
      rename_table 撞表与合法改名）+ validation.md E20xx 新表节
      （E2001/E2002 已接线、E2003/E2004 预留 M2）——本轮交付，勾选
- [x] M2 执行器与报告：`apply` 对 Canonical Model 变换 + 逐变更报告 +
      E2003/E2004；实装细化：apply(spec, &mut Document, from_schema)
      逐步骤原位变换（MigrateReport/StepReport 逐步骤行数），六类语义
      ——rename_field 保序改名、set_default 只补缺失或 Null 行、
      remove_field 计实删行、widen_type 方向表（整数/无符号族升序、
      无符号→有符号跨一步 8→16/16→32/32→64、Int32→Float64 允而
      Int64→Float64 拒——2^53 精度线）+ 逐行值域校验、remap_values
      未映射值原样通过、rename_table 表序保持（文档无此表 E2003）；
      失败即失败整段中止；reverify(doc, to_schema) 借 validation 全栈
      L0–L6 回验（GameRule 留 CLI check 通道），E2004 附诊断渲染；
      验收已过：单测×4（六类 Step 变换语义 + 报告计数逐项断言、加宽
      不安全方向两路/值超域/行缺字段/文档缺表 E2003 + 合法加宽通过、
      迁移产物回验全绿而未迁移产物 required 缺失 E2004、serde_json
      序列化两跑逐字节一致）+ validation.md E2003/E2004 转已接线 +
      codes.rs 两码 doc 转接线——本轮交付，勾选
- [x] M3 CLI 与文档收口：`cage migrate`（--all/--to/--write，默认
      dry-run）+ 可写源落盘 / Excel 只报告 + CLI 集成测试（json 工程
      dry-run → write → check 全绿 → 再跑无变更；excel 工程 E2004
      路径与报告形态）+ design §46 实装状态 + cli.md migrate 章节 +
      validation.md E20xx 转已接线 + 需求整理.md 迁移行同步 + 本勾选。
      实装细化：Commands::Migrate + run_migrate——目录缺失/空链提示
      nothing to migrate 退出 0，段选择默认首段（显式逐段推进）/ --all
      全链 / --to <ver> 链前缀（互斥，链外版本退出 2），逐段 apply 后
      当前 schema 一次回验（E2004 不落盘），CLI 不跑 validate_spec
      （无历史 from-schema，E2002 留库 API；apply 防线接力——rename
      目标行内/表级同存即 E2003 拒绝不合并两值）；落盘 write_migrated_
      sources 按表 source_file 分组、只写本地 source roots 内文本源
      （Excel 恒报告、registry/远程源只读报告、字节未变跳过 unchanged），
      JSON/YAML 写 {Table: [rows]}、CSV 单表按 schema 字段声明序渲染
      （JSON 读取不保文件键序，schema 序是唯一稳定渲染序——第二次
      --write 起字节恒定）、单元格按读取推断规则反写（整值 Float 带
      .0 防落回整数分支、数组/对象/Bytes 拒写）；apply 双路径——
      widen_type 的 current==to（CLI 恒载新 schema）跳方向表只跑值域、
      current!=to 走方向表，rows_changed 只计真实改变行（Int32→Int64
      canonical 表示不变 0 行）；验收已过：core 单测×3 增量（幂等重跑
      0 行字节不变、current==to 双路径、rename 撞名两路 E2003）+
      CLI 集成×6（dry-run 报告与零字节、write→check 绿→再跑 0 行
      unchanged、目录缺失 0/坏规则 E2001、--all/--to/链外版本 2、
      excel E2004 不落盘、excel 成功 skip 报告恒不写）+ parse_migrate
      单测（默认/--all/--write/--to/互斥）+ design §46 实装状态与
      实装决策记录（widen 双路径/validate_spec 取舍/幂等/渲染序/多段
      链回验）+ cli.md migrate 章节与顶部标注 + 需求整理.md 迁移行
      勾选——本轮交付，勾选；M 系列全数收官
- [x] M3.1 增量交付（§46 留待实现期两项）：`cage migrate --to latest`
      （解析为整链、同 `--all`；链上字面 `latest` 版本优先，具体目标赢过
      符号）+ Excel 报告单元格级定位（`StepReport.affected_locations`——
      `apply` 对 Excel 表（.xlsx/.xls 后缀判定）采集受影响行 `Row.location`、
      文本源不采集；CLI 源序渲染 `file | Sheet | Row`，超 8 行折叠
      `… +K more row(s)`，确定性封顶）；验收已过：core 单测×3 新增
      （Excel 定位采集源序、幂等重跑零定位、rename_table 全表定位与文本表
      空）+ CLI 集成×2（Excel 定位行渲染；`--to latest` 与 `--all` 输出
      逐字节一致 + 字面 latest 链优先）+ parse/封顶单测扩展——本轮交付，
      勾选

- [x] M3.2 增量交付（§46 留待实现期：schema diff 规则草稿）：
      `cage migrate-draft <from-schema> <to-schema> --from <ver> --to <ver> [-o file]`
      —— `migrate::diff` 模块（`diff_schemas` 表/字段名对齐结构 diff +
      `draft_migration` 机械安全变换成步、歧义项 `# TODO` +
      `render_draft` 规则稿渲染）；改名从不猜（rename/remap/非加宽换型/
      枚举增删/删表新表全留 TODO）；UInt 默认值走 canonical 邻接标签
      形态（`yaml_to_value` mapping 先试标签解码回落 plain；渲染端含
      UInt 一律发标签形态）规避裸标量回读 Int 族撞 E1101；字符串恒
      单引号防 `yes`/数字回读翻型；空稿 `steps: []` 由 parse_spec 按
      E2001 拒绝（全 TODO 稿不可直接运行）；`-o` 父目录自动创建；
      验收已过：core 单测×7（diff/draft/typed defaults/enum TODO/
      required 翻转/渲染往返含标签形态/空稿确定性）+ yaml_to_value
      标签解码单测 + CLI 集成×2（draft→migrate --all --write 全链落盘
      数据实迁；全 TODO 稿 E2001 拒绝）——本轮交付，勾选

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
- [x] G2 官方模板改写（已交付，7 crate 全数落地）：各语言
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
      + `enums.h.tera` 随包，19 测试不改一字全绿（含 g++ 编译回验）。
      Go 第六签：render_table / render_enums / header_lines / path_for
      摘除，generate 重写为 generate_official 调用（table.go.tera 恒在、
      enums.go.tera 仅在有非空 enum 时注册，file_template / enums_file
      运行时作模板名，package 闭包捕获传入 extras）；gofmt 对齐数学
      原样留 Rust——Doc/Cell/Term tabwriter 移植与 go/printer exprList
      换行分节启发不动，extras 把每个对齐块渲染成最终行（struct_lines /
      new_lines / 逐枚举 const_lines），模板只表达文件形状；决策进
      `Self::go_extras`——表侧：sorted_fields 名序、member 唯一 ident、
      field_go_type / render_default（enum 类型表克隆视图签名零改动）、
      needs_math 侦测、banner_head（裸名或主键列举）、new_doc 构造器
      文档行，枚举侧整包返回预计算 context（None 分发；emitted_enums
      键避开 schema IR 的 enums 键——extras 键不覆盖 IR 键）。
      `templates/table.go.tera` + `enums.go.tera` 随包（头注释即模板
      文档），Tera 空白控制逐字节复刻原 Doc 发射序列（header 六行、
      package 前后空行、math import 按需、banner 单/双行、struct 块与
      构造器块间空行、逐枚举空行与 type decl 后空行、文件收尾单换行）。
      验收：go 20 测试不改一字全绿——含路径序（表名序 + enums 收尾）、
      gofmt 列对齐三形态（多字段 vtab 对齐 / 单字段空格分隔 / 空
      struct{} 一行壳）、构造器几何均值分节（44 字符 ident）、枚举
      三桶 backing 与单 spec 行内注释、包级名冲突后缀（表先于枚举）、
      map 键序与跳项、未解析 enum 回退、确定性两跑一致。Java 第七签：
      render_table / header 摘除（header 转 `#[cfg(test)]` 支撑遗留
      renderer），render_enums 转 `#[cfg(test)]`
      （test_render_enums_skips_unallocated_idents 直调「跳过未分配
      ident」防御边界），push_javadoc 转 `#[cfg(test)]`（直调空 body
      边界），path_for / enums_path 保留——generate 重写为
      generate_official 调用（table.java.tera 恒在、enums.java.tera 仅
      有非空 enum 时注册），**产物按发射序重 path**（Java file stem =
      class ident 而非 schema name，引擎按 schema name 替换 {table}，
      zip 回写 legacy 路径——同 TS 双遍先例零引擎改动）；决策进
      `Self::java_extras`——表侧：rows 逐字段解析（member 唯一 ident、
      default 内联、enum_text qualified Holder.Enum 判定——表类与枚举
      同名走 `Holder.Enum` 免 import）、imports 四路收集（List /
      ArrayList / Map / HashMap 含嵌套扫描 + holder 枚举 import 去重）
      字典序排序、banner_head（裸名或主键列举）、members decl
      （optionality 分组规则 + default 内联 + field_doc），枚举侧整包
      返回预计算 context（None 分发；`java_enums_context` 三桶 payload
      int/long/String 判定、integral_literal/string_bucket_value 字面量
      与 note 预拼 decl 行、单行 javadoc banner、未分配 ident skip 语义
      保留）。templates/table.java.tera + enums.java.tera 随包（头注释
      即模板文档），Tera 空白控制逐字节复刻 legacy writeln 序列
      （package 先 header 后、import 块按需、javadoc 两形态——多行
      desc 在上单行 head、成员间单空行、空表类体单行闭合、枚举块
      构造器空行形状、文件尾单换行）。验收：java 20 测试不改一字
      全绿（含路径序 class ident stem、qualified 免 import、空表壳、
      枚举三桶与 note、子目录 enums_file、确定性两跑一致）+
      javac 真机编译回验（edge / sample 样本 javac 全过）。修正三处：
      模板文件尾补换行（输出收尾 \n 丢失即断言崩）、rows 解析 used
      集移出循环外（成员唯一性跨表共享）、uninlined_format_args 内联
- [x] G2b 余量改写（已交付）：Go → Java 逐 crate
      复制 Lua/C# 模式（每 crate 一笔：模板随包 + extras 提取 + 测试
      逐字节不变）——六签全数落地（C# / Python / TS-JS / C++ / Go /
      Java，Lua 首签在 G2 正条），G2 已勾选收口
- [x] G3 自定义模板加载（已交付）：`format = "template"` 进 CLI——
      `code_target_items` 开 `Option<CodeTargetItems>` Result 分支
      （七语言 `Some(Ok(…))` 原样，template 分支走
      `TemplateTargetGenerator::from_config` + `generate`），build 面
      板模板错走 `BuildFailure::Io`、gen 面板 eprintln + 退码 2，diff
      面板经 manifest 天然覆盖无需接线；`template_dir` 默认
      `.cage/templates/` 相对项目根解析（`options.template_dir` 覆盖），
      from_config 默认值同步收敛；Tera cause 链展平（`tera_err`），
      缺目录 / 空目录 / 语法错诊断指到模板文件路径；CLI 测试 5 新增
      （惯例目录 + options 覆盖 + build/manifest + 缺失两形态 + 语法错
      + 确定性），workspace 534 全绿
- [x] G4 过滤器库（已交付）：决策函数成库、模板里不写逻辑——语言
      无关层恒注册（kebab_case / SCREAMING_CASE 补齐 + field_order
      字段名序 + generate_with_setup 钩子），语言层各 crate pub
      register_filters（七语言 {lang}_type + 六语言 {lang}_default，
      枚举引用带官方分配标识符），CLI options.lang_filters 挂载
      （未知键渲染前报错），go/cpp allocate_names 零行为提取同源
      复用；测试 11 新增，workspace 545 全绿，过滤器表进 target.md
- [x] G5 文档收口（已交付）：target.md Template 小节随 G3/G4 落地
      （配置样例 / 渲染序约定 / IR 变量表 / 约定+语言过滤器两表 /
      错误口径）本签复查；需求整理.md 状态行 + 生成器清单 + crate
      口径补 template；architecture.md 结构树补 cage-target-template；
      README Target 插件行更新；design §22/§23 对账复查（G1–G4 全数
      已交付，命名约定与过滤器清单同步 G4 实况），G 系列收口

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
