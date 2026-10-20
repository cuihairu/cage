
# UI 门面：Web 增强（设计拍板与路线）

用户反馈：cage 用起来体感不好，想要界面。本篇拍板四件事：形态选型、
界面覆盖面、变更可视化口径、双端差异表达。定位不变（web.md）：Web UI
是**同一核心的另一个门面**，前端不复制任何规则语义；零新依赖、单二进制、
无 node 编辑器链路三条纪律全部延续。

## 实测基线（2026-10-12 拍板时点）

- release exe **10.6 MiB**（lto + strip + opt-level=z）。
- 编辑器页 48 KB（index.html 2.0 / app.js 37.6 / app.css 8.4 KB），
  `include_str!` 编译期嵌入（web.rs），单二进制自带界面。
- 现有 HTTP 面共 6 条：`GET /`、`/app.js`、`/app.css`、
  `GET /api/schema`、`POST /api/validate`、`POST /api/schema`；
  仅绑 `127.0.0.1`，无鉴权（本地工具定位）。
- manifest 哈希双层：`table_hashes` 逐表 blake3 行指纹 → 逐目标规范
  指纹（按 `(format, output_dir)` 排序）→ 逐文件 hash → 根
  `content_hash`；`build_id = blake3(profile + schema_hash +
  source_hash + content_hash)`（输入含 profile）。
- 增量构建已经消费这层哈希：`prev/cur table_hashes` 逐表比对 +
  依赖图传播决定跳过集（main.rs）。
- `cage diff` 现状：两份 manifest 的 `artifacts` 平面对比
  （`+/-/~` + 计数 + `content_hash` 行）；schema 级 diff 另有
  `migrate::diff_schemas`。

## 形态拍板：c（现有 web 增强）并吸收 d，a / b 转备选

| 维度 | a) Tauri 桌面壳 | b) 纯 Rust GUI（egui/iced） | c) cage web 增强 | d) 纯 CLI 打磨 |
|---|---|---|---|---|
| 包体 | 壳再增 ~8–15 MB（估，系统 WebView） | 增 ~10–20 MB（估，GUI 栈进 exe） | 嵌入字符串 + 薄 JSON API，约 ±0.3 MB，整体仍 ~11 MiB | 不变 |
| 发布渠道 / 打包 | NSIS/MSI 或便携 exe；Linux 无法交叉产出 Tauri exe，**必须上 Windows runner 打包**，nightly zip 结构要改 | Linux 交叉出 MSVC exe 需新工具链（cargo-xwin 一类），CI 新增面大 | **完全不变**：同一 exe、同一 nightly zip + SHA256SUMS、install.sh/ps1 原样 | 不变 |
| cage-core 复用 | 进程内直连 | 进程内直连 | 进程内直连（W2 交换模型已通） | 直连 |
| 脱离终端 | 桌面窗口 | 可以 | 浏览器——且天然跨机器：Linux 服务器跑 `cage web`，Windows 浏览器打开 | 不能 |
| 维护成本 | 高：巨型依赖树（破零新依赖）+ WebView2/webkit2gtk 双端怪癖 + 打包流水线 | 高：富数据视图（表格 / 树 / diff）在 immediate-mode 全是手写 | **最低**：vanilla JS 无构建链，全部复用 W1–W4 存量 | 最低，但不解决「要界面」 |

**定了什么**：界面形态 = 方向 c——增强现有 `cage web`（W2 本地服务 +
W3 编辑器页的直接延伸），把 manifest / 产物 / snapshot / registry /
diff 这些**既有数据**接上门面；方向 d 不单独立项，作为纪律吸收（见
「d 的吸收口径」）。主要使用拓扑是 Linux 服务器（跑构建）+ Windows
客户端（看结果、改数据），产物必须能出 exe。

**为什么**：① 拓扑匹配——浏览器是唯一零打包成本横跨「无头服务器 +
Windows 桌面」的面，桌面壳在服务器上开不了窗口；② 可发版——产物仍
是那一个 exe，nightly 滚动通道照常带出去（每日构建属正常交付面），
不触碰版本发布红线，CI 不加 Windows 打包面；③ 存量最高——W1 交换
模型、W2 本地服务、W3 编辑器、`/api/schema`、`POST /api/validate`、
E1701/E1004 全部已在，缺的只是门面面积，不是地基。

**备选**：a) Tauri 壳（复用最多 UI 资产但代价是整条打包流水线 +
依赖树爆炸，且服务器场景无效）；b) egui/iced 单二进制 GUI（依赖树
可控些但富数据视图全手写、工作最大，同样在无头服务器上是死路）——
两者转备选不实现。

**边界**：不做游戏逻辑框架（红线不变）；不做多用户 / 远程团队协作
（web 是单机本地工具）；不做编辑器之外的独立前端构建链（vanilla
ES module 纪律不变，仍编译期嵌入）。

**卡点条件（a/b 重议的唯一理由）**：出现「必须脱离浏览器」的真实
使用场景（完全离线桌面环境、无浏览器可用），或界面面积大到嵌入
字符串不可维护时，重启形态评估。

**远程访问口径（拍板）**：保持仅绑 `127.0.0.1`、无鉴权**不动**；
服务器 → Windows 客户端的远程场景走 SSH 端口转发（文档写清用法即
可）。`--bind` / 局域网暴露必须先有鉴权（registry 已有 `--auth-env`
先例可循），登记为卡点条件，本期不做——本地无鉴权服务暴露到局域网
是安全倒退，不为便利破例。

**d 的吸收口径（不单独立项）**：CLI 可读性本就是一等公民（E 码 +
定位 + hint），吸收为两条纪律：① 新界面端点落地时，同参数语义同步
CLI 输出（文本面与界面共用同一核心函数，单一入口纪律，同
`validate_project` 先例）；② 任何增量触碰到的 CLI 输出，按
「错误可读 + 汇总可读」标准过一遍。唯一显式交付：U3 落地时
`cage diff` 文本输出同步获得分层摘要（见变更可视化节）。

## 界面覆盖面（P0 → P1 → P2）

端点纪律：每个新面 = 一个薄 JSON 端点 + 前端接线。端点全部是对
cage-core 既有函数的直调，延续 W2 交换模型；服务端不持状态，每请求
重载落盘现状（与 `/api/schema` 一致）。前端不新增构建链，仍是
`docs/public/editor/` 单页扩展或平级静态页，编译期嵌入。

### P0（第一批，全是现成数据接线）

- **项目与构建状态**：profile 列表、校验状态（与 `check` 共用
  `validate_project` 单一入口，界面诊断与 CI 诊断永不分歧）、构建
  新鲜度（当前源 `table_hashes` vs 上次 manifest 里的旧值——增量
  构建本来就在比对的数字，直接告诉用户「哪几张表改过还没重建」）、
  最近一次构建事实（profile、build_id、content_hash、target 数）。
- **校验面板**：诊断表（code / 严重级 / 文件-Sheet-单元格定位 /
  hint），数据源 `POST /api/validate`，与 LSP / CLI 三处消费同一份
  `Diagnostics`。

### P1（第二批）

- **profile × target 产物浏览**：profile 选择器 → target 列表 →
  `manifest.artifacts` 树（每文件 hash / 大小 / 在线预览 / 下载）；
  按 `output_dir` 分组即天然按 profile 分层（build/client/*、
  build/server/*）。
- **schema 编辑器补齐**：现有 W3 编辑器挂载 E1701/E1004 诊断已通，
  补字段级 `targets` 徽标等双端可见性表达（见下文专节）。

### P2（第三批，口径先行、随增量落地）

- **快照**：列（历史 snapshot 目录）/ 验（`snapshot --verify` 同款
  核心调用）/ 打包三动作。（U3 已实装）
- **registry 条目浏览**：本地与远程条目列表（远程 list 已落地），
  版本、依赖 pin、哈希可见。（U3 已实装）
- **diff 与变更可视化**：语义分层 diff（下文专节）。（U3 已实装）

## 变更可视化：不做 Merkle，做语义分层 diff

**定了什么**：diff 的语义呈现分三层，数据全部来自现有 manifest，
**零格式变更、零哈希重定义**。diff API 返回：

```json
{
  "changed_tables": ["item", "stage"],
  "affected_targets": ["json:build/client/data", "..."],
  "files": { "added": [], "removed": [], "changed": [] },
  "roots": { "content_hash": "…", "build_id": "…" }
}
```

界面按 **表 → target → 文件** 三层折叠：改了哪些表 → 波及哪些
target → 落到哪些文件。`changed_tables` = 两份 manifest 的
`table_hashes` 逐表比对；`affected_targets` = 变更表过依赖图传播
（增量构建同款逻辑）；`files` = 现有 run_diff 的平面结果挂在对应
子树下。文本面同步：`cage diff` 获得分层摘要输出（表 / target 行
+ 文件清单），与界面共用同一语义 diff 函数（单一入口）。

**为什么**：现状 `cage diff` 是平面文件清单，「哪一部分变了」要
用户自己从路径里读。但 `table_hashes` **已经逐表在手**，语义分层
只差一次比对 + 一次依赖图传播 + 一次 join——全部是展示层工作，
不碰任何承重结构。

**备选（否决）：Merkle 化**——补内部节点摘要，让 diff 走不同根的
最小子树。否决理由：① manifest 根（`content_hash` / `build_id`）
是**承重墙**——确定性构建、registry 版本 pin、snapshot verify、
增量缓存全部挂在上面，重定义 = 全量缓存失效 + 旧 pin 断裂；② 本地
diff 两份 manifest 都在盘上，本来就要 O(manifest) 全扫，Merkle
剪枝**不省本地任何工作**——它的收益只在「远程只拉变化子树」的增量
同步场景，本仓没有这个场景。

**边界**：不做 manifest 格式版本化迁移（不改格式就没有迁移）；
diff 语义层只回答「哪里变了」，不回答「内容怎么变了」（逐字段值级
diff 是新能力面，留待需要时单独立项，schema 级已有
`diff_schemas` 可循）。

**卡点条件（重议 Merkle 的唯一理由）**：出现「消费者只向远程
registry / snapshot 拉取变化子树」的增量同步需求时重议。

## 双端差异的表达（profile × target × 字段可见性）

事实基础：profile 决定 target 列表与 `output_dir`（build/client/*、
build/server/*）；表级与字段级 `targets`（schema）经
`filter_by_profile` 把字段从该 profile 的 schema 与**全部产物**里
剔除（示例工程 server-only 字段 `internal_note` 即是）；`build_id`
输入含 profile。

界面表达四条：

- **产物归属**：manifest 自带 profile 与 build_id，产物浏览器按
  `output_dir` 分组 + 全局 profile 徽标——任何视图都标着「这是
  哪一端的产物」。
- **字段可见性徽标**：`field.targets` / `table.targets` 非空即在
  编辑器与浏览视图渲染徽标（`internal_note` → 「server only」），
  数据源就是 schema 本身，无新状态。（U2b 已实装）
- **profile 作用域视图**：`GET /api/schema?profile=` 出投影版
  schema（`filter_by_profile` 现成函数直调）；被剔除字段显示
  「本 profile 不可见」而非消失——藏起来的东西要看得见被藏了。
  （U2b 已实装）
- **跨端对照（P2）**：选一张表 → client / server 两列矩阵，字段行
  「仅 server」「仅 client」「两端可见」三态；产物侧同表并排，
  server 多出的字段就是可见性剔除的镜像。

## 路线图

- [x] **U1（P0）**：项目与构建状态 + 校验面板（状态端点 + 前端接线）
      —— **2026-10-12 已实装**，落地记录：
      `GET /api/status`（三块一文档：项目事实 / 校验面板 / 每 profile
      构建事实）+ `POST /api/build`（与 `cage build` 同一
      `build_project` 入口）+ `/status/` 状态页（校验面板 + 构建卡片，
      编译期嵌入，vanilla ES module 零依赖）。**口径精化两处**：
      ① freshness = `build_project` 第一层比对的「报告版」——同
      profile、同环境、同 schema/source/target 指纹、产物完整，外加
      `changed_tables`（第二层逐表 `table_hashes` 比对）告诉用户哪几张
      表改了还没重建；manifest 单槽（`<output_dir>/manifest.json` 一次
      记一个 profile），`last_build_profile` 标明归属，跨 profile 的
      fresh 直接为 false。② 校验面板经 `validate_project` 单一入口
      （check / lsp 同源），页面带 `?profile=&env=&level=` 三个旋钮，
      默认值与 `cage check` 一致（首个 profile、semantic、无环境）。
      裸 `/status` 301 到 `/status/`——相对资源引用不会撞编辑器的
      /app.js。验收：web.rs 单测 6 项 + tests/web.rs 集成 3 项 +
      web-smoke 第 5 步（真实服务：页面 → fresh → up-to-date）。
- [x] **U2a（P1）产物浏览**：`/api/artifacts`（manifest 账本，单槽跨
      profile 返回空清单 + `manifest_profile`，stale 复用第一层判语）+
      `/api/artifact`（只认 manifest 键 + 伺服前重算 blake3，篡改 409、
      非账本 404）+ 状态页「产物」区（按 output_dir 分组、预览/下载）。
      验收：web.rs 单测 3 项 + tests/web.rs 集成 1 项（列表 → 逐字节
      伺服 → 404 → 409）+ web-smoke 第 5 步产物断言
- [x] **U2b（P1）编辑器 targets 徽标 + profile 作用域视图**：
      `GET /api/schema?profile=` 附 `filter_by_profile` 投影（未知
      profile 400）；前端只做集合差，可见性语义零复制进 JS；targets
      非空即渲染徽标（左栏 / 字段行 / 表标题），被剔除项灰显 +
      「本 profile 不可见」而非消失；未保存改动时投影提示语标注
      「按已保存 schema 投影」。验收：web.rs 单测 +2 + 集成 1 项 +
      web-smoke 第 7 步（game-config `internal_note` 三态断言）
- [x] **U3（P2）快照列/验/打包 + registry 条目浏览 + 分层 diff**：
      —— **2026-10-12 已实装**，落地记录：
      ① `GET /api/snapshots`（列 `<output_dir>/snapshot/` 全部包，
      每包**现场** `verify_snapshot`——列表绝不宣称它没刚验过的包是
      完好的；附 manifest 事实：profile / env / build_id /
      content_hash / 产物数）+ `POST /api/snapshot`（与 `cage snapshot`
      同一管线：完整 build → pack_snapshot；校验失败 200 + 诊断，
      Io 400）+ 状态页「快照」区（verified 徽标 + 事实 chips + 打包钮）。
      ② `GET /api/diff?from=&to=`（两个 manifest 的语义分层 diff，
      形态即拍板原文：`changed_tables` / `affected_targets` /
      `files:{added,removed,changed:[path,table,target,from_hash,to_hash]}`
      / `roots:{content_hash,build_id}`；table/target 元数据随文件条目
      走，界面三层折叠就是纯分组，无新计算）+ 状态页「对比」区
      （基线/目标旋钮含 live 构建与各快照，表 → target → 文件三层
      折叠，逐字节一致出「两次构建逐字节一致」）。
      ③ `GET /api/registry`（读 `[registry] path`，未配置出 ok + 说明
      而非报错；每版本 build_id / content_hash / files chips）+ 状态页
      「Registry」区。④ CLI：`cage snapshot --list`（列包 + 每包自验
      判语；与 `--verify`/`--env` clap 冲突）、`cage diff` 增
      「tables changed / targets affected」分层摘要两行。**Merkle 化
      卡点维持原判**：出现「只拉变化子树」的增量同步需求前不做。
      验收：web.rs 单测 +3（快照打包/列表/坏体、diff 分层与 400、
      registry 浏览与未配置）+ `cage snapshot` CLI 测试 5 项（含
      --list 列包与篡改判语、describe 冲突断言）+ 集成 1 项
      （registry 未配置 → 打包 → 列表自验 → 跨快照 diff → ghost 400
      端到端）+ web-smoke 第 5 步（真实服务：打包 → 自验 → 零差异
      diff → registry 空态）

每项落地后在本节追加落地记录（惯例同 design.md）：实装面、口径
精化、验收方式。

## 策划规则界面（2026-10-20 用户令：先设计后推进）

> 令：策划配置规则没有界面很痛苦。定位见 todo「规则配置界面」块：
> 策划不碰 CLI/文件也能配规则、跑校验、看结果，复用既有资产
> （Diagnostics 定位 → 表/行/列点位；错误码表 → 可读面板；单一解析
> 口径不写两套）。本节是开工前的设计拍板。

**入口形态（已代决，2026-10-20）**：不做新的 `cage serve` 子命令，
全部落 `cage web` 既有单服务。依据：形态拍板 c（上文）已定「增强
现有 cage web」，且 W5–W8 已把项目级校验（`/api/status` 的
validation 面自带 code/severity/定位/hint 诊断 JSON）、构建、产物、
diff、registry 接上门面——另起子命令会分叉安全边界（loopback 校验、
限长读、8 MiB 上限，2026-10-18 表一#2 硬化）与文档面。todo 里的
`cage serve` 是排队时的示意命名，非用户原话。

**P0 增量盘点（对照已实装面，防重复造）**：

| todo 所述 | 现状 | 真实增量 |
|---|---|---|
| 一键 validate | `/api/status` 已跑 `validate_project` 全量诊断 | 无新校验——呈现层改造 |
| 诊断面板 | 状态页校验面板已有（code/严重级/定位/hint） | 过滤控件（错误码/严重级筛选） |
| 点击跳表格行列 | 编辑器 `focusDiag` 只跳 schema 字段 | **数据表格视图**（现不显示数据行）+ 诊断 Source 定位（file/Sheet/Row/Col）→ 单元格映射高亮 |
| 上传或选择文件 | 服务端只读仓内 `source_roots` | **选择**先做（仓内源文件清单）；**上传**暂缓（已代决：任意文件写仓与 2026-10-18 表一#1 路径穿越同类，需另立安全拍板：写哪、怎么防覆盖/穿越、多大上限，未拍前不实现） |

**端点纪律不变**：新增面 = 薄 JSON 端点（`GET /api/tables` 列表 +
`GET /api/table?name=` 行分页截断，直调 load 链既有函数）+ 前端
接线；服务端不持状态；vanilla ES module 零构建链，编译期嵌入；
诊断/校验单一入口（`validate_project`，check / lsp / web 三处同源
的既有纪律）。

**分期**：

- **P0 数据面板**：表列表 → 表格视图（分页 + 行列上限，防大表撑爆
  浏览器）→ 诊断过滤（错误码/严重级）→ 点击诊断跳表高亮单元格。
- **P1 规则表单 + 一键导出**：规则编辑挂现有 W3 编辑器（表单化 =
  编辑器交互增强，保存走既有 `POST /api/schema` 链，序列化由 cage
  统一）；导出复用既有 `POST /api/build` + `/api/artifacts` 下载
  （打包 zip 与否随实现拍，产物字节不重算）。
- **P2 版本对比 + 模板库**：对比复用既有 `GET /api/diff` 分层
  JSON；模板库 = 前端静态模板字典生成 schema 片段（数值范围/枚举/
  外键引用起步），落库仍走 schema 保存链，服务端零新面。

**验收口径**：每端点 web.rs 单测 + tests/web.rs 集成 + web-smoke
增步（同 U 系列惯例）；表格视图有分页截断断言；诊断点位断言
（坏数据 → 点击 → 高亮坐标）；上传不做即无上传面。

**边界**：不做多用户/远程（远程仍走 SSH 端口转发，上文拍板）；
不做上传（另立安全拍板）；不做规则执行逻辑的新增——校验永远
`validate_project` 单一入口。

**P0 落地记录（2026-10-20）**：数据面板已实装——`GET /api/tables` +
`GET /api/table`（web.rs，行窗分页 200/上限 1000，主键取 schema 定义）；
状态页「数据」区（表清单 + 粘性表头表格视图 + 分页）；诊断面板
severity/错误码过滤与点击跳行高亮（jumpToDiag）。测试：web 单测
+2（共 32 绿）+ 集成 +1（共 14 绿）+ web-smoke 第 5 步并入断言，
全脚本实跑通过。记录面：docs/web.md W9。余项：上传暂缓（安全
拍板未立，卡点如实挂账）；P1 规则表单/导出、P2 对比/模板库未动。
