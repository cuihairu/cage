# Glossary

These terms describe the implementation abstraction, not workspace directory
labels. Chinese mirror: [术语表](./zh/glossary.md).

| Term | English | Definition | Evidence |
| --- | --- | --- | --- |
| 应用接口 | Application Interface | A CLI, HTTP, MCP, or LSP surface that translates protocol requests into shared compiler operations. | `crates/cage-cli/src/main.rs:575` |
| 命令分发 | Command Dispatch | The CLI decision table that starts one concrete pipeline and returns the process exit code. | `crates/cage-cli/src/main.rs:575-778` |
| Web 服务 | Web Service | The loopback HTTP server exposing editor, status, build, artifact, snapshot, diff, upload, and registry APIs. | `crates/cage-cli/src/web.rs:8-29`, `crates/cage-cli/src/web.rs:61` |
| MCP 工具 | MCP Tool | A schema-described rmcp operation wrapping validation, inspection, dry-run, or config explanation. | `crates/cage-cli/src/mcp.rs:135`, `crates/cage-cli/src/mcp.rs:217` |
| LSP 会话 | LSP Session | A framed JSON-RPC session that validates the whole project and pushes diagnostics to open documents. | `crates/cage-cli/src/lsp.rs:8-27`, `crates/cage-cli/src/lsp.rs:202` |
| 工程配置 | Project Config | The `cage.toml` declaration of identity, profiles, sources, output, registry, dependencies, and remote settings. | `crates/cage-core/src/manifest/mod.rs:523-632` |
| 工程装载 | Project Loading | Config parsing, environment-name resolution, schema loading, source-root resolution, and document merging. | `crates/cage-cli/src/main.rs:787`, `crates/cage-cli/src/main.rs:1020` |
| Profile | Profile | A named build view containing targets and variables; it also drives field/table visibility validation. | `crates/cage-core/src/manifest/mod.rs:493-521`, `crates/cage-cli/src/main.rs:2220-2232` |
| 目标配置 | Target Configuration | One generator declaration containing format, output directory, file template, and options. | `crates/cage-core/src/manifest/mod.rs:508-521` |
| Schema | Schema | Ordered table, field, type, constraint, enum, reference, and expression declarations for configuration. | `crates/cage-core/src/schema/mod.rs:9`, `crates/cage-core/src/schema/mod.rs:133-287` |
| Canonical Value | Canonical Value | The format-neutral `Null/Bool/Int/UInt/Float/String/Bytes/Array/Object` value model. | `crates/cage-core/src/value/mod.rs:134-160` |
| Canonical Document | Canonical Document | The merged insertion-ordered table collection plus source files and metadata produced by ingestion. | `crates/cage-core/src/value/mod.rs:324-364` |
| Source Location | Source Location | File, format, sheet/row/column, line/offset, and field metadata used to pinpoint diagnostics. | `crates/cage-core/src/value/mod.rs:6-30` |
| 归一化 | Normalization | Default filling, cell-array splitting, and type-directed value coercion before validation/generation. | `crates/cage-core/src/normalize/mod.rs:116-232` |
| 校验级别 | Validation Level | Ordered L0-L7 checks from parse/schema through value/table/reference/semantic/game rules. | `crates/cage-core/src/validation/mod.rs:22-89` |
| 校验上下文 | Validation Context | Per-run references to schema/document/diagnostics plus level, profile, warning policy, and reference cache. | `crates/cage-core/src/validation/mod.rs:97-117` |
| 引用缓存 | Reference Cache | The transient primary-key index used to resolve cross-table references during one validation run. | `crates/cage-core/src/validation/mod.rs:117-150` |
| 依赖图 | Dependency Graph | Schema-derived table dependency relationships used by validation and incremental planning. | `crates/cage-core/src/reference/mod.rs:240` |
| 增量规划器 | Incremental Planner | Computes transitive affected tables from changed tables for selective regeneration. | `crates/cage-core/src/reference/mod.rs:439` |
| 构建清单 | Build Manifest | The durable build record containing profile/environment/input hashes, target records, and artifact ledger. | `crates/cage-core/src/manifest/mod.rs:9-107` |
| 增量构建 | Incremental Build | Full skip, dependency-aware table propagation, and target-config propagation against prior manifest state. | `crates/cage-cli/src/main.rs:2184`, `crates/cage-cli/src/main.rs:2290-2355` |
| 快照 | Snapshot | A self-verifying directory of manifest, schema, artifacts, and a BLAKE3 `HASHES.json` ledger. | `crates/cage-core/src/snapshot/mod.rs:83-206` |
| 配置注册表 | Configuration Registry | A local package directory of verified snapshots with deterministic per-package version indexes. | `crates/cage-core/src/registry.rs:6-31`, `crates/cage-core/src/registry.rs:1526-1569` |
| Bundle | Bundle | A deterministic tar/zstd export/import representation of one registry entry plus its index excerpt. | `crates/cage-core/src/registry.rs:777`, `crates/cage-core/src/registry.rs:883` |
| 直推 | Direct Push | HTTP PUT upload of registry entry files followed by the merged package index. | `crates/cage-core/src/registry.rs:1078-1330` |
| 远程源 | Remote Source | An HTTP API, MySQL/PostgreSQL query, Google Sheet, or registry reference consumed as configuration input. | `crates/cage-cli/src/main.rs:1020`, `crates/cage-core/src/manifest/mod.rs:610-631` |
| 远程缓存 | Remote Cache | Project-local deterministic cache bytes/path used for offline fallback and conditional GET. | `crates/cage-core/src/remote.rs:35-60`, `crates/cage-core/src/remote.rs:274-310` |
| 目标生成器 | Target Generator | A schema/document-to-output transform for data, language bindings, contracts, or user templates. | `crates/cage-cli/src/main.rs:3637-3768` |
| 诊断 | Diagnostic | A first-class error/warning record with code, source location, context, help, and rendering. | `crates/cage-core/src/diagnostics/mod.rs:13-110` |
| 资产清单 | Asset Inventory | A deterministic BLAKE3 baseline over rules, migrations, and local source files with verification report. | `crates/cage-core/src/assets/mod.rs:26-177` |
| 迁移 | Migration | Declarative ordered data/schema migration steps with execution reports and post-apply revalidation. | `crates/cage-core/src/migrate/mod.rs:35-90`, `crates/cage-core/src/migrate/mod.rs:513-880` |
| Schema 差异 | Schema Diff | Structural comparison that derives migration drafts between two schema versions. | `crates/cage-core/src/migrate/diff.rs:88-220` |
| 编辑器交换模型 | Editor Exchange Model | The core mapping between editor JSON and canonical schema YAML used by the Web editor. | `crates/cage-core/src/edit/mod.rs:35-90` |
