# 源码分层分析

本页把实现归纳为九个职责层，锚定源码而不是目录名。英文镜像：
[Source Layer Analysis](../source-layer-analysis.md)。

工作区本身已经把实现切为核心、输入适配器、输出适配器和二进制应用：
`Cargo.toml:3-27`。源/目标适配器依赖 core；只有 CLI 二进制负责组装全部适配器
（例如 `crates/cage-cli/Cargo.toml:20-40` 同时声明 Excel 和 JSON 等依赖）。

## 层级地图

| 层 | 职责 | 代表入口 |
| --- | --- | --- |
| 应用接口层 | 接收 CLI/HTTP/MCP/LSP 请求，驱动共享管线 | `crates/cage-cli/src/main.rs:575` |
| 工程装载层 | 读取工程配置、解析 schema/源，产出编译输入 | `crates/cage-cli/src/main.rs:787` |
| 核心模型层 | 用与文件格式无关的结构表示 schema 与规范化数据 | `crates/cage-core/src/schema/mod.rs:9`、`crates/cage-core/src/value/mod.rs:134` |
| 归一化层 | 填默认值、拆单元格数组、按声明类型转换 | `crates/cage-core/src/normalize/mod.rs:116`、`crates/cage-core/src/normalize/mod.rs:232` |
| 校验层 | 执行 L0-L7 检查并返回可定位诊断 | `crates/cage-core/src/validation/mod.rs:168` |
| 构建与分发层 | 哈希输入、增量重建、打包校验、发布产物 | `crates/cage-cli/src/main.rs:2184`、`crates/cage-core/src/snapshot/mod.rs:83` |
| 目标生成层 | 把 schema/文档转换成数据、绑定、契约或模板 | `crates/cage-cli/src/main.rs:3727` |
| 源接入层 | 把外部文件/表/API 解析为规范文档 | `crates/cage-cli/src/main.rs:1037`、`crates/cage-source-excel/src/lib.rs:54` |
| 治理与诊断层 | 追踪资产、迁移 schema/数据、统一错误报告 | `crates/cage-core/src/assets/mod.rs:86`、`crates/cage-core/src/diagnostics/mod.rs:13` |

## 应用接口层

`main` 解析 CLI 参数并分发命令（`crates/cage-cli/src/main.rs:575`）。四个协议
面共享编译内核，而不是各自重写管线：

- CLI 命令调用 `build_project`、`run_gen`、迁移、快照、注册表、资产和校验流程。
  `build` 是最宽入口（`crates/cage-cli/src/main.rs:2184`）。
- HTTP 在回环地址服务编辑器/状态 API，采用单线程循环并内嵌 UI 资源
  （`crates/cage-cli/src/web.rs:61-77`、`crates/cage-cli/src/web.rs:49-58`）。
- MCP 是 stdio 上的无状态 rmcp 工具路由；每个工具复用工程装载、校验或目标
  规划（`crates/cage-cli/src/mcp.rs:130-240`）。
- LSP 没有持久编译状态：framed JSON-RPC 请求触发全量工程校验并发布诊断
  （`crates/cage-cli/src/lsp.rs:97-230`）。

这里没有 Redis 或 MySQL 运行时缓存。HTTP 上下文只保存工程根和反序列化配置
（`crates/cage-cli/src/web.rs:84-87`）；MCP 明确无状态（`crates/cage-cli/src/mcp.rs:130-135`）；
LSP 只保存根目录、env/level 和打开文档 URI（`crates/cage-cli/src/lsp.rs:97-103`）。

## 工程装载层

`load_project_impl` 解析一份配置文件，填充环境 DSN/凭据名，装载 schema，并合并
各逻辑源根。注册表根必须先通过账本校验，再把其 `data/` 目录选作源
（`crates/cage-cli/src/main.rs:1133-1192`）。本地/远程根经 CLI 侧装载器分发，
注册表解析属于 core 逻辑（`crates/cage-cli/src/main.rs:1020-1065`）。

`cage.toml` 是顶层配置 schema：工程身份、profiles、source roots、schema 路径、
输出、registry、依赖 pin、远程源环境变量名和格式约定
（`crates/cage-core/src/manifest/mod.rs:523-632`）。远程配置刻意只存名称而不存
凭据：`dsn_env`、`credential_env` 和命名只读查询写入配置，秘密在装载时解析
（`crates/cage-core/src/manifest/mod.rs:610-631`）。

## 核心模型层

`Schema` 保存有序表定义；`Value` 是规范数据形状；`Document` 聚合插入序表和
源元数据（`crates/cage-core/src/schema/mod.rs:9-132`、`crates/cage-core/src/value/mod.rs:134-364`）。
core 把这些作为公共编译边界再导出（`crates/cage-core/src/lib.rs:57-80`）。模型
没有进程级单例状态；生命周期跟随一次命令或请求，输入来自装载，输出作为不可变
借用进入归一化、校验和生成。

schema 编辑器边界同样是模型转换，而不是独立的 UI 模型：editor JSON 和 canonical
YAML 都经 core 函数映射（`crates/cage-core/src/edit/mod.rs:35-90`）。

## 归一化层

归一化发生在装载之后、校验/生成之前。它填充字段默认值
（`crates/cage-core/src/normalize/mod.rs:116`），拆分单格数组
（`crates/cage-core/src/normalize/mod.rs:160`），并把 `Value` 转换到字段类型
（`crates/cage-core/src/normalize/mod.rs:232`）。它是无状态的：每个函数都把
value/document 映射为另一个 value/document。CLI 构建在 profile 校验后立即调用它
（`crates/cage-cli/src/main.rs:2226-2240`）。

## 校验层

`validate_with_profile` 构建主键/引用缓存，按 L0-L7 顺序执行，随后按需升级
warning 并排序诊断（`crates/cage-core/src/validation/mod.rs:168-227`）。级别是
有序枚举（`crates/cage-core/src/validation/mod.rs:22-89`）。游戏规则校验器作为
同一管线的 L7 被调用。校验不保存持久状态；缓存只存在于 `ValidationContext`。

CLI/MCP/LSP 都路由到这一入口，而不是重新解码诊断：build 使用它
（`crates/cage-cli/src/main.rs:2226`），MCP 工具体调用 `validate_project`
（`crates/cage-cli/src/mcp.rs:270-420`），LSP 把结果映射为编辑器诊断
（`crates/cage-cli/src/lsp.rs:109-177`）。

## 构建与分发层

CLI 构建编排完整生命周期：校验全量数据、归一化、计算 schema/source 哈希、
比较旧 manifest、只重建受影响表/目标、写产物、写确定性 manifest
（`crates/cage-cli/src/main.rs:2184-2520`）。增量状态是磁盘上的旧 `manifest.json`
加产物文件；安全路径检查会拒绝不安全的旧记录
（`crates/cage-cli/src/main.rs:2300-2355`）。快照打包 manifest/schema/产物集合
并生成 BLAKE3 账本，加载时再次校验（`crates/cage-core/src/snapshot/mod.rs:83-206`）。
注册表是自校验快照目录加每包 `index.json`；它只解析账本验证过的条目
（`crates/cage-core/src/registry.rs:6-31`、`crates/cage-core/src/registry.rs:1526-1569`）。

远程分发与远程读取共用一条有界 HTTP/重试策略。缓存键/目录、GET/条件 GET 和
PUT 都在 core（`crates/cage-core/src/remote.rs:35-310`、
`crates/cage-core/src/remote.rs:524-547`）。注册表 push 先上传条目文件，最后上传
合并索引（`crates/cage-core/src/registry.rs:1078-1330`）。

配置输入是 profile/target 列表和 registry auth-env 名称；产物和 manifest 是持久
状态。没有常驻进程把构建状态保留在内存中。

## 目标生成层

`generate_target_items` 是 build 和 MCP dry-run 共用的内存分发入口
（`crates/cage-cli/src/main.rs:3727-3768`）。数据格式是 JSON、CSV 和 MessagePack；
语言/契约/模板生成器由 schema 驱动（`crates/cage-cli/src/main.rs:3637-3725`）。
每个 target 是独立 crate，并从 `TargetConfig` 构造生成器
（`crates/cage-core/src/manifest/mod.rs:508-521`）。target 状态只有 template 格式
使用的工程 Tera 模板目录；所有生成字节返回 CLI，再由统一的产物写入器落盘。

## 源接入层

每个本地解析器都通过 source adapter 产出 `Document`。Excel 使用 calamine 和约定
元数据（`crates/cage-source-excel/src/lib.rs:54`、`crates/cage-source-excel/src/lib.rs:1284`）；
CSV/JSON/YAML/MessagePack 是格式专属解析 crate。HTTP、数据库、Sheets 适配器
先取远程字节/行集，再复用 JSON adapter 做规范缓存/转换
（`crates/cage-source-http/src/lib.rs:133`、`crates/cage-source-db/src/lib.rs:252-360`、
`crates/cage-source-sheets/src/lib.rs:298-330`）。

远程状态是由确定性 source key 派生的工程本地缓存路径
（`crates/cage-core/src/remote.rs:35-60`）。数据库只是只读查询/内省输入，不是
编译器持久状态。`RemoteSourceConfig` schema 只保存环境变量名和命名只读查询
（`crates/cage-core/src/manifest/mod.rs:610-631`）。

## 治理与诊断层

资产治理清点 rules、migrations 和本地 source roots；BLAKE3 哈希形成确定性
baseline 和验证报告（`crates/cage-core/src/assets/mod.rs:26-177`）。迁移解析声明式
spec、按序执行 step、回验文档并输出每步报告
（`crates/cage-core/src/migrate/mod.rs:35-90`、`crates/cage-core/src/migrate/mod.rs:513-880`）。
schema diff 派生迁移草案（`crates/cage-core/src/migrate/diff.rs:88-220`）。

诊断是一等管线结果，携带错误码、源位置、表/字段上下文、severity 和渲染。类型
定义在 core（`crates/cage-core/src/diagnostics/mod.rs:13-110`），所有接口层渲染或
映射同一个对象。错误码表是稳定的文案来源：见 `crates/cage-core/src/error/codes.rs`。

## 配置、状态与生命周期

| 关注点 | 配置输入 | 状态存储 | 调用通道 |
| --- | --- | --- | --- |
| CLI | argv、`cage.toml`、schema、源、registry pin | output/manifest/snapshot/registry 文件 | 进程 → 文件系统 |
| Web | HTTP 请求 + `cage.toml`、editor JSON、上传字节 | 工程文件、旧 manifest/产物 | 浏览器 fetch ↔ 回环 HTTP |
| MCP | JSON-RPC 工具参数 + 工程文件 | 仅命令内加载状态 | stdio MCP |
| LSP | initialize options + 工程文件 | 仅打开文档 URI 集合 | stdio LSP framing |
| Sources | 文件、SQL 结果集、HTTP payload、Sheets 行集 | 工程本地 remote cache / canonical cache JSON | ureq HTTP、MySQL/Postgres 协议、文件系统 |
| Registry | snapshot 字节、依赖 pin、bearer auth-env 名 | 包目录、`index.json`、`HASHES.json` | 文件系统 + 可选 HTTP PUT |

CLI 进程在一次分发后结束（`crates/cage-cli/src/main.rs:575-778`）；Web 循环到
被杀死（`crates/cage-cli/src/web.rs:61-79`）；MCP 在 stdio 客户端断开时结束
（`crates/cage-cli/src/mcp.rs:215-242`）；LSP 在 EOF 或 shutdown/exit 后退出
（`crates/cage-cli/src/lsp.rs:199-278`）。

## 相关页面

- [架构](./architecture.md)解释概念管线。
- [术语表](./glossary.md)定义各层使用的术语。
- [Source](./source.md)、[Schema](./schema.md)、[Validation](./validation.md)、
  [Target](./target.md)、[Build](./build.md)、[CLI](./cli.md) 和
  [Web](./web.md)分别展开各关注点。
