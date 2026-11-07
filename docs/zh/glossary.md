# 术语表

这些术语描述实现抽象，而不是工作区目录标签。英文镜像：
[Glossary](../glossary.md)。

| 术语 | 英文 | 定义 | file:line |
| --- | --- | --- | --- |
| 应用接口 | Application Interface | CLI、HTTP、MCP 或 LSP 面，把协议请求转换为共享编译操作。 | `crates/cage-cli/src/main.rs:575` |
| 命令分发 | Command Dispatch | CLI 决策表，启动一条具体管线并返回进程退出码。 | `crates/cage-cli/src/main.rs:575-778` |
| Web 服务 | Web Service | 回环 HTTP 服务，暴露编辑器、状态、构建、产物、快照、diff、上传和 registry API。 | `crates/cage-cli/src/web.rs:8-29`、`crates/cage-cli/src/web.rs:61` |
| MCP 工具 | MCP Tool | rmcp schema 描述的操作，包装校验、检查、dry-run 或配置解释。 | `crates/cage-cli/src/mcp.rs:135`、`crates/cage-cli/src/mcp.rs:217` |
| LSP 会话 | LSP Session | framed JSON-RPC 会话，全量校验工程并向打开文档推送诊断。 | `crates/cage-cli/src/lsp.rs:8-27`、`crates/cage-cli/src/lsp.rs:202` |
| 工程配置 | Project Config | `cage.toml` 声明身份、profiles、源、输出、registry、依赖和远程设置。 | `crates/cage-core/src/manifest/mod.rs:523-632` |
| 工程装载 | Project Loading | 解析配置、解析环境变量名、装载 schema、解析 source root 并合并文档。 | `crates/cage-cli/src/main.rs:787`、`crates/cage-cli/src/main.rs:1020` |
| Profile | Profile | 包含 targets 和变量的命名构建视图；同时驱动字段/表可见性校验。 | `crates/cage-core/src/manifest/mod.rs:493-521`、`crates/cage-cli/src/main.rs:2220-2232` |
| 目标配置 | Target Configuration | 一个生成器声明，含 format、output directory、file template 和 options。 | `crates/cage-core/src/manifest/mod.rs:508-521` |
| Schema | Schema | 配置的有序表、字段、类型、约束、枚举、引用和表达式声明。 | `crates/cage-core/src/schema/mod.rs:9`、`crates/cage-core/src/schema/mod.rs:133-287` |
| Canonical Value | Canonical Value | 与格式无关的 `Null/Bool/Int/UInt/Float/String/Bytes/Array/Object` 值模型。 | `crates/cage-core/src/value/mod.rs:134-160` |
| Canonical Document | Canonical Document | 接入产出的插入序合并表集合，以及 source files 和 metadata。 | `crates/cage-core/src/value/mod.rs:324-364` |
| Source Location | Source Location | 文件、格式、sheet/row/column、line/offset 和 field 元数据，用于精准定位诊断。 | `crates/cage-core/src/value/mod.rs:6-30` |
| 归一化 | Normalization | 校验/生成前的默认值填充、单元格数组拆分和按类型收敛。 | `crates/cage-core/src/normalize/mod.rs:116-232` |
| 校验级别 | Validation Level | 有序 L0-L7 检查，从 parse/schema 到 value/table/reference/semantic/game rules。 | `crates/cage-core/src/validation/mod.rs:22-89` |
| 校验上下文 | Validation Context | 一次校验中的 schema/document/diagnostics 引用，以及 level、profile、warning 策略和引用缓存。 | `crates/cage-core/src/validation/mod.rs:97-117` |
| 引用缓存 | Reference Cache | 一次校验内用于跨表引用的主键索引。 | `crates/cage-core/src/validation/mod.rs:117-150` |
| 依赖图 | Dependency Graph | schema 派生的表依赖关系，供校验和增量规划使用。 | `crates/cage-core/src/reference/mod.rs:240` |
| 增量规划器 | Incremental Planner | 从变更表计算传递受影响表，用于选择性重建。 | `crates/cage-core/src/reference/mod.rs:439` |
| 构建清单 | Build Manifest | 持久构建记录，包含 profile/environment/输入哈希、target record 和 artifact ledger。 | `crates/cage-core/src/manifest/mod.rs:9-107` |
| 增量构建 | Incremental Build | 基于旧 manifest 状态做整轮跳过、依赖感知表传播和 target 配置传播。 | `crates/cage-cli/src/main.rs:2184`、`crates/cage-cli/src/main.rs:2290-2355` |
| 快照 | Snapshot | 包含 manifest、schema、artifacts 和 BLAKE3 `HASHES.json` 账本的自校验目录。 | `crates/cage-core/src/snapshot/mod.rs:83-206` |
| 配置注册表 | Configuration Registry | 本地包目录，保存已校验快照和确定性的每包版本索引。 | `crates/cage-core/src/registry.rs:6-31`、`crates/cage-core/src/registry.rs:1526-1569` |
| Bundle | Bundle | 单个 registry entry 及其 index excerpt 的确定性 tar/zstd 导出/导入表示。 | `crates/cage-core/src/registry.rs:777`、`crates/cage-core/src/registry.rs:883` |
| 直推 | Direct Push | 通过 HTTP PUT 上传 registry entry 文件，最后上传合并包索引。 | `crates/cage-core/src/registry.rs:1078-1330` |
| 远程源 | Remote Source | HTTP API、MySQL/PostgreSQL 查询、Google Sheet 或 registry 引用形式的配置输入。 | `crates/cage-cli/src/main.rs:1020`、`crates/cage-core/src/manifest/mod.rs:610-631` |
| 远程缓存 | Remote Cache | 工程本地的确定性缓存字节/路径，支持离线回退和条件 GET。 | `crates/cage-core/src/remote.rs:35-60`、`crates/cage-core/src/remote.rs:274-310` |
| 目标生成器 | Target Generator | schema/document 到数据、语言绑定、契约或用户模板输出的转换。 | `crates/cage-cli/src/main.rs:3637-3768` |
| 诊断 | Diagnostic | 一等错误/警告记录，含错误码、源位置、上下文、帮助和渲染。 | `crates/cage-core/src/diagnostics/mod.rs:13-110` |
| 资产清单 | Asset Inventory | rules、migrations 和本地 source files 的确定性 BLAKE3 baseline 与验证报告。 | `crates/cage-core/src/assets/mod.rs:26-177` |
| 迁移 | Migration | 声明式有序数据/schema 迁移 step，带执行报告和 apply 后回验。 | `crates/cage-core/src/migrate/mod.rs:35-90`、`crates/cage-core/src/migrate/mod.rs:513-880` |
| Schema 差异 | Schema Diff | 两个 schema 版本间的结构比较，用于派生迁移草案。 | `crates/cage-core/src/migrate/diff.rs:88-220` |
| 编辑器交换模型 | Editor Exchange Model | Web 编辑器使用的 editor JSON 与 canonical schema YAML 的 core 映射。 | `crates/cage-core/src/edit/mod.rs:35-90` |
