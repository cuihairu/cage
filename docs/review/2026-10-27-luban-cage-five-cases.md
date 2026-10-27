# 评审：Luban 同题对照五案例（技能嵌套/副本波次/多态/可选默认/客户端投影）

<!--
来源: 本仓自制对照实验（2026-10-27）
工具: Luban 5.1.0（DOTNET_ROLL_FORWARD=Major，dotnet 10）+ Cage（本仓 target/debug）
数据: docs/review/luban-five-cases/gen.py 一键生成（同一逻辑配置 → luban 与 cage 两套工程）
复现: PYTHONPATH=/usr/lib/python3/dist-packages python3 docs/review/luban-five-cases/gen.py
      cd docs/review/luban-five-cases/luban && DOTNET_ROLL_FORWARD=Major dotnet ~/tools/luban/Luban/Luban.dll \
          -t all --conf luban.conf -d json -x outputDataDir=out
      cargo build --bin cage && target/debug/cage build <proj> --profile client|server
性质: 本仓对照结论；每条「行为不同」均给出源码锚点或实测证据，非外部转述。
-->

## 结论摘要

Luban 的 Excel 表达力明显高于 Cage：**嵌套 bean、多行列表、类型标签多态、单元格分隔列表、
nullable 类型、空单元格默认值** 六项中，Cage 只有「nullable 的近似替代（Any）」一项可用，
其余五项要么无等价语法（必须平表分解 + 外键），要么行为不同（默认值不作用于空单元格）。

性能上两者在同一数量级：本数据集（5 表、约 1100 行主数据；Cage 平表分解后约 1950 行）
Luban 5.1.0 单次 CLI 构建 0.84s / 峰值 RSS ≈ 83.5 MB，Cage debug 构建 0.75–0.82s / ≈ 59 MB
（Cage 峰值内存低约 29%）。注意两者都是进程启动主导的数值，本实验不构成「速度/内存优劣」的
定论（见测量节）。

> **补充（2026-10-27 回流-1..6 落地后）**：本文是修复前的对照快照。文中「默认值不作用于
> 空单元格」一项（案例 4.2）已被回流-1 修复——`fill_field_defaults`
> （`crates/cage-core/src/normalize/mod.rs:116`）现在把 absent 与 Null（Excel 空单元格）
> 都填为 `FieldSchema.default`，与 Luban `#default` 行为一致。「类型不塌缩的空值」已被
> 回流-2（`nullable` 字段标志）修复，「单元格分隔列表」已被回流-3（`separator`，Luban
> `(list#sep=,)` 等价）修复，「整数值单元格入 Float 字段」已被回流-4（数值塔向上统一）
> 修复，「tagged union 类型」已被回流-6（`FieldType::TaggedUnion` + 单格 `sep="|"` 拆分，
> 输出 `{"$type":"Damage","amount":13,"element":"wind"}` 扁平对象）修复。剩余差距
> （多级表头/嵌套 bean/多行记录）即回流-5，按本文结论仍成立。

## 实验设置

一个 `gen.py` 生成同一份逻辑配置，分别落成 Luban 与 Cage 两套工程，数据完全一致：

| 表 | 行数 | 案例覆盖 |
|---|---|---|
| Hero | 300 | 可选字段 + 默认值 + 空值（空单元格） |
| Skill（+SkillEffect） | 200（约 400 效果行） | 多层嵌套 + 继承 |
| DungeonStage（+DungeonWave） | 150（约 450 波次） | 多波次 / 怪物组 / 奖励 |
| Gameplay | 250 | 类型标签多态（单格 `sep="|"`） |
| ShopItem | 200 | 客户端/服务端字段投影（group / targets） |

生成器与配置：`docs/review/luban-five-cases/gen.py`（自包含，重跑即重建全部 xlsx/配置）。
Luban 工程由 `##` 元行 Excel 驱动（`##var` / `##+` / `##type`，见 2026-10-23 对标）；Cage 工程为
`cage.toml` + `schemas/main.yaml` + 平表 xlsx。

## 五案例三分类

### 案例 1：技能多层嵌套 + 继承（Skill）

- **Luban**：`cost` 为嵌套 bean（表头合并 + `##+` 子行 `mp/cd`）；`effects` 为多态列表
  （`*effects` 多行 + `$type` 标签列 + `##+` 子行）。输出单文件嵌套对象。
- **Cage**：**未实现**——Excel 源只有单行表头（`crates/cage-source-excel/src/lib.rs:47-54`），
  无嵌套 bean、无表内子表、无多行列表、无类型标签列。被迫**平表分解**：`cost` → `cost_mp/cost_cd`
  两列；多态效果列表 → 独立 `SkillEffect` 表（`kind` enum + `skill_id` 外键 + `trigger_event/chance`
  平铺 + `damage/heal` 两列）。
- **行为差别**：数据不丢但结构丢失——消费者须自行 join，`Skill`/`SkillEffect` 两文件 vs Luban
  单文件嵌套对象；多态判别从 `$type` 变为 `kind` enum。

### 案例 2：副本阶段多波次 / 怪物组 / 奖励（DungeonStage）

- **Luban**：`waves` 为 `(list),StageWave` 多行列表；`monster_groups` 为 `(list#sep=,),string`
  （单元格 `"wolf,skeleton"` → 数组），`rewards` 为 `(list#sep=;),string`。
- **Cage**：**部分未实现**——Excel 单元格只能产出标量，无法产出数组
  （`value_matches_type` 只接受 `Value::Array` 对应 Array 类型，`crates/cage-core/src/validation/mod.rs:474`；
  calamine 单元格只有标量变体）。`(list#sep=)` 无等价物，`monster_groups/rewards` 只能以
  **原始字符串**承载（实测 `"2|1;4|1"` 原样输出）。波次结构靠 `DungeonWave` 平表 + `stage_id` 外键。
- **行为差别**：Cage 侧列表语义丢失（字符串需消费端再 split），且无 `min_items` 类约束的数组级校验。

### 案例 3：类型标签多态（Gameplay）

- **Luban**：抽象 bean `Gameplay` + 子类 `Damage/Heal/Buff`，单格 `sep="|"` 多态
  （`"Damage|410|wind"`），输出 `{"$type":"Damage","amount":13,"element":"wind"}`。
- **Cage**：**未实现**——无 tagged union 类型。workaround 为 `kind` enum + 每分支可选负载列
  （`amount/element/buff_id/duration`），类型安全丢失（负载列 `Any`，无按 kind 的字段约束）。
  输出 `{"kind":"Damage","amount":13,"element":"wind","buff_id":null,"duration":null}`。
- **行为差别**：Cage 用显式 `null` 占满所有非当前分支列；`Any` 负载列不校验类型。

### 案例 4：可选字段 + 默认值 + 空值 + 显式空串（Hero）

三处「实现但行为不同」，是本组最有价值的发现：

1. **nullable 类型无等价物**：Luban `string?`/`int?` 精确接受 `null|T`；Cage 无 nullable 类型字段，
   `Value::Null` 只匹配 `FieldType::Null`/`Any`（`crates/cage-core/src/validation/mod.rs:501,514`），
   可空字段只能声明为 `Any`（丢类型检查）。实测 `title/nickname` 声明 `String` 时，空单元格 → 300 行
   E1101。
2. **默认值不作用于空单元格**：Luban `#default=1` 填充空单元格（实测 i%3==0 行空 cell → 输出
   `level:1`）；Cage `FieldSchema.default` 文档为「applied when absent」
   （`crates/cage-core/src/schema/mod.rs:217`），但 **absent ≠ 空单元格的 Null**——实测 `default:1`
   的 Int32 字段遇 100 行空单元格 → E1101「Expected type: Int32, got: null」。默认值只在值真正
   缺失（对象源无该键）时生效，Excel 空单元格产生的是 Null，不触发默认。
3. **JSON 输出形态**：Cage 对 `Any` 空值输出显式 `"title": null`，Luban 省略缺失键——语义等价，
   但序列化形态不同（消费端按「键存在与否」判断会分歧）。

4. **显式空串与空单元格不可区分（两侧一致）**：对可空字段写显式 `""`（i%5==0 且 i%2==0 行），
   实测 Luban 与 Cage **都**将其坍缩为 null（`"title": null` / 键省略），`""` 计数为 0。
   需要区分「空串 vs 未填」的消费场景，两侧都做不到（在可空字段路径上）。此项为**一致**，
   非差异。

### 案例 5：客户端/服务端字段投影（ShopItem）

- **Luban**：`group="c"` / `group="s"` + `-t client` / `-t server` 目标导出。
- **Cage**：字段级 `targets: ["client"|"server"]` + profiles。
- **投影结果一致**：client = `id,name,price,icon`；server = `id,name,cost,margin`（实测双侧输出
  字段集与取值完全相同）。
- **实现但行为不同**：Cage 的 E9006 要求被投影隐藏的 required 字段必须带 `default`
  （`crates/cage-core/src/validation/mod.rs:333`）——实测 `price/icon/cost/margin` 四个 required 字段
  在反侧 profile 各报 1 个 E9006，全部补 `default` 后才可双端构建；Luban 的 group 无此约束。

## 附：过程中发现的 Cage 数值/枚举边界

| 现象 | 证据 | 分类 |
|---|---|---|
| Excel 整数值单元格被推断为 Int → Float32 字段 E1101 | 实测 `cost_cd` 40 行（`i%60==0` 时值为整数）报「Expected type: Float32, got: int」；数据被迫恒小数 | 实现但行为不同 |
| Enum 字段只接受字符串单元格 | 实测 `Gameplay.kind` 写数字 1/2/3 → 250 行 E1101；改写字面量名（`"Damage"` 等）通过 | 实现但行为不同 |
| 诊断定位质量 | Cage E1101 带 table/row/col/value/expected 精确锚点；Luban 默认文本日志（`--errorFormat json` 需显式开） | 一致（Cage 略优） |

## 测量（构建耗时 + 内存峰值）

数据集同上；每侧 2 次运行；`/usr/bin/time -v`；Cage 为 `target/debug` 未 strip 构建。

| 侧 | 命令 | wall（run1 / run2） | 峰值 RSS |
|---|---|---|---|
| Luban 5.1.0 | `dotnet Luban.dll -t all --conf luban.conf -d json -x outputDataDir=out`（含 .NET 启动） | 0.84s / 0.84s | 83.6 MB / 83.2 MB |
| Cage | `cage build <proj> --profile client` | 0.82s / 0.75s | 58.8 MB / 59.0 MB |

**解读（避免过度归纳）**：两者都是进程启动主导（数据量小、构建瞬时），此数值只说明
「同数量级、Cage 峰值内存约低 29%」；**不以单一速度数字评价整个系统**（与 2026-10-23 定位评审
「不构成 Luban 导出器重写」一致）。Luban 为一次性 CLI，无常驻服务对比项。

## 缺失能力回流（本仓条目建议）

按「未实现」与「行为不同」拆成可立项条目：

1. **Excel 空单元格默认值**：`FieldSchema.default` 语义从「absent」扩展到「Null 单元格」——这是
   本组最影响 Excel-first 工作流的差距（案例 4.2）。
2. **nullable 类型字段**：`string?`/`int?` 等价物，或「可空字段 Null 可匹配其基类型」开关
   （案例 4.1）。
3. **单元格分隔列表**：`(list#sep=,)` 等价物——Excel 源把 `"a,b"` 单元格解析为数组
   （案例 2）。
4. **数值塔统一**：Excel 整数值单元格可入 Float 字段（Float32 拒绝 Int 单元格）
   （附表现象 1）。
5. **多级表头 / 嵌套 bean / 多行记录 / 多态**：已在 2026-10-23 定位评审列为最大空缺，本组再次
   证实（案例 1、2、3），并入既有立项。
6. **tagged union 类型**：kind + 按分支载荷的类型安全表达（案例 3）。

## 证据文件

- 生成器与配置：`docs/review/luban-five-cases/gen.py`
- 本仓源码锚点：`crates/cage-source-excel/src/lib.rs:47-54`（单行表头）、
  `crates/cage-core/src/validation/mod.rs:333`（E9006）、`:474`（value_matches_type）、
  `:501,514`（Null 只匹配 Null/Any）、`crates/cage-core/src/schema/mod.rs:217`（default 语义）
