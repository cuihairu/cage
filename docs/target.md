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
