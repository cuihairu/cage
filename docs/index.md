---
layout: home

hero:
  name: Cage
  text: 游戏配置编译与验证框架
  tagline: 把分散、异构、面向人的配置数据，可靠地编译成经过验证、确定性构建的运行时配置资产
  actions:
    - theme: brand
      text: 从架构开始
      link: /architecture
    - theme: alt
      text: CLI 速览
      link: /cli
    - theme: alt
      text: GitHub
      link: https://github.com/cuihairu/cage

features:
  - title: Source Adapter 插件
    details: Excel / CSV / JSON / YAML 起步，只负责读取解析，不做业务校验；后续扩展 XML、SQLite、数据库、Google Sheets。
  - title: Schema 与 Source 解耦
    details: 同一份 Schema 可用于多个配置源：类型、必填、默认值、范围、枚举、唯一、引用独立定义。
  - title: 八级验证流水线
    details: Parse / Schema / Type / Value / Table / Reference / Semantic / Game Rule 逐级执行，cage check --level 可分级验证。
  - title: 跨配置引用校验
    details: 存在性之外还支持谓词与兼容性检查，引用对象存在但类型不合法同样报错。
  - title: Diagnostics 一等公民
    details: 错误码 + 精确定位（文件 / Sheet / 单元格 / 字段 / 值）+ 修复提示，CLI 与 Web UI 直接消费，IDE 留待后续。
  - title: 确定性构建
    details: 相同输入 → 相同产物、哈希与 Manifest，可追踪、可回滚、可增量。
  - title: Profile 机制
    details: client / server 配置不同 targets 与字段可见性，一份配置两端复用，不维护两套表。
  - title: Target 插件
    details: JSON / CSV 起步，已扩展 C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java 代码绑定（Protobuf 等留待后续）；数据序列化与代码生成分离。
---

## 核心链路

```text
Authoring Sources → Source Adapters → Canonical Model
      → Validation Pipeline（L0 Parse ~ L7 Game Rules）
      → Normalize / IR → Target Generators → Manifest / Hash
```

Excel 只是 Cage 的一种输入源。所有输入进入统一 Canonical Model，验证通过后按目标平台产出确定性资产，转换之外校验、引用、规范化都在同一条链上完成。

## 快速开始

```bash
cage check config/                   # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage gen config/ --profile server    # 只生成代码绑定（cs/python/lua/ts/js/cpp/go/java）
cage diff build/a build/b            # 比较两个配置版本
cage inspect Item                    # 查看 Schema 与配置结构
```

## 文档栏目

- [架构](/architecture)：定位、核心概念、插件模型与工程结构
- [Source](/source) / [Schema](/schema) / [Validation](/validation) / [Target](/target)：数据管线四阶段
- [CLI](/cli) / [Build](/build)：命令行与确定性构建
- [Web UI](/web)：第三阶段 Schema 编辑器（交换模型 W1、本地服务 W2、编辑器界面 W3 已实装）
- [Registry](/cli#registry)：第三阶段 R 系列配置仓库（本地发布/列表 + `registry:` 源与 Schema 解析 + `[dependencies]` 版本 pin + 远程只读解析 + verify 全册审计 / gc 滚动窗口 / remove 显式移除，R1–R4 已实装）
