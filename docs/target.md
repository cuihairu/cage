# Target：规范化与产出

验证通过后进入规范化（Normalize），再由 Target Generator 产出目标资产（Artifact）。

## Normalize（归一化）

流水线里的 Normalize 做的是**表示层**归一，不是跨型转换：

- 字符串 trim、去除行尾 `\r`；
- 浮点 `-0.0` → `0.0`；
- 对象键按 `BTreeMap` 排序输出。

类型归属在源头确定、在 L2 严格把关：文本源解析期做类型推断（CSV 的
`true`/`false`/`yes`/`no` → Bool、整数/浮点文本 → Int/Float、空 →
Null），L2 按**类型族**校验（Int 只配 Int8–Int64、UInt 只配 UInt8–
UInt64、跨族即 `E1101`，不做字符串↔数字重校准——数值文本写进 String
列、`100` 写进 uint32 字段都会报类型错误）。

跨型强制转换（`"100"` → Int32、`1` → Bool(true) 等）是 cage-core 的
库 API（`normalize::coerce_to_type`），**当前未接入构建流水线**——
嵌入方可自行调用；流水线接入属规划项。

Normalize 的目标：

> **相同语义的数据应该产生相同的 Canonical Representation**（同表示层
> 内成立：trim 后相同的字符串、排序后相同的对象键，规范形式一致）。

归一化使用确定性规则（`BTreeMap`、稳定排序），相同输入必然得到相同输出，为[确定性构建](/build)打底。

## Transform

Transform 负责：

> Canonical Model → Target Artifact

而不是重新验证数据。验证在 [Validation 流水线](/validation)已完成，Transform 只做转换：

```text
                  Valid Model
                       |
       +---------------+---------------+
       |               |               |
      CSV             JSON             Code
                                       |
                  +--------------------+--------------------+
                  |         |         |         |          |
                 C#      Python      Lua    TS/JS   C++  Go  Java
```

## Target Generator

Target Generator 也是插件。

MVP 阶段（JSON / CSV）：

```text
CSV
JSON
```

第二阶段扩展（已实装）：

```text
C#
Python
Lua
TypeScript
JavaScript
C++
Go
Java
```

以后：

```text
FlatBuffers
Binary
SQLite
```

### Data Targets 与 Code Targets 分离

这两类 Target 应明确区分。

Data Targets（数据序列化）：

```text
JSON            ← 已实装
CSV             ← 已实装
MessagePack     ← 已实装
YAML            ← 规划
FlatBuffers     ← 规划
Binary          ← 规划
```

（Protobuf 落地为 schema 驱动的 .proto 定义文件生成（见下文
「Protobuf Target」），不在此列——它不做数据 wire 编码。）

Code Targets（代码生成）：

```text
C#              ← 已实装
Python          ← 已实装
Lua             ← 已实装
TypeScript      ← 已实装
JavaScript      ← 已实装
C++             ← 已实装
Go              ← 已实装
Java            ← 已实装
JSON Schema     ← 已实装（标准 JSON Schema 文档）
Template        ← 已实装（用户自定义模板）
```

（「规划」项不在 `format =` 支持范围内，构建报
`unsupported target format '<fmt>'` 并以退出码 2 失败。）

三个数据 target 各有自己的 options：

- `json`：`pretty`（默认 `true`，缩进美化输出）、`sort_keys`（默认
  `true`，对象键名序；`false` 时保留源字段序）。库 API 另有
  `generate_combined` 产出 `all.json`（表名 → 行数组），CLI 暂未暴露。
- `csv`：`delimiter`（单字符，默认 `,`）、`write_header`（默认
  `true`）、`quote_style`（`always` / `never` / `non_numeric`，非法值
  静默回退默认的「按需加引号」）。
- `msgpack`：`sort_keys`（默认 `true`，行字段按名序进 map；`false`
  时保留源字段序——嵌套对象键仍经 normalize 排序，与 `json` 同口径）。

### MessagePack Target（已实装）

数据类 target：Canonical Model → MessagePack 二进制，每表一个
`{table}.msgpack`（表 = 行数组，行 = 字段 map）。

该线格式同时是源格式：`cage-source-msgpack` 重消费自己写的
`.msgpack` 文件（registry 条目 `data/**` 即以此格式打包，见
[CLI · registry](/cli#registry) 的保真度选择），浮点/Bytes/超 i64
整数往返无损。

```toml
[[profiles.client.targets]]
format = "msgpack"              # 别名 messagepack
output_dir = "build/msgpack"
file_template = "{table}.msgpack"

[profiles.client.targets.options]
sort_keys = true                # 默认 true：行字段按名序
```

编码与确定性要点：

- 编码走 `rmp` 最小形（smallest-form）：定值定字节，同输入文档恒产出
  逐字节一致的产物（golden 测试锁定）
- 类型映射：Null → nil、Bool → bool、Int/UInt → 整数族最小表示、
  Float → f64、String → str、Bytes → 原生 bin（不做 base64 绕道）、
  Array → array、Object → map
- 非有限浮点（NaN / ±Inf）编码为 nil——NaN 位型不跨平台稳定，二进制
  直编会破坏确定性契约（JSON target 同规则：非有限 → null）
- `sort_keys = false` 时行字段保留源字段序；嵌套对象键仍由 normalize
  的 `BTreeMap` 排序（`json` 同口径）

### Protobuf Target（已实装）

设计稿 §23 把 Protobuf 列在 Data Targets 下但未定 wire 编码口径，落地
拍板（2026-10）：**输出 .proto3 定义文件**——schema 驱动、与 C#/Go 等
代码绑定同族（数据不参与生成；消费方拿 `protoc` 与自有 runtime 编译）。
每表一个 `{table}.proto`（表 → message，字段号按名序 1..n 分配），共享
整数枚举集中 `cage_enums.proto`，表文件按需 `import`。

```toml
[[profiles.client.targets]]
format = "proto"               # 别名 protobuf
output_dir = "build/proto"

[profiles.client.targets.options]
package = "cage.generated"     # 默认 cage.generated（原样写入 package 子句）
enums_file = "cage_enums.proto" # 默认 cage_enums.proto（import 路径随之）
```

类型映射表（proto3）：

| Cage 类型 | proto3 类型 | 说明 |
|-----------|-------------|------|
| int8 / int16 / int32 | `int32` | 值域被完全覆盖，无损 |
| int64 | `int64` | |
| uint8 / uint16 / uint32 | `uint32` | |
| uint64 | `uint64` | |
| float32 | `float` | |
| float64 | `double` | |
| bool / string / bytes | `bool` / `string` / `bytes` | |
| array\<T\> | `repeated T'` | T' 为 T 的映射；T 是 array/map 时包一层嵌套 message |
| object | `google.protobuf.Struct` | 自由形状口径（与 C++ 的 `std::map<string, any>` 同族决策），import well-known type |
| null / any | `google.protobuf.Value` | 同上 |
| map\<string, V\> | `map<string, V'>` | |
| map\<int, V\> | `map<int64, V'>` | 键 int → int64（go/cpp/java 同族口径） |
| enum（成员全整数且在 int32 值域） | 对应 proto enum | 见下 |
| enum（含字符串成员/缺字面量/超 int32 值域） | `string` | 同族回退口径，字段上注释标注原枚举名 |

生成规则要点：

- 确定性：表按名序、字段按名序、字段号 1..n 随名序分配；标识符共享
  同一去重集合（表名先、枚举名后，冲突追加 `_`）；同 Schema 恒产出
  字节一致的文件，文件头锤 manifest 记录的 schema 哈希
- proto3 要求枚举首成员为 0：无 0 值成员时前置合成
  `{Enum}_UNSPECIFIED = 0;`；成员值重复时发 `option allow_alias = true;`
- 数组/嵌套 map 落在 repeated 元素或 map 值位置时（proto 不允许
  `repeated repeated` / map 套 map），生成 `{字段}Value` 嵌套 message
  承载（深度叠后缀：`{字段}ValueValue`）
- 非必填字段带 `optional`（proto3 显式存在性）；repeated/map 恒不带
- 保留字与标量类型名作标识符 → 尾部 `_`；非法字符归一 `_`；数字开头
  前缀 `_`；同名成员确定性去重
- `repeated` 元素与 map 值类型为 object/null/any 时用 well-known type
  消息，合法无需包装

### JSON Schema Target（已实装）

标准 JSON Schema 文档生成：每表一个**自包含**的 `{table}.schema.json`
（`properties` 按 schema 字段声明序），编辑器、ajv 等非 cage 工具链直接
消费——不需要 cage 运行时。draft-07 默认（工具支持面最宽），2020-12 经
`options.draft` 切换。文档是纯标准 JSON Schema（不带自定义扩展键），
确定性 = 表名序 × 声明序键插入序；枚举一律内联 `enum` 数组，无共享枚举
文件。

```toml
[[profiles.client.targets]]
format = "jsonschema"          # 别名 json-schema / json_schema
output_dir = "build/jsonschema"

[profiles.client.targets.options]
draft = "07"                   # 默认 07；2020-12 切换；未知值报错（exit 2）
```

类型映射表（JSON Schema 关键字）：

| Cage 类型 | JSON Schema | 说明 |
|-----------|-------------|------|
| null | `{"type": "null"}` | |
| bool | `{"type": "boolean"}` | |
| int8–64 / uint8–64 | `{"type": "integer"}` | 整数族同型，`min`/`max` → `minimum`/`maximum`（整值边界渲染为整数） |
| float32 / float64 | `{"type": "number"}` | 同上 |
| string | `{"type": "string"}` | `min_length`/`max_length`/`pattern` → `minLength`/`maxLength`/`pattern` |
| bytes | `{"type": "string", "contentEncoding": "base64"}` | `contentEncoding` 仅 draft-07（2020-12 已删该关键字） |
| array\<T\> | `{"type": "array", "items": T}` | `min_items`/`max_items` → `minItems`/`maxItems` |
| object | `{"type": "object", "properties": …, "additionalProperties": false}` | 声明了属性的类型化对象闭形（与校验器一致：未知键拒绝）；空 Object 自由形状 |
| map\<string, V\> | `{"type": "object", "additionalProperties": V}` | 开形对象，值域即 V |
| map\<int, V\> | 同上 + `propertyNames.pattern: ^-?[0-9]+$` | int 键在数据模型里是数字字符串，与校验器同口径 |
| enum | `{"type": "string", "enum": [成员名…]}` | **实例恒为成员名**（L2 把 enum 字段类型定为 string，整型字面量只是代码生成元数据）；悬空枚举名降级纯 `string`（L1 另行报错） |
| any | `{}` | 不加约束 |

生成规则要点：

- 表文档键序固定：`$schema` → `title` → `description` → `type` →
  `properties` → `required` → `additionalProperties`；属性内键序同理
  固定（type → description → default → 约束关键字）——固定插入序锁字节
  确定性
- `required` 收集必填字段（声明序）；非有限 min/max 不落盘（JSON 无
  NaN/Inf）；内联 `enum_values` 的整数字符串域在整型字段上渲染为数字
  枚举（实例是 JSON 数字，字符串枚举会错配）
- 跨表引用（reference）与语义规则（rules）无标准 JSON Schema 拼写，
  不进文档——它们仍由 cage 自己的流水线在事实源上执行
- `options.draft` 未知值是唯一失败路径（与 template 同族：配置错在
  generate 期报 exit 2），产物一个不落

Code Target 的现役生成方式（plan → render → verify 直渲染，不依赖 AST
库）及其选型理由见仓库设计稿 `docs/design.md` 的 Code Targets 章节；
模板化形态（Template Target，Tera）已全量交付——官方语言模板随包
（G2），用户自定义模板经 `format = "template"` 接入（G3，见下节），
决策仍在 Rust 过滤器层，同见 §22。

例如同一份数据：

```text
Canonical Model
      |
      +---- JSON     （稳定键序、确定性字节输出）
      |
      +---- CSV
      |
      +---- C#
      |
      +---- Python
      |
      +---- Lua
      |
      +---- TypeScript / JavaScript
      |
      +---- C++ / Go / Java
```

### C# Target（已实装）

Code Target 的第一个落地：从 Schema 生成 C# 类绑定（数据不参与代码生成，
类型与元数据全部来自 Schema）。每个表一个 `{table}.cs`，共享枚举单独一个
编译单元；输出确定性排序（表/字段按名序、枚举值保持 Schema 顺序），同
Schema 必产出字节一致的文件，文件头会锤入 manifest 记录的 schema 哈希。

```toml
[[profiles.client.targets]]
format = "csharp"            # 别名 cs
output_dir = "build/cs"
file_template = "{table}.cs"

[profiles.client.targets.options]
namespace = "Cage.Generated"  # 默认即此值
enums_file = "CageEnums.cs"  # 默认 CageEnums.cs
```

生成规则要点：

- 整数枚举（每个成员都有整数值）生成 `enum`，按值域自动选 `int`/`long`/`ulong` 底座；字符串/无值枚举生成 `static class` 常量（Cage 枚举按字符串比较）
- 非必填且无默认值的字段生成可空类型（`string?`）；必填引用类型补空初始化（`= string.Empty;`），保证 `#nullable enable` 下零告警编译
- Schema default 渲染为初始化字面量（对象/不匹配类型跳过）；未解析枚举回退 `string` 并在 doc 注释标注
- 保留字加 `@` 前缀，非法标识符字符归一为 `_`，同名成员确定性去重
- Map 字段（`Map<K,V>`，第 19 种字段类型）：`Dictionary<K, V>`（键
  string→`string`、int→`long`），值类型递归（`map<string, Array<Int32>>`
  → `Dictionary<string, IReadOnlyList<int>>`）；空 map 默认值渲染
  `= new();`，非空标量成员渲染集合初始化器，键序确定性排序

### Python Target（已实装）

从 Schema 生成 `@dataclass(frozen=True)` 绑定。每个表一个 `{table}.py`，
共享枚举集中在 `cage_enums.py`；确定性口径与 C# Target 完全一致（表按名
序、字段名序、枚举值保持 Schema 顺序、文件头锤 schema 哈希）。

```toml
[[profiles.server.targets]]
format = "python"            # 别名 py
output_dir = "build/python"
file_template = "{table}.py"

[profiles.server.targets.options]
enums_file = "cage_enums.py" # 默认 cage_enums.py
```

生成规则要点：

- dataclass 语法要求无默认值字段在前：必填且无可渲染默认值的字段先出
  （组内名序），带默认值的字段随后；无默认值的非必填字段注解为
  `T | None = None`
- 整数枚举（每个成员都有整数值）生成 `IntEnum`；其余生成常量类
  （成员名/字符串值即常量值，Cage 枚举按字符串比较）
- Schema default 渲染为初始化字面量：数组默认值走
  `field(default_factory=lambda: [...])`（避免实例间共享可变对象），
  对象/不匹配类型跳过；未解析枚举回退 `str` 并在注释标注
- 保留字加尾部 `_`（PEP 8），非法标识符字符归一为 `_`，同名成员确定性去重
- Map 字段：`dict[K, V]`（键 string→`str`、int→`int`），值类型递归
  （`dict[str, list[int]]`）；空 map 默认值 `field(default_factory=dict)`，
  非空 `field(default_factory=lambda: {...})`（实例间不共享）

### Lua Target（已实装）

从 Schema 生成表绑定模块。每个表一个 `{table}.lua`，共享枚举集中在
`cage_enums.lua`（`M.<Enum> = { 成员 = 值, ... }` 的扁平表）；确定性口径
与 C#/Python 一致。

```toml
[[profiles.server.targets]]
format = "lua"
output_dir = "build/lua"
file_template = "{table}.lua"

[profiles.server.targets.options]
enums_file = "cage_enums.lua" # 默认 cage_enums.lua
```

每个表模块导出：

- `M.name` / `M.description` / `M.primary_key`：元信息
- `M.fields`：字段元数据列表（名序：`name` 原 schema 名、`key` 归一后的
  Lua 键、`type` 类型标签、`required`），约束摘要以注释形式保留
- `M.defaults`：可渲染的 Schema 默认值表（对象/不匹配类型/非有限浮点
  无 Lua 字面量，跳过）
- `M.new(t)`：行构造器，调用方字段优先，缺失字段回退 `M.defaults`
  （数组默认值复制填充，行与行不共享状态）

生成规则要点：

- 枚举值为整数时保留数字，字符串/布尔/无值成员用其字符串形式（Cage 枚
  举按字符串比较，成员名是最后的回退值）
- 保留字加尾部 `_`（Lua 无 `@` 式转义），非法标识符字符归一为 `_`，
  同名成员确定性去重
- 字符串转义用 `\n` / `\ddd`（Lua 无 `\xHH`）
- Map 字段：类型标签 `map<K, V>`（键 string/int、值递归），字段注释写明
  「键值表（hash part），与 Array 的数组 table（sequence part）不同」，
  int 键提示消费端 `tonumber` 转换；空 map 默认值 `{}`，`M.new` 经
  `copy_default` 深拷贝，行间不共享 table

### TypeScript / JavaScript Target（已实装）

一个 crate 两种形态的类型化模块绑定。`format = typescript`（别名 `ts`）
每表产出 `{table}.ts`（`export interface` + Defaults 常量 + 工厂函数）；
`format = javascript`（别名 `js`）每表产出带 JSDoc 类型的 `{table}.js`，
默认再配对一份 `.d.ts`。共享枚举集中在 `cage_enums.ts` / `cage_enums.js`；
确定性口径与 C#/Python/Lua 完全一致。

```toml
[[profiles.client.targets]]
format = "typescript"          # 别名 ts
output_dir = "build/ts"
file_template = "{table}.ts"

[profiles.client.targets.options]
enums_file = "cage_enums.ts"   # 默认 cage_enums.ts（js 形态默认 cage_enums.js）
```

```toml
[[profiles.server.targets]]
format = "javascript"          # 别名 js
output_dir = "build/js"

[profiles.server.targets.options]
emit_dts = false               # 默认 true：为每个 .js 配对 .d.ts
```

生成规则要点：

- 每表一个 `export interface`：必填或带可渲染默认值的字段为必选属性，
  其余 `?` 可选；配套 `export const {Table}Defaults: Partial<T>` 与
  `export function new{Table}(init?)` 工厂——默认值字面量内联进工厂，
  每次调用新建数组，行间不共享
- 枚举导出为 `as const` 常量对象 + 派生类型别名（`ItemKind.Sword` 既可
  取值也可作类型），数值桶/字符串桶按成员值划分；表文件只经
  `import type` 引用真正用到的枚举
- JS 形态用 `@typedef` / `@property`（可选属性写 `[name]`），枚举经
  JSDoc `import("…")` 类型引用、不产生运行时 import；`emit_dts` 默认
  为每个文件配对 `.d.ts`（`declare const` + 字面量类型）
- 属性名不做保留字转义（属性位置合法，保证与 Schema 字段 1:1）；绑定
  标识符（interface/const/函数名）保留字加尾部 `_`，非法字符归一 `_`，
  同名成员确定性去重
- 非有限浮点有原生字面量：`NaN` / `Infinity` / `-Infinity` 照常渲染
- Map 字段：`Map<K, V>`（键 string→`string`、int→`number`），值类型递归
  （`Map<string, number[]>`）；默认值渲染 `new Map()` /
  `new Map([["k", v]])` 而非对象字面量，JSDoc / `.d.ts` 同口径

### C++ Target（已实装）

从 Schema 生成头文件绑定：每表一个 `{table}.h`（`struct` + 成员初始化
器），共享枚举一个 `cage_enums.h`；确定性口径与 C#/Python/Lua 一致。

```toml
[[profiles.client.targets]]
format = "cpp"                 # 别名 c++ / cxx
output_dir = "build/cpp"
file_template = "{table}.h"

[profiles.client.targets.options]
namespace = "cage::generated"  # 默认 cage::generated
enums_file = "cage_enums.h"    # 默认 cage_enums.h
```

生成规则要点：

- 必填或带可渲染默认值的字段为普通成员（无默认值时值初始化 `{}`，
  默认值渲染为成员初始化器 `{"pvp"}`）；非必填且无默认值用
  `std::optional<T>` 包裹
- 整数枚举（成员全有整数值）按值域自动选 `std::int32_t` /
  `std::int64_t` / `std::uint64_t` 底座生成 `enum class`；字符串/无值
  枚举生成 `namespace` + `inline constexpr std::string_view` 常量
- 类型映射：`std::string`、`std::vector<T>`、bytes →
  `std::vector<std::uint8_t>`、对象 → `std::map<std::string, std::any>`、
  Map 字段 → `std::unordered_map<K, V>`（键 string→`std::string`、
  int→`std::int64_t`，值类型递归；与对象的 `std::map` 区分）、
  字符串桶枚举字段 → `std::string_view`（桶本身是 namespace 无独立类型，
  字段持有其常量的视图）
- include 集合按实际使用裁剪（`cstdint` / `optional` / `string` /
  `vector` / `map` / `unordered_map` / `any` / `string_view` / `limits`），
  系统头按名序、引号头单独成组；引用枚举头用 `enums_file` 配置的相对路径
- 非有限浮点用 `std::numeric_limits<T>::quiet_NaN()` / `infinity()`；
  字符串转义用 `\n` 与三位八进制（C++ 的 `\x` 会吞掉后续十六进制位）
- 保留字加尾部 `_`，非法字符归一 `_`；表结构体与枚举共享同一命名空间
  的去重集合，同名确定性避让

### Go Target（已实装）

从 Schema 生成 struct 绑定：每表一个 `{table}.go`（导出字段 + `json`
tag + `New{Table}` 默认值构造器），共享枚举一个 `cage_enums.go`
（`type` + `const` 块）；所有文件同包，包级声明共享统一去重集合。
确定性口径与 C#/Python/Lua 一致，输出与 gofmt 字节一致。

```toml
[[profiles.server.targets]]
format = "go"                  # 别名 golang
output_dir = "build/go"

[profiles.server.targets.options]
package = "config"             # 默认 config
enums_file = "cage_enums.go"   # 默认 cage_enums.go
file_template = "{table}.go"   # 默认 {table}.go
```

生成规则要点：

- 字段名 PascalCase 导出（`player_id` → `PlayerId`），`json:"原始字段
  名"` tag 恒带；必填或带默认值为普通类型，非必填且无默认值的值类型用
  指针（`*int32` / `*string` / `*枚举`），切片与 map 用 nil 表缺席
- `New{Table}()` 总是生成：只填有可渲染默认值的字段，字面量每次调用
  新建（行间不共享）；非有限浮点用 `math.NaN()` / `math.Inf(±1)`
  （唯一会引入 `import "math"` 的场景）
- 整数枚举按值域选 `int32` / `int64` / `uint64` 底座 `type` + 前缀
  const（`ItemKindSword`，包级唯一）；其余生成 `string` 底座 const 块
- 文件头首行带 Go 工具链识别标记
  `// Code generated by Cage — DO NOT EDIT.`；字段对齐、const 块、
  尾随注释均与 gofmt 输出字节一致
- Map 字段：`map[K]V`（键 string→`string`、int→`int64`——
  `encoding/json` 原生支持整型键的带引号字符串解码），值类型递归
  （`map[string][]int32`）；与切片同惯例 nil 表缺席，不出指针；
  `New{Table}` 默认值渲染 `map[K]V{}` / 条目字面量

### Java Target（已实装）

从 Schema 生成类绑定：每表一个 `{table}.java`（公有字段 + 字段初始化
器默认值），共享枚举一个 `CageEnums.java`（holder 类嵌套 `public
enum`，带 value 载荷）；确定性口径与 C#/Python/Lua 一致。

```toml
[[profiles.server.targets]]
format = "java"
output_dir = "build/java"

[profiles.server.targets.options]
package = "cage.generated"     # 默认 cage.generated
enums_file = "CageEnums.java"  # 默认 CageEnums.java
file_template = "{table}.java" # 默认 {table}.java
```

生成规则要点：

- **文件名 = 公有类名**（javac 硬约束）：`{table}` 占位用最终类标识符
  而非原始表名；表类与枚举 holder 同包，统一去重集合确定性避让
- 必填或带可渲染默认值为普通类型；非必填且无默认值的基础类型升包装
  （`Integer` / `Double` / …），引用类型天然可空；数组默认值
  `new ArrayList<>(List.of(…))` 每实例新建
- 无符号宽度提升一档：`UInt8 → short`、`UInt16 → int`、`UInt32 /
  UInt64 → long`（Java 全有符号；超出 `Long.MAX_VALUE` 的 u64 **枚举
  值**以二补数 long 字面量落盘并在行尾注释 `wrapped from unsigned …`；
  字段默认值超界则整个跳过默认值）
- 整数枚举按值域选 `int` / `long` 载荷（`public final value` + 构造
  器）；字符串/无值枚举 `String` 载荷（成员名兜底）；非有限浮点用
  `Double.NaN` / `…POSITIVE_INFINITY`
- 保留字加尾部 `_`（单独的 `_` 非法，归一为 `__`），非法字符归一 `_`，
  同名成员确定性去重
- Map 字段：`HashMap<K, V>`（键 string→`String`、int→`Long`，值类型
  装箱递归：`HashMap<String, List<Integer>>`）；空默认值
  `new HashMap<>()`，标量成员 `new HashMap<>(Map.of(…))` 每实例新建
  （超过 `Map.of` 的 10 对重载上限整体跳过）；import 按嵌套字段类型
  递归扫描注入

## Template：用户自定义模板

`format = "template"` 用 [Tera](https://keats.github.io/tera/) 模板渲染
Schema IR——语言绑定之外的任意文本产物（配置文件、文档、DSL 脚本）走这
条通道。模板从磁盘加载，不随包：

```toml
[[profiles.client.targets]]
format = "template"
output_dir = "build/tpl"

[profiles.client.targets.options]
template_dir = "my_templates"   # 默认 .cage/templates（相对项目根）
lang_filters = "py,go"          # 挂载语言过滤器库（见下），缺省不挂载
```

模板文件约定：

- **输出名 = 模板相对路径去掉 `.tera` 后缀**（`defs.tera` →
  `defs`、`{table}.tpl.tera` → `Item.tpl`），子目录结构原样保留
- 文件名含 `{table}` 占位符的模板**每表渲染一次**（表名序）；不含的
  **全局渲染一次**（模板名序）。产物顺序 = 模板名序 × 表名序，
  两跑逐字节一致
- 目录不存在、目录里没有 `*.tera`、模板语法错 → 退出码 2，错误信息
  指到对应模板文件（Tera 的 cause 链完整展开）

模板上下文（Schema IR 原样序列化 + 两个补充键）：

| 变量 | 形状 | 可用性 |
|------|------|--------|
| `tables` | 表名 → 表（`name` / `description` / `primary_key` / `fields` / `unique_constraints` / `order_by` / `targets`） | 恒有 |
| `table` | 当前表（同上形状） | 仅 `{table}` 模板 |
| `enums` | 枚举名 → 枚举定义 | 恒有 |
| `metadata` | Schema 级元数据 | 可空 |
| `schema_hash` | 内容哈希（确定性构建口径） | 恒有（缺省 `(unavailable)` 场合为 null，配 `default` 过滤器兜底） |

`fields` 是字段名 → 字段的映射，至少含 `name` / `type`，其余键按
Schema 实情出现（`description` / `required` / `default` / `min` / `max` /
`min_length` / `max_length` / `pattern` / `enum_values` / `min_items` /
`max_items` / `reference` / `targets` / `rules` 等；`required` 为
`false` 时该键省略）。

### 过滤器

Tera 内置过滤器全量可用。约定过滤器恒注册（语言无关层）：

| 过滤器 | 输入 | 输出 |
|--------|------|------|
| `snake_case` / `camelCase` / `PascalCase` | 字符串 | 按词边界归一的标识符 |
| `kebab_case` | 字符串 | `http-server` 形 |
| `SCREAMING_CASE` | 字符串 | `HTTP_SERVER` 形 |
| `field_order` | `fields` 映射 | 按字段名排序的数组（官方各语言统一的发射序） |

语言过滤器经 `options.lang_filters` 挂载（逗号分隔多语言，如
`"py,go"`；未知键构建报错退出码 2）。决策仍在 Rust——过滤器调用的是
官方生成器同一套映射函数，模板里拿到的类型名 / 字面量与官方产物一致：

| 过滤器 | 输入 | 输出 |
|--------|------|------|
| `py_type` / `cs_type` / `ts_type` / `go_type` / `java_type` / `cpp_type` / `lua_type` | 字段对象 | 该语言的类型文本（枚举引用带官方分配后的标识符；`go_type` 含指针可选形，`java_type` 含包装类升级） |
| `py_default` / `cs_default` / `ts_default` / `go_default` / `java_default` / `cpp_default` / `lua_default` | 字段对象 | 字段默认值的语言字面量；无默认或不可渲染 → null |

（`lua_type` 输出官方 Lua 面的类型标签；`cpp_type` 不做每文件 include
聚合——那是生成器级关注点。）

```text
.cage/templates/{table}.tpl.tera   →   build/tpl/Item.tpl
────────────────────────────────────────────────────────────
table={{ table.name }} pk-snake={{ table.name | snake_case }}
{% for f in table.fields | field_order %}{{ f.name }}: {{ f | py_type }}
{% endfor %}
```

## Profile：前端 / 后端

不要把「客户端」和「服务端」写死在 Core。可以定义 Build Profile（真实
句法见 [CLI: build](/cli#build)，工程配置 `cage.toml`）：

```toml
[profiles.client]
name = "client"

[[profiles.client.targets]]    # JSON + CSV 数据产物
format = "json"
output_dir = "build/client/json"

[[profiles.client.targets]]    # C# 代码绑定
format = "csharp"
output_dir = "build/cs"
file_template = "{table}.cs"

[profiles.server]
name = "server"

[[profiles.server.targets]]    # 服务端语言组合
format = "python"
output_dir = "build/python"
```

然后：

```bash
cage build --profile client
cage build --profile server
```

## Field Visibility（字段可见性）

不同目标可能不需要全部字段。例如：

```yaml
fields:

  id:
    type: uint32

  name:
    type: string

  admin_note:
    type: string
    targets:
      - server
```

`targets` 省略 = 对所有 profile 可见；`"*"` 通配等价全部。表级也有
`targets`：列表不含当前 profile 时整表从该 profile 视图剔除。

客户端产出：

```text
id
name
```

服务端产出：

```text
id
name
admin_note
```

这样可以避免：

> 为客户端和服务器维护两套配置。

一份配置，按 Profile 过滤，两端复用。
