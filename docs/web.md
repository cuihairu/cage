# Web UI / Schema Editor（第三阶段）

第三阶段第一项：为策划与程序提供 Schema 的 Web 编辑入口。红线不变——
不做 Excel 编辑器、不做游戏逻辑框架；Web UI 只做 Schema 的门面，不做
数据运行库。

## 定位

Cage 的 CLI 是编译链事实接口；Web UI 是**同一核心的另一个门面**：
编辑器不解析、不渲染 YAML，只与 core 交换文档并委托校验。所有编辑
语义（19 种字段类型、约束、引用、targets 可见性、表达式规则）都来自
`cage-core`，前端不做任何规则复制。

## 架构决策

### 交换模型 = 现有 serde Canonical 形状

编辑器文档就是 `Schema` 的 serde 形状——与 Configuration Snapshot 里
的 `schema.json`、HTTP API 同一文档，一份文档三处消费：

```text
snapshot/schema.json  ≡  cage-core::edit::to_editor_json  ≡  HTTP GET /api/schema
```

编辑器 JSON 保持插入序（表/字段按文档顺序），省略 `skip_serializing_if`
覆盖的可选字段，`default` 无 skip 属性故显式为 `null`（前端两者皆可编辑）。

### 保存 = 规范化 YAML 单文件写回

编辑器面向**合并后的规范 Schema**（cli 装载时把 schemas/ 下多文件合并
成一张 `Schema`——编辑器即增值这一形态），保存写回为单文件 canonical
YAML（`to_canonical_yaml`）：同 Schema → 同字节（含唯一结尾换行），
经 `from_canonical_yaml` 回环幂等。多文件拆分是作者侧的持久化选择，
编辑器不产生拆分产物。确定性契约：编辑器保存的 YAML 经 `cage build`
得到的产物，与手写同一 Schema 构建的产物逐字节一致——生成器只见过
`Schema` 值，编辑回环只要保值的即满足。

### E1701 编辑态错误族

编辑文档无法反序列化为 `Schema`（语法/形状/未知 kind）报 **E1701**，
铺 JSON 路径（`tables.Item.fields.id.type`）定位到具体输入（前端的
高亮锚点）；schema 内部一致性（主键缺失、引用悬空）仍走 L1 的
`E1004`——两类错误分开，编辑器与校验器各自汇报职责清晰。

### 无 node 前端工具链

前端是 `docs/public` 下静态单页（vanilla ES 模块），`cage web` 直接
伺服——仓库不引入 Node/npm 构建链，cargo 一键即得编辑器。

## 现状

### W1 已实装：编辑器交换模型（cage-core::edit）

- [x] `to_editor_json` / `from_editor_json`——Schema ↔ 编辑器 JSON；
      解析失败 → `E1701`（JSON 路径定位，根级错误归一为无路径）
- [x] `to_canonical_yaml` / `from_canonical_yaml`——确定性保存写回与回读
- [x] 测试：编辑器 JSON round-trip 确定性、canonical YAML 幂等
      （再渲染字节不变）、E1701 路径定位（含未知 kind / 缺必填成员 /
      根级错误 / YAML 语法错误）、与 snapshot schema.json 形状一致；
      `E1701` 入错误码表（crates/cage-core/src/error/codes.rs
      `editor` 族 + 校验文档）

### W2 已实装：`cage web` 本地 HTTP 服务（cage-cli/src/web.rs）

- [x] 项目装载复用 CLI 加载链（`load_project_config` / `load_schema`），
      只绑定 `127.0.0.1`，无鉴权（本地工具）
- [x] `GET /api/schema`——合并 Schema 的编辑器文档 + 项目事实
      （project / schema_path / profile_names / warnings_as_errors）；
      每请求重载，保存后立即可见
- [x] `POST /api/validate`——编辑态校验：E1701（文档无法反序列化，
      JSON 路径定位）→ L1 一致性（E1004 族）；诊断以裸数组输出
      （`Diagnostics` serde 的 `{"items": [...]}` 壳在 API 层摊平）
- [x] `POST /api/schema`——canonical YAML 写回：单文件 `schema_path`
      直接覆盖；目录（多文件 schema）→ 409 拒写（编辑器不重写
      作者侧拆分）；未配置 → 写 `schema.yaml` + note 提醒接线
      cage.toml（服务器绝不改配置文件）
- [x] 集成测试（crates/cage-cli/tests/web.rs）：真实子进程 + 原始
      TCP 请求，覆盖文档伺服 / E1701+E1004 校验 / 保存后重载与
      `cage check` 复跑 / 目录拒写 / 空 schema 起始

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
- [x] 保存：Ctrl/Cmd+S 或保存按钮 → `POST /api/schema`；未保存标记 +
      beforeunload 拦截；409（目录 schema_path）/ 未接线 note /
      500 均以错误 toast 呈现
- [x] 外科手术式文档编辑：未知/遗留键（如 fuzzyField 的 items/properties
      旧字段）原样保留，只改可控键——与「编辑器 JSON ≡ canonical 形状」的
      交换契约一致；保存全程不经前端渲染 YAML
- [x] 集成测试：GET / 返回嵌入编辑器页（含模块/样式引用）、/app.js、
      /app.css 伺服与 404；测试原始客户端补 chunked 解码
      （tiny_http 大响应走 chunked，分块边界可切开 UTF-8 序列）

### W4 已实装：编辑流程定稿 + 示例工程冒烟 + CI

- [x] `examples/web-demo/`：单文件 schema 冒烟工程（schema_path 指向
      schema.yaml；`[source_roots] main = "config"` 让示例表立即可
      check/build）——「浏览器打开 → 编辑 → 校验 → 保存 → `cage build`
      复现」整条路径的最小闭环
- [x] `examples/web-smoke.sh`：端到端冒烟脚本，在临时拷贝上运行（不脏
      仓库），真实命令逐条回显；任一步失败即 exit 非零。六步：基线
      check；编辑器会话（GET 文档 → 加 `stack_size` 字段（UInt8 /
      min 1 / default 1）→ `/api/validate` 零诊断 → `/api/schema` 保存）；
      断言 schema.yaml 落盘 canonical YAML 且含新字段；`cage check` +
      `cage build` 全链路复现；同一 schema 两次重建产物树哈希逐字节
      一致（确定性契约）；对 `examples/game-config`（schemas/ 目录）
      断言 `POST /api/schema` 得 409 拒写（多文件是作者侧组织，编辑器
      不重写）
- [x] CI 集成：`web-smoke` job 镜像 example job（checkout + toolchain +
      rust-cache + `bash examples/web-smoke.sh`）

## 路线图

第三阶段 W 系列至此全部完成（W1 交换模型 → W2 本地服务 → W3 编辑器
前端 → W4 文档与示例）。示例入口：`examples/web-smoke.sh`（CI 同款）。