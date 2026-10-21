[English](README.md) | [中文](README.zh.md)

<div align="center">

<img src="docs/public/logo.svg" width="64" alt="Cage logo" />

# Cage

[![Rust](https://img.shields.io/badge/Rust-cargo-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/cuihairu/cage/actions/workflows/ci.yml/badge.svg)](https://github.com/cuihairu/cage/actions/workflows/ci.yml)
[![Codecov](https://codecov.io/gh/cuihairu/cage/branch/main/graph/badge.svg)](https://codecov.io/gh/cuihairu/cage)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://cuihairu.github.io/cage/)
[![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)](LICENSE)

**游戏配置编译与验证框架**

English: **Cage is a configuration compiler and validation framework for game development. It transforms heterogeneous authoring data into validated, deterministic runtime artifacts.**

</div>

---

Cage 将各种人类可维护的配置源（Excel / CSV / JSON / YAML / …）解析为统一的中间配置模型，经过多层次验证、跨配置引用解析与规范化处理后，按目标平台生成可直接消费的配置资产或代码。

它不绑定某一种输入或输出格式：Excel 只是一种输入源，目标是把分散、异构、面向人的配置数据，编译成经过验证、确定性构建的运行时配置资产。

## 核心链路

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline（L0 Parse ~ L7 Game Rules）
      → Normalize / IR → Target Generators → Manifest / Hash
```

## 核心能力

- Source Adapter 插件：Excel / CSV / JSON / YAML，以及 HTTP API / MySQL / PostgreSQL / Google Sheets 远程源（后续 XML、SQLite…），只读取解析，不做业务校验
- Schema 与 Source 解耦：同一 Schema 可用于多个配置源（类型/必填/默认值/范围/枚举/唯一/引用）
- 八级验证流水线：Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule，可分级执行（`cage check --level`）
- 跨配置引用校验：存在性 + 兼容性检查（引用对象存在但类型不合法同样报错；引用谓词按语义求值，不满足报 E1410）
- Diagnostics 一等公民：错误码 + 精确定位（文件/Sheet/单元格/字段/值）+ 修复提示
- 确定性构建：相同输入 → 相同产物、哈希与 Manifest（可追踪、可回滚、可增量）
- Profile 机制：client / server 不同 targets 与字段可见性，一份配置两端复用
- Configuration Snapshot：`cage snapshot` 打包配置快照（manifest + schema + 产物 + 逐文件哈希账本），加载前可检查文件是否与包内账本一致；若数据与账本被同时替换，该检查无法识别。发布者认证走 registry 签名 bundle 路径
- Configuration Registry：`cage registry` 多版本配置仓库（发布/列表/审计/GC/移除 + bundle 导出/导入/直推远端，含 `--presign-map` presigned URL 直推），消费方 `registry:包@版本` 引用 + `[dependencies]` 版本 pin，远程 http(s) 根解析（消费侧缓存离线可用，`registry push` 直推远端）
- 库表内省 Schema 草稿：`cage schema-draft` 只读内省 MySQL / PostgreSQL 表（绑定参数）产出可审阅 schema 草稿——表、主键、`NOT NULL → required`，DECIMAL / 日期时间 / JSON 等需要作者拍板的类型以行内注释标出
- 声明式数据迁移：`cage migrate` 按版本步进链对已验证配置做规则变换（改名/补默认/删字段/类型加宽/值重映射），默认 dry-run 报告，`--write` 原路写回并全栈回验
- Target 插件：JSON / CSV 数据产物起步，已扩展 C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java 代码绑定与 Template Target（用户自定义 Tera 模板 + 语言过滤器库），并交付 MessagePack 数据产物、Protobuf `.proto` / FlatBuffers `.fbs` 定义文件与标准 JSON Schema 文档（draft-07 / 2020-12）生成；后续 Binary 等（数据序列化与代码生成分离）

## 快速开始

```bash
bash examples/run.sh   # 一键跑通完整示例工程：校验 → 构建全部 14 个 target → 生成 8 语言代码与 proto/fbs/jsonschema 定义文件 → E1601 坏数据诊断演示
```

示例工程详解见 [`examples/game-config/`](examples/game-config/README.md) 与文档站「完整示例」页。其余常用命令：

```bash
cage init my-game            # 新项目：交互式脚手架（终端问答生成骨架）
cage init my-game --source csv --envs four --targets json,csv --git   # 全 flag 非交互
```

`cage init` 生成的项目零改动即可 `cage check .` 与 `cage build . --env dev` 通过。常用命令：

```bash
cd my-game                       # 在项目根（cage.toml 所在目录）内执行
cage check .                 # 只验证，不生成
cage build . --profile client  # 验证并生成目标产物
cage snapshot . --profile client  # 构建 + 打包带哈希账本的快照；快照目录 --verify 载入前按账本校验
cage gen . --profile server    # 只生成代码绑定与定义文件（cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema/template）
cage diff build/a build/b          # 比较两个配置版本
cage inspect . Item          # 查看 Schema 与配置结构（项目路径必填，表名可选）
cage migrate .               # 迁移计划预览（--write 落盘，--all 全链）
cage web ./                        # 启动 Schema 编辑器本地服务（只绑 127.0.0.1；/status/ 项目状态与构建新鲜度）
cage lsp ./                        # stdio Language Server（IDE 诊断，与 check 同一校验管线）
cage assets . --write assets.json   # 资产清单基线（blake3 逐文件盘点；--check 复核工作树）
cage graph . --format dot    # 表引用依赖图（text/dot/json，含拓扑序与环清单）
cage verify build/                 # 审计已生成产物（对照 manifest 账本逐文件重算 blake3）
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0   # 从两版 schema 起草迁移规则稿
```

## AI 助手（MCP）

`cage mcp --stdio` 把校验/结构/构建预演包成 [Model Context Protocol](https://modelcontextprotocol.io) 工具，让 AI 客户端（Claude Desktop、Claude Code 等）直接驱动真正的编译器：读 JSON 结构化结果、拿到错误码 + `file:line` 定位 + 可操作建议，然后告诉你怎么改。所有工具只读——不写盘、不改配置。

在 MCP 客户端里加一段（Claude Desktop / Claude Code 的 `claude_desktop_config.json` 或 `.mcp.json` 通用写法）：

```json
{
  "mcpServers": {
    "cage": {
      "command": "cage",
      "args": ["mcp", "--stdio"]
    }
  }
}
```

工具清单（参数 `path` 一律是配置工程根目录，默认当前目录）：

| 工具 | 用途 |
| --- | --- |
| `check_schema` | 校验配置（同 `cage check`）：返回错误码、`file:line`/行列表定位、原因与建议（缺字段/类型不符/环境未声明…） |
| `list_tables` | 全部表：主键、字段、行数、跨表引用、各表声明的环境 |
| `list_fields` | 字段定义：类型、必填、默认值、范围/长度/正则、枚举、引用、profile 可见性 |
| `list_envs` | 已声明环境（`--env` 合法值）与各表 `env_overrides` 差异（如 Shop.discount 四环境 0.9→0.5） |
| `build_dry_run` | 构建预演（不落盘）：产物清单（路径/大小/格式/表）、目标列表、schema/source 哈希与 build_id |
| `explain_config` | 解释 `cage.toml` 每节的语义、项目实际取值与配置优先级链 |

示例问答：AI 调 `check_schema` 得到

```json
{
  "code": "E1201",
  "file": "config/Item.csv",
  "where": "config/Item.csv:row 1:price",
  "message": "Value below minimum",
  "suggestion": "Minimum allowed: 0"
}
```

即可直接回答「把 `config/Item.csv` 第 1 行的 `price` 调回 ≥ 0，或放宽 schema 的 `min`」。不带 `--stdio` 跑 `cage mcp` 只打印工具清单，方便先看看有什么。

## 本地测试口径

CI 与本地开发按改动面三路分流——改一行文档不该全量编译。

1. **文档改动**（`*.md`、`docs/**`、图片）：不编译。主 CI workflow 的
   `paths-ignore` 已跳过这些路径。
2. **代码改动**：只跑改动子集，不全量。
   - `cargo test -p <改动 crate>`
3. **全量**（`cargo test --workspace --locked`）：仅以下情形触发——Cargo.toml / Cargo.lock、
   `.github/**`、Docker/Compose 配置、发布前。

## 一键安装

从滚动 [nightly Release](https://github.com/cuihairu/cage/releases/tag/nightly) 匿名直链下载二进制（不走 Actions artifacts、不带 token），自动校验 SHA256，无需 Rust 工具链。

**Linux / macOS**（`sh`）：

1. 执行安装（默认装最新 nightly）：

   ```sh
   curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh | sh
   ```

   安装脚本**默认**把安装目录加入 PATH：往 shell profile 追加一行
   `export PATH=...`（`~/.zshrc` / bash 登录 profile / `~/.profile`），写入前先备份为
   `<profile>.bak-<日期>`；重复执行是幂等的，最多留一行。脚本会打印写入的文件并提示
   `source` 或重开终端生效；写入失败（profile 不可写等）会显式报错并给出手动命令，
   不会静默。传 `--no-path` 可完全关闭 PATH 写入（此时只打印手动 `export` 行）。

   预期输出（末两行）：

   ```text
   安装完成：/home/<你>/.local/bin/cage
   cage 0.1.0
   ```

2. 验证（`source` profile 或重开终端后）：

   ```sh
   cage --version
   ```

   回滚 PATH 行：删除 profile 中 `# added by cage install.sh` 那一行，
   或直接还原安装脚本留在旁边的 `.bak-<日期>` 备份。

**Windows**（PowerShell 5.1+，无需管理员）：

1. 执行安装（默认装最新 nightly）：

   ```powershell
   irm https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.ps1 | iex
   ```

   预期输出（末两行）：

   ```text
   安装完成：C:\Users\<你>\AppData\Local\cage\cage.exe
   cage 0.1.0
   ```

2. 脚本默认把安装目录加入用户级 PATH（幂等；`-NoPath` 关闭并打印手动命令），重开终端生效。

3. 验证：

   ```powershell
   cage --version
   ```

**装指定版本**（默认最新 nightly；指定 Release tag 需该 Release 带对应平台资产）：

```sh
sh <(curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh) v0.1.0   # Linux / macOS
```

```powershell
irm https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.ps1 -OutFile install.ps1; .\install.ps1 -Version v0.1.0   # Windows
```

安装目录可用环境变量覆盖（`CAGE_INSTALL_DIR`，Windows 固定 `%LOCALAPPDATA%\cage`）；两个平台默认都把安装目录写进 shell profile / 用户级 PATH（`--path`（sh）保留为默认行为的兼容写法；`--no-path` / `-NoPath` 关闭）；`--uninstall`（sh）/ `-Uninstall`（ps1）卸载——删除二进制后，再手动从 profile / 用户级 PATH 移除对应条目即可。

平台覆盖：Linux x86_64（正式腿）、macOS aarch64 与 Windows x86_64（试运行腿）；其余架构暂无资产。脚本与本体一样随 `nightly` Release 分发（`install.sh` / `install.ps1` 资产），源码在 [`scripts/`](scripts/)。

## 每日构建

不装 Rust 工具链可直接取二进制：[Nightly Release](https://github.com/cuihairu/cage/releases/tag/nightly) 每日随 main 滚动更新（Linux x86_64 正式腿，macOS / Windows 试运行腿），压缩包内即 `cage` 单文件，附 `SHA256SUMS` 校验单。

## 文档

完整设计与使用文档：**https://cuihairu.github.io/cage/**

- [设计文档](docs/zh/design.md) · [需求整理](docs/zh/requirements.md) · [生命周期](https://cuihairu.github.io/cage/zh/lifecycle) · [UI 原型](https://cuihairu.github.io/cage/zh/design-prototypes) · [Roadmap](todo.md)

## 底座

Cage 基于 Rust 生态的开源库构建：Excel 读取用 [calamine](https://crates.io/crates/calamine)，序列化用 [serde](https://serde.rs) / serde_yaml / toml，命令行解析用 [clap](https://docs.rs/clap)，内容哈希用 [blake3](https://crates.io/crates/blake3)，本地 HTTP 服务用 tiny_http，远程注册表客户端用 ureq，CSV 读写用 csv；文档站由 [VitePress](https://vitepress.dev) 构建。Source/Target 适配器、验证流水线、Registry 等业务逻辑在本仓库内实现。

## 许可

[Apache-2.0](LICENSE)
