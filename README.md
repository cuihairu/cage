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

- Source Adapter 插件：Excel / CSV / JSON / YAML（后续 XML、SQLite、数据库、Google Sheets…），只读取解析，不做业务校验
- Schema 与 Source 解耦：同一 Schema 可用于多个配置源（类型/必填/默认值/范围/枚举/唯一/引用）
- 八级验证流水线：Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule，可分级执行（`cage check --level`）
- 跨配置引用校验：存在性之外还支持谓词与兼容性检查（引用对象存在但类型不合法同样报错）
- Diagnostics 一等公民：错误码 + 精确定位（文件/Sheet/单元格/字段/值）+ 修复提示
- 确定性构建：相同输入 → 相同产物、哈希与 Manifest（可追踪、可回滚、可增量）
- Profile 机制：client / server 不同 targets 与字段可见性，一份配置两端复用
- Configuration Snapshot：`cage snapshot` 打包自校验配置快照（manifest + schema + 产物 + 逐文件哈希账本），服务器启动载入前验证，篡改/增删文件即暴露
- Configuration Registry：`cage registry` 多版本配置仓库（发布/列表/审计/GC/移除），消费方 `registry:包@版本` 引用 + `[dependencies]` 版本 pin，远程 http(s) 根只读解析（缓存离线可用）
- Target 插件：JSON / CSV 数据产物起步，已扩展 C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java 代码绑定，后续 Protobuf 等（数据序列化与代码生成分离）

## 快速开始

```bash
bash examples/run.sh   # 一键跑通完整示例工程：校验 → 构建 10 个 target → 生成 8 语言代码 → E1601 坏数据诊断演示
```

示例工程详解见 [`examples/game-config/`](examples/game-config/README.md) 与文档站「完整示例」页。其余常用命令：

```bash
cage check config/                 # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage snapshot config/ --profile client  # 构建 + 打包自校验快照；快照目录 --verify 载入前校验
cage gen config/ --profile server    # 只生成代码绑定（cs/python/lua/ts/js/cpp/go/java）
cage diff build/a build/b          # 比较两个配置版本
cage inspect Item                  # 查看 Schema 与配置结构
cage web ./                        # 启动 Schema 编辑器本地服务（只绑 127.0.0.1）
```

## 一键安装

从滚动 [nightly Release](https://github.com/cuihairu/cage/releases/tag/nightly) 匿名直链下载二进制（不走 Actions artifacts、不带 token），自动校验 SHA256，无需 Rust 工具链。

**Linux / macOS**（`sh`）：

1. 执行安装（默认装最新 nightly）：

   ```sh
   curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh | sh
   ```

   预期输出（末两行）：

   ```text
   安装完成：/home/<你>/.local/bin/cage
   cage 0.1.0
   ```

2. 若提示安装目录不在 PATH 中，重开终端，或手动加入：

   ```sh
   export PATH="$HOME/.local/bin:$PATH"
   ```

3. 验证：

   ```sh
   cage --version
   ```

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

2. 脚本已把安装目录加入用户级 PATH，重开终端生效。

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

安装目录可用环境变量覆盖（`CAGE_INSTALL_DIR`，Windows 固定 `%LOCALAPPDATA%\cage`）；`--path`（sh）/ 自动（ps1）把安装目录写进 shell profile / 用户级 PATH；`--uninstall`（sh）/ `-Uninstall`（ps1）卸载——删除二进制后，再手动从 profile / 用户级 PATH 移除对应条目即可。

平台覆盖：Linux x86_64（正式腿）、macOS aarch64 与 Windows x86_64（试运行腿）；其余架构暂无资产。脚本与本体一样随 `nightly` Release 分发（`install.sh` / `install.ps1` 资产），源码在 [`scripts/`](scripts/)。

## 每日构建

不装 Rust 工具链可直接取二进制：[Nightly Release](https://github.com/cuihairu/cage/releases/tag/nightly) 每日随 main 滚动更新（Linux x86_64 正式腿，macOS / Windows 试运行腿），压缩包内即 `cage` 单文件，附 `SHA256SUMS` 校验单。

## 文档

完整设计与使用文档：**https://cuihairu.github.io/cage/**

- [设计文档](docs/design.md) · [需求整理](docs/需求整理.md) · [Roadmap](todo.md)

## 底座

Cage 基于 Rust 生态的开源库构建：Excel 读取用 [calamine](https://crates.io/crates/calamine)，序列化用 [serde](https://serde.rs) / serde_yaml / toml，命令行解析用 [clap](https://docs.rs/clap)，内容哈希用 [blake3](https://crates.io/crates/blake3)，本地 HTTP 服务用 tiny_http，远程注册表客户端用 ureq，CSV 读写用 csv；文档站由 [VitePress](https://vitepress.dev) 构建。Source/Target 适配器、验证流水线、Registry 等业务逻辑在本仓库内实现。

## 许可

[Apache-2.0](LICENSE)
