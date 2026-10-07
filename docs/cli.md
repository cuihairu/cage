# CLI

Cage 的命令行入口（`cage-cli` crate，基于 [clap](https://crates.io/crates/clap)）：

```bash
cage check
cage build
cage gen
cage inspect
cage diff
cage snapshot
cage web
cage registry
```

MVP 落地前四个（`check` / `build` / `inspect` / `diff`），`gen` / `snapshot`
为第二阶段（均已实装），`web` 为第三阶段（Schema 编辑器本地服务），
`registry` 为第三阶段 R 系列（本地 Configuration Registry 发布/列表，
R1–R4 均已实装）。`cage verify runtime/`（验证已生成产物）与
`cage graph`（读依赖图）在 [design §30](https://github.com/cuihairu/cage/blob/main/docs/design.md#30-cli) 是规划命令，
尚未实装：配置依赖图经 `cage build --incremental` 实时参与构建决策。

## check

```bash
cage check config/
```

只验证，不生成 Runtime Artifact。可以分级执行：

```bash
cage check --level schema       # 只做 Schema 层
cage check --level reference    # 只做到引用层
cage check --profile client     # 按 Profile 验证
```

## build

```bash
cage build config/ --profile client
cage build config/ --profile server
cage build config/ --incremental     # 哈希与上次 manifest 一致时跳过重新生成
```

验证并生成目标产物，同时输出 [Build Manifest](/build#build-manifest)。
`--incremental` 依据 manifest 里的 schema/source 哈希跳过未变化的重建
（校验仍全量执行；改了 cage.toml 的 targets 请全量重建，详见
[增量构建](/build#增量构建)）。

## gen

```bash
cage gen config/ --profile server
```

只生成代码类产物（[C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java](/target#code-targets-与-data-targets-分离)），
不跑数据校验：代码生成是 Schema 驱动的，类型与元数据全部来自 Schema，
不依赖配置行数据。Profile 里的数据类 Target（json/csv）会被跳过，需要
数据产物时用 `cage build`。产物同样写入 Build Manifest（与 build 同一口
径），后写者胜。

## inspect

```bash
cage inspect Item
```

查看 Schema 和配置结构。

## `--no-cache`：远程源离线回退开关

```bash
cage build config/ --no-cache    # check / build / gen / inspect 通用
```

`check` / `build` / `gen` / `inspect` 四命令都带 `--no-cache`：关闭
远程源（HTTP / MySQL / PostgreSQL / Google Sheets，见
[source](/source#离线回退与-no-cache远程源通用s4)）的离线回退——
默认传输类取数失败会回退上一份缓存副本并发 `E1906` WARNING，
`--no-cache` 下取不到远端即失败（`E1901`，退出码 2），即使有缓存
副本。适合发布前「必须以远端最新字节构建」的核对。

## diff

```bash
cage diff build/a build/b
```

比较两个配置版本。

## snapshot

```bash
cage snapshot <project> --profile client    # 构建 + 打包自校验快照
cage snapshot <snapshot-dir> --verify       # 载入前校验（服务器入口）
```

打包[Configuration Snapshot](/build#configuration-snapshot)：把 profile
构建产物打成可独立加载、自带 blake3 账本的目录
`<output_dir>/snapshot/<profile>-<build_id[..12]>`（manifest.json /
schema.json / data/ / generated/ / HASHES.json），构建后自校验。`--verify` 对既有快照
目录载入前校验：逐文件重哈希比对，篡改/增删文件逐条列出并以退出码 1
失败。`--profile` 默认 `client`。

## verify（规划中）

```bash
cage verify runtime/
```

验证已经生成的 Artifact。尚未实装，已生成产物的校验由
[`cage snapshot <dir> --verify`](#snapshot) 承担。

## graph（规划中）

```bash
cage graph
```

输出[配置依赖图](/build#配置依赖图)。尚未实装独立命令，依赖图
（环检测 / 拓扑序）由 `cage build --incremental` 消费，见
[增量构建](/build#增量构建)。

## Remote Source（远程源）

Cage 支持四种远程输入源，全部在 `cage.toml` 的 `[source_roots]` 以统一语法声明，凭据只存环境变量名（永不进 `cage.toml`、不落日志）：

| 语法 | crate | 关键特性 |
| --- | --- | --- |
| `https://...` / `http://...` | `cage-source-http` | GET 响应字节缓存后走标准 JSON 解析；`E1901` 取数失败、`E1902` 401/403、`E0001` 坏 JSON |
| `mysql:<表|具名查询>` | `cage-source-db` | SELECT 白名单（单条、SELECT 开头，拒分号/注释/行锁/CTE/`INTO`）→ `dsn_env` 解析 → 会话钉只读（`SESSION TRANSACTION READ ONLY`）；DECIMAL 文本保真、`E1905` 白名单、`E1904` 凭据、`E1901` 连接/语句 |
| `pg:<表|具名查询>` | `cage-source-db` | 同上；PG 侧只读用 `default_transaction_read_only` |
| `gsheet:<spreadsheet_id>/<tab>` | `cage-source-sheets` | Sheets API v4 `values` + `UNFORMATTED_VALUE`；首行表头、空行跳过、短行补 null；形状门 `E1903`（非行集/空表头/majorDimension 非 ROWS） |

三源共享缓存布局 `.cage-cache/source/<指纹>/`：远端字节先落盘，再走标准 JSON 解析与 L0-L7 全量校验，**无旁路**。详见 [Source](/source)。

### 离线回退与 `--no-cache`

见上文 [`--no-cache` 小节](#no-cache远程源离线回退开关)。

## web

```bash
cage web ./ --port 8765
```

启动 Schema 编辑器本地服务（第三阶段），只绑定 `127.0.0.1`、无需鉴权。
浏览器打开打印的地址即是编辑器单页（编译期嵌入二进制），与 HTTP API
共用一份交换文档，API 契约见 [Web UI](/web)：

```text
GET  /api/schema      → 合并 Schema 的编辑器文档（每请求重载，保存后立即可见）
POST /api/validate    → 编辑态校验（E1701 / E1004 诊断）
POST /api/schema      → 保存回环 canonical YAML（单文件 schema_path 才可写）
```

Ctrl+C 停止服务。`--port` 默认 8765。

## registry

```bash
cage registry publish <project> --registry <dir> [--package name] [--version 1.0.0] [--profile client]
cage registry list    --registry <dir>
cage registry verify  --registry <dir>
cage registry gc      --registry <dir> [--keep 3] [--dry-run]
cage registry remove  <package> <version> --registry <dir> [--dry-run]
```

本地 Configuration Registry（第三阶段 R 系列，[design §29](https://github.com/cuihairu/cage/blob/main/docs/design.md#29-configuration-registry)）。
`publish` 全量构建 → 打包[自校验快照](/build#configuration-snapshot) → 账本
校验通过后入册 `<registry>/<包>/<版本>/`（包默认 `project.name`、版本默认
`project.version`）；同版本同字节重发是幂等 no-op，同版本异字节报
`E1801` 版本冲突：注册表不改写历史。`list` 按确定性序列出包/版本/
build_id/content_hash/文件数。

消费方在 cage.toml 里声明注册表根并引用包作为源根（R1 源解析）：

```toml
[registry]
path = "../registry"          # 相对项目根

[source_roots]
main = "registry:common@1.0.0"   # 省略 @版本 = 最高点分序版本
```

解析在载入前先过条目账本校验（篡改/增删文件 → `E1803` 拒载；包/版本
不存在或未接 `[registry].path` → `E1802`）。条目的 `data/` 按最高保真
格式载入（json > yaml > csv > excel），表名以条目 manifest.json 的
artifact 记录为准。

Schema 侧同样可取自条目（R2）：`schema_path = "registry:common"` 读条目
`schema.json`（发布时打包的 profile 投影 schema）。此时 schema 归发布方
所有，`cage web` 的 POST /api/schema 对这类工程返回 409；改 schema 请在
发布方工程改并重新 publish。

### 远程注册表（R3，只读）

`[registry].path` 也可以是 HTTP(S) 根：解析走网络、发布仍限本地。

```toml
[registry]
path = "https://registry.example.com/config"   # http(s):// 前缀 = 远程根
```

协议为匿名 GET（design §29）：`GET <根>/<包>/index.json`、`GET
<根>/<包>/<版本>/HASHES.json`、`GET <根>/<包>/<版本>/<文件>`；条目发布后
字节不可变。解析流程：取 index → 按 pin 选版本（规则与本地相同）→ 按
账本逐文件下载并逐一校验 blake3（不符 → `E1803`）→ 落项目内缓存
`.cage-cache/registry/<url 指纹>/` → 过 `verify_snapshot` 信任门才交付。
缓存再校验干净则直接复用，首次在线拉取后**离线构建可用**（index 也有
本地副本兜底）；不可达且无缓存 → `E1802`。

远程根只读：`cage registry publish` 与 `cage registry list` 对远程根报错
退出（协议无包枚举资源，发布方在本地注册表发布后用任意静态服务器托管，
或留待未来的上传/同步协议）。鉴权方案留待后续立项。

### 依赖声明与版本区间（R2）

`[dependencies]` 为注册表包声明版本 pin：引用省略 `@版本` 时按 pin 解析
（取满足区间的最高版本）；显式 `@版本` 也必须落在 pin 内，否则 `E1802`。

```toml
[dependencies]
common = ">=1.0, <2.0"   # 区间 AND，取满足的最高版本
items   = "^1.2"         # >=1.2.0, <2.0.0（左起首个非零分量 +1，其后归零）
monsters = "~1.2"        # >=1.2.0, <1.3.0（锁定到次末位给定分量）
stages  = "1.2.3"        # 精确等于（裸版本 = 精确匹配）
```

比较符支持 `=` `>` `>=` `<` `<=`，逗号分隔为 AND；`^` caret 与 `~` tilde
展开为下闭区间；比较按点分数字序逐分量补零（`>=1.2` 不排除 `1.2.0`）。
区间内无已发布版本满足 → `E1802`（错误信息附已发布版本列表）。

### 校验、回滚与清理（R4）

`cage registry verify` 是全册审计：逐包逐条目重过账本校验（字节对
blake3 复核），并交叉核对 index.json 记录与条目账本的
build_id/content_hash 一致（漂移 → `E1803`）；磁盘上有条目目录而
index 无记录（中断的 remove/gc、手工改动）同样报 `E1803`。审计只读，
发现问题逐条列出并以退出码 2 失败。

`cage registry gc` 落实多版本共存的 GC 策略：**滚动窗口**，每包保留
最新 `--keep N`（默认 3，下限 1，任何一次 GC 都给每包留下至少一个版本）
个版本，窗口外旧版删除并从 index 摘除，孤儿目录一并清扫。窗口即回滚
面：消费方 pin 住 `@1.0.0` 这类旧版本时仍能解析构建，gc 后被窗口挤出
的版本则对消费方 `E1802`。收窄窗口前先确认没有消费方还 pin 在将被
移除的版本上。`--dry-run` 报告完全相同的移除清单而不触碰注册表。

条目移除是显式行政操作（`cage registry remove <包> <版本>`），注册表
自身不改写历史（同版本异字节一律 `E1801`）。remove 连版本目录带
index 记录一并删除（`--dry-run` 只校验存在性与名字合法性），包的
index 保留（即便变空）；显式移除后的版本槽位可用同字节快照重新
publish 干净入册，重发不是冲突；除此之外的同版本重发仍按版本冲突拒绝。
verify / gc / remove 同 publish / list 一样只对本地注册表生效，远程根
报 read-only 错误退出。

## 快速开始

```bash
cage check config/                 # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage gen config/ --profile server    # 只生成代码绑定（cs/python/lua/ts/js/cpp/go/java）
cage diff build/a build/b          # 比较两个配置版本
cage inspect Item                  # 查看 Schema 与配置结构
```
