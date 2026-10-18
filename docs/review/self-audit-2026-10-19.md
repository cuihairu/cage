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

- `docs/web.md`/`web.en.md` 与 web.rs:16-38 的 API 端点表一致（7 个端点逐一对上，详见第五节表二）；文档声称「仅绑定 127.0.0.1、无鉴权」，与 web.rs:59-65 一致。

## 第三节：cage-core 审核

重点抽查 `manifest`、反序列化配置与快照模块。单元测试覆盖面广，尤其 manifest 哈希/校验、快照完整性；但快照账本路径信任边界有高风险缺口。

### 3.1 错误处理与边界条件

- ✅ manifest 生产代码将哈希校验、缺失产物等作为 `Result`/诊断返回；测试有 hash/size 不符、丢失及额外产物路径（`crates/cage-core/src/manifest/mod.rs:382-451,889-936`）。
- ❌ **缺陷 C1（高）**：`verify_snapshot` 从账本取 `rel` 后直接 `dir.join(rel)` 读取，没校验绝对路径、`..` 或规范化后的根目录包含关系（`crates/cage-core/src/snapshot/mod.rs:119-124`）。`load` 又按目录遍历得到的 `data/` / `generated/` 路径直接 join 读取（`snapshot/mod.rs:173-177`）；在快照内放置指向根外文件的 symlink 时，`files_on_disk` 将该 symlink 作为文件列出，verify 会跟随读取；若它位于 data/ 或 generated/，load 也会把根外内容放入返回的 artifact map。账本中的绝对路径或 `..` 同样会让 verify 读取根外文件（`snapshot/mod.rs:119-124`）。适用条件是攻击者能提供/替换快照及账本；建议拒绝非规范相对路径并拒绝 symlink，且读取前校验 canonical path 在根目录内。
- ⚠️ **缺陷 C2（可用性）**：递归遍历以 `path.is_dir()` 跟随目录 symlink（`snapshot/mod.rs:192-202`）；快照目录内指向自身/祖先的 symlink 可能递归不终止。没有发现循环检测或 symlink 专项测试（`snapshot/mod.rs:247-386`）。
- ✅ 生产快照的序列化 `expect` 对 serde_json `Value` 是内存序列化不变量（`snapshot/mod.rs:85-89`）；测试内的 `unwrap` 属于断言路径。

### 3.2 输入校验与反序列化

- `BuildManifest` 和 `ProjectConfig` 使用 serde 类型反序列化，配置序列化往返及默认/可选字段有测试（`manifest/mod.rs:938-1176`）；超大 JSON/YAML 输入的资源限制未见实现/测试，未核实上游是否限制文件大小。
- 快照加载解析账本 JSON 并要求 `files` 为 object、hash 为字符串；没有校验 hash 长度/hex 格式，最终只比较字符串（`snapshot/mod.rs:110-131`）。无效 hash 会成为 mismatch；拒绝结果正常，但路径缺少对应约束（同 C1）。

### 3.3 测试覆盖

- 有快照打包确定性、篡改/新增/删除检测和 load round-trip 测试（`snapshot/mod.rs:247-386`）；manifest 覆盖哈希、字段和序列化（`manifest/mod.rs:889-1204`）。
- ❌ **测试缺口 C-T1**：未见账本 `../`、绝对路径、重复/非规范路径、symlink escape 用例（`snapshot/mod.rs:247-386`），没有锁住 C1 的安全边界。
- ❌ **测试缺口 C-T2**：未见递归 symlink 环与超深目录/超大 ledger 资源边界测试（`snapshot/mod.rs:185-209,247-386`）。

### 3.4 文档一致性

- README 中快照“数据与账本一起替换无法检测”的说明符合 trust-root 设计（`README.md:42`、`README.zh.md:44`；实现说明在 `snapshot/mod.rs:12-19`）。
- `docs/build.md` 将 HASHES 称为 trust root，概述 verify/load（`docs/build.md:214-234,244`），与代码目标一致；文档未提到账本路径必须限制在快照根内，而代码确实未做限制（`snapshot/mod.rs:119-124,173-177`），属于安全边界描述不完整，列第五节。

## 第四节：cage-source-db 审核

### 4.1 错误处理与边界条件

- ✅ 查询形状、DSN 缺失、连接/缓存错误以 `Result` 返回；缓存回退仅在 transport fetch 失败时启用，strict 模式拒绝回退（`crates/cage-source-db/src/lib.rs:100-129,163-175,241-310`）。
- ✅ PostgreSQL idle pool 错误连接不归还；锁只保护 idle 栈，连接交给调用闭包时不持锁（`crates/cage-source-db/src/pool.rs:22-56`）；pool 有上限及并发/失败/重用测试（`pool.rs:137-241`）。MySQL 连接池由驱动管理，数据库集成测试受服务环境条件限制（`mysql.rs:330-432`、`pg.rs:351-553`）。
- ⚠️ `canonical_json` 将列和值用 `zip` 配对，若 RowSet 内部列数与行宽不匹配会静默截断（`lib.rs:218-228`）；当前后端构造是否永远保证行宽一致未核实，建议在序列化入口显式检查并测试畸形 RowSet。
- ✅ 测试内的 `unwrap`/`expect` 是断言；唯一生产 `expect` 为 JSON Value 序列化（`lib.rs:237-238`），不会因外部输入触发序列化错误。

### 4.2 输入校验与路径处理

- ✅ `mysql:` / `pg:` spec 拒绝空名（`lib.rs:92-98`）；自动拼表名要求 ASCII 标识符字符（`lib.rs:132-138,143-158`），避免表名直接 SQL 注入。
- ⚠️ named query 由配置直接取出后才进入 `load` 的静态 SELECT 检查；检查是字符串规则（SELECT 前缀、分号/注释/行锁/INTO），不能替代数据库只读会话边界（`lib.rs:100-129,313-335`）。README/CLI 描述的只读会话保护在实现端存在（MySQL `crates/cage-source-db/src/mysql.rs:49`；PG `crates/cage-source-db/src/pg.rs:49`）。
- ✅ DSN 从配置指定的环境变量读取，不从 cage.toml 明文读取（`lib.rs:160-175`）。缓存 key 路径由 cache_key 派生，缓存路径细节在 core remote 中；此处未核实额外 symlink 边界。
- ✅ 草稿生成将不安全 YAML token 加引号并转义单引号（`crates/cage-source-db/src/draft.rs:25-45,143-175`）；测试覆盖空格、引号等名称（`draft.rs:269-347`）。

### 4.3 测试覆盖

- crate 内共 36 个测试属性：共享逻辑覆盖 spec、SELECT allow/deny、查询名、DSN、类型映射、排序/规范 JSON、缓存成功/离线回退（`lib.rs:353-618`）；draft、pool、MySQL/PostgreSQL 另有单元/集成测试（`draft.rs:194-347`、`pool.rs:137-241`、`mysql.rs:225-432`、`pg.rs:245-553`）。
- ❌ **测试缺口 DB-T1**：`canonical_json` 没覆盖 cells 少于/多于 columns、重复列名；当前 zip 截断/Map 覆盖语义会改变数据（`lib.rs:218-228,457-519`）。
- ⚠️ SQL whitelist 测了常见拒绝和 CTE，但未见混淆大小写/关键字作为字符串字面量、数据库特定只读函数、超长 query 的边界测试（`lib.rs:362-390`）。DB 集成路径依赖外部服务；未核实 CI 是否配置 MySQL/PostgreSQL 服务。

### 4.4 文档一致性

- README 中 DB schema-draft、只读内省和 DECIMAL/日期/JSON 的说明与源适配器目标一致（`README.md:44`、`README.zh.md:46`；`draft.rs:132-179`）。
- CLI 表格描述单条 SELECT 白名单、拒绝注释/分号/CTE/INTO、DSN env 与只读会话（`docs/cli.md:319-320`）；校验器和后端有对应实现（`lib.rs:100-129`，只读会话精确实现行号未核实）。没有核实出行为矛盾。

## 第五节：汇总表

### 缺陷

| 编号 | 问题 | 影响 | 证据 |
|---|---|---|---|
| C1 | 快照账本路径不限制在快照根内；load 可返回根外文件内容 | 高；不可信快照可越界读取宿主文件 | `crates/cage-core/src/snapshot/mod.rs:119-124,173-177` |
| C2 | 快照目录递归跟随 symlink，可能循环 | 可用性/挂起或栈耗尽 | `crates/cage-core/src/snapshot/mod.rs:192-202` |
| DB1 | RowSet 列和值 zip 静默截断，畸形行数据丢值 | 数据完整性 | `crates/cage-source-db/src/lib.rs:218-228` |
| CLI1 | web `api_diff` 查询参数 join 未做路径约束 | 本机 web 服务可能读项目外 manifest 元信息 | `crates/cage-cli/src/web.rs:1078,1087` |
| CLI2 | LSP Content-Length 无上限分配 | 本地编辑器输入可触发内存耗尽 | `crates/cage-cli/src/lsp.rs:71-76` |

### 文档不符 / 描述不完整

| 位置 | 发现 | 证据 |
|---|---|---|
| docs/build.md | 快照 trust-root / load 说明未说明账本相对路径和 symlink 必须限制在根目录；实现也缺该限制，读者容易误解验证范围 | `docs/build.md:214-244`; `crates/cage-core/src/snapshot/mod.rs:119-124,173-177` |
| README / docs CLI | 未发现三模块核心能力描述与代码相矛盾；DB 实际只读会话的精确代码位置本次未核实 | `README.md:35-46`; `README.zh.md:37-48`; `docs/cli.md:319-320` |

### 值得补的能力

| 能力 | 理由 | 证据 |
|---|---|---|
| 快照 ledger path validator + symlink-safe traversal | 拒绝绝对路径/父目录段，并拒绝 symlink 或验证 canonical path 的根目录包含关系 | `crates/cage-core/src/snapshot/mod.rs:119-124,185-202` |
| RowSet shape validation | 在生成 JSON 前拒绝行宽不匹配和重复列名 | `crates/cage-source-db/src/lib.rs:218-228` |
| LSP frame size cap、递归文件遍历 symlink-cycle guard | 给本地输入边界设置明确资源上限 | `crates/cage-cli/src/lsp.rs:71-76`; `crates/cage-cli/src/main.rs:1110-1134` |
| SQL lexer/parser 或数据库级只读测试矩阵 | 静态字符串拒绝规则可维护性有限，验证每后端只读 pin 的失败语义 | `crates/cage-source-db/src/lib.rs:100-129`; `lib.rs:362-390` |

## 结论：最需先解决的三件事

1. 修复快照 ledger 的根目录逃逸：验证时拒绝不安全 rel，并在 load 每次读取前做根目录约束；补 `../`、绝对路径和 symlink 测试（`crates/cage-core/src/snapshot/mod.rs:119-124,173-177`）。
2. 为 LSP 帧设 Content-Length 上限，并测试拒绝超限帧（`crates/cage-cli/src/lsp.rs:71-76`）。
3. 给 web `api_diff` 的 `from` / `to` 参数做安全路径解析，并测绝对路径和 `..`（`crates/cage-cli/src/web.rs:1078,1087`）。

附注：按要求本次未改代码，未运行测试；本文为静态审阅。所有未能确认的实现细节均标为“未核实”。
