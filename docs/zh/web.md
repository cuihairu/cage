
# Web UI / Schema Editor（第三阶段）

第三阶段第一项：为策划与程序提供 Schema 的 Web 编辑入口。红线不变：
不做 Excel 编辑器、不做游戏逻辑框架；Web UI 只做 Schema 的门面，不做
数据运行库。

## 定位

Cage 的 CLI 是编译链事实接口；Web UI 是**同一核心的另一个门面**：
编辑器不解析、不渲染 YAML，只与 core 交换文档并委托校验。所有编辑
语义（19 种可编辑字段类型——core 共 20 种，`TaggedUnion` 无专属编辑器、
原样保留——约束、引用、targets 可见性、表达式规则）都来自
`cage-core`，前端不做任何规则复制。

## 架构决策

### 交换模型 = 现有 serde Canonical 形状

编辑器文档就是 `Schema` 的 serde 形状，与 Configuration Snapshot 里
的 `schema.json`、HTTP API 同一文档，一份文档三处消费：

```text
snapshot/schema.json  ≡  cage-core::edit::to_editor_json  ≡  HTTP GET /api/schema
```

编辑器 JSON 保持插入序（表/字段按文档顺序），省略 `skip_serializing_if`
覆盖的可选字段，`default` 无 skip 属性故显式为 `null`（前端两者皆可编辑）。

### 保存 = 规范化 YAML 写回（单文件整写 / 多文件增量）

编辑器面向**合并后的规范 Schema**（cli 装载时把 schemas/ 下多文件合并
成一张 `Schema`，编辑器即增值这一形态），写回统一走 canonical YAML
（`to_canonical_yaml`）：同 Schema → 同字节（含唯一结尾换行），经
`from_canonical_yaml` 回环幂等。单文件 `schema_path` 整份覆盖（原子
替换：兄弟临时文件 + rename）。多文件（目录）`schema_path` 走增量写
回：`GET /api/schema` 附带 `sources` 来源索引（每份源文件的 sha256
revision + 表/枚举归属），保存信封 `{"schema": …, "sources": …}` 把索
引交回——冲突检查（任一文件磁盘 sha256 与装载时不符 → 409 拒写、
一字节不动）→ 只改写涉及变更表/枚举/元数据的源文件 → 原子替换，响
应回传保存后的新索引供客户端下一次保存复用。`registry:` 拆分仍不在
编辑器侧改。确定性契约：编辑器保存的 YAML 经 `cage build` 得到的产
物，与手写同一 Schema 构建的产物逐字节一致。生成器只见过 `Schema`
值，编辑回环只要保值的即满足。

### E1701 编辑态错误族

编辑文档无法反序列化为 `Schema`（语法/形状/未知 kind）报 **E1701**，
铺 JSON 路径（`tables.Item.fields.id.type`）定位到具体输入（前端的
高亮锚点）；schema 内部一致性（主键缺失、引用悬空）仍走 L1 的
`E1004`。两类错误分开，编辑器与校验器各自汇报。

### 无 node 前端工具链

前端是 `docs/public` 下静态单页（vanilla ES 模块），`cage web` 直接
伺服。编辑器链路不引入 Node/npm 构建链，cargo 一键即得（文档站自身
用 VitePress 构建，与编辑器无关）。

## 现状

### W1 已实装：编辑器交换模型（cage-core::edit）

- [x] `to_editor_json` / `from_editor_json`：Schema ↔ 编辑器 JSON；
      解析失败 → `E1701`（JSON 路径定位，根级错误归一为无路径）
- [x] `to_canonical_yaml` / `from_canonical_yaml`：确定性保存写回与回读
- [x] 测试：编辑器 JSON round-trip 确定性、canonical YAML 幂等
      （再渲染字节不变）、E1701 路径定位（含未知 kind / 缺必填成员 /
      根级错误 / YAML 语法错误）、与 snapshot schema.json 形状一致；
      `E1701` 入错误码表（crates/cage-core/src/error/codes.rs
      `editor` 族 + 校验文档）

### W2 已实装：`cage web` 本地 HTTP 服务（cage-cli/src/web/）

- [x] 项目装载复用 CLI 加载链（`load_project_config` / `load_schema`），
      只绑定 `127.0.0.1`，无鉴权（本地工具）
- [x] 请求边界与资源上限（2026-10-19 补记）：Host 必须指名 loopback
      主机（`127.0.0.1` / `localhost` / `[::1]`，端口不参与匹配，缺失
      Host 一律拒），POST 带 `Origin` 时必须是 loopback 源——违者 403
      （防 DNS rebinding 与跨站写）；请求体上限 8 MiB（413）、30 秒
      wall-clock 读取预算（408，vendor 版 tiny_http 连接级 1s idle
      timeout 兜底单次滞留读）、不可读体 400
- [x] `GET /api/schema`：合并 Schema 的编辑器文档 + 项目事实
      （project / schema_path / profile_names / warnings_as_errors）；
      每请求重载，保存后立即可见
- [x] `POST /api/validate`：编辑态校验。E1701（文档无法反序列化，
      JSON 路径定位）→ L1 一致性（E1004 族）；诊断以裸数组输出
      （`Diagnostics` serde 的 `{"items": [...]}` 壳在 API 层摊平）
- [x] `POST /api/schema`：canonical YAML 写回。单文件 `schema_path`
      直接覆盖（原子替换）；目录（多文件 schema）凭 `sources` 索引
      增量写回：sha256 冲突检查（过期索引 → 409 拒写）、只改涉及
      变更的源文件、响应回传新索引；裸文档（无索引）仍 409；
      `registry:` 开头的 `schema_path` 同样 409（已发布
      entry 的 schema 不在编辑器改，须回发布方工程重发布）；未配置 →
      写 `schema.yaml` + note 提醒接线
      cage.toml（服务器绝不改配置文件）
- [x] `POST /api/schema/preview`：保存预演。同 body、同拒绝与冲突检查，
      但什么都不写；应答列出保存将改写的每个文件及其 unified diff
      （`files: [{file, action: "write"|"create", diff}]`，另有
      `unchanged`/`bytes`）。编辑器保存前先调它并弹确认对话框，
      作者没见过 diff 之前绝不落盘
- [x] `POST /api/schema/rename`：安全重命名（P0-4）。body
      `{"schema": <编辑文档>, "target": {"kind": "table"|"field"|"enum", …},
      "new_name": "…"}`；改名在 `cage_core::refactor`（Rust Core，非 JS）
      执行——唯一一处知道名字出现的每个站点：表/字段/枚举的映射键**与**
      实体自身 `name` 成员、主键项、`unique_constraints[].fields`、`order_by`、
      各环境覆盖键、跨表 `reference.table`/`reference.field`、类型树里任意
      深度的 `Enum(name)`、规则表达式的字段操作数（属主表的行规则 +
      过滤被引用表行的 `reference.predicate`）。应答返回改名后的文档 +
      改动清单（`changes: [{path, kind, before, after}]`，path 即编辑器
      `data-path` 锚点；`notes` 点名一处改名碰不了的站点——如解析不了的
      规则表达式，按原文逐字节保留）。重名目标、幽灵目标、非标识符新名
      （`1bad`）→ 400 带原因且文档不动；文档无法反序列化 → 200 带 E1701。
      不写盘：编辑器先看清单、作者确认后草稿才采纳。文档顺序保留（改名
      保持索引位，canonical YAML 不因改名乱序）
- [x] `POST /api/validate/data`：草稿 Schema 完整预检（P0-3）。编辑文档
      顶替磁盘 schema，载入项目真实数据，走 `cage check` 完全一致的
      `load → validate_with_profile` 管线——编辑器能看到一份还没保存的
      schema 对数据造成的影响。编辑态口径（E1701 解析 + 结构
      `Schema::validate` lint）与数据口径合并为一个诊断列表；每条带
      `scope`——`"data"`（真实数据上的全管线）或 `"schema"`（纯结构），
      两侧都报的发现只出现一次并保留 `data` 侧。旋钮同 `/api/status`
      （`?profile=`/`?env=`/`?level=`），默认与 `cage check` 一致：首个
      配置 profile、无环境、级别 `semantic`（上面的快 `POST /api/validate`
      保持编辑态——数据载入不能挂在编辑器 700ms 自动重校验上）
- [x] 集成测试（crates/cage-cli/tests/web/）：真实子进程 + 原始
      TCP 请求，覆盖文档伺服 / E1701+E1004 校验 / 保存后重载与
      `cage check` 复跑 / 目录拒写 / 空 schema 起始 /
      保存信封过 HTTP（只重写变更片段、未动片段字节不变、过期索引 →
      409）/ 保存预览 / 草稿预检（干净草稿对真实数据；一份同时破坏数据
      与结构的草稿——一条合并列表、每侧一个 scope 徽标）/
      安全重命名（一个字段跨主键、行规则、跨表引用与 predicate
      改名——返回文档每个站点都已补、磁盘逐字节不动、重名拒绝）/
      P0-6 验收闭环（打开 Monster → 加字段 → 改名 → 关联 Items →
      范围规则 → Excel 数据验证精确到违规行 → 修后 diff 预览 →
      增量保存回源文件；全程不手编 YAML，且 CLI `check` 对同一份文档
      报出与 Web 相同的单一违规、同一 Excel 行——见
      `p0_acceptance.rs`）

### W3 已实装：Schema 编辑器前端（docs/public/editor/，编译期嵌入）

- [x] 静态单页（vanilla ES module，零依赖、无 node 构建链）：三栏——
      左表/枚举树、中实体表单、右诊断面板；`cage web` 编译期把
      index.html / app.js / app.css 嵌入二进制直接伺服（docs/public 是
      唯一作者副本，文档站亦以 /editor/ 发布），单文件 cage 自带编辑器
- [x] 表/枚举编辑：增删、保序重命名（fields/tables/enums 键随改）、
      主键/排序/targets/唯一约束、枚举取值（名称/字面值/描述）
- [x] 19 字段类型：递归类型编辑器（Array 元素 / Map key_type+值 /
      Object 属性 / Enum 引用 datalist）；类型切换重建载荷
      （Array/Object/Map 保留兼容内层）；嵌套深度上限 5
- [x] 约束编辑按类型显示：数值 min/max、字符串 min_length/max_length/
      pattern/白名单、数组 min_items/max_items、默认值（JSON 字面量）、
      required、引用（目标表/字段/基数 one|many|optional/兼容 profile）、
      表达式规则（name/assert/message/warning_only）、自定义元数据
      （flatten 额外键，JSON 对象编辑，预留键拒绝覆盖）
- [x] E 码诊断面板：校验按钮 / Ctrl+Enter → `POST /api/validate`；按
      严重级配色计数；点诊断 → 定位字段（`data-path` 精确锚点 +
      滚动高亮）；保存被拒（E1701）诊断同样入面板
- [x] 完整预检：「完整预检」按钮 / Ctrl+Shift+Enter → `POST /api/validate/data`；
      合并列表每条打「结构」（`scope: "schema"`）或「数据」（`scope: "data"`）徽标；
      点数据类诊断 → 选中该表与该列，编辑态诊断仍走 JSON 路径锚点
- [x] 保存：Ctrl/Cmd+S 或保存按钮 → 先 `POST /api/schema/preview`，
      确认对话框展示逐文件 unified diff（服务端算出、不写盘），确认后
      才 `POST /api/schema`（GET 交付过 `sources` 索引时随信封交回，
      启用多文件增量保存）；未保存标记 + beforeunload 拦截；409（过期
      索引或 `registry:` schema_path）/ 未接线 note / 500 均以错误 toast 呈现
- [x] 外科手术式文档编辑：未知/遗留键原样保留，只改可控键，与
      「编辑器 JSON ≡ canonical 形状」的交换契约一致；保存全程不经
      前端渲染 YAML
- [x] 集成测试：GET / 返回嵌入编辑器页（含模块/样式引用）、/app.js、
      /app.css 伺服与 404；测试原始客户端补 chunked 解码
      （tiny_http 大响应走 chunked，分块边界可切开 UTF-8 序列）

### W4 已实装：编辑流程定稿 + 示例工程冒烟 + CI

- [x] `examples/web-demo/`：单文件 schema 冒烟工程（schema_path 指向
      schema.yaml；`[source_roots] main = "config"` 让示例表立即可
      check/build）。「浏览器打开 → 编辑 → 校验 → 保存 → `cage build`
      复现」整条路径的最小闭环
- [x] `examples/web-smoke.sh`：端到端冒烟脚本，在临时拷贝上运行（不脏
      仓库），真实命令逐条回显；任一步失败即 exit 非零。七步：基线
      check；编辑器会话（GET 文档 → 加 `stack_size` 字段（UInt8 /
      min 1 / default 1）→ `/api/validate` 零诊断 → `/api/schema` 保存）；
      断言 schema.yaml 落盘 canonical YAML 且含新字段；`cage check` +
      `cage build` 全链路复现；状态页会话（U1，`/status/` 页面 →
      `/api/status` 断言 client fresh → `POST /api/build` 断言
      up-to-date）；同一 schema 两次重建产物树哈希逐字节
      一致（确定性契约）；对 `examples/game-config`（schemas/ 目录）
      断言 `POST /api/schema` 得 409 拒写（多文件是作者侧组织，编辑器
      不重写）
- [x] CI 集成：`web-smoke` job 镜像 example job（checkout + toolchain +
      rust-cache + `bash examples/web-smoke.sh`）

### W5 已实装：项目状态面（U1，路线见 UI 门面）

- [x] `GET /api/status[?profile=&env=&level=]`：一份文档三块——项目
      事实（name / schema_path / output_dir / profile 列表）、校验面板
      （**与 `cage check` / `cage lsp` 共用 `validate_project` 单一入口**
      ：计数 + 全量诊断 + `visible_tables`，页面、CLI、LSP 三处诊断
      永不分歧）、每 profile 构建事实（freshness = `build_project`
      第一层比对——同 profile、同环境、同 schema/source/target 指纹、
      产物完整——**报告而不执行**；`changed_tables` = 第二层的逐表
      `table_hashes` 比对，告诉用户「哪几张表改了还没重建」；manifest
      单槽，`last_build_profile` 标明当前归属）
- [x] `POST /api/build`：触发构建，与 `cage build` 同一 `build_project`
      管线（完整校验 + 三层增量 + 确定性 manifest）。校验失败 →
      `{"ok": false, "diagnostics": [...]}`（/api/validate 同形状，页面
      直接渲染进面板）；已是最新 → `up_to_date: true` 不动任何字节；
      未知 profile / I/O → 400 + error
- [x] `/status/` 状态页（docs/public/status/，编译期嵌入，vanilla
      ES module 零依赖，与编辑器同一深色主题）：校验面板（级别 /
      环境 / profile 三旋钮）+ 构建卡片（新鲜度徽标、变化表 chips、
      target 清单、build_id / content_hash 前 12 位、构建按钮）；
      裸 `/status` 301 到 `/status/`（相对资源引用不撞编辑器的
      /app.js）；编辑器顶栏加「项目状态 →」互链
- [x] 测试：web/tests.rs 单测 6 项（状态默认 profile / 未知 profile 400 /
      构建后 fresh 与 up-to-date / 改源后 `changed_tables` 命名 /
      坏数据构建被拒带诊断 / 未知 profile 构建拒绝）+ 集成 3 项
      （状态页路由与 301 / 状态-构建端到端 / 坏数据端到端）+
      web-smoke 第 5 步（真实服务上状态页 → fresh → up-to-date）

### W6 已实装：产物浏览（U2a，路线见 UI 门面）

- [x] `GET /api/artifacts[?profile=&env=]`：产物账本 = build manifest。
      manifest 单槽，**跨 profile 不读遗留**——manifest 属于别的
      profile 时返回 `manifest_profile` + 空清单（未构建的 profile 没有
      可信产物，构建即可浏览）；`stale` 复用 U1 的第一层判语
      （产物照列，但页面标注「已过期」）；每个条目带
      path / blake3 hash / size / format / 所属表 / encoding，前端按
      `output_dir` 分组即天然按 profile 分层
- [x] `GET /api/artifact?path=REL`：单文件伺服给预览/下载。安全口径
      两条：① **只认 manifest 键**（查表，不 join-and-pray，路径穿越
      一并拒绝）；② 伺服前**重算 blake3 比对记录哈希**，不一致 409
      ——与 snapshot loader 同一 verify-first 姿态，绝不送出被改过的字节
- [x] 状态页「产物」区：按 target `output_dir` 分组列出文件（路径 /
      大小 / hash 前缀 / 预览 / 下载），预览对 JSON 格式化、文本原样、
      msgpack 标「二进制不预渲染」；未构建/跨 profile 出说明而非空壳；
      过期产物出过期提示
- [x] 测试：web/tests.rs 单测 3 项（清单跨 profile 与空态 / 伺服 + 非账本
      404 + 篡改 409 / stale 标注）+ 集成 1 项（列表 → 逐字节伺服 →
      404 → 409 端到端）+ web-smoke 第 5 步补产物断言

### W7 已实装：targets 徽标 + profile 作用域视图（U2b，路线见 UI 门面）

- [x] `GET /api/schema?profile=NAME`：附 `projection`（`filter_by_profile`
      直调——构建管线同一投影函数，未知 profile 400）。前端语义零复制：
      只做「全量 − 投影」集合差得到隐藏集，可见性规则不进 JS
- [x] targets 徽标：`field.targets` / `table.targets` 非空即在编辑器渲染
      chip（左栏表项 / 字段行 / 表标题），数据源就是 schema 本身，无新状态
- [x] 作用域视图：编辑器顶栏「作用域」旋钮（（全部）/ 各 profile）；
      被投影剔除的表/字段**灰显 + 「本 profile 不可见」徽标**而非消失——
      藏起来的东西要看得见被藏了；投影按已保存 schema 计算，未保存改动
      时提示语显式标注（保存全程作者主导，不自动保存）
- [x] 测试：web/tests.rs 单测 +2（投影 gate 表与字段 / 未知 profile 400）+
      集成 1 项（无 profile 无 projection → client 剔除 server-only 字段
      → server 保留 → ghost 400）+ web-smoke 第 7 步（game-config 的
      `internal_note`：client 投影剔除、server 保留、ghost 400）

### W8 已实装：快照 / 分层 diff / registry 浏览（U3，路线见 UI 门面）

- [x] `GET /api/snapshots`：列 `<output_dir>/snapshot/` 下全部包，
      每包**现场** `verify_snapshot`——列表绝不宣称它没刚验过的包是
      完好的；另附 manifest 事实（profile / env / build_id /
      content_hash / 产物数，读不出 manifest 的包仍列，标注坏）
- [x] `POST /api/snapshot`：与 `cage snapshot` 同一管线（完整
      `build_project` → `pack_snapshot`，同参数含环境）；校验失败
      200 + 诊断（与构建面板同一合并逻辑），Io 400
- [x] `GET /api/diff?from=&to=`：两个 manifest 的语义分层 diff——
      `changed_tables` / `affected_targets` / `files:{added,removed,
      changed}`（每条带 table 与 target 归属）/ `roots`（content_hash
      与 build_id 的 from→to）。table/target 元数据随文件条目走，
      界面三层折叠（表 → target → 文件）就是纯分组；不做 Merkle 化，
      卡点（出现「只拉变化子树」的增量同步需求）见 UI 门面
- [x] `GET /api/registry`：浏览 `[registry] path` 下的包与版本
      （cage_core registry 同一入口）；未配置出 ok + 说明而非报错
- [x] 状态页：新增「快照」（自验徽标 + 打包钮）、「对比」（基线/
      目标旋钮：live 构建与各快照；三层折叠，逐字节一致出判语）、
      「Registry」三个区
- [x] CLI 对齐：`cage snapshot --list`（列包 + 每包自验判语，与
      `--verify`/`--env` 互斥）；`cage diff` 增「tables changed /
      targets affected」分层摘要，与状态页同数据源（manifest
      `table_hashes` 与产物路径）
- [x] 测试：web/tests.rs 单测 +3（快照打包/列表自验/坏体 400 / diff 分层
      与缺参、未知基线 400 / registry 未配置说明 + 已配置浏览）+
      `cage snapshot` CLI 测试（--list 列包与篡改判语、--list 与
      --verify/--env 冲突）+ 集成 1 项（registry 未配置 → 打包 →
      列表 → 跨快照 diff → ghost 400）+ web-smoke 第 5 步（打包 →
      自验 → 零差异 diff → registry 空态）

### W9 已实装：数据面板（策划界面 P0，2026-10-20，路线见 UI 门面）

- [x] `GET /api/tables[?profile=&env=]`：仓内数据表清单（每表
      rows / source_file / sheet / primary_key；主键取自 schema
      定义，JSON 加载路径的 `Table.primary_key_fields` 为空不作为
      数据源）；profile 未指明取第一个已配置项，未配置 500，
      未知 profile 400
- [x] `GET /api/table?name=NAME[&profile=&env=&offset=&limit=]`：
      分页行窗（limit 默认 200、上限 1000、offset 归一），每行
      index / pk / 按列 cells；总 JSON 映射全值域（bytes 出
      `{"bytes": n}` 标记对象而非失败整请求）；ghost 表 404，
      缺 name 400
- [x] 状态页「数据」区：左侧表清单（行数/来源/sheet），右侧表格
      视图（粘性表头 + 分页旋钮）；诊断面板新增 severity / 错误码
      过滤（过滤后计数显示 m/n）；带数据点位的诊断可点击 → 直接
      跳到对应表行高亮（jumpToDiag：pk 匹配 → 换页 → 滚动高亮）
- [x] 校验本体零新增：数据面板消费的仍是 `/api/status` 同一份
      `validate_project` 诊断（check / lsp / web / 状态页同一入口）
- [x] 测试：web/tests.rs 单测 +2（表清单事实与 ghost 400 / 分页窗口与
      隐藏表 404）+ 集成 +1（表清单 → 行窗 → ghost 404 → 页面
      引用断言）+ web-smoke 第 5 步并入数据面板断言（/api/tables
      → /api/table → ghost 404）

### W10 已实装：编辑即时重校验 + 产物一键导出（策划界面 P1，2026-10-20）

- [x] 编辑器顶栏「即时重校验」开关（默认开）：每次改动后 700ms
      防抖静默重校验（只更新诊断面板与计数，不弹 toast），手动
      「校验」/Ctrl+Enter 照旧且会取消未发的自动校验；规则表单本体
      复用 W3（零新编辑面）
- [x] 状态页产物卡「一键导出」钮（多文件 target 才出现）：逐个触发
      `/api/artifact` 下载，每文件各自先验 blake3 再发；已代决不新增
      zip/打包端点（产物字节不重算、零新依赖，批量分发走
      `cage snapshot` 包），拍板记录见 UI 门面
- [x] 测试：集成断言（编辑器页含开关与 `scheduleAutoValidate`、状态
      页导出走 `/api/artifact`）+ 两页脚本 `node --check`；P1 零服务端
      新面，web 单测/集成数量不变

### W11 已实装：诊断对比 + 字段模板库（策划界面 P2，2026-10-20）

- [x] 零新端点：本节全部为既有页面的前端增量，校验/产物数据源不变
- [x] 校验面板诊断对比：会话内两次运行的新增/消失（摘要「较上次
      新增 n / 消失 m」+ 新诊断「新」徽标）；同 profile+env+level 才
      比对，构建被拒时换入的诊断也计入——覆盖「改数据 → 构建 →
      看多了什么错」主路径。跨快照历史诊断不做（需写入快照包，属
      新格式面；产物对比已有 `/api/diff`，见 W8）
- [x] 编辑器字段模板库：「从模板加字段」下拉（数值范围 Int+min/max、
      枚举引用、外键引用 reference），生成草稿走既有保存/校验链
      （与 W10 即时重校验联动），服务端零新面
- [x] 测试：集成断言（`FIELD_TEMPLATES` / 外键引用、`markDiagDiff` /
      `diag-new`）+ 两页脚本 `node --check`；web 单测 73 绿 / 集成
      15 绿不变

### W12 已实装：表源文件上传替换（策划界面 P0，2026-10-21，路线见 UI 门面）

- [x] `POST /api/upload?name=NAME[&profile=&env=]`：原始字节体 = 该表
      登记源文件的整文件替换。安全策略已代决（docs/ui.md 上传落地
      记录）：只写登记源文件——`resolve_upload_target` 在
      canonicalize 后双守卫（项目根内 + 至少一个 `source_roots` 内，
      symlink 逃逸同拒）；tmp + rename 原子落盘，原文件先挪旁
- [x] 校验与回滚：换入后走同一 `validate_project` 全量校验（单一入口
      纪律不变），有错误或装载失败即恢复原文件再应答——被拒上传
      磁盘上字节不落地；恢复失败时备份原文件保留并报错点名路径
      （UploadSwap 守卫 disarm），不静默丢数据
- [x] 拒绝面：多文件分片表直接 400（单文件替换无法不丢行）；空体
      400、ghost 404、缺 name 400、未在 source_roots 内 400；大小走
      既有 8 MiB 请求体上限（非 UTF-8 只允许 upload 路由，其余
      String 路由保持旧「cannot read request body」拒绝）
- [x] 状态页「数据」区：「上传替换…」钮（选中表后出现，标题带源
      文件路径）→ confirm → 选文件 → 原始字节 POST → 成功/被拒
      toast → 刷新校验面板与表视图
- [x] 测试：web 单测 +5（接受落盘无残留 / 坏数据拒绝字节原样 / 不可
      解析拒绝 / ghost 缺参 / 路径双守卫含 symlink 逃逸）+ 集成 +1
      （替换 → 面板可见 → 拒绝回滚 → 无 temp 残留）+ web-smoke 第 5
      步并入上传往返（接受 → 面板见新行 → 坏数据拒 → 原文件还原）

### W13 已实装：三层编辑模式（评审 P1-1，2026-11-21）

- [x] 基础/规则/专家三层编辑模式共享一份 Schema 语义模型：顶栏模式切换，
      三层读写同一份 `state.doc`、同一个 `field.rules`（ExpressionRule）
      数组。「禁三套规则格式」的契约落在可 node --test 的纯模块
      `modes.js`（`MODES` / `isMode` / `applyExpertText` / `ruleFormat` /
      `addRule`），作为独立嵌入资产伺服
- [x] 基础模式 = 既有字段表单（字段、类型、默认值、范围、枚举、引用——
      无代码），不变
- [x] 规则模式 = 规则专用视图，编辑同一 `field.rules` 数组（复用表达式
      规则编辑器），表级概览带字段规则入口；可视化条件构建器（P1-2）
      与九项模板（P1-3）在此层接入
- [x] 专家模式 = 原始文档编辑（Schema 语义模型 serde 形状的 JSON
      textarea）；编辑器不解析、不渲染 YAML，保存仍写 canonical YAML。
      「应用」原子（`applyExpertText`）：解析失败现有文档一字节不动，
      结构问题由「校验」（E1701）报告
- [x] 模式选择持久化于 `localStorage`（`cage-editor-mode`），载入时恢复；
      专家整文档改写后切回基础/规则时清理幽灵选中
- [x] 测试：`modes.test.mjs`（node --test 10 项——回环、原子拒绝、禁三套
      规则格式契约、addRule 顺序）+ 集成断言（页面与 `/modes.js` 资产）；
      两页脚本 `node --check`；web 单测 80 绿 / 集成 19 绿；web-smoke 绿

### W14 已实装：结构化规则 AST + 可视化条件构建器（评审 P1-2，2026-11-22）

- [x] `cage_core::rule_ast`——创作期 AST（`Compare{field, op, rhs}` /
      `And{items}` / `Unparseable{assert}`，rhs = 字面量或同行字段）。它只是
      创作中间表示：`compile` 把它**向下**编译成 Schema 的唯一规则表示
      （`field.rules` 上的 `ExpressionRule` 数组——`And` 逐项成规则，Schema
      对字段 rules 取 AND），`decompile` 把断言解析回构建器的行。Schema
      永不存储 AST，W13 的「禁三套规则格式」成立
- [x] 表达能力边界是拒绝而不是假装：引擎渲染不了的字面量（数组/对象/
      null、同时含两种引号的字符串、超出 `i64` 的 `UInt`）以
      `CompileError::Unsupported` 拒绝并携带人话原因——编译端点应答
      `{ok: false, unsupported: true, error}`，编辑器以边界提示呈现
      （P1-5 使其更醒目）
- [x] Web API：`POST /api/rule/compile`（AST + `RuleMeta` →
      `ExpressionRule[]`）与 `POST /api/rule/decompile`（`ExpressionRule` →
      AST，引擎解析不了的断言得 `Unparseable`）
- [x] 可视化条件构建器位于编辑器规则模式，在表达式规则编辑器之上：行即
      `字段 OP 字面量/字段`（操作符 `== != < <= > >=`，跨字段比较从同表
      选另一字段），带规则名/提示消息/仅警告元数据与「替换全部/追加」
      写入方式。构建器只产出 AST（`builderRowsToAst` 纯函数 node 可测；
      `parseLiteralInput`——`200` 即数字、`hi` 即字符串）；断言文本一律由
      Rust Core 渲染。「从既有规则装载」把字段既有规则反编译回行
      （无法可视化的跳过并提示）
- [x] 测试：rule_ast 单测 8（compile/decompile 回环、`And` 命名、不可
      表示字面量、解析不了断言）+ web 单测 3（端点形状含 unsupported
      拒绝）+ 集成 1（真实 HTTP：两行 `And` → 两条带后缀命名的规则、
      反编译回行、unsupported 应答）+ `modes.test.mjs` 12 绿（两个新纯
      函数）；workspace 1290 全绿；web-smoke 绿

### W15 已实装：内置规则模板九项（评审 P1-3，2026-11-22）

- [x] `rule_templates.js`——纯逻辑、node 可测的九项模板目录（主键唯一 /
      数值范围 / 枚举选择 / 外键关系 / 字段比较 / 条件必填 / 数值成长 /
      跨表总量 / 配置可达性），每项带底层实现与面向策划的提示
- [x] `planTemplate` 只把规则落到 Schema 的既有表示——绝不引入新的规则
      格式。可表达的五项产出 path 补丁（primary_key、min/max、枚举类型、
      reference）或 AST（字段比较，经 `/api/rule/compile` 编译进唯一的
      `field.rules` 数组）；前端仍不手写 assert
- [x] 表达式引擎表达不了的四项（条件必填 / 数值成长 / 跨表总量 / 配置
      可达性）**不假装能自动转换**：应答 `{kind:"boundary"}` 并引导
      Python 扩展（P2-2）或专门 Rust 校验器（P3-2），规则模式以醒目
      提示呈现（P1-5 使其更醒目）
- [x] 规则模式带模板库区（选模板 → 小参数表单 → 应用）。参数表单的选项
      取自文档本身（字段/枚举/表下拉），模板不会误配到不存在的字段
- [x] 测试：`rule_templates.test.mjs`（node --test 8 项——目录 id/engine/
      boundary 旗标、各补丁形状、boundary 拒绝、自比较与未知字段报错）
      + 集成资产断言（模块带 `RULE_TEMPLATES` / `planTemplate` 伺服、
      forms.js 已接线）+ 改动脚本 `node --check`；web 集成 20 绿；
      workspace 全绿；web-smoke 绿

### W16 已实装：规则人话层（评审 P1-4，2026-11-22）

- [x] `ExpressionRule` 增补可选的人话层字段：`title`（中文标题）、
      `description`（业务说明）、`examples`（正反例）。错误信息原本就有
      （`message`）。三者省略时不序列化——老 Schema 一字节不变（测试钉住），
      编辑器文档（`to_editor_json`）原样透传——任何地方都没有第二套规则
      格式
- [x] 正反例是可执行的：`cage_core::rule_examples::check_examples` 用与
      校验主链同一套表达式引擎试算（沿用 L6 语义：缺/为 null 的字段视为
      通过、类型不可比视为违反），逐例回报「期望 vs 实际」，不符时带人话
      备注。纯提示性——不改文档、不改数据
- [x] Web API：`POST /api/rule/examples`（规则 → 每例一个判定：
      `{index, expected, actual, ok, note}`）
- [x] 编辑器（基础模式字段表单与规则模式共用）在同一个表达式规则编辑器
      里编辑人话层：中文标题 / 业务说明输入、每条规则的正反例子编辑器
      （样例行 JSON、期望 pass/violate、备注）与「试例」按钮——POST 规则，
      显示「试算：N/M 例符合期望」并逐例给判定
- [x] 可视化构建器（P1-2）的 `RuleMeta` 携带中文标题 / 业务说明（编译到
      每条产出规则），替换写入时把既有规则中同 assert 的人话层合并回来
      （`mergeHumanLayer`，纯函数 node 可测）——一次无关的重编译不丢策划
      填写的内容。「从既有规则装载」从被载规则带上人话层
- [x] 测试：rule_examples 单测 8（引擎判定含不符备注、缺字段按通过、样例
      形状非法、断言解析不了、serde 回环、无人话层时一字节不写）+ web
      单测 1（端点形状）+ 集成 1（真实 HTTP 判定）+ modes.test.mjs 15
      （mergeHumanLayer 三个用例）；workspace 全绿；web-smoke 绿

### W17 已实装：表达能力边界提示（评审 P1-5，2026-11-22）

- [x] 所有被拒路径共用同一份诚实的边界提示：可视化构建器编译出引擎表达
      不了的规则、或模板落在 `{kind:"boundary"}`（或其 AST 编译被拒）时，
      编辑器显示 `renderBoundaryNotice(reason)`——`⚠` 标题点名具体原因、
      稳定的 `EXPRESSIVENESS_LIMITS` 限制清单、两条诚实出路（内置规则模板 /
      Python 插件 P2-2 或专门 Rust 校验器 P3-2）。绝不假装能自动把规则转成
      `assert`（评审 141 行）
- [x] `boundaryGuidance(reason)` 与 `EXPRESSIVENESS_LIMITS` 放在
      `rule_templates.js`（纯数据、node 可测、无 DOM），构建器（`boundaryBox`）
      与模板区（`note`）都经 `forms.js` 的 `renderBoundaryNotice` 复用——单一
      事实源，不再有三处近似的重复文案
- [x] 测试：`rule_templates.test.mjs` 增 2 项（提示标题点名具体原因、不假装
      能转换；限制清单钉住）——共 10 项；workspace 全绿；web-smoke 绿

## 路线图

第三阶段 W 系列与后续 UI 增量至此全部完成（W1 交换模型 → W2 本地服务
→ W3 编辑器前端 → W4 文档与示例 → W5 项目状态面 → W6 产物浏览 →
W7 作用域视图 → W8 快照 / 分层 diff / registry 浏览；策划界面
增量见 UI 门面（数据面板 W9 已落）。
示例入口：`examples/web-smoke.sh`（CI 同款）。后续 UI 增量的路线与
拍板见 UI 门面。
