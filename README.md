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

它不绑定 Excel，也不绑定 JSON——Excel 只是一种输入源；它解决的不是「Excel 转 JSON」，而是**把分散、异构、面向人的配置数据，可靠地编译成经过验证、确定性构建的运行时配置资产**。

## 核心链路

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline（L0 Parse ~ L7 Game Rules）
      → Normalize / IR → Target Generators → Manifest / Hash
```

## 核心能力

- **Source Adapter 插件**：Excel / CSV / JSON / YAML（后续 XML、SQLite、数据库、Google Sheets…），只读取解析，不做业务校验
- **Schema 与 Source 解耦**：同一 Schema 可用于多个配置源（类型/必填/默认值/范围/枚举/唯一/引用）
- **八级验证流水线**：Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule，可分级执行（`cage check --level`）
- **跨配置引用校验**：存在性之外还支持谓词与兼容性检查（引用对象存在但类型不合法同样报错）
- **Diagnostics 一等公民**：错误码 + 精确定位（文件/Sheet/单元格/字段/值）+ 修复提示
- **确定性构建**：相同输入 → 相同产物、哈希与 Manifest（可追踪、可回滚、可增量）
- **Profile 机制**：client / server 不同 targets 与字段可见性，一份配置两端复用
- **Target 插件**：JSON / CSV 数据产物起步，已扩展 C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java 代码绑定，后续 Protobuf 等（数据序列化与代码生成分离）

## 快速开始

```bash
cage check config/                 # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage gen config/ --profile server    # 只生成代码绑定（cs/python/lua/ts/js/cpp/go/java）
cage diff build/a build/b          # 比较两个配置版本
cage inspect Item                  # 查看 Schema 与配置结构
```

## 文档

完整设计与使用文档：**https://cuihairu.github.io/cage/**

- [设计文档](docs/design.md) · [需求整理](docs/需求整理.md) · [Roadmap](todo.md)

## 许可

[Apache-2.0](LICENSE)
