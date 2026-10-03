# Target：规范化与产出

验证通过后进入规范化（Normalize），再由 Target Generator 产出目标资产（Artifact）。

## Normalize（归一化）

数值统一：

```text
"100"
100
100.0
```

统一为：

```text
UInt(100)
```

布尔值统一：

```text
yes
YES
true
1
```

统一为：

```text
Bool(true)
```

Normalize 的目标：

> **相同语义的数据应该产生相同的 Canonical Representation。**

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
                              +--------+--------+
                              |        |        |
                             C#      Python    Lua
```

## Target Generator

Target Generator 也是插件。

MVP 阶段（JSON / CSV）：

```text
CSV
JSON
```

第二阶段扩展：

```text
C#
Python
Lua
```

以后：

```text
TypeScript
C++
Go
Protobuf
MessagePack
FlatBuffers
Binary
SQLite
```

### Data Targets 与 Code Targets 分离

这两类 Target 应明确区分。

Data Targets（数据序列化）：

```text
JSON
CSV
YAML
MessagePack
Protobuf
FlatBuffers
Binary
```

Code Targets（代码生成）：

```text
C#
Python
Lua
C++
Go
TypeScript
```

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
namespace = "Game.Config"    # 默认 Cage.Generated
enums_file = "CageEnums.cs"  # 默认 CageEnums.cs
```

生成规则要点：

- 整数枚举（每个成员都有整数值）生成 `enum`，按值域自动选 `int`/`long`/`ulong` 底座；字符串/无值枚举生成 `static class` 常量（Cage 枚举按字符串比较）
- 非必填且无默认值的字段生成可空类型（`string?`）；必填引用类型补空初始化（`= string.Empty;`），保证 `#nullable enable` 下零告警编译
- Schema default 渲染为初始化字面量（对象/不匹配类型跳过）；未解析枚举回退 `string` 并在 doc 注释标注
- 保留字加 `@` 前缀，非法标识符字符归一为 `_`，同名成员确定性去重

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

- `M.name` / `M.description` / `M.primary_key` —— 元信息
- `M.fields` —— 字段元数据列表（名序：`name` 原 schema 名、`key` 归一后的
  Lua 键、`type` 类型标签、`required`），约束摘要以注释形式保留
- `M.defaults` —— 可渲染的 Schema 默认值表（对象/不匹配类型/非有限浮点
  无 Lua 字面量，跳过）
- `M.new(t)` —— 行构造器：调用方字段优先，缺失字段回退 `M.defaults`
  （数组默认值复制填充，行与行不共享状态）

生成规则要点：

- 枚举值为整数时保留数字，字符串/布尔/无值成员用其字符串形式（Cage 枚
  举按字符串比较，成员名是最后的回退值）
- 保留字加尾部 `_`（Lua 无 `@` 式转义），非法标识符字符归一为 `_`，
  同名成员确定性去重
- 字符串转义用 `\n` / `\ddd`（Lua 无 `\xHH`）

## Profile：前端 / 后端

不要把「客户端」和「服务端」写死在 Core。可以定义 Build Profile：

```yaml
profile: client

targets:
  - json
  - csv
  - csharp
```

服务端：

```yaml
profile: server

targets:
  - json
  - csv
  - python
  - lua
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
