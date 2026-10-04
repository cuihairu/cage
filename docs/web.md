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

## 路线图

- [ ] **W3** Schema 编辑器前端（docs/public 静态单页，无 node 构建链）：
      表/枚举树、19 字段类型、约束编辑、E 码诊断面板、保存
      （API 契约已定稿，前端纯消费）
- [ ] **W4** 文档与示例：编辑流程定稿、示例工程 web 冒烟（编辑 → 校验 →
      保存 → `cage build` 复现）、CI 集成
      19 字段类型、约束编辑、E 码诊断面板、保存
- [ ] **W4** 文档与示例：编辑流程定稿、示例工程 web 冒烟（编辑 → 校验 →
      保存 → `cage build` 复现）、CI 集成