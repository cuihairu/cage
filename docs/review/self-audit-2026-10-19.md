# cage 核心模块自审报告（2026-10-19）

范围：crates/ 下代码量最大的 3 个模块（错误处理/边界、测试覆盖、文档一致性、输入校验与路径处理）。未通读全仓，未改动任何代码。

## 第一节：模块清单

`ls crates/` 输出（24 个 crate，按代码量降序标注）：

| crate | Rust 行数 | 本次是否审核 |
|---|---|---|
| cage-cli | 24138 | ✅（第 2 节） |
| cage-core | 21717 | ✅（第 3 节） |
| cage-source-db | 2284 | ✅（第 4 节） |
| cage-target-java | 2188 | — |
| cage-target-go | 2161 | — |
| cage-target-ts | 2111 | — |
| cage-target-cpp | 2038 | — |
| cage-target-cs | 1350 | — |
| cage-target-py | 1318 | — |
| cage-source-http | 1302 | — |
| cage-target-lua | 1292 | — |
| cage-source-excel | 1162 | — |
| cage-target-proto | 1008 | — |
| cage-target-flatbuffers | 969 | — |
| cage-target-csv | 800 | — |
| cage-target-template | 796 | — |
| cage-source-csv | 794 | — |
| cage-target-json | 746 | — |
| cage-source-yaml | 736 | — |
| cage-target-jsonschema | 732 | — |
| cage-source-sheets | 651 | — |
| cage-target-msgpack | 598 | — |
| cage-source-json | 577 | — |
| cage-source-msgpack | 533 | — |

仓库状态：todo 144 完成 / 0 未完成（用户口径，未复核 todo 文件）。工作区 version 0.1.0，edition 2021，workspace 共 24 成员（Cargo.toml:3-27）。

## 第二节：cage-cli 审核

结构：`src/main.rs`（7349 行）+ `web.rs`（2356）/`mcp.rs`（1206）/`init.rs`（1135）/`lsp.rs`（594）/`remote.rs`（268）。测试：`tests/` 24 个文件共 164 个 `#[test]`，另 src 内嵌单元测试（main.rs 51 个、web.rs 29+ 个）。

### 2.1 错误处理与边界条件

整体评价：生产代码 panic 面很小。`#[cfg(test)]` 边界（main.rs:4237、web.rs:1228、mcp.rs:914、lsp.rs:296、init.rs:877）之外，unwrap/expect 仅 9 处，且多为不变量保护。逐条：

- ✅ `crates/cage-cli/src/main.rs:1045` `.expect("non-empty file list")` — 前置 `files.is_empty()` 分支已在 main.rs:1030 返回 E1802，不变量成立。
- ✅ `crates/cage-cli/src/main.rs:1081,1094` — `strip_prefix`/`swap_remove_entry` 的 expect 均由同函数先前的收集逻辑保证；风险低。
- ✅ `crates/cage-cli/src/main.rs:1887-1894` `render_csv` 先做 `tables.len() != 1` 检查再取 `tables[0]`，无越界。
- ⚠️ `crates/cage-cli/src/main.rs:1349` `chain.into_iter().next().expect("non-empty chain")` — 依赖上游保证链非空；若 `--to` 过滤逻辑后续重构（main.rs:1340 已有 `picked.last().is_none_or` 分支处理空链），此分支可能被绕过。建议改为 `ok_or` 返回错误。
- ✅ `crates/cage-cli/src/main.rs:2721,2801,4038` — 序列化内存结构，serde_json 对 String/Map 不会失败，可接受。
- ✅ `crates/cage-cli/src/web.rs:65` bind 后 `expect("bound to an ip address")` — bind 成功后必然有地址，安全。
- ✅ `crates/cage-cli/src/remote.rs:108` `.expect("empty index serializes")` — 常量构造的空 `RegistryIndex`，不会失败。
- ✅ `crates/cage-cli/src/remote.rs:244,247` `abs.parent().unwrap()` — `rel` 经 `safe_rel`（remote.rs:132-141，拒绝空段/`..`/绝对路径/反斜杠）后 `join` 必有父目录。安全。
- ✅ 无 `panic!`/`unreachable!`/`todo!` 于生产代码（全 src grep 核实）。
- ✅ 无资源泄漏模式：web/mcp 均为单线程循环，无未关闭句柄累积；tempfile 用 RAII。

### 2.2 输入校验与路径处理

- ✅ **web.rs 双重防线做得好**：`api_artifact`（web.rs:868）按 manifest ledger 查表而非 join（web.rs:887「never join-and-pray」），再 blake3 复核（web.rs:904）；增量删除/读取路径先过 `manifest_rel_path_is_safe`（main.rs:4219-4226）再过 `resolved_within_root` symlink 防线（main.rs:4230-4235，使用点 main.rs:2083,2087,2198,2280）。
- ✅ web 请求边界：8 MiB body 上限 + 30 s 读超时（web.rs:87-91，`read_body_with` web.rs:167-197）；Host/Origin 环回校验（web.rs:105-134）。
- ❌ **【缺陷 W1】`crates/cage-cli/src/web.rs:1078,1087`（api_diff）**：`ctx.root.join(&from)` / `join(&to)`，`from`/`to` 是查询参数且未校验。`Path::join` 遇绝对路径会整体替换基路径 — `?from=/etc/x&to=build` 可让服务端解析项目外任意 `manifest.json`；`..` 亦可外逃。与同文件 `api_artifact` 的 ledger 查表标准不一致。缓解因素：仅环回绑定 + Host 校验 + 响应只回错误/统计不回文件内容，风险限于本地，但属同文件内自相矛盾的防护标准。
- ⚠️ **【缺陷 W2】`crates/cage-cli/src/lsp.rs:76`**：`vec![0_u8; content_length?]` — LSP 帧的 `Content-Length`（lsp.rs:71-72 来自对端解析）无上限即分配。对比 web.rs 有 8 MiB 上限；LSP 虽走本地 stdio，但一个异常/被劫持的编辑器插件发一个超大 Content-Length 即可触发巨量分配（abort）。建议与 web.rs 同标准设上限。
- ⚠️ `crates/cage-cli/src/main.rs:1110-1134` `collect_files` — `p.is_dir()` 跟随符号链接，目录符号链接成环时栈会无限增长（本地工具、低概率，未核实实际场景）。无 symlink 循环防护测试。

### 2.3 测试覆盖

- 集成测试 24 文件/164 用例，覆盖 cli_paths、registry(本地/remote/push/signing)、web、mcp_server、lsp、incremental、migrate、gen、golden、db/sheets/remote_source、env_validation/packaging、assets、init_scaffold 等，规模同量级于 src（11230 vs ~12900 生产行）。
- ✅ 路径安全已测：`manifest_rel_path_is_safe` 各形态（main.rs:4244-4254）、symlink 逃逸（main.rs:4260-4274）、`api_artifact` 拒绝 `../cage.toml`（web.rs:1735）。
- ❌ **【测试缺口 T1】** `api_diff` 的 `from`/`to` 无绝对路径与 `..` 用例（现有测试 web.rs:1821-1881 仅覆盖正常/缺参/幽灵目录），恰与缺陷 W1 对应。
- ❌ **【测试缺口 T2】** LSP 无超大 `Content-Length` 用例（tests/lsp.rs 与 src/lsp.rs 测试均未覆盖），与 W2 对应。
- ⚠️ `collect_files` 的符号链接目录环无用例。

### 2.4 文档一致性

- `docs/web.md`（W6/W7/W8 节）与 web.rs:16-38 的 API 端点注释一致；已逐个核对 `api_artifacts` / `api_artifact`（安全口径：只认 manifest 键 + 伺服前重算 blake3，`docs/web.md:177-179` vs web.rs:868-913）、`api_diff`（`docs/web.md:214-216` vs web.rs:1063+）、`api_snapshot` / `api_snapshots` / `api_registry`。「仅绑定 127.0.0.1、无鉴权」与 web.rs:59-65 一致（`docs/web.md:68-70`）。`api_diff` 未提路径约束，与实现一致但暴露 W1 的标准不一致（见 2.2、表二）。
- `docs/cli.md:372` 对 LSP 帧协议的描述（`Content-Length` 头 + 定长体）与 lsp.rs 一致；帧大小上限在文档与实现中均不存在（W2，见表二）。

## 第三节：cage-core 审核

结构：`registry.rs`（4193）/`validation/mod.rs`（3937）/`migrate/mod.rs`（2267）/`schema/mod.rs`（1538）/`manifest/mod.rs`（1499）/`reference/mod.rs`（1239）/`migrate/diff.rs`（1220）/`normalize/mod.rs`（1021）/`diagnostics`（857）/`value`（719）/`remote.rs`（697）/`snapshot`（384）/`edit`（329）/`rules`（294）/`assets`（254+165 测试）。`#[cfg(test)]` 边界：registry.rs:1956、validation:1381、migrate:887、manifest:621、schema:636、remote:495。**cage-core 无 `tests/` 集成测试目录（`ls crates/cage-core/` 只有 `Cargo.toml` 与 `src`），全部 ~275 个用例是 src 内嵌单元测试。**

**修订说明**：本报告路径上早先残留的一版 draft 把「快照账本路径可逃逸出快照根」列为高风险，并已在本节/第五节/结论中据此展开。逐行核实后**该声称不成立**，已全部删除并在 3.5 记录证据。本节其余结论为本次实际审阅结果。

### 3.1 错误处理与边界条件

- ✅ 生产代码无 `panic!`/`unreachable!`/`todo!`；unwrap/expect 全部落在「父目录必然存在」「上游已判空」「内存结构序列化」三类不变量上，逐条核实：
  - ✅ `crates/cage-core/src/registry.rs:479,482`、`508`、`597,600,961` — `create_dir_all(...).expect("index parent")` 与 `strip_prefix(dir).expect("under dir")`：路径均由 `join` 构造、文件由 `read_dir(dir)` 产生，不变量成立。
  - ✅ `crates/cage-core/src/registry.rs:1508-1509` `.expect("non-empty entries")` — 函数开头 registry.rs:1459-1461 已对 `index.entries.is_empty()` 返回 E1802，不变量成立。
  - ✅ `crates/cage-core/src/validation/mod.rs:1192` `target_row_idx.unwrap()` — not-found 分支在 mod.rs:1173-1185 提前 `return`，到 1192 必为 Some。
  - ✅ `crates/cage-core/src/migrate/mod.rs:225` `raw.into_iter().next().unwrap()` — mod.rs:220-224 已判 `len() != 1` 报错；`migrate/mod.rs:728` `.expect("just inserted")` 为紧接插入后的同位置断言，安全。
  - ✅ `crates/cage-core/src/migrate/diff.rs:595,599,613,617` `write!(out, ..).unwrap()` — 写入 `String` 的 `fmt::Write` 不会失败。
  - ✅ `crates/cage-core/src/manifest/mod.rs:246,277` — 序列化自有结构，实践中不会失败。
- ❌ **缺陷 C1（中）**：`crates/cage-core/src/remote.rs:319-332` `read_response` 用 `read_to_end(&mut bytes)` 读全响应，**无响应体大小上限**。恶意或已被劫持的远程 registry / 远程配置源可流式返回任意大 body → 内存耗尽。同一仓的 HTTP 出口没有字节边界，而 cage-cli 的 Web 入口有 8 MiB 上限（`cage-cli/src/web.rs:87-91`），两处标准不一致。`ureq` 的 30 s timeout（remote.rs:210,235,266）只限时间、不限字节。建议 `into_reader().take(MAX)` 或逐块计数截断并返回 E21xx。
- ⚠️ **缺陷 C2（低-中）**：`crates/cage-core/src/remote.rs:165-186` `with_retries` 末尾 `Err(last.expect("retry policy with zero attempts"))`。`RetryPolicy` 是 pub struct + pub 字段（remote.rs:105-107，默认 `attempts: 3`），`with_retries` 也是 pub（remote.rs:165）；任何外部调用方构造 `attempts: 0` 即在函数内 panic。remote.rs:183-184 的注释承认了这个假设，却用 `expect` 而非返回 `Err`。建议改为 `None` 分支返回 `FetchFailure::Transport("retry policy has zero attempts")`。
- ⚠️ **缺陷 C3（中）**：`crates/cage-core/src/registry.rs:596-614` `publish` 逐文件 `fs::copy` 直写 `<root>/<package>/<version>/`，**无临时目录、无失败回滚、非原子**：中途 `fs::copy` 失败（磁盘满、权限）会留下半成品 entry 且无任何清理。`write_index`（registry.rs:474-490）同样是 `fs::write` 直写，崩溃中途可把 `index.json` 写成半截 → 之后 `read_index` 报「corrupt registry index」（registry.rs:460-465），fail-visible 但不可自愈，需人工删除。对「本地发布、不可重写历史」的 registry 语义，建议先写 `.tmp` 再 `rename`，失败时移除半成品。
- ⚠️ **缺陷 C4（中，正确性）**：`crates/cage-core/src/migrate/mod.rs:434-436` `parse_migration_dir` 用 `entries.filter_map(Result::ok)` **静默丢弃不可读的目录项**。单个迁移文件权限异常会被无声跳过 → **迁移链可能悄悄漏掉一步**；而同类逻辑在 `cage-cli/src/main.rs:1118-1120` `collect_files` 里是逐条 `map_err` 上报。建议统一为报错。
- ✅ `resolve` / `resolve_pinned`（registry.rs:1518-1543）在返回 entry 前强制 `snapshot::verify_snapshot` 信任门，不通过报 E1803。`read_index`（registry.rs:449-466）区分「不存在 → 空索引」「损坏 → 报错，不静默重写」，边界处理正确。
- ⚠️ **C5（未核实）**：在 value / normalize / registry / remote 中 grep `recursion|depth|MAX_DEPTH` 均无命中，未见任何嵌套深度限制。JSON 侧 serde_json 自带 128 层递归上限（默认开启）；**YAML 深层嵌套的解析与析构栈深度未核实**，恶意配表理论上可触发栈溢出。建议补一条「深层嵌套 YAML 被拒绝或安全解析」的测试后再定级。

### 3.2 输入校验与反序列化

- ✅ `parse_spec`（registry.rs:429-448）：`registry:` 前缀 + `valid_component` 双字段白名单校验，直接阻断路径穿越；已有测试 `parse_spec_rejects_path_escape_attempts`（registry.rs:2028）。
- ✅ 迁移 Spec 反序列化严格：Step 用「恰好一个 transform key」的自定义 `Deserialize`（migrate/mod.rs:213-228），多 key / 空 steps 报 E2001（测试 mod.rs:1080-1116）。
- ✅ `FieldType` / `MapKeyType` 反序列化拒绝未知 kind 与裸标量（schema/mod.rs:979-996,1031-1033）。
- ✅ 迁移目录条目来自 `read_dir` 的 `file_name`，本身无分隔符 → `dir.join(name)` 无穿越面（migrate/mod.rs:434-440）。
- ✅ `read_index` 对损坏索引不静默修复（registry.rs:460-465），符合「corrupt 是错误」的约定。
- ✅ `verify_snapshot` 只把账本当字节哈希清单用，不对其中的路径做任何语义假设之外的读取——见 3.5，所有读取都过 `snapshot_path`。

### 3.3 测试覆盖

- 单元测试覆盖面广：validation 56 个（含 E1101 类型不符、E1401 引用缺失、E1403 环检测、E1404 基数、E1410 predicate、E1501/E1601 规则、复合唯一约束），registry 48，migrate 26，schema 18，manifest 16，reference 13，value 14，normalize 23，migrate/diff 12，snapshot 3。
- ✅ 篡改可恢复性已测：`verify_accepts_clean_and_flags_tamper_growth_and_loss`（snapshot/mod.rs:277）、tar 确定性 header（registry.rs:2122-2130）、路径逃逸三重用例（snapshot/mod.rs:279-312）。
- ❌ **测试缺口 T3**：`remote.rs` 的 5 个用例（cache_key、重试恢复、预算耗尽、429 协商）全部针对 `with_retries`，**无响应体大小用例**（对应 C1），**无 `attempts: 0` 用例**（对应 C2）。
- ❌ **测试缺口 T4**：`publish` 的中途失败 / 半成品目录清理无用例（C3）；`index.json` 半截损坏后的行为无用例。
- ❌ **测试缺口 T5**：`parse_migration_dir` 遇到不可读条目时无用例（C4）。
- ⚠️ 整个 cage-core 无集成测试目录，跨模块路径（schema 解析 → 校验 → 规范化）目前只由 cage-cli 的 24 个集成测试间接触达，core 侧缺端到端回归网。

### 3.4 文档一致性

- `docs/build.md:226` 称 `HASHES.json` 为「逐文件 blake3 账本（trust root，不自我哈希）」，与 `snapshot/mod.rs:12-19` 及 `verify_snapshot` 的实现（ledger 自身不参与校验）一致。
- `docs/build.md:244` 起描述 `verify_snapshot` / `load` 的服务器加载流程，与 registry.rs:1518-1543 的「先验再交」顺序一致。
- 未见 README / docs 对 registry、migrate、remote 三块功能的描述与实现相矛盾。

### 3.5 已证伪的残留结论（重要）

早先残留在同一文件的一版 draft 声称 `verify_snapshot` / `load` 存在「账本 rel 未限制在快照根内」的高风险缺陷，并据此给出第五节与结论。逐行核实后**不成立**：

- `snapshot_path`（crates/cage-core/src/snapshot/mod.rs:39-55）已实现三重拒绝：空 / 绝对路径 / 非 `Component::Normal`（含 `..`、根、前缀）直接报 `unsafe snapshot-relative path`；再对 root 与 target 分别 `canonicalize`，`!target.starts_with(&root)` 报 `resolves outside snapshot root`。
- 所有读取都走它：`read_snapshot_file`（mod.rs:57-62）与 `verify_snapshot` 的账本循环（mod.rs:120-131 `snapshot_path(dir, rel).and_then(|p| fs::read(p))`）无例外。
- symlink 不递归：`files_on_disk` 用 `symlink_metadata`（mod.rs:215-224），`is_symlink()` 判断在 `is_dir()` 之前，目录型 symlink 被当作文件条目列出、不会进入 `walk`。
- 三重用例都在：`../`（snapshot/mod.rs:288-290）、绝对路径（:298-299）、指向根外的 symlink（:304-312，断言 `resolves outside snapshot root`）。

残留稿另称 `docs/build.md` 与实现不符——同样不成立（见 3.4）。该两节内容已整体删除。

## 第四节：cage-source-db 审核

结构：`lib.rs`（631）/`pg.rs`（556）/`mysql.rs`（514）/`draft.rs`（345）/`pool.rs`（238）；`#[cfg(test)]` 边界：lib.rs:337、pool.rs:70+120、draft.rs:189、mysql.rs:221+305、pg.rs:241+326。共 36 个 `#[test]`（lib 10 / pool 7 / draft 7 / mysql 6 / pg 6）。

### 4.1 错误处理与边界条件

- ✅ 查询形状、DSN 缺失、连接失败均以 `Result` 返回；缓存回退只在 transport fetch 失败时启用，strict 模式拒绝回退（`crates/cage-core/src/remote.rs:53-88` `source_cache_fallback`，db 侧调用点 lib.rs:241-310）。
- ✅ PostgreSQL 用自研 idle 栈：锁只保护 `idle` Vec，连接交出去执行时不持锁，失败连接直接丢弃不归还（`pool.rs:37-52`）；池有上限（MySQL `PoolConstraints::new(0, 8)` pool.rs:96；PG `max_idle = 4` pool.rs:117），无句柄增长型泄漏。
- ✅ MySQL `SET SESSION TRANSACTION READ ONLY` 在连接建立时钉住（`mysql.rs:44-45`、`114-115`）；PostgreSQL `SET default_transaction_read_only = on`（`pg.rs:49-50`、`148-149`）。有真数据库用例断言 pin 生效且 INSERT 被拒（`pg.rs:440-465`）。
- ⚠️ **缺陷 D1（低-中，正确性）**：`crates/cage-source-db/src/lib.rs:218-228` `canonical_json` 用 `row_set.columns.iter().zip(cells)` 配对——**行宽不匹配时静默截断**（多列丢弃尾部列、少列丢弃尾部值），且 `obj.insert` 对重复列名静默覆盖（后者胜）。`RowSet` 是本 crate 内部构造、由驱动结果回填，当前后端实际总能给出一致宽度，所以是潜在不变量而非活缺陷；但序列化入口没有任何形状校验，也没有畸形 RowSet 测试，一旦某个后端漏列就会静默改数据。
- ⚠️ **缺陷 D3（低）**：`pool.rs:55,62`（另 pool.rs:85,96,104,116）`self.idle.lock().unwrap()` / `map.lock().unwrap()` — Mutex 中毒会把「前一个线程闭包里的 panic」升级成第二次 panic，掩盖原始错误。当前 CLI 单线程，属潜在项；建议 `lock().map_err(...)` 或 `into_inner` 取回连接继续用。
- ✅ 生产代码无 `panic!`/`unreachable!`；唯一生产 `expect` 为 JSON 序列化不变量（lib.rs:238「serialized map is valid JSON」），其余 `unwrap` 均在 `#[cfg(test)]` 之后。

### 4.2 输入校验与路径处理

- ✅ `parse_spec`（lib.rs:92-98）拒绝空名；`valid_table_name`（lib.rs:134-139）要求纯 ASCII 标识符字符（字母/数字/`_`/`.`/`$`，首字符不得为数字或 `_`），避免表名直接拼进 SQL。
- ✅ `resolve_query`（lib.rs:143-156）：具名查询优先，否则必须是安全表名，否则 E1905。`resolve_dsn`（lib.rs:163-177）只从 `dsn_env` 指名的环境变量取，cage.toml 不落明文 DSN，未设置/空值报 E1904。
- ⚠️ **缺陷 D2（低，纵深防御）**：`validate_select`（lib.rs:104-130）是**字符串规则**白名单，`INTO` 用 `upper.contains(" INTO ")` 匹配（lib.rs:118-120）——只按空格界定，制表符/换行分隔的 `SELECT a FROM t\tINTO OUTFILE '/x'` 会绕过该条检查；MySQL 的行注释符 `#` 未纳入拒绝名单（只拒 `--`、`/*`、`*/`，lib.rs:114）。真正的写保护来自会话 pin（mysql.rs:44、pg.rs:49），pin 确实挡住表写入；但 `SELECT … INTO OUTFILE` 属服务器侧文件写，只读事务并不阻止（取决于该账号是否有 `FILE` 权限），此时字符串白名单是第一道也是唯一一道防线。缓解：具名查询只能由配置作者声明、DSN 账号通常无 `FILE` 权限。建议把 `INTO` 检查改成词边界匹配并补 `#` 注释。
- ✅ 草稿生成对不安全 YAML token 加引号并转义单引号（`draft.rs:25-45` `yaml_token`，渲染点 `draft.rs:143-175`），`tables` 为空时仍输出合法空文档（draft.rs:134-140）；测试覆盖含空格/引号等名称（draft.rs:269-347）。
- ⚠️ 缓存路径由 `cage_core::remote::cache_key` 派生（remote.rs:28-33），本 crate 未另行约束；缓存目录内的 symlink 边界未在本 crate 核实（core 侧见 3.5，快照路径已约束）。

### 4.3 测试覆盖

- ✅ 覆盖：spec 拆分、SELECT 白名单 allow/deny（含 `;`、`--`、`/*`、`FOR UPDATE`、`FOR SHARE`、CTE，lib.rs:377-390）、查询名解析、DSN 解析、类型映射（含 DECIMAL 文本保真与 Bytes base64）、排序与 canonical JSON、缓存成功/离线回退（lib.rs:353-618）；pool 上限与重用（pool.rs:137-241）；draft 渲染（draft.rs:269-347）；MySQL/PostgreSQL 真库集成用例受 `live_dsn()` 环境条件门控（mysql.rs:330-432、pg.rs:351-553）。
- ❌ **测试缺口 DB-T1**：`canonical_json` 无行宽不匹配（cells 少于/多于 columns）与重复列名用例，未锁住 D1 的语义。
- ❌ **测试缺口 DB-T2**：`validate_select` 无制表符/换行分隔的 `INTO`、无 `#` 注释、无超长 query 用例（对应 D2）。
- ⚠️ DB 集成路径依赖外部服务；CI 是否配置 MySQL / PostgreSQL 服务未核实（未查看 CI 工作流）。

### 4.4 文档一致性

- ✅ `docs/cli.md:319-320` 对两个 DB 源的描述与实现**逐条对上**：SELECT 白名单（单条、SELECT 开头、拒分号/注释/行锁/CTE/INTO）、`dsn_env` 解析、错误码（E1905 白名单 / E1904 凭据 / E1901 连接语句）、以及 MySQL `SESSION TRANSACTION READ ONLY` 与 PG `default_transaction_read_only` 两个 pin 的具体名字都与 mysql.rs:44、pg.rs:49 一致。
- ✅ `README.md:44` / `README.zh.md:46` 的「schema draft from a live database（只读、绑定参数）」与 `draft.rs:132-179` 的渲染产物一致。
- ✅ 未见行为矛盾；D2 的绕过形态属于文档与实现都未覆盖的边界，不计为文档不符。

## 第五节：汇总表

### 表一：缺陷

| 编号 | 问题 | 严重度 | 证据 |
|---|---|---|---|
| C1 | HTTP 响应体读取无字节上限，可内存耗尽 | 中（网络面） | `crates/cage-core/src/remote.rs:319-332` |
| W1 | web `api_diff` 的 `from`/`to` 直接 `ctx.root.join`，未校验，绝对路径与 `..` 可解析项目外 manifest | 中（本地 Web） | `crates/cage-cli/src/web.rs:1078,1087` |
| C3 | registry `publish` 与 `write_index` 非原子写，无失败回滚/半成品清理，`index.json` 可被写半截 | 中 | `crates/cage-core/src/registry.rs:474-490,596-614` |
| C4 | 迁移目录不可读条目被 `filter_map(Result::ok)` 静默丢弃，迁移链可漏步 | 中（正确性） | `crates/cage-core/src/migrate/mod.rs:434-436` |
| W2 | LSP 帧按对端 `Content-Length` 直接分配，无上限 | 低-中（本地 stdio） | `crates/cage-cli/src/lsp.rs:71-76` |
| C2 | `with_retries` 在 `attempts: 0` 时 panic，而策略与函数均 pub | 低-中 | `crates/cage-core/src/remote.rs:165-186,105-107` |
| D1 | `canonical_json` 的 `zip` 静默截断行宽不匹配、重复列名覆盖 | 低-中 | `crates/cage-source-db/src/lib.rs:218-228` |
| D2 | `validate_select` 的 `INTO` 检查按空格界定，`#` 注释未拒；纵深防御缺口 | 低 | `crates/cage-source-db/src/lib.rs:114,118-120` |
| D3 | `idle.lock().unwrap()` 等 Mutex 中毒会二次 panic | 低 | `crates/cage-source-db/src/pool.rs:55,62,85,96,104,116` |
| W3 | `chain.into_iter().next().expect("non-empty chain")` 依赖上游不变量 | 低 | `crates/cage-cli/src/main.rs:1349` |
| W4 | `collect_files` 跟随目录 symlink，无环路防护 | 低（未核实场景） | `crates/cage-cli/src/main.rs:1110-1134` |
| C5 | 未见嵌套深度限制，YAML 深层嵌套栈风险 | 未核实 | `crates/cage-core/src/{value,normalize}/mod.rs`（grep 无命中） |

### 表二：文档不符 / 描述不完整

| 位置 | 发现 | 证据 |
|---|---|---|
| `docs/web.md:177-179` vs `:214-216` | 同页对 `api_artifact` 明确写出「只认 manifest 键（查表，不 join-and-pray）+ 伺服前重算 blake3」的安全口径，`api_diff` 只描述返回形状、未提任何路径约束——文档与代码都一致（确实没有约束），但两条端点的安全标准不自洽，属 W1 的直接旁证 | `docs/web.md:177-179,214-216`; `crates/cage-cli/src/web.rs:868-913,1078,1087` |
| `docs/cli.md:372` | LSP 帧协议描述为「`Content-Length` 头 + 定长体」，未披露任何帧大小上限；实现也确实无上限（W2）。不属事实错误，属边界未披露 | `docs/cli.md:372`; `crates/cage-cli/src/lsp.rs:71-76` |
| `docs/build.md:226,244` | HASHES 作为 trust root、先验再交的描述与实现一致 | `docs/build.md:226,244`; `crates/cage-core/src/snapshot/mod.rs:12-19` |
| `docs/cli.md:319-320` | DB 两个源的白名单、`dsn_env`、只读 pin 名（MySQL `SESSION TRANSACTION READ ONLY` / PG `default_transaction_read_only`）与实现逐条一致 | `crates/cage-source-db/src/mysql.rs:44,114`; `pg.rs:49,148` |
| 残留 draft（已删除） | 同路径早先未提交的一版 draft 称快照账本可逃逸根目录、`docs/build.md` 与实现不符——均经核实不成立（`snapshot_path` 三重拒绝 + 三重测试），已从本报告删除 | `crates/cage-core/src/snapshot/mod.rs:39-55,279-312` |
| README / 三模块能力描述 | 未发现行为矛盾 | `README.md:44`; `README.zh.md:46` |

### 表三：值得补的能力

| 能力 | 理由 | 证据 |
|---|---|---|
| HTTP 响应体字节上限 + 4xx 上报 | 唯一网络出口没有字节边界，与 cage-cli Web 入口的 8 MiB 上限标准不一致 | `crates/cage-core/src/remote.rs:319-332`; `crates/cage-cli/src/web.rs:87-91` |
| 复用 `manifest_rel_path_is_safe` + `resolved_within_root` 到 `api_diff` | 仓内已有双重防线的现成实现，`api_diff` 是唯一没套上的 Web 端点 | `crates/cage-cli/src/main.rs:4219-4235`; `web.rs:1078,1087` |
| LSP 帧 `Content-Length` 上限（与 Web 同标准） | 本地 stdio 输入同样应设明确资源上限 | `crates/cage-cli/src/lsp.rs:71-76` |
| registry 发布改「临时目录 + rename + 失败清理」 | 匹配「本地发布、不可重写历史」语义，避免半成品 entry 与半截 `index.json` | `crates/cage-core/src/registry.rs:474-490,596-614` |
| `RowSet` 形状校验（列数一致、列名唯一） | 把 D1 的静默数据改写变成显式 E19xx 错误 | `crates/cage-source-db/src/lib.rs:218-228` |
| 迁移目录条目逐条 `map_err` + 不可读条目测试 | 与 `collect_files` 的错误口径统一，避免漏步 | `crates/cage-core/src/migrate/mod.rs:434-436`; `crates/cage-cli/src/main.rs:1118-1120` |
| SQL 侧只读 pin 的失败语义测试矩阵（两后端） | 目前只有 PG 一例断言写被拒，MySQL 侧未见等价断言 | `crates/cage-source-db/src/pg.rs:440-465`（MySQL 侧未见） |
| cage-core 集成测试目录 | 275 个单元测试全是 src 内嵌，跨模块路径缺端到端回归网 | `ls crates/cage-core/` 只有 `Cargo.toml`、`src` |

## 结论：最需先解决的三件事

1. **给 HTTP 响应体设字节上限**（C1，`crates/cage-core/src/remote.rs:319-332`）：这是三模块里唯一由网络对手直接控制、且没有字节边界的入口。改法很小（`into_reader().take(MAX)` 或逐块计数截断后返回 E21xx），但收益是消除整个远程源/远程 registry 路径上的内存耗尽面。
2. **把仓内已有的双重路径防线套上 `api_diff`**（W1，`crates/cage-cli/src/web.rs:1078,1087`）：`manifest_rel_path_is_safe`（main.rs:4219-4226）+ `resolved_within_root`（main.rs:4230-4235）在别处已用且已测，唯独 `api_diff` 裸 `join` 查询参数；同时与同页 `api_artifact` 的显式安全口径（`docs/web.md:177-179`）对齐。
3. **让 `parse_migration_dir` 报错而不是静默丢条目**（C4，`crates/cage-core/src/migrate/mod.rs:434-436`）：这是唯一一条会让用户**静默得到错误结果**的缺陷——迁移链少跑一步、数据看起来一切正常。改一行为 `map_err`，并补一条不可读条目测试。

紧随其后（同一轮顺手做）：LSP 帧 `Content-Length` 上限（W2，lsp.rs:71-76）、registry 发布原子化与半成品清理（C3，registry.rs:474-490,596-614）、`canonical_json` 的 RowSet 形状校验（D1，lib.rs:218-228）。

附注：按要求本次未改代码、未 bump 版本号、未 force push；未运行 `cargo test`，故「测试是否存在」已核实、「测试是否通过」未核实。C5 与 CI 的 DB 服务配置为显式「未核实」项。第三节 3.5 记录了同一文件上一版 draft 中被逐行证伪的结论及其证据。
