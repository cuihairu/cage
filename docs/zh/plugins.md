# 插件：版本化 JSON IPC 协议

Cage 的扩展点按稳定性与安全性分层（见评审文档
`2026-10-22-schema-workbench-python-plugins.md` §五）。本页描述
**业务校验插件**层：一套语言无关、版本化的 JSON IPC 协议，以及实现该协议的
Python SDK。

| 扩展层 | 机制 | 状态 |
|---|---|---|
| 规则模板包 | 声明式 JSON/Schema AST | 已随 P1-3 落地（内置模板） |
| **业务校验插件** | **版本化 JSON IPC 工作进程** | **P2-1：协议 + Python SDK（本页）** |
| Rust 原生校验器 | Trait / 内置注册表 | 已有 `GameRuleValidator` |
| 数据源插件 | 独立 Source Adapter | 已有 `cage-source-*` |
| 生成器插件 | 内置 Target / 模板 | 已有 `cage-target-*` |

校验插件应当是一个*纯函数*：

```text
Diagnostics = f(Schema, Data, Options)
```

插件不自行打开 `Monster.xlsx`，不自行实现 CSV/Excel/JSON 解析，也不回写
数据。Cage Core 先把数据规范化，再把只读的、已定型的数据表交给插件。这样
同一插件在策划电脑、构建服务器和 CI 上都能得到一致结果。

## 架构

```text
Cage Studio / CLI / CI
        │  构造请求（schema + 规范化数据表 + options）
        ▼
   Rust Core（L0–L6）──► 统一 L7 执行器 ──► Python Worker（本协议）
        │                                              │  Diagnostics = f(...)
        └──────────────► 统一诊断结果 ◄────────────────┘
```

协议在 `crates/cage-plugin-protocol`（Rust，规范线格式类型）中定义一次，并由
`sdk/python`（`cage_sdk`）镜像实现。任何语言都可以实现同一份契约；协议刻意
做到与语言无关。

## 协议 v1

### 帧格式（MVP）

一次插件运行是**单请求/单应答**交换：

1. 宿主向 worker 的 stdin 写入恰好一个紧凑 UTF-8 JSON 对象并加 `\n`，随后
   关闭 stdin。
2. worker 读取全部 stdin（一个请求），运行插件，向 stdout 写入恰好一个紧凑
   UTF-8 JSON 对象并加 `\n`，随后退出。

MVP 为单次交换：无 keep-alive、无流式、无批处理。未来的常驻 worker 会在
同一批消息类型之上叠加自己的帧格式。

### 版本

每条消息都带 `protocol` 整数，当前为 `1`。收到 `protocol` 与自身支持版本
不同的请求时，worker **必须**回一条 `code` 为 `PROTOCOL_VERSION_MISMATCH`
的错误并以非零码退出，**不得**尝试解读该消息。宿主版本即契约：任何破坏
消息形状的变化都要提升 `PROTOCOL_VERSION`。

### 方法

MVP 只暴露一个方法：`run`。

#### 请求（宿主 → worker）

```json
{
  "protocol": 1,
  "id": "build-2026-11-22-0001",
  "method": "run",
  "params": {
    "plugin": { "id": "monster_balance", "version": "1.0.0" },
    "schema": { "name": "Monster", "tables": [] },
    "tables": [
      {
        "name": "Monster",
        "primary_key": ["id"],
        "rows": [
          {
            "index": 0,
            "location": "Monster.xlsx#Sheet1!A2",
            "fields": { "id": 1, "level": 3, "cost": 144 }
          }
        ]
      }
    ],
    "options": { "base_cost": 100, "growth": 1.2, "max_level": 10 }
  }
}
```

| 字段 | 含义 |
|---|---|
| `protocol` | 必须等于支持的版本（`1`）。 |
| `id` | 不透明关联 id，原样回显在应答里。 |
| `method` | 必须是 `run`。 |
| `params.plugin` | 插件 id 与版本，取自项目配置。 |
| `params.schema` | 规范化 Schema 的 JSON（与编辑器存储的形状一致）。 |
| `params.tables` | 规范化数据表，按声明顺序。 |
| `params.rows[].index` | 表内 0 起的行序号（诊断用它回指行）。 |
| `params.rows[].location` | 可选的人话源位置（如 `Monster.xlsx#Sheet1!A2`）。 |
| `params.rows[].fields` | 字段名 → 规范化 JSON 值。 |
| `params.options` | 本项目配置给该插件的参数（只读）。 |

#### 应答（worker → 宿主），成功

```json
{
  "protocol": 1,
  "id": "build-2026-11-22-0001",
  "ok": true,
  "diagnostics": [
    {
      "severity": "error",
      "code": "E1601",
      "message": "cost 100 is below the expected curve 144.0 (base 100, growth 1.2)",
      "table": "Monster",
      "row": 1,
      "field": "cost",
      "hint": "raise cost, or adjust base_cost/growth in the plugin options"
    }
  ]
}
```

诊断的 `severity` 取 `error` / `warning` / `info` 之一。`code` 缺省为
`E1601`（gamerule 族的「自定义校验器插件返回错误」码）。`table`/`row`/
`field` 可选，用于把发现回指到具体源位置。

#### 应答（worker → 宿主），失败

```json
{
  "protocol": 1,
  "id": "build-2026-11-22-0001",
  "ok": false,
  "error": { "code": "PLUGIN_ERROR", "message": "ValueError: bad options at plugin.py:41" }
}
```

### 错误码

| `code` | 含义 |
|---|---|
| `PROTOCOL_VERSION_MISMATCH` | 请求的 `protocol` 与支持版本不同。 |
| `PROTOCOL_MALFORMED` | 请求不可解析（坏 JSON、缺字段）。 |
| `PROTOCOL_UNKNOWN_METHOD` | 请求命名了协议未实现的方法。 |
| `PLUGIN_ERROR` | 插件抛异常（消息截断到 400 个 UTF-8 字节）。 |
| `PLUGIN_INVALID_RESULT` | 插件产出了协议形状之外的输出。 |

### 退出码

| 码 | 含义 |
|---|---|
| `0` | 已写出应答（即使是带着 `error` 级*发现*的应答——发现严重性的处置由宿主决定）。 |
| `1` | 协议或插件失败；`error` 给出原因。 |

## Python SDK（`sdk/python`）

`cage_sdk` 镜像协议，并给插件作者提供四个只读能力和受控的诊断写入。

```bash
pip install -e sdk/python
```

```python
from cage_sdk import Plugin, RunContext, run

class MonsterBalance(Plugin):
    id = "monster_balance"
    version = "1.0.0"

    def validate(self, ctx: RunContext) -> None:
        table = ctx.table("Monster")
        if table is None:
            ctx.warning("no Monster table to check")
            return
        for row in table["rows"]:
            level = row["fields"].get("level")
            if isinstance(level, (int, float)) and level > 10:
                ctx.error(
                    "level too high",
                    table="Monster", row=row["index"], field="level",
                )

if __name__ == "__main__":
    run(MonsterBalance())
```

### 上下文 API

| 方法 | 作用 |
|---|---|
| `ctx.schema()` | 规范化 Schema 的 JSON（只读）。 |
| `ctx.table_names()` | 本次运行交给插件的表名。 |
| `ctx.table(name)` | 一张规范化数据表（`{"name","primary_key","rows"}`），无则 `None`。 |
| `ctx.options()` | 本项目配置给该插件的参数（只读）。 |
| `ctx.error(message, *, code, table, row, field, hint)` | 报告硬违例。 |
| `ctx.warning(message, *, ...)` | 报告软发现。 |
| `ctx.info(message, *, ...)` | 报告提示信息。 |

`run(plugin)` 从 stdin 服务一个请求并以协议退出码退出。`serve(plugin,
input_stream, output_stream)` 是可测的核心。

### 运行随附示例

示例插件实现怪物战力/等级曲线校验：

```bash
python3 examples/plugin-python/monster_balance.py \
    < examples/plugin-python/sample_request.json
```

## P2-1 的范围（与未覆盖部分）

本协议 MVP 覆盖：

- 版本化 JSON IPC 契约（Rust `cage-plugin-protocol` + Python 镜像）。
- Python SDK（上下文 API、版本/畸形/未知方法拒绝、插件异常捕获、诊断校验）。
- 跨语言互操作测试：Rust 构造的请求经真实子进程被 Python SDK 解码，其应答
  再被 Rust 解码回读。

P2-1 之后已交付（各自在 `cage_core`，已接入 CLI）：

- **P2-2** — 宿主侧 worker：spawn、超时、崩溃恢复，以及失败行为表（插件缺失、
  版本不符、超时、崩溃、非法输出）到 Cage 诊断的映射（`cage_core::plugin`）。
- **P2-3** — 统一 L7 执行器，让 Rust 内置校验器与 worker 插件走同一诊断面
  （`GameRuleRegistry`）。
- **P2-4** — `ValidationAttestation`：插件标识/版本、配置摘要、依赖锁定、
  环境指纹进入可审计记录。
- **P2-5** — `[validation]` 项目策略：`required_level` 抬高 CLI 默认等级使 L7
  真正运行，`required_plugins` 使校验以 `E1602` 失败于声明 id 的缺失——由
  `check`、`build`、LSP 通道与 Web 编辑器数据预检一致应用。
