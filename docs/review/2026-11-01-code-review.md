# 代码深审报告（2026-11-01 批次）

> 范围：cage 仓库全量源码（24 crate，约 83.6k 行 Rust）。按用户 2026-11-02 令执行：
> ① 安全静态扫描 ② 隐藏 bug 猎捕 ③ 质量门 ④ 本报告 ⑤ P0/P1 当场修（一问题一 commit，测试绿）⑥ 汇报。
> 方法：静态扫描（grep/模式匹配）+ 定向代码阅读 + 质量门实测（fmt / clippy -D warnings / 全量测试）。

## 结论速览

| 严重度 | 发现数 | 已修 | 待办 |
| --- | --- | --- | --- |
| P0 | 0 | 0 | 0 |
| P1 | 1 | 1 | 0 |
| P2 | 2 | 0 | 2 |
| P3 | 2 | 0 | 2 |
| **合计** | **5** | **1** | **4** |

- **P0 = 0**：无内存不安全（全仓 **零 `unsafe`**）、无注入、无认证绕过、无数据损坏路径。
- **P1 = 1，已当场修复**：E2205 诊断截断在非 UTF-8 字符边界切片 → panic（`crates/cage-core/src/convention/mod.rs`）。已修 + 回归测试，测试绿。
- **P2 = 2**：HTTP 远端响应体无上限读取（OOM 风险）；flatbuffers `(key)` 不复查 flatc 类型资格（沿用文档审计 D10）。
- **P3 = 2**：schema/FieldType 递归深度无显式护栏；`valid_component` 允许单个 `.` 作为包/版本名（无害但可收紧）。

## ① 安全静态扫描

| 面 | 结果 | 证据 |
| --- | --- | --- |
| 硬编码密钥 | 无 | 全仓 grep `api_key/secret/password/token/access_key/private_key = "≥8 字符"` 零命中（非测试） |
| shell 注入 | 无 | 无 `shell(true)`、无 `sh -c`/`bash -c`/`/bin/sh` 拼接；全部子进程走 `Command::new` 直接 exec |
| SQL 注入 | 受防护 | `cage-source-db`：表名拼接受 `valid_table_name`（仅 `[A-Za-z0-9_.$]`，禁 `;`/引号/空格）保护（`crates/cage-source-db/src/lib.rs:128-141`）；命名查询与表展开统一过 `validate_select`（单条 SELECT、拒 `;`/注释/`FOR UPDATE|SHARE`/`INTO`）（`lib.rs:104-126,322-323`）；内省 SQL 参数化（`mysql.rs:48,66`、`pg.rs:57,84`） |
| eval | 无 | 无 OS eval；`validation/expr.rs` 的 `eval` 为自定义断言求值器（非系统调用） |
| 反序列化 | 安全 | 全仓**零 `unsafe`**（`transmute`/`from_raw`/`from_utf8_unchecked`/`CString` 零命中）；serde_json/serde_yaml/rmp 均为内存安全 |
| 路径穿越 | 受防护 | `snapshot_path` 拒绝对对/`..`/非 Normal 组件 + canonicalize 且 symlink 目标须落根内（`crates/cage-core/src/snapshot/mod.rs:39-64`）；`resolve_script` 同规（`crates/cage-core/src/convention/mod.rs:1094-1131`）；`valid_component` 拒 `..`/`/`/`\`（`crates/cage-core/src/registry.rs:61-68`）；bundle import 成员拒绝对对/`..` + 前缀校验（`crates/cage-core/src/registry.rs:893-947`） |
| Web 服务面 | 受防护 | 只绑 `127.0.0.1`（`crates/cage-cli/src/web.rs:61`）；`Host` 头必须名回环主机 + POST `Origin` 校验（`web.rs:108-130,218`）；请求体上限 8 MiB + 30s 墙钟（`web.rs:92-96,162-166`） |
| 配置文件驱动的代码执行 | 按设计 | `cage.excel-meta/1` 脚本适配器：`Command::new` 直接执行、脚本路径经 `resolve_script` 全校验（绝对/`..`/symlink 逃逸/非文件均 E2203）、参数为独立 argv 非 shell；文档明示「以作者自身权限运行，cage 不沙箱、不宣称」 |

## ② 隐藏 bug 猎捕

### P1-1（已修）E2205 诊断截断在非字符边界切片 → panic

`run_script` 的 E2205 分支把脚本 stderr 截断到 400 字节：

```rust
let stderr = if stderr.len() > 400 {
    &stderr[..400]        // 字节 400 落在多字节字符内部时 panic（&str 切片要求字符边界）
} else { stderr };
```
`crates/cage-core/src/convention/mod.rs:1243-1244`

**触发**：Excel 元行脚本（`cage.excel-meta/1`）退出非零且 stderr > 400 字节、第 400 字节落在一个多字节字符（如中文）内部 → `&str[..400]` panic，把「脚本失败应报 E2205」变成 CLI 崩溃。该路径是库级函数（`cage-source-excel` 适配器在解析期调用），panic 直接上抛。

**修复**：在字符边界回退截断（`is_char_boundary` 回退），并新增回归测试 `test_script_failure_truncates_cjk_stderr_at_char_boundary`（`crates/cage-source-excel/src/lib.rs`，`'爆' * 300` 即 300 个 CJK 字符 = 900 字节 stderr → E2205 不 panic、保留 payload，stderr 段 ≤400 字节且在字符边界）。提交见「⑤ 修复提交」。

### P2-1 远端 HTTP 响应体无上限读取

`remote.rs::read_response` 对 GET 响应体 `read_to_end` 无字节上限：

```rust
response.into_reader().read_to_end(&mut bytes)
```
`crates/cage-core/src/remote.rs:326`（亦见 `remote.rs:490`）

**影响**：`cage-source-http` 源与 `registry:` 远程根（R3）从远端取字节，恶意/失陷服务器可无限流式响应 → 客户端内存耗尽（OOM）。web 服务面有 8 MiB 上限（`web.rs:92`），此处**没有**对应上限，两处口径不一致。

**建议**：设上限（如 64 MiB，与配置源规模量级匹配）超限报错；`read_to_end` 改 `take(limit)` + 溢出判定。**登记待办**，不改码（非 P0/P1，需产品口径）。

### P2-2 flatbuffers `(key)` 不复查 flatc 类型资格

`cage-target-flatbuffers` 对单字段主键直接加 `(key)`，不复查 flatc 资格规则（仅 string/整型标量合法）：

```rust
let key_field = table.primary_key.first()
    .filter(|_| table.primary_key.len() == 1)
    .map(String::as_str);
```
`crates/cage-target-flatbuffers/src/lib.rs:294-300`

**影响**：schema 声明非资格类型（float/composite）单字段主键时，产出可能被 `flatc` 拒绝的 `.fbs`。**登记待办**（沿文档审计 D10），不改码。

### P3-1 schema/FieldType 递归深度无显式护栏

`FieldType`（`Array`/`Map`/`Object`/`TaggedUnion`）递归；normalize/生成器随之递归。深度仅受 serde 解析默认递归上限约束，无显式 `MAX_DEPTH` 护栏（`crates/cage-core/src/schema/mod.rs:288-371`、`crates/cage-core/src/normalize/mod.rs`）。超深嵌套的恶意 schema 可致栈溢出。**登记待办**（配置属作者自有、信任边界内，低风险）。

### P3-2 `valid_component` 允许单个 `.`

`valid_component` 拒绝 `..`/`/`/`\`，但允许单个 `.`（或 `-`/`_`）作为包/版本名（`crates/cage-core/src/registry.rs:61-68`）。单 `.` 作路径段是 no-op，不构成逃逸，仅不严谨。**登记待办**（收紧为须含字母/数字）。

### 排查过的边界（未见问题）

- `literal_break_terms`（go 生成器）`terms[i-1]`/`members[i]` 界检查：`i` 从 0 起、仅 `i>0` 写 `terms[i-1]`，`terms[n-1]` 有 `n>0` 守卫（`crates/cage-target-go/src/lib.rs:777-798`）。
- `composite`（flatbuffers/proto）`unreachable!`：外层 match 已保证 `Array|Map`，内层 `_` 结构不可达，安全（`crates/cage-target-flatbuffers/src/lib.rs:513-563`、`crates/cage-target-proto/src/lib.rs:513-524`）。
- msgpack 解码索引 `row.columns_ref()[idx]`：`idx in 0..row.len()` 内，越界安全（`crates/cage-source-db/src/mysql.rs:135`）。
- `run_script` 超时线程：`try_wait` + 20ms 轮询 + `kill`/`wait`，abort 通道唤醒，线程有界终止，无泄漏（`crates/cage-core/src/convention/mod.rs:1180-1218`）。
- Web 服务单线程（`for request in server.incoming_requests()`），`Ctx` 只读共享，无数据竞态（`crates/cage-cli/src/web.rs:76`）。

## ③ 质量门

| 门 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 通过（0 差异） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过（**0 warnings**） |
| `cargo test --workspace` | **1033 passed / 0 failed**（含本批新增 1 回归测试） |
| `npm run docs:build`（VitePress） | 通过（build complete） |

测试基线：本批之前 1032 全绿，本批 +1（`test_script_failure_truncates_cjk_stderr_at_char_boundary`），现 1033 全绿。门禁与 CI（`.github/workflows/ci.yml`：fmt / clippy / rustdoc / test）一致。

## ④ 修复提交

| commit | 内容 |
| --- | --- |
| `2c88304` | `fix: truncate script stderr at char boundary` —— P1-1 修复 + 回归测试（2 文件，+46/-1） |

## ⑤ 汇报（发现/已修/剩余）

- **发现 5**（P1×1、P2×2、P3×2），**已修 1**（P1-1），**剩余 4**（P2×2、P3×2，已登记待办）。
- P2/P3 待办登记：P2-1 远端响应体上限、P2-2 flatbuffers `(key)` 资格复查、P3-1 递归深度护栏、P3-2 `valid_component` 收紧。
- 修复一问题一 commit（P1-1），测试绿；无代码行为改动超范围。
