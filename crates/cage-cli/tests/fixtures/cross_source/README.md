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

项目读取约定：`source_roots.main = "config"`；Excel 工作表名 = 表名（`Item`/
`Drop`）；CSV 文件名主 = 表名；JSON 根键 = 表名；YAML 根键 = 表名。
