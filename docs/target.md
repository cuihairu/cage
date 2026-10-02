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
