# P0 三源一致性对拍 fixture

同一逻辑配置在 Excel / CSV / JSON（以及嵌套形专用的 YAML）三种物理描述下的
对拍素材。`cage build` 把每种格式编译为规范化 IR，测试据此断言：

- **公共核心**（`Item` + `Drop` + `ItemKind`，`common-schema.yaml`）三源
  逐字节一致：标量默认显式值、`i32` 上下界、`f32` 小数、布尔、字符串枚举、
  `Any` 空值、`Drop` 外键引用。
- **嵌套对象**（`nested.json` / `Nested.yaml` + `nested-schema.yaml`）JSON 与
  YAML 逐字节一致：Object / Array / Map 在统一模型里不失真。
  表格源（CSV/Excel）是扁平网格，没有对象表达面，不参与本对拍。
- **显式空串** 仅 JSON（及 YAML）可表示（`""` → `String("")`）；CSV/Excel
  把空单元格坍缩为 `Null`（`infer_and_parse_value` / `infer_string_type` 对
  空串返回 Null），这是表格源的保真缺口，测试把两边行为都断言出来。
- **空单元格 + 模式默认值**（`data-empty-tier.xlsx`）Excel 空 `tier` 单元格
  坍缩为 `Null` 后，被 `FieldSchema.default` 填充（回流-1），构建成功且输出
  携带默认值 `tier: 3`——与 `data.xlsx`（显式 `tier`）同值，不构成保真缺口。
- **空单元格 + nullable 字段**（`data-nullable.xlsx`）空 `note` 单元格在
  `nullable: true` 字段上被接受（回流-2），输出 `"note": null`；同一字段在
  非 nullable schema 上仍报 `E1101`（`common-schema.yaml` 的 `note` 无
  nullable，即回归锚点）。
- **单元格分隔列表**（`data-separator.xlsx`）`tags` 列的标量单元格
  （`"a, b"` / `"c,d"`）在 `Array<String>` + `separator: ","` 字段上于加载期
  拆成数组元素（回流-3，Luban `(list#sep=,)` 等价），token trim 后输出
  `["a","b"]` / `["c","d"]`；JSON 源自带数组，不走拆分。
- **整数值单元格入 Float 字段**（`data-int-weight.xlsx`）`weight` 列为整数
  单元格（2 / 3，加载为 `Int`），schema 声明 `Float32`——数值塔向上统一
  （回流-4）后构建成功，输出 `"weight": 2` / `"weight": 3`；反向（小数入
  整数字段）仍报 `E1101`。

项目读取约定：`source_roots.main = "config"`；Excel 工作表名 = 表名（`Item`/
`Drop`）；CSV 文件名主 = 表名；JSON 根键 = 表名；YAML 根键 = 表名。
