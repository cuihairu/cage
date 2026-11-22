# Cage 评审：策划友好的 Schema 编辑器与 Python 插件架构

<!--
来源: 外部评审（ChatGPT 分享链 https://chatgpt.com/s/t_6ada304f5bb481919d1d605622432766）
整理日期: 2026-10-22
性质: 外部建议，非仓内事实；每条声称落地前须对照源码核验（见下表）。
-->

## 核验结论（对照 origin/main 源码，2026-10-22）

| 评审声称 | 核验 | 证据 |
|---|---|---|
| cage web 文档与 Web API 存在 | 属实 | docs/web.md 在库 |
| 示例工程用目录管理多文件 Schema | 属实 | examples/{asset-workspace,game-config}/cage.toml: schema_path="schemas" |
| 内置规则 GameRuleRegistry::with_builtins() 含游戏特定示例规则 | 属实 | crates/cage-core/src/validation/rules.rs:45 |
| 表达式引擎仅支持两操作数比较、无完整逻辑/算术/聚合 | 基本属实 | validation/expr.rs 词法/比较结构（lex/render/field_refs），无组合算子实现 |
| GameRuleValidator 是 Rust Trait 非动态插件系统 | 属实 | 定义在 validation/rules.rs |

结论：源码事实层可信。推进顺序采其建议：P0 编辑闭环 → P1 规则设计器 →
P2 Python Worker → P3 规则生态；P0 验收以「策划全流程」实测为准。

---


这意味着一个项目可以采用目录管理 Schema。但 Web 保存接口目前的处理对多文件模式并不完整，会遇到无法直接保存的问题。citeturn341940view1citeturn843508view3

对实际游戏项目而言，多文件是必然需求。

比如：

```text
schemas/
├── character.yaml
├── equipment.yaml
├── monster.yaml
├── item.yaml
├── skill.yaml
└── economy.yaml
```

不同策划负责不同模块。如果可视化编辑器只能读取聚合后的 Schema，却不能正确把修改写回原始文件，那这个编辑器就还无法成为正式的生产工具。

我建议从 Schema 加载阶段就保留独立的来源信息：

```text
TableDefinition
 ├── name: Monster
 ├── source_file: schemas/monster.yaml
 ├── source_revision: sha256:...
 └── fields: ...
```

`source_file` 和 `source_revision` 可以属于编辑器维护的来源索引，不一定要直接加进 Cage 的配置 Schema。

保存时必须完成版本冲突检查、仅修改涉及的源文件、原子替换写入，并尽可能保留未修改文件的注释与格式。

### 第二个问题：重命名应该是语义重构

现在字段和表可以在界面中更名，但更名操作不能仅修改名字。

例如把 `Item.id` 修改为 `Item.item_id`，需要同时处理主键定义、其他表引用，以及引用了原名称的规则表达式。

我建议引入：

```rust
SchemaRefactor {
    rename_table(...)
    rename_field(...)
    find_references(...)
    preview_changes(...)
    apply_changes(...)
}
```

这个能力应该放在 Rust Core，而不是由浏览器遍历 JSON 字符串实现。

这样未来 CLI、LSP、Web 编辑器、MCP 都能共享相同的 Schema 重构能力。

---

## 三、我建议的 Schema 工作台应该是什么样

**不要让策划先理解 YAML、字段类型和规则表达式，再去创建配置。应该让他们先表达配置设计意图。**

可以做成下面这种界面。

**Cage Studio**

(草稿有效) [设计原型]

配置模块

角色

怪物配置

道具配置

装备配置

技能配置

**怪物配置 · Monster** 字段设计

| **字段** | **类型** | **规则** |
|--------|--------|--------|
| 怪物 ID | `int` | 唯一、必填 |
| 等级 | `int` | 1～100 |
| 攻击力 | `int` | 非负数 |
| 掉落道具 | `ref` | 引用 Item |

可视化规则构建

当 怪物等级大于 10，并且攻击力大于等级对应上限时：

(触发错误) 攻击力超过允许范围

样例数据验证

1 条错误

| 怪物 ID | 等级 | 攻击力 | 结果 |
|--------|--------|--------|--------|
| 1001 | 5 | 120 | 通过 |
| 1002 | 12 | 2800 | 超出等级上限 |

建议的交互示意，不是 Cage 当前界面的截图。数据为虚构示例。

### 我推荐三层编辑模式

| 模式 | 面向对象 | 能力 | 是否需要写代码 |
|--------|--------|--------|--------|
| 基础模式 | 普通游戏策划 | 字段、类型、默认值、范围、枚举、引用 | 不需要 |
| 规则模式 | 系统策划、高级策划 | 条件组合、跨字段比较、规则模板、测试用例 | 不需要 |
| 专家模式 | 技术策划、程序员 | YAML、表达式、Python 插件、完整诊断 | 可选 |

这套设计有一个重要原则：**三个模式共享一份 Schema 语义模型，不应维护三套互不兼容的规则格式。**

基础模式与规则模式生成结构化规则 AST，再由 Rust Core 转换为规范化 Schema。

而在现有表达式引擎不能表达某项业务规则时，应该明确提示策划选择预装规则模板或 Python 扩展，不要假装所有复杂条件都能够自动转换成已有 `assert`。

### 策划需要的不是更多字段，而是规则模板

比如下面这些规则模板可以先做成内置能力。

| 规则模板 | 策划界面操作 | 底层实现 |
|--------|--------|--------|
| 主键唯一 | 勾选唯一标识 | Rust Schema |
| 数值范围 | 输入最小/最大值 | Rust Schema |
| 枚举选择 | 选择枚举列表 | Rust Schema |
| 外键关系 | 选择目标表及字段 | Rust 引用校验 |
| 字段比较 | 价格 A 不得高于价格 B | L6 表达式 |
| 条件必填 | 物品类型为装备时必须有装备部位 | 扩展规则 AST |
| 数值成长 | 怪物攻击力随等级满足公式 | 规则模板或 Python |
| 跨表总量 | 一组奖励权重总和为 100 | Rust 内置扩展或 Python |
| 配置可达性 | 技能升级链不能出现环 | Python 或专门的 Rust 校验器 |

更重要的是，每条规则都应该允许策划填写**中文标题、业务说明、错误信息和正反例**。

例如技术错误 `E1601` 对策划的价值很有限，但“怪物 1002 的攻击力 2800 超过等级 12 允许的上限 1250”就可以直接指导修复。

这意味着规则定义除了机器逻辑，还应该包含面向人的说明信息。

---

## 四、Python 插件：我非常推荐，但应该采用独立进程

目前 `GameRuleValidator` 是 Rust Trait，验证注册表在核心校验过程中构建。它并不是一个已经能直接加载项目级 Python 文件的插件系统。citeturn177851view0citeturn501915view0

对于游戏配置，我认为 Python 比要求所有业务规则都由 Rust 实现更合适。

原因不是 Python 更快，而是**很多游戏项目已经使用 Python 编写数据检查、资源工具、数据迁移和内部脚本**，策划或技术策划也更容易理解 Python 规则。

### 我推荐的架构

**Cage Studio / CLI / CI**

可视化操作、命令行、自动化构建

**Rust Core**

Schema · 规范化数据 · L0–L6 · Diagnostics

统一 L7 校验执行器

Rust 内置规则

稳定、高性能

Python Worker

独立进程 / IPC

**统一诊断结果 → 构建门禁 → Snapshot / Manifest**

同一套规则用于本地、Web 与 CI

### 为什么不建议直接嵌入 Python 解释器？

| 技术方案 | 优点 | 主要问题 | 推荐程度 |
|--------|--------|--------|--------|
| Rust 内嵌 Python（如 PyO3） | 调用方便、数据交换快 | GIL、解释器生命周期、崩溃隔离、发行复杂度 | 中 |
| Python 独立进程 + IPC | 故障隔离较好、兼容不同 Python 环境、易于限时 | 存在进程启动和序列化成本 | **首选** |
| Python 通过 WASM 运行 | 未来更便于限制能力 | Python 生态兼容与部署成本较高 | 后续评估 |
| C ABI 动态库插件 | 可支持高性能原生扩展 | ABI 稳定、跨平台、安全性复杂 | 不作为 Python 路线 |

Cage 设计文档已经考虑过动态库和 WASM 扩展方向。我的建议不是废弃它们，而是优先实现一种**与语言无关的插件协议**，再由 Python SDK 实现该协议。citeturn390010view3

这样未来 JavaScript、Lua、Go 等语言如果有明确需求，也不需要更换核心架构。

### Python 插件示例

下面是我建议的未来接口，不是现有 Cage 已支持的 API。

假设策划需要校验怪物攻击力的数值曲线。

plugins/monster\_balance/plugin.toml

```toml
id = \\






## （续）


```

同时让 Web、CLI 和 CI 使用相同的策略。

还有一点：当前内置 `GameRuleRegistry::with_builtins()` 中包含面向特定游戏属性的示例规则。citeturn177851view0

我更倾向于将这些规则改为显式选择的规则包，而不是所有达到 L7 的项目无条件套用某种游戏的数值曲线。Cage 是通用配置工具，不应该假定每个游戏都有相同的数值约束。

### 2\. Python 插件必须有工程级的失败处理和可复现性

独立 Python 进程并不等于安全沙箱。对于受信任的项目插件，它能改善崩溃隔离；对于不受信任的插件，则还需要容器或操作系统级的进程权限限制。

我建议明确以下行为：

| 情况 | Cage 应当如何处理 |
|--------|--------|
| 插件缺失 | 必需插件导致校验失败 |
| Python 版本不符 | 提前检查并给出可读提示 |
| 插件超时 | 终止 Worker、生成失败诊断 |
| 插件崩溃 | 不拖垮 Cage 主进程 |
| 插件输出非法结果 | 报协议错误，不静默忽略 |
| 依赖版本变化 | 影响验证环境指纹 |
| 插件参数变化 | 影响验证结果记录 |
| 插件报告错误 | 自动定位到表、行、字段 |

`Manifest` 已经记录构建标识和若干输入/产物信息，但未来如果校验插件会影响某个版本能否发布，就需要把插件标识、版本、配置摘要、依赖锁定信息或验证环境指纹记录到可审计的验证结果中。citeturn316824view2

可以不立即修改现有 Manifest 的结构，但至少应该引入一个与构建产物关联的 `ValidationAttestation`，避免“相同配置，不同插件环境，验证结果不同”却无法追溯。

---

## 七、建议如何拆成开发阶段

我不建议直接启动一个庞大的插件市场项目。先使现有编辑器成为生产可用工具，再引入插件。

**P0 — 修好 Schema 编辑闭环** (最高优先级)

多文件保存、字段和表的安全重命名、保存冲突、修改差异、快速 Schema 校验与完整草稿预检。修复新增引用字段时默认目标表选择异常等具体 UI 问题。

目标：策划能够安全修改一个真实项目，而不是只能编辑一个演示 Schema。

---

**P1 — 策划友好的规则设计器**

字段向导、类型推荐、表关系选择器、条件构建器、规则模板、中文诊断、样例数据与测试用例，以及 Excel/CSV 导入后的 Schema 草稿生成。

目标：常见配置结构和规则无需编写 YAML 或 Python。

---

**P2 — Python 插件运行时**

插件 Manifest、进程协议、Python SDK、统一诊断结果、超时和崩溃隔离、依赖锁定、CI 集成、规则单元测试。

目标：开发者能独立增加复杂校验，不需要修改 Cage Core。

---

**P3 — 可复用规则生态**

规则包版本化、跨项目复用、规则模板参数编辑、插件能力声明、升级兼容性检查，再按真实需求扩展数据源和产物生成插件。

目标：规则可分享、可维护、可测试，而不只是可以安装。

### 什么才算 P0 完成？

我建议用一个非常具体的策划操作流程验收：

从已有项目打开 `Monster` 表，增加一个字段，修改一个字段名称，给某个字段关联 `Item`，配置一个范围规则，拿现有 Excel 数据立即验证，看到准确的出错行，修复后预览差异，并保存回原本的 Schema 文件。

全程不需要手动编辑 YAML，不产生不必要的跨文件格式变化，不破坏其他字段引用；CLI 与 Web 的最终校验结果一致。

如果这个流程不能稳定完成，那么开发更多插件通常不会解决产品最核心的问题。

---

## 八、对 Cage 的最终技术选型

| 领域 | 我的选择 | 原因 |
|--------|--------|--------|
| 核心语言 | 保留 Rust | 确定性、性能、跨平台发行 |
| Schema 定义 | 保留现有 DSL，增加结构化规则 AST | 兼容现有语义并增强可视化能力 |
| 策划端 | 继续完善 `cage web` | 已有实现，不重复建设 |
| 前端框架 | 暂不强制迁移 | 先解决编辑行为与业务能力 |
| 规则基础能力 | Rust 内置声明式校验 | 快速、稳定、无需插件环境 |
| 自定义业务规则 | Python 独立 Worker | 易扩展、故障可隔离 |
| 插件通信 | MVP 使用版本化 JSON IPC | 简单、便于调试；性能瓶颈出现后再考虑二进制协议 |
| 插件发行 | Manifest + 锁定依赖 | 可复现、可审计 |
| UI 插件 | 优先声明式规则面板 | 避免任意第三方前端代码带来的复杂度 |
| 版本发布 | 复用现有 Snapshot/Registry | 不重复建设版本系统 |

对于前端是否升级到 React、Vue、Svelte，我不会现在就强制做技术迁移。现有纯 JavaScript 编辑器如果继续快速膨胀，组件状态、规则 AST 编辑和复杂表单确实会逐渐难维护；到规则设计器正式开发时，可以依据实际维护成本评估 Svelte 或 Vue。但这应该是 UI 工程选型，而不是为了“做得现代”去推翻已经能工作的后端。

还有一个边界需要明确：Cage 当前的 `cage web` 是本地优先、面向可信环境的工具，尚非具有完整认证、权限和多人协作机制的服务器产品。citeturn530597view0

我建议先把单人工作台与 Git 协作做扎实，真正需要集中式多策划协作时，再发展权限、审核、冲突合并和版本发布流程。不要直接把本地 Web 端口暴露为公网管理服务。

## 九、最终判断

我认为 Cage 目前的技术路线总体合理，特别是把格式读取、验证、规范化和产物生成集中在 Rust Core 这件事，具有长期价值。

但从你的实际使用场景来看，**当前最主要的短板不是编译能力，而是规则定义的可用性与可扩展性。**

我希望未来 Cage 能做到以下分工：

**普通策划**可以通过界面定义表结构、字段类型、引用、约束、规则及测试，不必理解底层实现。

**技术策划**可以组合规则模板、维护复杂数值约束、编写数据测试，必要时使用少量 Python。

**程序员**负责创建 Python 规则插件、编写自定义数据源或输出适配器，并管理依赖、版本和 CI。

**Cage Core** 则始终负责提供统一的数据语义、可复现校验、错误诊断与构建发布。

这样 Cage 才不仅是一个游戏配置编译器，而是一个具备独立扩展能力、真正适合策划使用的配置规则平台。

**如果由我决定开发顺序，我会明确采用：先修复多文件编辑与保存，再做无代码规则设计器，之后做 Python 插件运行时，最后才考虑更广泛的插件生态。**

本次结论来自仓库文档及 Schema、Web、Validation、CLI 等关键源码的检查；没有完成本地编译、全量测试或性能实测，因此上述问题是源码层面的判断，性能与兼容性仍需在实际工程中验证。

---







## （续）

# Cage 深度分析：策划友好的 Schema 编辑器与 Python 插件架构

**我的核心结论：Cage 应该继续保留 Rust 编译器内核，但产品重心需要从“程序员使用的配置编译工具”，进一步转向“策划可以自主定义配置规则、开发者可以扩展校验能力的配置工程平台”。**

你的两个想法都值得做：

1. **Schema 可视化编辑器应当成为策划的主要操作入口**，而不只是一个方便编辑 YAML 的工具。
2. **Python 插件非常值得引入**，尤其适合复杂的游戏配置校验、跨表业务约束和项目定制工具，但不建议把 Python 嵌入 Rust 主进程，也不建议让所有基础规则都依赖 Python。

## 一、首先明确：Cage 已经有可视化编辑器和插件基础

检查仓库后，一个重要发现是：这两项能力实际上都已经开始实现了，不需要从零重新设计。

| 模块 | 当前仓库情况 | 我的判断 |
|--------|--------|--------|
| Rust 配置编译核心 | 已实现多层校验、规范化、构建及产物生成 | 应保留 |
| Schema 可视化编辑器 | 已有 `cage web` 本地 Web 编辑器 | 需要提升策划体验 |
| 配置数据浏览 | 已有数据表查看相关功能 | 可继续扩展 |
| Schema 校验 | 支持类型、范围、枚举、引用、表达式等 | 基础扎实 |
| 自定义规则 | 已有 Rust `GameRuleValidator` 基础 | 需要动态插件化 |
| Python 插件执行 | 尚未看到完整独立 Python 插件运行体系 | 建议新增 |
| Python 代码生成 | 已有 Python Target | **与 Python 插件是两回事** |
| 配置版本管理 | 已有 Snapshot、Registry、Diff | 可以支撑审核与发布工作流 |

仓库的 https://github.com/cuihairu/cage/blob/main/docs/web.md 明确采用 Rust Core 负责 Schema 语义、前端负责编辑交互的设计，也已实现 Schema JSON 交换、YAML 保存及校验接口。citeturn944533view1

因此，我不建议推翻现有 Cage。真正值得调整的是：

**从“Schema 字段编辑器”升级为“规则设计工作台”，再给这个工作台增加安全、可测试、可复用的插件系统。**

## 二、现有实现最需要改造的地方

我进一步检查了 `cage-core` 的 Schema 模型、校验器、CLI、Web API 和前端代码。下面几个问题比较重要，而且会直接影响策划能否真正使用。

| 优先级 | 当前实现 | 实际问题 | 建议 |
|--------|--------|--------|--------|
| P0 | 编辑器已经提供字段表单 | 复杂规则仍需要输入表达式 | 增加可视化条件构建器 |
| P0 | Schema 可以通过 Web API 保存 | 多文件 Schema 项目目前存在保存限制 | 支持按源文件增量保存 |
| P0 | 编辑器可以执行 Schema 校验 | 编辑态校验与完整数据校验分离 | 增加草稿 Schema 对真实数据的完整预检 |
| P0 | 可以重命名表和字段 | 尚未形成完整的跨引用重构机制 | 增加引用追踪和安全重命名 |
| P1 | 有 Rust `GameRuleValidator` | 当前还不是动态插件系统 | 抽象统一插件执行器 |
| P1 | 已有 L7 游戏规则校验 | CLI 默认校验等级为 `semantic` | 根据项目策略启用必需插件校验 |
| P1 | Schema 序列化后保存 YAML | 需要关注注释、布局、覆盖冲突 | 引入修订检测、差异预览与安全写入 |

这些不是凭空设想。

例如，现有 `rules` 的编辑方式仍主要是填写断言表达式，而 `validation/expr.rs` 的规则语言主要支持两个操作数之间的比较，并不具备完整的逻辑运算、算术表达式和聚合规则能力。citeturn843508view2citeturn749177view2

### 最值得优先解决的：多文件 Schema

仓库示例工程已经使用：

```toml
schema_path = \\






## （续）


                ),
            )
```

这段代码背后，Cage 应自动完成数据读取、类型映射、错误收集和源位置关联。

Python 插件不应该自行打开 `Monster.xlsx`，也不应该自行理解 CSV、Excel、JSON 不同文件格式。它拿到的应当是经过 Rust Core 规范化后的数据。

这一点非常重要，否则每个插件都要重新实现一遍 Excel 解析、字段类型转换与报错定位，维护成本会迅速失控。

### Python 插件需要支持哪些能力？

我建议首先只开放四类能力。

| API | 作用 | 开放原则 |
|--------|--------|--------|
| `ctx.table()` | 读取规范化的配置表 | 只读 |
| `ctx.schema()` | 查询字段、枚举、引用定义 | 只读 |
| `ctx.error()/warning()` | 返回结构化诊断信息 | 受控写入诊断 |
| `ctx.options()` | 获取插件参数 | 项目配置驱动 |

不要在第一版就把修改原始数据、调用网络服务、执行任意构建命令全部暴露给插件。

配置校验插件最好是一个尽可能纯粹的函数：

\[
\\	ext{Diagnostics} = f(\\	ext{Schema}, \\	ext{Data}, \\	ext{Options})
\]

这样同一个插件在策划电脑、构建服务器及不同 CI 环境中，更容易产生一致结果。

---

## 五、插件机制不要只支持 Python：应该分层

我认为 Cage 不应该把所有扩展能力都笼统称为插件。

不同的扩展点有不同的稳定性和安全性需求。

| 扩展层 | 推荐机制 | 主要使用者 | 何时实现 |
|--------|--------|--------|--------|
| **规则模板包** | 声明式 JSON/Schema AST | 普通策划 | 最先 |
| **业务校验插件** | Python 进程协议 | 技术策划、程序员 | 第二阶段 |
| **Rust 原生校验器** | Trait / 内置注册 | 引擎开发者 | 已有基础，需改造 |
| 数据源插件 | 独立 Source Adapter | 工具开发者 | 有实际需求时 |
| 生成器插件 | 内置 Target、模板、外部 Worker | 程序员 | 有实际需求时 |
| UI 扩展 | 声明式面板、字段组件元数据 | 平台开发者 | 最后考虑 |

### 最先需要做的是规则模板包

假设开发团队编写一个技能成长规则插件，普通策划其实不应该直接看到 Python。

策划应该看到这样的规则配置：

**技能升级消耗规则** (规则模板)

基础消耗

[输入框的值不可用]

等级倍率

[输入框的值不可用]

预览等级 **Lv.**

> **滑块**
>
> 当前值不可用。
>
> **范围：** 1 至 10
>
> **步长：** 1

预期升级消耗 · 示例计算

基础消耗 × 等级倍率 \^ (等级 − 1)

这是拟议规则模板的交互示意，与现有 Python 插件无关。

策划只需要配置参数、选择对应字段，并通过实例测试验证。

底层由 Python 插件提供复杂业务逻辑。

**因此我建议 Cage 的最终插件设计是“开发者提供规则能力，策划配置规则实例”，而不是“策划通过界面管理 Python 脚本”。**

规则模板包应该能够描述参数类型、默认值、合法范围、字段选择器、帮助文案、诊断消息模板和测试示例。

这样插件真正服务的是策划，而不只是开发者。

---

## 六、两个现有架构问题必须一起处理

### 1\. L7 游戏规则可能被绕过

当前 CLI 的 `check` 和 `build` 默认校验级别为 `semantic`，而不是 `gamerule`。这意味着 L7 规则并非默认执行。citeturn749177view3

引入 Python 插件以后，如果仍维持这种行为，就可能出现：

策划在页面看到规则校验通过，但 CI 构建没有运行插件，或者 CI 构建通过但实际缺失必需插件。

建议引入项目级校验策略：

```toml
[validation]
required_level = "gamerule"
required_plugins = ["drop_rewards"]
```

策略声明后，CLI 的默认级别被抬高到声明的级别（绝不静默降级），必需插件缺失时以 E1602 使校验失败而非静默跳过。

这段代码背后，Cage 应自动完成数据读取、类型映射、错误收集和源位置关联。

Python 插件不应该自行打开 `Monster.xlsx`，也不应该自行理解 CSV、Excel、JSON 不同文件格式。它拿到的应当是经过 Rust Core 规范化后的数据。

这一点非常重要，否则每个插件都要重新实现一遍 Excel 解析、字段类型转换与报错定位，维护成本会迅速失控。

### Python 插件需要支持哪些能力？

我建议首先只开放四类能力。

| API | 作用 | 开放原则 |
|---|---|---|
| `ctx.table()` | 读取规范化的配置表 | 只读 |
| `ctx.schema()` | 查询字段、枚举、引用定义 | 只读 |
| `ctx.error()/warning()` | 返回结构化诊断信息 | 受控写入诊断 |
| `ctx.options()` | 获取插件参数 | 项目配置驱动 |

不要在第一版就把修改原始数据、调用网络服务、执行任意构建命令全部暴露给插件。

配置校验插件最好是一个尽可能纯粹的函数：

\[
\\	ext{Diagnostics} = f(\\	ext{Schema}, \\	ext{Data}, \\	ext{Options})
\]

这样同一个插件在策划电脑、构建服务器及不同 CI 环境中，更容易产生一致结果。

---

## 五、插件机制不要只支持 Python：应该分层

我认为 Cage 不应该把所有扩展能力都笼统称为插件。

不同的扩展点有不同的稳定性和安全性需求。

| 扩展层 | 推荐机制 | 主要使用者 | 何时实现 |
|---|---|---|---|
| **规则模板包** | 声明式 JSON/Schema AST | 普通策划 | 最先 |
| **业务校验插件** | Python 进程协议 | 技术策划、程序员 | 第二阶段 |
| **Rust 原生校验器** | Trait / 内置注册 | 引擎开发者 | 已有基础，需改造 |
| 数据源插件 | 独立 Source Adapter | 工具开发者 | 有实际需求时 |
| 生成器插件 | 内置 Target、模板、外部 Worker | 程序员 | 有实际需求时 |
| UI 扩展 | 声明式面板、字段组件元数据 | 平台开发者 | 最后考虑 |

### 最先需要做的是规则模板包

假设开发团队编写一个技能成长规则插件，普通策划其实不应该直接看到 Python。

策划应该看到这样的规则配置：







## （续）

>规则包版本化、跨项目复用、规则模板参数编辑、插件能力声明、升级兼容性检查，再按真实需求扩展数据源和产物生成插件。

### 什么才算 P0 完成？

我建议用一个非常具体的策划操作流程验收：

从已有项目打开 `Monster` 表，增加一个字段，修改一个字段名称，给某个字段关联 `Item`，配置一个范围规则，拿现有 Excel 数据立即验证，看到准确的出错行，修复后预览差异，并保存回原本的 Schema 文件。

全程不需要手动编辑 YAML，不产生不必要的跨文件格式变化，不破坏其他字段引用；CLI 与 Web 的最终校验结果一致。

如果这个流程不能稳定完成，那么开发更多插件通常不会解决产品最核心的问题。

---

## 八、对 Cage 的最终技术选型

| 领域 | 我的选择 | 原因 |
|---|---|---|
| 核心语言 | 保留 Rust | 确定性、性能、跨平台发行 |
| Schema 定义 | 保留现有 DSL，增加结构化规则 AST | 兼容现有语义并增强可视化能力 |
| 策划端 | 继续完善 `cage web` | 已有实现，不重复建设 |
| 前端框架 | 暂不强制迁移 | 先解决编辑行为与业务能力 |
| 规则基础能力 | Rust 内置声明式校验 | 快速、稳定、无需插件环境 |
| 自定义业务规则 | Python 独立 Worker | 易扩展、故障可隔离 |
| 插件通信 | MVP 使用版本化 JSON IPC | 简单、便于调试；性能瓶颈出现后再考虑二进制协议 |
| 插件发行 | Manifest + 锁定依赖 | 可复现、可审计 |
| UI 插件 | 优先声明式规则面板 | 避免任意第三方前端代码带来的复杂度 |
| 版本发布 | 复用现有 Snapshot/Registry | 不重复建设版本系统 |

对于前端是否升级到 React、Vue、Svelte，我不会现在就强制做技术迁移。现有纯 JavaScript 编辑器如果继续快速膨胀，组件状态、规则 AST 编辑和复杂表单确实会逐渐难维护；到规则设计器正式开发时，可以依据实际维护成本评估 Svelte 或 Vue。但这应该是 UI 工程选型，而不是为了“做得现代”去推翻已经能工作的后端。

还有一个边界需要明确：Cage 当前的 `cage web` 是本地优先、面向可信环境的工具，尚非具有完整认证、权限和多人协作机制的服务器产品。






## （续）

# Cage 深度分析：策划友好的 Schema 编辑器与 Python 插件架构

**我的核心结论：Cage 应该继续保留 Rust 编译器内核，但产品重心需要从“程序员使用的配置编译工具”，进一步转向“策划可以自主定义配置规则、开发者可以扩展校验能力的配置工程平台”。**

你的两个想法都值得做：

1. **Schema 可视化编辑器应当成为策划的主要操作入口**，而不只是一个方便编辑 YAML 的工具。
2. **Python 插件非常值得引入**，尤其适合复杂的游戏配置校验、跨表业务约束和项目定制工具，但不建议把 Python 嵌入 Rust 主进程，也不建议让所有基础规则都依赖 Python。

## 一、首先明确：Cage 已经有可视化编辑器和插件基础

检查仓库后，一个重要发现是：这两项能力实际上都已经开始实现了，不需要从零重新设计。

| 模块 | 当前仓库情况 | 我的判断 |
|---|---|---|
| Rust 配置编译核心 | 已实现多层校验、规范化、构建及产物生成 | 应保留 |
| Schema 可视化编辑器 | 已有 `cage web` 本地 Web 编辑器 | 需要提升策划体验 |
| 配置数据浏览 | 已有数据表查看相关功能 | 可继续扩展 |
| Schema 校验 | 支持类型、范围、枚举、引用、表达式等 | 基础扎实 |
| 自定义规则 | 已有 Rust `GameRuleValidator` 基础 | 需要动态插件化 |
| Python 插件执行 | 尚未看到完整独立 Python 插件运行体系 | 建议新增 |
| Python 代码生成 | 已有 Python Target | **与 Python 插件是两回事** |
| 配置版本管理 | 已有 Snapshot、Registry、Diff | 可以支撑审核与发布工作流 |

仓库的 [Web 编辑器设计](https://github.com/cuihairu/cage/blob/main/docs/web.md) 明确采用 Rust Core 负责 Schema 语义、前端负责编辑交互的设计，也已实现 Schema JSON 交换、YAML 保存及校验接口。






## （续）

/>

因此，我不建议推翻现有 Cage。真正值得调整的是：

**从“Schema 字段编辑器”升级为“规则设计工作台”，再给这个工作台增加安全、可测试、可复用的插件系统。**

## 二、现有实现最需要改造的地方

我进一步检查了 `cage-core` 的 Schema 模型、校验器、CLI、Web API 和前端代码。下面几个问题比较重要，而且会直接影响策划能否真正使用。

| 优先级 | 当前实现 | 实际问题 | 建议 |
|---|---|---|---|
| P0 | 编辑器已经提供字段表单 | 复杂规则仍需要输入表达式 | 增加可视化条件构建器 |
| P0 | Schema 可以通过 Web API 保存 | 多文件 Schema 项目目前存在保存限制 | 支持按源文件增量保存 |
| P0 | 编辑器可以执行 Schema 校验 | 编辑态校验与完整数据校验分离 | 增加草稿 Schema 对真实数据的完整预检 |
| P0 | 可以重命名表和字段 | 尚未形成完整的跨引用重构机制 | 增加引用追踪和安全重命名 |
| P1 | 有 Rust `GameRuleValidator` | 当前还不是动态插件系统 | 抽象统一插件执行器 |
| P1 | 已有 L7 游戏规则校验 | CLI 默认校验等级为 `semantic` | 根据项目策略启用必需插件校验 |
| P1 | Schema 序列化后保存 YAML | 需要关注注释、布局、覆盖冲突 | 引入修订检测、差异预览与安全写入 |

这些不是凭空设想。

例如，现有 `rules` 的编辑方式仍主要是填写断言表达式，而 `validation/expr.rs` 的规则语言主要支持两个操作数之间的比较，并不具备完整的逻辑运算、算术表达式和聚合规则能力。






## （续）

/>

我建议先把单人工作台与 Git 协作做扎实，真正需要集中式多策划协作时，再发展权限、审核、冲突合并和版本发布流程。不要直接把本地 Web 端口暴露为公网管理服务。

## 九、最终判断

我认为 Cage 目前的技术路线总体合理，特别是把格式读取、验证、规范化和产物生成集中在 Rust Core 这件事，具有长期价值。

但从你的实际使用场景来看，**当前最主要的短板不是编译能力，而是规则定义的可用性与可扩展性。**

我希望未来 Cage 能做到以下分工：

**普通策划**可以通过界面定义表结构、字段类型、引用、约束、规则及测试，不必理解底层实现。

**技术策划**可以组合规则模板、维护复杂数值约束、编写数据测试，必要时使用少量 Python。

**程序员**负责创建 Python 规则插件、编写自定义数据源或输出适配器，并管理依赖、版本和 CI。

**Cage Core** 则始终负责提供统一的数据语义、可复现校验、错误诊断与构建发布。

这样 Cage 才不仅是一个游戏配置编译器，而是一个具备独立扩展能力、真正适合策划使用的配置规则平台。

**如果由我决定开发顺序，我会明确采用：先修复多文件编辑与保存，再做无代码规则设计器，之后做 Python 插件运行时，最后才考虑更广泛的插件生态。**

本次结论来自仓库文档及 Schema、Web、Validation、CLI 等关键源码的检查；没有完成本地编译、全量测试或性能实测，因此上述问题是源码层面的判断，性能与兼容性仍需在实际工程中验证。

---







## （续）

/>

我更倾向于将这些规则改为显式选择的规则包，而不是所有达到 L7 的项目无条件套用某种游戏的数值曲线。Cage 是通用配置工具，不应该假定每个游戏都有相同的数值约束。

### 2. Python 插件必须有工程级的失败处理和可复现性

独立 Python 进程并不等于安全沙箱。对于受信任的项目插件，它能改善崩溃隔离；对于不受信任的插件，则还需要容器或操作系统级的进程权限限制。

我建议明确以下行为：

| 情况 | Cage 应当如何处理 |
|---|---|
| 插件缺失 | 必需插件导致校验失败 |
| Python 版本不符 | 提前检查并给出可读提示 |
| 插件超时 | 终止 Worker、生成失败诊断 |
| 插件崩溃 | 不拖垮 Cage 主进程 |
| 插件输出非法结果 | 报协议错误，不静默忽略 |
| 依赖版本变化 | 影响验证环境指纹 |
| 插件参数变化 | 影响验证结果记录 |
| 插件报告错误 | 自动定位到表、行、字段 |

`Manifest` 已经记录构建标识和若干输入/产物信息，但未来如果校验插件会影响某个版本能否发布，就需要把插件标识、版本、配置摘要、依赖锁定信息或验证环境指纹记录到可审计的验证结果中。






## （续）

>基础消耗 × 等级倍率 ^ (等级 − 1)

策划只需要配置参数、选择对应字段，并通过实例测试验证。

底层由 Python 插件提供复杂业务逻辑。

**因此我建议 Cage 的最终插件设计是“开发者提供规则能力，策划配置规则实例”，而不是“策划通过界面管理 Python 脚本”。**

规则模板包应该能够描述参数类型、默认值、合法范围、字段选择器、帮助文案、诊断消息模板和测试示例。

这样插件真正服务的是策划，而不只是开发者。

---

## 六、两个现有架构问题必须一起处理

### 1. L7 游戏规则可能被绕过

当前 CLI 的 `check` 和 `build` 默认校验级别为 `semantic`，而不是 `gamerule`。这意味着 L7 规则并非默认执行。

## 落地跟踪（勾一项补 commit hash+证据）

- [x] P0-1 多文件 Schema 按源文件增量保存（来源索引 source_file/revision、版本冲突检查、只改涉及文件、原子替换、保留未改文件注释格式）——13f2d7c
- [x] P0-2 保存差异预览（修改 diff 展示后再落盘）——764a155
- [x] P0-3 草稿 Schema 对真实数据的完整预检（编辑态校验与数据校验合并口径）——84673da
- [x] P0-4 安全重命名：SchemaRefactor（rename_table/rename_field/find_references/preview/apply 放 Rust Core，覆盖主键/跨表引用/规则表达式）——5163bfa（cage_core::refactor + `/api/schema/rename` + 前端接线；单测 9 + web 单测 +1 + 集成 +1 全绿，examples/run.sh 与 web-smoke 均 EXIT=0）
- [x] P0-5 修「新增引用字段默认目标表选择异常」等具体 UI 问题（走查取证后修）——14ceb7b
- [x] P0-6 P0 验收流程实测：打开 Monster→加字段→改名→关联 Item→范围规则→Excel 数据验证→准确报错行→修后 diff 预览→保存回源文件，全程不手编 YAML；CLI 与 Web 校验结果一致——9526c61（crates/cage-cli/tests/web/p0_acceptance.rs：真实子进程 + 真实 HTTP + 真实 Excel 数据（merged_cells.xlsx）；循环全绿，CLI `check` 对同一份草稿报出与 Web 相同的单一 E1501、同一 Excel 行（Sheet Items / Row 4）；workspace 1278 测试 + run.sh + web-smoke 全绿）
- [x] P1-1 三层编辑模式（基础/规则/专家）共享一份 Schema 语义模型（禁三套规则格式）——6953b63（顶栏模式切换，三层读写同一 `state.doc` / 同一 `field.rules`（ExpressionRule）数组；「禁三套规则格式」契约落 node --test 纯模块 `docs/public/editor/modes.js`（10 项：回环/原子拒绝/契约/addRule 顺序）+ 规则模式（复用表达式规则编辑器，P1-2/P1-3 接入层）+ 专家模式（原始 JSON 文档，`applyExpertText` 原子应用）；web 单测 80 绿 / 集成 19 绿（页面带 mode-switch 三档 + `/modes.js` 资产断言）/ workspace 1278 全绿 / web-smoke 绿）
- [x] P1-2 结构化规则 AST + 可视化条件构建器（生成 AST 由 Rust Core 转规范化 Schema）——30ff2e1（`cage_core::rule_ast`：`Compare`/`And`/`Unparseable` 创作期 AST 单向编译到唯一 `field.rules`（ExpressionRule）表示，Schema 永不存储 AST，禁三套规则格式不破坏；表达式引擎表达不了的（数组/对象/null、双引号字符串、超 `i64` 的 UInt）以 `CompileError::Unsupported` 带人话拒绝，编译端点应答 `unsupported:true` 供边界提示（P1-5）；规则模式可视化构建器只产 AST（行 = `字段 OP 字面量/同行字段`，跨字段比较、替换/追加写入、「从既有规则装载」经 decompile 反编译回行，无法可视化的跳过提示）；rule_ast 单测 8 + web 单测 3 + 集成 1（真实 HTTP 回环）+ modes.test.mjs 12 绿 / web 集成 20 绿 / workspace 1290 全绿 / web-smoke 绿）
- [x] P1-3 内置规则模板九项（主键唯一/范围/枚举/外键/字段比较/条件必填/数值成长/跨表总量/可达性）——04ef9d7（`rule_templates.js` 纯逻辑目录（node --test 8 项）：五项可表达模板只落 Schema 既有表示——path 补丁（primary_key/min/max/枚举类型/reference）或 AST（字段比较经 /api/rule/compile 编译进唯一 `field.rules`，前端仍不手写 assert）；表达不了的四项（条件必填/数值成长/跨表总量/可达性）**不假装自动转换**——应答 boundary 引导 Python 扩展（P2-2）/专门 Rust 校验器（P3-2），规则模式醒目呈现（P1-5 使更醒目）；表单层参数取自文档（字段/枚举/表下拉），不误配；web 集成 20 绿 / workspace 1290 全绿 / node 20 绿 / web-smoke 绿）
- [x] P1-4 规则人话层：中文标题/业务说明/错误信息/正反例入规则定义——c20d13e（`ExpressionRule` 增可选 `title`/`description`/`examples`（省略不序列化，老 Schema 一字节不变，测试钉住；编辑器文档原样透传，无第二套规则格式）；`cage_core::rule_examples::check_examples` 用校验主链同一表达式引擎试算正反例（沿用 L6 语义：缺字段按通过、不可比按违反），逐例回报「期望 vs 实际」+ 不符人话备注，纯提示不改文档数据；`POST /api/rule/examples`；编辑器共用规则编辑器加中文标题/业务说明输入、正反例子编辑器（样例行 JSON + 期望 pass/violate + 备注）与「试例」按钮（试算 N/M）；构建器 RuleMeta 带标题/说明，替换写入 `mergeHumanLayer` 合并同 assert 既有规则人话层（无关重编译不丢策划填写），「从既有规则装载」带人话层；rule_examples 单测 8 + web 单测 1 + 集成 1（真实 HTTP 判定）+ modes.test.mjs 15 / web 集成 21 绿 / workspace 1300 全绿 / web-smoke 绿）
- [x] P1-5 表达能力边界提示：现有引擎表达不了的规则明确引导模板或 Python 扩展，不假装可自动转换——e35ed07（编辑器边界提示 `renderBoundaryNotice`（rule_templates.js 纯逻辑 `boundaryGuidance`/`EXPRESSIVENESS_LIMITS`，node 可测）：标题点名具体拒绝原因、引擎限制清单、两条诚实出路（内置模板 / Python P2-2 / 专门 Rust 校验器 P3-2）；构建器编译被拒与模板 boundary 共用同一提示，绝不假装能自动转 assert；rule_templates.test.mjs 10 绿 / workspace 1300 全绿 / web-smoke 绿）
- [x] P1-6 Excel/CSV 导入→Schema 草稿生成——0b6dafd（`cage_core::schema_infer` 纯推断：逐列按观测值定类型（Bool/Int/UInt/Float64/String/Null）+ 必填/可空按空白单元格 + 数值 min/max + 低基数字符串列（2..=16 不同值）建议行内 enum_values + 主键取每行有值且唯一的首列，空/重名表头合成新名，warnings 标注枚举提升与混型回退；`POST /api/import/schema` 经构建管线同一套源适配器解析（xlsx/xls/ods/csv）再推断，不写任何文件；编辑器「导入草稿」模态框：路径候选取自 /api/tables、生成草稿逐表预览（名称/类型/必填/枚举、主键标 🔑）、应用为当前文档替换 state.doc 标脏走常规保存链；schema_infer 单测 13 + web 单测 1 + 集成 1 + 资产断言；workspace 1315 全绿 / node 25 绿 / web-smoke 绿）
- [x] P2-1 插件协议（语言无关、版本化 JSON IPC MVP）+ Python SDK——07016cd（新 crate `cage-plugin-protocol`：PROTOCOL_VERSION=1、Request/Response/WireDiagnostic/ProtocolError 线格式、run 单方法、单发单收 NDJSON、版本/畸形/未知方法/深载荷拒绝且版本先于形状检查；Python SDK `sdk/python/cage_sdk`：Plugin/RunContext 四能力（schema/table/options 只读 + error/warning/info 受控诊断）、serve/run、插件异常捕获（400 UTF-8 字节截断）、诊断形状校验（PLUGIN_INVALID_RESULT）；跨语言互操作测试：Rust 构造请求→真实 python3 子进程→SDK 解码运行→Rust 解码应答（3 条：成功诊断回读/插件异常/版本不符）；docs/plugins.md+zh 新页入导航；crate 单测 11 + 互操作 3 + python unittest 34 + 示例插件对 sample_request 端到端；workspace 1329 全绿 / node 25 绿 / web-smoke 绿 / docs:build 绿）
- [x] P2-2 Python 独立进程 Worker（禁嵌解释器；超时/崩溃/非法输出/缺插件/版本不符七类失败行为表落地）——7242c08（`cage_core::plugin`：`python3 <entry>` 独立子进程跑插件，永不嵌解释器；`plugins/<id>/plugin.toml`（id/version/runtime=python/entry/python 约束 `>=X.Y`|`==X.Y.Z`/timeout_secs），entry 拒绝对路径与 `..` 逃逸；七类失败表 → `WorkerFailure` 七变体全部映射 gamerule 既有码：Missing→E1602，ProtocolVersionMismatch/PythonVersionMismatch/Timeout/Crash/InvalidOutput/PluginError→E1603；Python 版本 spawn 前预检（`python3 --version`）；超时沿用 convention/script.rs 线程 + `try_wait` 循环 + 死线 kill（`wait_with_output` 排空管道，>64KB 输出不死锁不误报超时），stderr 400 字节字符边界截断；插件 findings→E1601 且 table/row/field 解析回真实 `SourceLocation`（file/sheet/row）+ source；协议 crate 补 `Request::to_bytes`（NDJSON 帧）；单测 17：清单解析/约束形式/路径逃逸/七类失败各一（真实子进程：缺入口、`>=99.0` 预检、崩溃带 exit code+stderr、非 JSON 输出、2s 超时及时 kill）/真实 SDK 插件三档诊断+定位回读/插件异常/协议错误映射/截断助手（CJK 不劈字符）；workspace 1346 全绿 / node 25 绿 / python 34 绿 / web-smoke 绿 / docs:build 绿）
- [x] P2-3 统一 L7 校验执行器（Rust 内置 + Worker 插件同诊断面）——ab986b3（`GameRuleRegistry` 扩为统一执行器：注册表三种条目——进程内 Rust 验证器（`register`）/数据声明内置规则（`register_rule`）/独立进程 Worker 插件（`register_worker(root, manifest, options)`），单一 `next_order` 决定确定性执行序；`run` 单入口把三来源诊断并进同一 `Vec<Diagnostic>`：Worker findings→E1601 原样并入、`WorkerFailure`→`to_diagnostic()`（E1602/E1603），绝不静默跳过；`register_worker_dir`/`discover_workers(root)` 扫描 `plugins/<id>/plugin.toml` 按 id 序确定性发现，坏清单在 run 时以 E1602 现形（缺 plugins/ 目录视为零插件非错误）；执行器测试 4：真子进程 worker（SDK LevelCap，cap=10）与内置 power_curve 同面各报 E1601 / 不可加载插件 → E1602 校验失败 / 发现顺序 + 坏清单不静默 / 缺目录零插件；workspace 1349 全绿（cage-core lib 467）/ node 25 绿 / python 34 绿 / web-smoke 绿）
- [x] P2-4 ValidationAttestation：插件标识/版本/配置摘要/依赖锁定/验证环境指纹进可审计记录——5d6d3cb（`cage_core::plugin::attestation`：不动现有 Manifest 结构，与构建产物关联；`ValidationAttestation` = schema blake3 + document blake3 + `ValidationEnvironment`（cage 版本/协议版本/python 解释器版本/OS，仅 worker 参与时探测解释器子进程）+ 每插件 `PluginAttestation`（id/version/`options_digest`=选项 JSON blake3 配置摘要/`content_digest`=插件目录全部文件按路径序 blake3 依赖锁定，含 requirements.txt）+ `AttestationOutcome::Ran{errors,warnings,infos}`|`Failed{class,detail}`（七类 class 名与 `WorkerFailure::class()` 一一对应）+ `passed`；`digest()`=整记录 blake3（同输入同摘要，改动任一输入即变）；执行器 `run_attested()` 返回（诊断，记录）与 `run()` 同一执行序，坏清单/崩溃/缺插件同样入记录（failed 不静默）；单测 10：摘要确定性/改文件即变/子目录依赖/环境无 python/记录 passed 与 JSON 往返/真子进程 worker 记录身份+环境+复现摘要/改 options 摘要变/崩溃记 failed 类名/缺清单记 missing/无 worker 无解释器探测；workspace 1372 全绿（cage-core lib 477）/ node 25 绿 / python 34 绿 / web-smoke 绿）
- [x] P2-5 CLI 默认 semantic 等级按项目策略启用必需插件校验——271a2c6（`cage.toml [validation]`：`required_level` 只抬高 CLI 默认等级绝不降级，无法解析值=硬错误 exit 2；`required_plugins` 强制 L7 关卡运行且声明 id 不在 `plugins/` → E1602 校验失败（合成不可加载 worker，同步进 P2-4 审计记录），已注册坏清单不重复报；`effective_validation` 单点解析，check/build/lsp 经共用 `validate_project` 生效，web 编辑器 `/api/validate/data` 预检同套应用——页面与 CI 不再分叉；`ValidationContext.registry` 注入（新 `validate_with_policy`），None 回落内建注册表；测试 +15：cage-core 4（policy TOML 往返/必需缺 E1602+attested missing/在场不误报/坏清单不重复）+ CLI 单测 5（无策略保持 semantic/只升不降/坏值硬错误/required_plugins 强制 gamerule 缺则 E1602/semantic 声明不变）+ gamerule 集成 4（默认级被策略抬高报 E1601 且 clean 报 level<=gamerule/必需缺 E1602/坏值 exit 2/build 同受门禁）+ web 2（预检 level=gamerule/必需缺 E1602）；评审 §六.1 garbled TOML 同步修复；workspace 1375 全绿（cage-core lib 481）/ node 25 绿 / python 34 绿 / web-smoke 绿 / docs:build 绿）
- [x] P3-1 内置游戏特定示例规则改显式选择的规则包（Cage 通用不假定数值曲线）——6d8933f（`cage_core::validation::rules::RulePackage`：`power_curve` 样例改为显式选择包，`with_builtins()` 删除、换 `with_packages(&[RulePackage])`；`[validation] rule_packages` 进 cage.toml 策略面（`ValidationPolicy`），选中即强制 L7（同 `required_plugins`），未知名硬错误 exit 2 并点名已知名表；L7 回退路径（`validate_game_rule` registry=None）改跑零游戏规则——通用项目不再默认套数值曲线；CLI `effective_validation` 单点解析，check/build/lsp/web 预检同套；测试 +8 净新增：cage-core 3（无选择零规则 / 包名往返+未知名拒 / 选中包 E1601 经 `validate_with_policy`）+ CLI 单测 3（选包强制 L7 且样例真发 E1601 / 未知名硬错误 / 无选择空注册表）+ gamerule 集成 2（裸项目 `--level gamerule` 不跑样例 OK / 未知名 exit 2）；既有 5 处 `with_builtins` 调用点全部改显式选包；docs validation/design/cli/requirements 中英同步；workspace 78 套件 1383 测试全绿 / clippy -D warnings 0 错 / fmt 干净）
- [x] P3-2 规则包版本化/跨项目复用/参数编辑/能力声明/升级兼容检查——592aeed（`cage_core::validation::rules`：包升级为可复用单元——`RulePackageSelection`（包 + 版本固定 + 参数覆盖）、`RulePackage::version()`（自带 `1.0.0`）与 `required_fields()`（能力声明）；`GameRuleRegistry::with_package_selections` 校验固定串与参数后建校验器（`with_packages` 保留默认路径）；`[validation.rule_packages_config.<name>]` 进 cage.toml 策略面（`RulePackageConfig`：`version` semver 固定串 + `params` 覆盖，`BTreeMap<String, serde_json::Value>`），CLI `effective_validation` 单点解析——同主版本固定接受（`1`/`1.0.0`/`*`），异主版本或无法解析是硬用法错误 exit 2（升级兼容检查，点名自带版本 1.0.0），未知键/非数值参数、配置未选择包的键均硬用法错误；`power_curve` 参数化为 `attack <= level * slope + intercept`（默认 100/50），hint 带配置后公式；能力声明落 `E1609`——schema 无表声明 level+attack 时选中包报 E1609 并 fail check，不再静默空跑；跨项目复用：包无状态按名寻址，同一选择块任意项目生效；测试净新增：cage-core 8（参数覆盖改曲线+hint/包规格跨注册表复用/未知键拒/非数值拒/同主版本三形式接受/异主版本拒/无法解析拒/无能力 E1609/能力字段清单）+ CLI 单测 5（参数端到端生效 E1601 与公式 hint/异主版本硬错误点名 1.0.0/配置未选包硬错误/无能力 E1609/非法参数硬错误）+ gamerule 集成 3（参数配置 E1601 端到端+公式 hint/异主版本 exit 2 且同主版本通过/无字段 E1609 exit 1）；既有 6 处 power_curve 测试与 4 处 executor 测试改用带 level/attack 的 schema；docs validation/design/cli/requirements 中英同步（E1609 进码表，P3-1 陈述更新为含 P3-2）；workspace 78 套件 1400 测试全绿 / clippy -D warnings 0 错 / fmt 干净）
- [x] P3-3 边界守住：cage web 保持本地优先可信环境工具，不做公网管理服务；先单人工作台+Git 协作——50bc91d（决策记录落 docs/web.md「可信环境边界（P3-3）」中英 + docs/ui.md 远程访问口径补 P3-3 段中英 + docs/design.md §35 与 docs/architecture.md 安全与隔离节中英同记：cage web = 本地优先、可信环境的**单人工作台**，非托管多用户管理服务——无账号/登录/角色、无远程管理、无密钥管理、无作者间多人协调；协作模型 = **Git**（Schema 文件是仓库真相源，合并冲突由 Git 自身工具化解，`cage check` 是合并树的共同仲裁者），绝非 Web 服务器；重估条件 = 仅真实集中式多策划场景出现，届时鉴权/权限/评审流/冲突合并/发版流程**作为产品设计**（评审 §八），不往本工具补丁式加装；边界为已执行且测试钉住（实装先于本项、本次核实取证）：伺服循环硬编码绑 `127.0.0.1`（web/mod.rs:85）且 CLI 无 `--bind`（`Commands::Web { path, port }`，`--port` 文档自带「binds 127.0.0.1 only」），绑定失败测试断言伺服循环尝试的地址（web/tests.rs:3175）——改绑定地址即挂套件；`local_boundary_violation` Host/Origin 门（非回环 Host / 缺 Host / 跨源 POST → 403，DNS 重绑定与跨站写守卫）由 `local_boundary_checks_host_and_origin` 钉住（web/tests.rs:2283）；8 MiB 请求体上限（413）+ 30s 读取预算（408）；上传双重把关 source_roots（symlink 逃逸拒）、产物仅按构建 manifest 台账伺服且 blake3 重验后发；远端访问走 SSH 端口转发，`--bind`/LAN 暴露必须先有鉴权、保持已登记卡点条件——本地无鉴权服务暴露到 LAN 是安全回退，不为便利破例；workspace 78 套件测试全绿（exit 0）/ clippy -D warnings 0 错 / fmt 干净 / docs:build 绿）

