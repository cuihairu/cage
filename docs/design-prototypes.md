[English](design-prototypes.en.md) | 中文

# UI 设计原型（待评审）

两个静态 HTML 原型，只做设计、不接代码。**待评审通过后再实现**。

源码在 `docs/design/`（本目录随仓入库，浏览器直接打开即可交互）：

- `docs/design/init-wizard.html` —— 交互式 `cage init` 问答流程
- `docs/design/validation-result.html` —— 配置校验结果可视化

## 一、`cage init` 问答流程

9 题终端问答。每题可 `b` 回退上一题，留空取方括号默认值；全部 flag 化即可
非交互跑 CI。

![cage init 交互问答原型（第 4 步 · 环境套）](design/init-wizard.png)

### 交互契约

- 每题为单行输入，回车确认，`b` 回退，留空取默认值。
- 多选题（目标生成器）用逗号分隔，编号/值均可。
- 未知取值提示可选值并重问，不静默兜底。
- 生成前打印待生成清单并要求确认。
- 同目录已存在工程时拒绝覆盖并 exit 1（不覆盖用户数据）。

与现状 `crates/cage-cli/src/init.rs` 的 9 题顺序一致：项目目录 → 项目名 →
配置源类型 → 环境套 → 目标生成器 → 模板风格 → git init → 带示例表 → 确认。

## 二、配置校验结果可视化

三视图：**错误列表**（错误码 + 定位 + 建议）、**结构树**（schema 与字段
约束）、**环境差异**（四套 `env_overrides` 收敛热力图）。

![校验结果 · 错误列表](design/validation-result.png)

![校验结果 · 结构树](design/validation-result-tree.png)

![校验结果 · 环境差异](design/validation-result-env.png)

### 交互契约

- 错误列表支持按表/级别过滤；点击定位跳到源文件对应行。
- 一键复制诊断 JSON（与 `cage mcp check_schema` 的返回同构，可直接喂给 AI）。
- 结构树展示 schema 与字段的真实层级、类型、约束与 profile 可见性。
- 环境差异用热力图呈现四套 `env_overrides` 的字段级约束收敛（越往 prod 越严）。
- 默认只读：诊断与修复建议分离，不在这里改数据。

诊断数据来自真实的 `cage check` 输出：E1201（取值低于最小值）、E1101（类型
不符）、Shop.discount 四环境 `max` 0.9 → 0.7 → 0.6 → 0.5、
`assert: min_level <= max_level`（L6）与 `hp <= attack`（L7）。

## 三、待拍板

1. 是否做 Web UI 形态（扩展现有 `cage web` 的 `/status/`），还是保持 CLI +
   MCP 双门面、Web 只做 Schema 编辑？
2. 结构树是否需要跨表引用的高亮联动（点 `Ref(Item.id)` 跳到 Item 表）？
3. 环境差异热力图是否需要"合并相邻同值环境"的折叠（如 staging/prod 一致时）？
