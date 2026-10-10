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
cage migrate
```

MVP 落地前四个（`check` / `build` / `inspect` / `diff`），`gen` / `snapshot`
为第二阶段（均已实装），`web` 为第三阶段（Schema 编辑器本地服务），
`registry` 为第三阶段 R 系列（本地 Configuration Registry 发布/列表，
R1–R4 均已实装），A 系列分发（bundle 导出/导入、直推远端，design §47）
已随 A1–A3 实装，M 系列迁移（`cage migrate`，design §46）已随 M1–M3
实装。`cage verify runtime/`（验证已生成产物）与
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
cage check --env prod           # 按 Schema 的 prod 环境约束验证
```

`--level` 取值：parse / schema / type / value / table / reference /
semantic / gamerule（默认 `semantic`，即默认执行到 L6，含 L7 业务规则
需显式 `--level gamerule`）。`--profile` 默认 `client`（check / build /
gen 同）。`--env`（check / build / gen 同）按 Schema `env_overrides`
声明的环境抹约束后验证——未声明的环境名是用法错误（exit 2），详见
[Schema 分环境约束覆盖](/schema#分环境约束覆盖env_overrides设计-48)。

## build

```bash
cage build config/ --profile client
cage build config/ --profile server
cage build config/ --env prod          # 按环境约束验证并生成
cage build config/ --incremental       # 哈希与上次 manifest 一致时跳过重新生成
```

与 check 同口径的 `--level`（默认 semantic）可上调/下调验证层级。

验证并生成目标产物，同时输出 [Build Manifest](/build#build-manifest)。
`--incremental` 依据 manifest 里的 schema/source/target 指纹做三层增量
（校验仍全量执行；schema 变化回退全量，target 配置变更只重生对应
target，详见[增量构建](/build#增量构建)）。

## gen

```bash
cage gen config/ --profile server
cage gen config/ --env prod    # 按环境约束生成
```

只生成代码类产物（[C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java / Protobuf](/target#code-targets-与-data-targets-分离)，
另支持 `template` 用户自定义 Tera 模板 target），
不跑数据校验：代码生成是 Schema 驱动的，类型与元数据全部来自 Schema，
不消费配置行数据（源文件仍需可正常加载解析）。Profile 里的数据类
Target（json/csv/msgpack）会被跳过，需要
数据产物时用 `cage build`。产物同样写入 Build Manifest（与 build 同一口
径），后写者胜。

## inspect

```bash
cage inspect config/ Item     # 项目路径必填；表名可选，省略时列出全部表
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

比较两个配置版本；两个位置参数也接受 manifest.json 文件路径（不限于
构建目录）。

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
| `https://...` / `http://...` | `cage-source-http` | GET 响应字节缓存后走标准 JSON 解析；`Link: rel="next"` 分页沿链合并（页=行数组）；429 带 `Retry-After` ≤30s 退避重试；`E1901` 取数失败 / 分页违规、`E1902` 401/403、`E0001` 坏 JSON |
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
cage registry publish <project> [--registry <dir>] [--package name] [--version 1.0.0] [--profile client]
#                                     └ 可省：缺省回落 cage.toml [registry].path
cage registry list    --registry <dir>
cage registry verify  --registry <dir>
cage registry gc      --registry <dir> [--keep 3] [--dry-run]
cage registry remove  <package> <version> --registry <dir> [--dry-run]
cage registry export  <package>[@<version>] -o <file> --registry <dir> [--compress zstd] [--sign --key-env VAR]
cage registry import  <file> [--verify-sig --key-env VAR] [--dry-run] --registry <dir>
cage registry keygen  -o <file>
cage registry push    <project> [package[@version]] --registry <remote-url> [--auth-env VAR] [--dry-run]
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
格式载入（msgpack > json > yaml > csv > excel；msgpack 浮点/Bytes/
超 i64 整数往返无损），表名以条目 manifest.json 的 artifact 记录为准。

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
退出（协议无包枚举资源）；发布方在本地注册表发布后用任意静态服务器
托管，或用 `cage registry push` 直推远端（见下文 bundle 分发与直推）。

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

### bundle 导出（A 系列，[design §47](https://github.com/cuihairu/cage/blob/main/docs/design.md#47-artifact-distributiona-系列2026-10-拍板)）

```bash
cage registry export common@0.1.0 -o common-0.1.0.tar --registry ../registry
cage registry export common       -o common-latest.tar --registry ../registry   # 省略 @版本 = 最高点分序版本
cage registry export common -o common.tar.zst --compress zstd --registry ../registry   # zstd 压缩容器（固定压缩级别）
```

把已入册条目打成**确定性 tar bundle**：条目全部文件（`data/`、
`generated/`、`manifest.json`、`schema.json`、`HASHES.json` 账本）加上
包 index 摘录（`index.json`，只含导出的那个条目），成员路径为
`<包>/<版本>/<文件>`。同条目必得同字节——成员按名序写入、mtime/uid/gid
归零、固定 0o644 权限位，不随导出机器与时间变化，可直接进对象存储或
差分/审计流程。bundle 自带账本：接收侧（`cage registry import`）入册前
先过 `verify_snapshot` 信任门，未经校验的字节不入册。

`--compress zstd` 把同一份确定性 tar 原样包进单个 zstd 帧
（`zstd::stream::encode_all`，压缩级别钉死为 19、不对用户开放）：同 tar +
同级别 + 同 zstd 库版本 → 同容器字节，两次导出逐字节一致。`--compress`
只收字面量 `zstd`，其他值按参数错误退出 2；不带该参数仍是纯 tar，行为
不变。

`--sign` 在导出落盘后对 bundle **文件字节**做一次 ed25519 签名，把
分离式签名写到 `<bundle>.sig`（JSON：`algorithm`/`public_key`/
`signature`，base64）。签名覆盖精确字节——纯 tar 与 zstd 容器都按落盘
字节签，文件任何一处被改动都无法通过验证。签名密钥只从 `--key-env`
命名的环境变量解析（32 字节 Ed25519 种子的 base64；密钥不进
cage.toml、不进日志）——`--sign` 缺 `--key-env` 是用法错误退出 2，
密钥未设 / 空 / 非 base64 / 长度不对 → `E2106`。配套
`cage registry keygen -o <file>` 生成新密钥对：种子（私密）只写入
`-o` 指定的文件（unix 下 0600 权限）、永不打印，stdout 只出公钥与
export/import 的环境变量用法两行。

导出只读注册表，不重新构建（发布仍是 `cage registry publish`）；包或
版本不存在、条目缺账本、bundle 写不出 → `E2101`。远程根不支持导出
（只读协议无文件枚举），远端消费走 `registry:` 源解析。

### bundle 导入（A 系列）

```bash
cage registry import common-0.1.0.tar --registry ../registry          # 入册
cage registry import common-0.1.0.tar --registry ../registry --dry-run # 只报告不落笔
```

把 bundle 入册到本地注册表：解包进临时暂存区 → 骑乘账本过
`verify_snapshot` 信任门 → index 摘录与账本交叉核对（build_id /
content_hash / 文件数一致）→ 通过后走与 publish 相同的入册路径。
**未经校验的字节永不接触目标注册表**——任何拒绝路径（篡改、缺账本、
成员路径逃逸、摘录与账本漂移）都只碰暂存区，`E2103` 报告问题清单。
同字节重导是幂等 no-op；同版本异字节报 `E1801` 冲突（注册表不改写
历史，分发通道也不例外）。导入后的条目与本地 publish 的条目完全
同质：`registry:` 源解析、`schema_path`、`registry verify` 全部照常。

导入目标是本地根（导入即写入，远程根不收写）；`--dry-run` 跑完整
信任门并报告将入册的条目与文件数，不写任何字节。

`--verify-sig --key-env <VAR>` 在账本信任门**之前**先过签名门：读
`<bundle>.sig` 分离式签名，用消费方自己带来的可信公钥（32 字节
Ed25519 公钥的 base64，从环境变量解析）验证 bundle 文件字节。
**sidecar 里随行的 public_key 不是信任锚**——验证只认 `--key-env`
给的钥匙；签名验不过、sidecar 缺失或畸形、算法名不认 → `E2107`，
bundle 不触碰注册表。blake3 账本保证完整性（字节没被改过），ed25519
签名在其上加**抗抵赖**（能证明哪个密钥产出了这份 bundle）——两层各
管一事，都过才算过。

导入按**文件头**嗅探容器形态，不看扩展名：头四字节是 zstd 帧魔数
（`28 B5 2F FD`）就先解压再解 tar，否则按纯 tar 解析，两种容器在任何
扩展名下导入结果一致。压缩只在容器层，`HASHES.json` 账本哈希的是解压后
的内容，zstd 包裹的 bundle 与纯 tar 形态过同一信任门。声明了 zstd 帧却
解不开的损坏容器按 `E2103` 拒绝，不会当纯 tar 静默解析。

### 直推远端（A 系列）

```bash
# 从项目 [registry].path 声明的本地注册表推送（镜像 publish 的项目路径加载）
cage registry push ./game                  --registry https://registry.example.com/config --auth-env CAGE_TOKEN
cage registry push ./game common@0.1.0     --registry https://registry.example.com/config --auth-env CAGE_TOKEN
cage registry push ./game common           --registry https://registry.example.com/config --dry-run
```

把本地已入册条目直推到 http(s) 注册表根（design §47 A3）：源是项目
cage.toml 里 `[registry].path` 声明的**本地**注册表（未声明 → 报错提示
先配 `[registry]` 并 `publish`）；`--registry` 是**远端**目标，只收
http(s) 根（本地目标是 `cage registry publish` 的领地，混用直接拒绝）。
包名缺省 `project.name`，版本缺省点分序最新。

推送流程：匿名 GET 远端包 index 作状态探针（R3 读协议不变）→ 逐文件
PUT（`Authorization: Bearer $TOKEN` 只随 PUT，探针不带）→ index 与远端
现有条目**合并**后最后整体 PUT（远端历史条目永不改写或删除；条目文件
全部成功才写 index，中断不产生半成品条目引用）。探针结果决定动作：
远端无此包（404）→ 全量上传；同版本同 content_hash → 幂等零 PUT；
同版本异 content_hash → `E1801` 拒推（字节不可变纪律跨分发通道一致）。

鉴权：token 经 `--auth-env` 指定的环境变量名解析，缺省回落项目
`[registry].auth_env` 声明——cage.toml 里**只存变量名**，token 永不进
配置文件、错误文本与日志（E1904 同口径）。env 未设置或为空 → `E2105`
在任何网络触达之前失败。错误映射：传输失败（重试用尽）/ 远端 index
不可读 → `E2101`；探针或 PUT 收到 401/403 → `E2102`（token 不入错误
文本）；405/409 等明确 4xx → `E2104`（服务端无写通道，回退「本地
publish + 静态托管」的部署形态）。`--dry-run` 跑完整本地读取与状态
探针，零 PUT。

## migrate（M 系列）

```bash
# 先改 schema.yaml 到新版，再写 migrations/0001-xxx.yaml（from/to/steps）
cage migrate .                    # 单段 dry-run 预演（默认：只报告，不落盘）
cage migrate . --write            # 单段执行：数据变换 + 落盘本地文本源
cage migrate . --all --write      # 整条链一次跑完
cage migrate . --to 1.2.0 --write # 链前缀：跑到指定目标版本（含）为止
cage migrate . --to latest --write # 整链（latest 解析为链终点，与 --all 同选段）
```

对存量源数据执行声明式迁移（design §46）：`migrations/` 目录按文件名
序构成版本步进链（`0001-…`、`0002-…`，非 `.yaml`/`.yml` 文件忽略），
每段声明 `from`/`to`/`steps`，六类步骤——`rename_field`（保序改名）、
`set_default`（只补缺失或 null）、`remove_field`、`widen_type`（安全
加宽方向表 + 逐行值域）、`remap_values`（未映射值原样通过）、
`rename_table`（表序保持）。执行完对**当前 schema**（L0–L6 全栈）回验。

- **流程**：载入工程 → 逐段 apply（每步报行数，失败即失败整段中止，
  `E2003`；Excel 表的步骤行下附受影响行的单元格级定位——
  `file.xlsx | Sheet: 名 | Row: 行号`，按源序，超 8 行折叠为
  `… +K more row(s)` 一行；JSON/YAML/CSV 源保持文件级报告不加行级
  噪声）→ 回验（`E2004` 附完整诊断，失败**不落盘**）→ 报告每个
  源文件的处置。`migrations/` 目录缺失或空链 → 提示 nothing to migrate
  并以 0 退出；坏规则文件 → `E2001`；规则引用不合法 → `E2002`（库侧
  `validate_spec`，CLI 路径由 apply 的 `E2003` 防线接力——rename 目标
  已被占用、引用的表文档里没有，都在变换时拒绝）。
- **段选择**：默认跑文件名序**首段**（显式逐段推进）；`--all` 整链；
  `--to <ver>` 取链前缀（到目标版本含）；`--to latest` 解析为整链（与
  `--all` 同选段）——链上真有版本名叫 `latest` 的段时按字面版本优先
  （只跑到该段含）。`--all` 与 `--to` 互斥；
  `--to` 给了链上不存在的版本 → 用法错误退出 2。链上每个中间态都要
  自洽满足当前 schema——回验永远对磁盘上那一份 schema 跑。
- **默认 dry-run**：完整跑 apply + 回验并报告「将写哪些文件」，
  一个字节不动；`--write` 才落盘。
- **落盘边界**：只写项目本地 source roots 内的 JSON/YAML/CSV（按表
  的 `source_file` 分组原路写回，行字段按 schema 声明序渲染，字节无
  变化跳过——重复 `--write` 是 no-op）；Excel 源**恒只报告不落盘**
  （红线「不做 Excel 编辑器」——按报告手工改，改完重跑 migrate 校验
  收敛）；registry 条目与远程源（registry:/mysql:/pg:/gsheet:/http）
  是只读构建输入，同样只报告。

## migrate-draft（M 系列）

```bash
# 对比两份 schema 版本，起草一份迁移规则稿（write 到 stdout 或 -o 落盘）
cage migrate-draft old-schema.yaml new-schema.yaml --from 1.0.0 --to 2.0.0
cage migrate-draft old.yaml new.yaml --from 1.0.0 --to 2.0.0 -o migrations/0001-draft.yaml
```

从 schema 演进**起草**迁移规则（design §46）：`diff_schemas` 按表名/
字段名对齐两版 schema，机械上安全的变换直接成步——`remove_field`
（旧版有新版无的字段）、`widen_type`（加宽方向表内的换型）、
`set_default`（新增字段带默认值、或 required false→true 且有默认值，
默认值按目标字段的类型族取值——UInt 列渲染为 canonical 带标签形态
`{type: UInt, value: 5}`，避免裸标量回读成 Int 族撞 E1101）；
结构 diff 无法判断意图的留给作者，以 `# TODO` 注释列在文件头——
字段/表改名（rename 是 remove+add 还是 rename 只有作者知道）、
非加宽换型、枚举成员增删、required 无默认值翻转、被删的表。**改名
从不猜**：`rename_field`/`rename_table`/`remap_values` 只会出现在
手工修订里。

- **产物是草稿不是规则**：带 TODO 的稿子可以直接修；无步可提的稿子
  渲染 `steps: []`，`cage migrate` 按 `E2001` 拒绝解析——全部手写完
  再跑。渲染-回读往返（render → `parse_spec`）有单测兜底，`set_default`
  的值一律以字符串安全引用或 canonical 形态呈现，不会出现 `yes`/`5`
  回读翻型。
- **退出**：0 一律成功（含全 TODO 稿，stdout 摘要报 step/TODO 数并
  在空步时给 `steps: [] must be filled` 提示）；1 schema 文件读不了
  或写不出。命令不做 `validate_spec`（与 `cage migrate` 的库路径
  不同）：`set_default` 引用的是 to-schema、`remove_field` 引用的是
  from-schema，单独哪一版都验不全——防线由 apply 时的 `E2003` 与
  回验时的 `E2004` 接力。

## 快速开始

```bash
cage check config/                 # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage gen config/ --profile server    # 只生成代码绑定（cs/python/lua/ts/js/cpp/go/java/proto）
cage diff build/a build/b          # 比较两个配置版本
cage inspect Item                  # 查看 Schema 与配置结构
cage migrate .                     # 迁移预演（dry-run，不改任何文件）
```
