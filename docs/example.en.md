
# Complete Example

The repository permanently hosts an end-to-end example project [`examples/game-config/`](https://github.com/cuihairu/cage/tree/main/examples/game-config) — five tables ("Character Growth + Item + Quest + Shop + Stage"), five source formats (CSV / YAML / JSON / Excel / MsgPack), one schema covering common field types and constraint usage, dual profiles (client 14 targets / server 3 targets), per-environment constraint overrides across four environments (`--env dev/test/staging/prod`), snapshot / diff / migration / registry / stdio LSP diagnostics (`cage lsp`), plus a set of bad data that triggers L7 Game Rules and per-environment required fields.

All commands and outputs on this page are **real execution results** (run from the repository root), not pseudocode.

## One-Command Run

```bash
bash examples/run.sh
```

The script goes from `cage check` to `cage build` to `cage gen` to `cage migrate` to `cage registry` (including the full remote distribution chain: direct push / presigned push / consumer remote-resolve build, plus bad-data E1501 / E1601 / E1001 assertions, `cage inspect` and the `cage lsp` stdio diagnostics session) — all 15 steps in one command, printing the real command at each step; any failing step exits immediately, and CI runs it identically (a broken example turns CI red).

<details>
<summary>examples/run.sh full output (click to expand)</summary>

```text
==> 构建 cage 可执行文件


============================================================
==> [1/15] cage check examples/game-config   —— L0-L6 全量校验（好数据应通过）
============================================================
$ cage check examples/game-config
cage check: OK (5 tables, 0 warnings, level <= semantic)

============================================================
==> [2/15] cage check --level gamerule examples/game-config   —— L7 业务规则（好数据应通过）
============================================================
$ cage check examples/game-config --level gamerule
cage check: OK (5 tables, 0 warnings, level <= gamerule)

============================================================
==> [3/15] 分环境验证 —— --env dev/test/staging/prod 四套放行、未知环境 exit 2
============================================================
$ cage check examples/game-config --env dev
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env test
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env staging
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env prod
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage build examples/game-config --profile client --env dev
cage build: OK (profile 'client', 86 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)
$ cage build examples/game-config --profile client --env test
cage build: OK (profile 'client', 86 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)
$ cage build examples/game-config --profile client --env staging
cage build: OK (profile 'client', 86 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)
$ cage build examples/game-config --profile client --env prod
cage build: OK (profile 'client', 86 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)
  dev|6271d907b39e|b74285f8edb1
  test|fcd7f8107102|a22c71cd3872
  staging|158f898a3913|04a35a5f73f9
  prod|55dae1a8e145|f81e230d62e9
$ cage check examples/game-config --env qa   # 期望退出码 2
error: unknown environment 'qa' (declared: dev, prod, staging, test)

============================================================
==> [4/15] cage build --profile client   —— 验证 + 生成全部 14 个 target（5 张表）
============================================================
$ cage build examples/game-config --profile client
cage build: OK (profile 'client', 86 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)

============================================================
==> [5/15] cage build --profile client --incremental   —— 指纹一致 → 跳过重建
============================================================
$ cage build examples/game-config --profile client --incremental
cage build: up to date (profile 'client', 86 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)

============================================================
==> [6/15] cage snapshot --profile client (+ --env prod) + --verify   —— 基线/环境化快照打包与回验
============================================================
$ cage snapshot examples/game-config --profile client
cage snapshot: OK (profile 'client', 89 files, 86 artifacts, verified, /home/cui/workspaces/cage/examples/game-config/build/snapshot/client-850ac5e6ac32)
$ cage snapshot <dir> --verify
cage snapshot: verified (/home/cui/workspaces/cage/examples/game-config/build/snapshot/client-850ac5e6ac32/ — 88 files checked)
$ cage snapshot examples/game-config --profile client --env prod   —— 环境化出包（目录名带 env 段）
cage snapshot: OK (profile 'client', env 'prod', 89 files, 86 artifacts, verified, /home/cui/workspaces/cage/examples/game-config/build/snapshot/client-prod-55dae1a8e145)
$ cage snapshot <prod-dir> --verify
cage snapshot: verified (/home/cui/workspaces/cage/examples/game-config/build/snapshot/client-prod-55dae1a8e145/ — 88 files checked)

============================================================
==> [7/15] cage gen --profile client   —— 只生成代码绑定与 JSON Schema（含 proto/fbs 定义）
============================================================
$ cage gen examples/game-config --profile client
cage gen: OK (profile 'client', 71 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)

============================================================
==> [8/15] cage diff build build   —— 同一构建前后比对（确定性：全不变）
============================================================
$ cage diff examples/game-config/build examples/game-config/build
cage diff: 0 added, 0 removed, 0 changed, 71 unchanged

============================================================
==> [9/15] cage build --profile server   —— 服务端视图（server-only 字段 internal_note 可见，client 视图剔除）
============================================================
$ cage build examples/game-config --profile server
cage build: OK (profile 'server', 16 artifacts, manifest /home/cui/workspaces/cage/examples/game-config/build/manifest.json)

============================================================
==> [10/15] cage migrate --to 0.2.0   —— 声明式迁移 dry-run（不写盘）
============================================================
$ cage migrate examples/game-config --to 0.2.0
cage migrate: segment 0001_rename_desc.yaml (0.1.0 → 0.2.0)
  rename_field(Item.desc → description): 3 row(s)
  unchanged /home/cui/workspaces/cage/examples/game-config/config/Character.csv
  would write /home/cui/workspaces/cage/examples/game-config/config/Item.yaml (1 table(s))
  skip /home/cui/workspaces/cage/examples/game-config/config/Quest.xlsx (Excel source: report only — 1 table(s), 3 row(s))
  skip /home/cui/workspaces/cage/examples/game-config/config/Shop.msgpack (not a text source)
  would write /home/cui/workspaces/cage/examples/game-config/config/Stage.json (1 table(s))
cage migrate: OK (1 segment(s), 3 row(s) migrated, schema 0.1.0) — dry run, nothing written

============================================================
==> [11/15] cage registry publish + list   —— 版本化自校验资产发布到本地仓库（含环境化出包与 E1801）
============================================================
$ cage registry publish examples/game-config --registry /tmp/tmp.Jn5f7l8eVG
cage registry: published rpg-demo/0.1.0 (profile 'client', 89 files, build_id 850ac5e6ac32, content_hash edf7f24417fe)
$ cage registry publish examples/game-config --env prod --registry /tmp/tmp.Jn5f7l8eVG   # 同版本换环境 → E1801（一版本一包）
error: E1801 registry version conflict: rpg-demo/0.1.0 already published with build_id 850ac5e6ac32005bd1d5220a / content_hash edf7f24417fe87bfd50fec7f5d6fc91c7cc955cc4b6cfe0447c0fc879a7357ea — republish under a new version
$ cage registry publish examples/game-config --env prod --registry /tmp/tmp.Jn5f7l8eVG --version 0.1.0-prod
cage registry: published rpg-demo/0.1.0-prod (profile 'client', env 'prod', 89 files, build_id 55dae1a8e145, content_hash f81e230d62e9)
$ cage registry list --registry /tmp/tmp.Jn5f7l8eVG
cage registry: 1 package(s) in /tmp/tmp.Jn5f7l8eVG
rpg-demo
  0.1.0        build 850ac5e6ac32  content edf7f24417fe  89 files
  0.1.0-prod   build 55dae1a8e145  content f81e230d62e9  89 files

============================================================
==> [12/15] 远程分发全链路   —— push 直推 + presigned 直推 + 消费方远端 resolve 构建
============================================================
$ cage registry publish examples/game-config   # 发布进示例本地注册表（push 源）
cage registry: published rpg-demo/0.1.0 (profile 'client', 89 files, build_id 850ac5e6ac32, content_hash edf7f24417fe)
(本地对象库 A=http://127.0.0.1:32905 直推 / B=http://127.0.0.1:36077 presigned)
$ cage registry push examples/game-config --registry http://127.0.0.1:32905
cage registry: pushed rpg-demo/0.1.0 → http://127.0.0.1:32905 (89 file(s))
cage registry: pushed rpg-demo/0.1.0 → http://127.0.0.1:32905 (89 file(s)) — identical, no-op
$ cage registry push examples/game-config --presign-map /tmp/tmp.sa450hDEAV/presign.json
cage registry: pushed rpg-demo/0.1.0 → presigned targets (/tmp/tmp.sa450hDEAV/presign.json) (89 file(s))
$ cage build /tmp/tmp.9ROByJsNlN/consA --profile client   # 消费方直连远端根构建（consA ← 32905）
cage build: OK (profile 'client', 5 artifacts, manifest /tmp/tmp.9ROByJsNlN/consA/build/manifest.json)
$ cage build /tmp/tmp.9ROByJsNlN/consB --profile client   # 消费方直连远端根构建（consB ← 36077）
cage build: OK (profile 'client', 5 artifacts, manifest /tmp/tmp.9ROByJsNlN/consB/build/manifest.json)
消费方产物与发布方逐字节一致（5 张表 × 2 条分发路）

============================================================
==> [13/15] 坏数据诊断   —— L6 E1501 违反 hp<=attack + E1601 越过 power curve + --env prod E1001 缺昵称
============================================================
$ cage check examples/game-config/bad   # 期望 E1501（hp > attack，L6 语义规则）+ 退出码 1
ERROR E1501 — Assertion Failed
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 1
  Table: Character
  Row: 0
  Message: Semantic rule violation
  Hint: hp must not exceed attack


cage check: FAILED (1 errors, 0 warnings)
$ cage check examples/game-config/bad --level gamerule   # 期望 E1601 + 退出码 1
ERROR E1501 — Assertion Failed
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 1
  Table: Character
  Row: 0
  Message: Semantic rule violation
  Hint: hp must not exceed attack

ERROR E1601 — Game Rule Validation Failed
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 1
  Table: Character
  Row: 0
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)


cage check: FAILED (2 errors, 0 warnings)
$ cage check examples/game-config/bad --env prod   # prod 收紧 nickname 必填 → E1001
ERROR E1001 — Missing Required Field
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 1 | Field: nickname
  Table: Character
  Row: 0
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 2 | Field: nickname
  Table: Character
  Row: 1
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 3 | Field: nickname
  Table: Character
  Row: 2
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 4 | Field: nickname
  Table: Character
  Row: 3
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1501 — Assertion Failed
  Source: /home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 1
  Table: Character
  Row: 0
  Message: Semantic rule violation
  Hint: hp must not exceed attack


cage check: FAILED (5 errors, 0 warnings)

============================================================
==> [14/15] cage inspect examples/game-config   —— 查看表结构
============================================================
$ cage inspect examples/game-config
project: rpg-demo
version: 0.1.0
tables:
  Character (4 rows)
  Item (5 rows)
  Quest (3 rows)
  Shop (3 rows)
  Stage (3 rows)
schemas:
  Character (10 fields, primary key: id)
  Item (7 fields, primary key: id)
  Quest (4 fields, primary key: id)
  Shop (3 fields, primary key: id)
  Stage (10 fields, primary key: id)

============================================================
==> [15/15] cage lsp   —— stdio 诊断 Language Server（编辑器传输层，手写 JSON-RPC 帧协议）
============================================================
$ cage lsp examples/game-config   # stdin: initialize → didOpen → shutdown → exit
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "capabilities": {
      "textDocumentSync": {
        "openClose": true,
        "change": 1
      }
    },
    "serverInfo": {
      "name": "cage-lsp",
      "version": "0.1.0"
    }
  }
}
{
  "jsonrpc": "2.0",
  "method": "textDocument/publishDiagnostics",
  "params": {
    "uri": "file:///editor/schemas/item.yaml",
    "diagnostics": []
  }
}
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": null
}
$ cage lsp examples/game-config/bad   # 坏工程 → 面板带 E1501（与 cage check 同一管线）
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "capabilities": {
      "textDocumentSync": {
        "openClose": true,
        "change": 1
      }
    },
    "serverInfo": {
      "name": "cage-lsp",
      "version": "0.1.0"
    }
  }
}
{
  "jsonrpc": "2.0",
  "method": "textDocument/publishDiagnostics",
  "params": {
    "uri": "file:///editor/bad.schema.yaml",
    "diagnostics": [
      {
        "range": {
          "start": {
            "line": 0,
            "character": 0
          },
          "end": {
            "line": 0,
            "character": 0
          }
        },
        "severity": 1,
        "code": "E1501",
        "source": "cage",
        "message": "E1501: Semantic rule violation (/home/cui/workspaces/cage/examples/game-config/bad/config/Character.csv | Format: csv | Row: 1) [hint: hp must not exceed attack]"
      }
    ]
  }
}
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": null
}

全部通过。
产物目录：examples/game-config/build/{client,server}/{json,csv,msgpack,proto,fbs,cs,py,lua,ts,js,cpp,go,java,jsonschema}
快照目录：examples/game-config/build/snapshot/
```

</details>

## Project Structure

```text
examples/game-config/
├── cage.toml          # 工程配置：client（13 target）/ server（3 target）双 profile
├── schemas/           # 五个 schema 文件（按名序合并加载，metadata 取自 character.yaml）
│   ├── character.yaml # 角色成长：枚举/唯一约束/正则/范围/默认值/保留字字段 class
│   │                  #   + nickname：env_overrides 四套环境（dev 32 → test 24 → staging 20、prod 必填）
│   │                  #   + internal_note：targets ["server"]（字段级可见性）
│   ├── item.yaml      # 道具：整型枚举 + 字符串枚举 + 可选字段
│   ├── quest.yaml     # 任务：Excel 源、跨表引用、order_by 行排序
│   ├── shop.yaml      # 商店：MsgPack 源、env_overrides 四套环境（折扣上限 0.9/0.7/0.6/0.5）
│   └── stage.yaml     # 关卡：数组/对象/跨表引用/Map<string, Array<Int32>>
├── config/            # 五种源格式（同一 profile 一起加载）
│   ├── Character.csv  # CSV：文件名即表名
│   ├── Item.yaml      # YAML：根键 = 表名
│   ├── Quest.xlsx     # Excel：工作表名即表名（源适配自动发现）
│   ├── Shop.msgpack   # MsgPack：裸行对象数组，文件名即表名
│   └── Stage.json     # JSON：根键 = 表名
├── migrations/        # 声明式数据迁移规则（cage migrate dry-run）
│   └── 0001_rename_desc.yaml   # Item.desc → description（from 0.1.0 → 0.2.0）
├── ci/                # CI 参考片段（Jenkinsfile / github-actions.yml / gitlab-ci.yml）
└── bad/               # 坏数据：完整复制品，attack=500 + 全行无 nickname
    ├── cage.toml      # schema_path = "../schemas"（共用同一份 schema）
    └── config/
```

## Command by Command

### cage check — validate without generating

```console
$ cage check examples/game-config
cage check: OK (5 tables, 0 warnings, level <= semantic)
```

`--level` runs the validation pipeline in tiers, with `gamerule` adding the L7 business rules:

```console
$ cage check examples/game-config --level gamerule
cage check: OK (5 tables, 0 warnings, level <= gamerule)
```

### cage check --env — per-environment constraint overrides

Environments are declared by the schema's `env_overrides` key (design §48); this example declares four — dev for development / test / staging / prod for production. The same field takes four different values: `Character.nickname`'s `max_length` is dev 32 → test 24 → staging 20, with prod switching it to required; `Shop.discount`'s cap is dev 0.9 → test 0.7 → staging 0.6 → prod 0.5 (the sample data's maximum discount is 0.4, so all four pass). `--env` resolves the overrides into the schema before validation; validation, generation, and hashing all operate on the resolved form:

```console
$ cage check examples/game-config --env dev
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env test
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env staging
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env prod
cage check: OK (5 tables, 0 warnings, level <= semantic)
$ cage check examples/game-config --env qa
error: unknown environment 'qa' (declared: dev, prod, staging, test)
$ echo $?
2
```

Build each of the four once: the manifest's `environment` field names the current environment, the resolved schema feeds `schema_hash`, and the constraints land in the generated JSON Schema (`Shop.schema.json`'s `maximum` takes 0.9/0.7/0.6/0.5 by environment), while `build_id` / `content_hash` differ per environment — switching environments means a different build (data bytes unchanged):

```console
$ for env in dev test staging prod; do
>   cage build examples/game-config --profile client --env $env > /dev/null
>   python3 -c "import json; m=json.load(open('examples/game-config/build/manifest.json')); print(m['environment']+'|'+m['build_id'][:12]+'|'+m['content_hash'][:12])"
> done
dev|6271d907b39e|b74285f8edb1
test|fcd7f8107102|a22c71cd3872
staging|158f898a3913|04a35a5f73f9
prod|55dae1a8e145|f81e230d62e9
```

An unknown environment is a usage error (exit 2), not a new E-code; the environment name goes into the manifest's `environment` field and participates in the incremental fingerprint.

### cage build — validation + full build

```console
$ cage build examples/game-config --profile client
cage build: OK (profile 'client', 86 artifacts, manifest examples/game-config/build/manifest.json)
```

86 artifacts = 15 data files (5 tables × json/csv/msgpack) + 6 proto files (5 tables + shared `cage_enums.proto`) + 6 fbs files (5 tables + shared `cage_enums.fbs`) + 54 code files (8 languages × 5 tables each + 1 enum unit, with the JS form additionally paired with a `.d.ts`) + 5 JSON Schema files (5 tables × `{table}.schema.json`); the manifest lives at `build/manifest.json` and is not counted in the `client/` totals:

```text
build/client/
├── json/    Character.json  Item.json  Quest.json  Shop.json  Stage.json
├── csv/     Character.csv   Item.csv   Quest.csv   Shop.csv   Stage.csv
├── msgpack/ Character.msgpack  Item.msgpack  Quest.msgpack  Shop.msgpack  Stage.msgpack
├── proto/   Character.proto Item.proto Quest.proto Shop.proto Stage.proto cage_enums.proto
├── fbs/     Character.fbs   Item.fbs   Quest.fbs   Shop.fbs   Stage.fbs   cage_enums.fbs
├── cs/      Character.cs    Item.cs    Quest.cs    Shop.cs    Stage.cs    CageEnums.cs
├── py/      Character.py    Item.py    Quest.py    Shop.py    Stage.py    cage_enums.py
├── lua/     Character.lua   Item.lua   Quest.lua   Shop.lua   Stage.lua   cage_enums.lua
├── ts/      Character.ts    Item.ts    Quest.ts    Shop.ts    Stage.ts    cage_enums.ts
├── js/      Character.js/.d.ts  Item.js/.d.ts  Quest.js/.d.ts  Shop.js/.d.ts  Stage.js/.d.ts  cage_enums.js/.d.ts
├── cpp/     Character.h     Item.h     Quest.h     Shop.h     Stage.h     cage_enums.h
├── go/      Character.go    Item.go    Quest.go    Shop.go    Stage.go    cage_enums.go
├── java/    Character.java  Item.java  Quest.java  Shop.java  Stage.java  CageEnums.java
├── jsonschema/ Character.schema.json  Item.schema.json  Quest.schema.json  Shop.schema.json  Stage.schema.json
└── manifest.json
```

Determinism spot check: after a full rebuild, msgpack / proto / fbs are byte-identical (same sha256).

### cage build --incremental — skip rebuild when fingerprints match

```console
$ cage build examples/game-config --profile client --incremental
cage build: up to date (profile 'client', 86 artifacts, manifest examples/game-config/build/manifest.json)
```

Dual-layer guards — L1 (schema + source fingerprints) and L2 (artifact fingerprints); `environment` participates in the fingerprint, so changing `--env` triggers a rebuild.

### cage snapshot — self-verifying snapshot packaging and re-verification

```console
$ cage snapshot examples/game-config --profile client
cage snapshot: OK (profile 'client', 89 files, 86 artifacts, verified, examples/game-config/build/snapshot/client-850ac5e6ac32)
$ cage snapshot examples/game-config/build/snapshot/client-850ac5e6ac32/ --verify
cage snapshot: verified (examples/game-config/build/snapshot/client-850ac5e6ac32/ — 88 files checked)
$ cage snapshot examples/game-config --profile client --env prod
cage snapshot: OK (profile 'client', env 'prod', 89 files, 86 artifacts, verified, examples/game-config/build/snapshot/client-prod-55dae1a8e145)
```

A snapshot = the profile-projected canonical schema (`schema.json`) + all artifacts + a per-file hash ledger (`HASHES.json`) — self-verifying, rollback-ready, and auditable. `--env` produces an environment-scoped package: `env_overrides` are resolved into the schema before validation and packaging, the directory name carries an env segment (`client-prod-<build_id>`), the `manifest.json` inside records `environment`, and `schema.json` is exactly the prod rules.

### cage gen — generate only code bindings

```console
$ cage gen examples/game-config --profile client
cage gen: OK (profile 'client', 71 artifacts, manifest examples/game-config/build/manifest.json)
```

It skips the json/csv/msgpack data targets; the code artifacts (including proto/fbs definitions) and the JSON Schema documents are exactly the same as with build.

### cage diff — determinism comparison

```console
$ cage diff examples/game-config/build examples/game-config/build
cage diff: 0 added, 0 removed, 0 changed, 65 unchanged
```

Comparing the same build with itself shows no changes at all — determinism is the precondition for trustworthy artifacts.

### cage build --profile server — server view

```console
$ cage build examples/game-config --profile server
cage build: OK (profile 'server', 16 artifacts, manifest examples/game-config/build/manifest.json)
```

`Character.internal_note` declares `targets: ["server"]`: all 4 rows in the server view carry the field, while it never appears in the client view (field-level visibility, E9006 ordering-safe).

### cage migrate — declarative migration dry-run

```console
$ cage migrate examples/game-config --to 0.2.0
cage migrate: segment 0001_rename_desc.yaml (0.1.0 → 0.2.0)
  rename_field(Item.desc → description): 3 row(s)
  unchanged examples/game-config/config/Character.csv
  would write examples/game-config/config/Item.yaml (1 table(s))
  skip examples/game-config/config/Quest.xlsx (Excel source: report only — 1 table(s), 3 row(s))
  skip examples/game-config/config/Shop.msgpack (not a text source)
  would write examples/game-config/config/Stage.json (1 table(s))
cage migrate: OK (1 segment(s), 3 row(s) migrated, schema 0.1.0) — dry run, nothing written
```

Migration rules are declarative segments on the version chain; dry-run only reports and writes nothing to disk, and Excel sources are report-only (binary sources do not take part in text migrations).

### cage registry — versioned publishing of self-verifying artifacts

```console
$ cage registry publish examples/game-config --registry /tmp/reg
cage registry: published rpg-demo/0.1.0 (profile 'client', 89 files, build_id 850ac5e6ac32, content_hash edf7f24417fe)
$ cage registry publish examples/game-config --env prod --registry /tmp/reg   # 同版本换环境 → E1801（一版本一包）
error: E1801 registry version conflict: rpg-demo/0.1.0 already published with build_id 850ac5e6ac32005bd1d5220a / content_hash edf7f24417fe87bfd50fec7f5d6fc91c7cc955cc4b6cfe0447c0fc879a7357ea — republish under a new version
$ cage registry publish examples/game-config --env prod --registry /tmp/reg --version 0.1.0-prod
cage registry: published rpg-demo/0.1.0-prod (profile 'client', env 'prod', 89 files, build_id 55dae1a8e145, content_hash f81e230d62e9)
$ cage registry list --registry /tmp/reg
cage registry: 1 package(s) in /tmp/reg
rpg-demo
  0.1.0        build 850ac5e6ac32  content edf7f24417fe  89 files
  0.1.0-prod   build 55dae1a8e145  content f81e230d62e9  89 files
```

The snapshot is registered as the `rpg-demo/0.1.0` entry: self-verifying, addressable, and reclaimable by rolling. `publish --env` produces an environment-scoped package: one version, one package — the same version under a different environment is a different package (schema/manifest both change), E1801 blocks the conflict, and switching to a different version number (`0.1.0-prod`) gets it registered; the `manifest.json` inside the entry records `environment`. For managing schemas as artifacts and consumer-side loading, see the [Assets page](/en/assets).

### cage registry push — remote distribution and consumer integration

The example project declares `[registry] path = "reg"` (the artifact directory, not under version control); the local registry is both the publish target and the push source. Push a registered entry straight to a remote (design §47):

```console
$ cage registry publish examples/game-config                       # 发布进示例本地注册表（push 源）
cage registry: published rpg-demo/0.1.0 (profile 'client', 89 files, build_id 850ac5e6ac32, content_hash edf7f24417fe)
$ cage registry push examples/game-config --registry http://127.0.0.1:8080   # 直推：匿名探针 + 逐文件 PUT + index 合并收尾
cage registry: pushed rpg-demo/0.1.0 → http://127.0.0.1:8080 (89 file(s))
$ cage registry push examples/game-config --registry http://127.0.0.1:8080   # 重推同字节 → 幂等零 PUT
cage registry: pushed rpg-demo/0.1.0 → http://127.0.0.1:8080 (89 file(s)) — identical, no-op
$ cage registry push examples/game-config --presign-map presign.json          # 对象存储直推：URL 表即凭据（无 Bearer）
cage registry: pushed rpg-demo/0.1.0 → presigned targets (presign.json) (89 file(s))
```

The consumer (another project) pulls no source files and builds directly against the remote root — `schema_path` and `source_roots` both point at registry entries, and at build time it first passes the ledger trust gate, then runs the full pipeline:

```toml
# 消费方 cage.toml
schema_path = "registry:rpg-demo"

[source_roots]
main = "registry:rpg-demo@0.1.0"

[registry]
path = "http://127.0.0.1:8080"
```

```console
$ cage build ./consumer --profile client
cage build: OK (profile 'client', 5 artifacts, manifest ./consumer/build/manifest.json)
```

The remote entry is the very same bytes the publisher built, and the consumer's artifacts are **byte-identical** to the publisher's — `run.sh` step 12 asserts exactly this (5 tables × direct-push / presigned, two distribution paths), using a local python3 object-store stand-in to push one copy down each path and start one consumer build for each.

### cage check (bad data) — E1601 row-level diagnostics + per-environment E1001

`bad/` differs from the good data in two ways: the first row of `Character.csv` has `attack` changed from 120 to 500, and no row carries a `nickname` column. The built-in power_curve rule requires `attack <= level * 100 + 50`, a cap of 150 at level=1:

```console
$ cage check examples/game-config/bad --level gamerule
ERROR E1601 — Game Rule Validation Failed
  Source: examples/game-config/bad/config/Character.csv | Row: 1
  Table: Character
  Row: 0
  Message: Game rule violation: power curve
  Hint: power_curve: attack 500 exceeds the level 1 cap 150 (level * 100 + 50)

cage check: FAILED (1 errors, 0 warnings)
$ echo $?
1
```

The same bad data breaks in a different way under the prod environment — once `nickname` is required, all 4 rows are missing it:

```console
$ cage check examples/game-config/bad --env prod
ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 1 | Field: nickname
  Table: Character
  Row: 0
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 2 | Field: nickname
  Table: Character
  Row: 1
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 3 | Field: nickname
  Table: Character
  Row: 2
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

ERROR E1001 — Missing Required Field
  Source: examples/game-config/bad/config/Character.csv | Row: 4 | Field: nickname
  Table: Character
  Row: 3
  Field: nickname
  Message: Missing required field
  Hint: Add required field 'nickname' to this row

cage check: FAILED (4 errors, 0 warnings)
```

Diagnostics pinpoint file/line/table/row, and the hint shows the computation; exit code 1 plugs straight into CI.

### cage inspect — view table structure

```console
$ cage inspect examples/game-config
project: rpg-demo
version: 0.1.0
tables:
  Character (4 rows)
  Item (5 rows)
  Quest (3 rows)
  Shop (3 rows)
  Stage (3 rows)
schemas:
  Character (10 fields, primary key: id)
  Item (7 fields, primary key: id)
  Quest (4 fields, primary key: id)
  Shop (3 fields, primary key: id)
  Stage (9 fields, primary key: id)
```

## Schema Highlights (usage covered by this example)

| Usage | Location |
| --- | --- |
| Heterogeneous sources: csv / yaml / json / xlsx / msgpack, five source formats | `config/` |
| Integer enums (with integer values) | `CharacterClass`, `ItemType` |
| String enums (no values) | `Rarity` |
| Valueless enums (member name doubles as the fallback value) | `Difficulty` |
| Numeric ranges min/max, string length and pattern | `Character.level/name` |
| Composite unique constraint | `Character`'s `name_unique` |
| Cross-table references (L5 validation) | `Stage.boss_item_id`, `Quest.reward_item_id`, `Shop.item_id → Item.id` |
| Optional fields and schema defaults | `Item.attack_bonus/desc`, `Character.unlocked` |
| Optional object group (absent in some rows) | `Stage.boss` |
| Array field min_items/max_items | `Stage.tags` |
| Map<K,V> (nesting included; an empty map is legal) | `Stage.drop_table: map<string, Array<Int32>>` |
| Row ordering constraint (E1304) | `Quest.order_by: [id]` |
| Per-environment constraint overrides (`--env`, four environments) | `Character.nickname` (max_length dev 32 → test 24 → staging 20, prod required), `Shop.discount` (cap dev 0.9 → test 0.7 → staging 0.6 → prod 0.5) |
| Environment-scoped packaging (snapshot / registry publish `--env`) | env segment in the directory name `client-prod-*`, the `0.1.0-prod` entry, E1801 on same version + different environment |
| Field-level visibility (targets + E9006-safe) | `Character.internal_note`, server view only |
| Automatic escaping of reserved-word fields (`class`) | `Character.class` → cs `@class`, py/java `class_`, go `Class`; lua/ts/js are not escaped (consumers read values by key name) |
| Standard JSON Schema document (draft-07, enums inlined, closed objects) | `build/client/jsonschema/` |

Schema syntax for Map fields (`schemas/stage.yaml`):

```yaml
drop_table:
  name: drop_table
  type:
    kind: Map
    value:
      key_type: string            # string | int（int 键在数据里是数字字符串）
      value_type:
        kind: Array               # 值类型任意，可嵌套
        value: { kind: Int32 }
  required: true
```

Corresponding data (`config/Stage.json`):

```json
"drop_table": { "common": [1001, 2001], "rare": [1002], "epic": [1002, 2002] }
```

Schema syntax for per-environment overrides (`schemas/character.yaml` / `schemas/shop.yaml`):

```yaml
env_overrides:
  dev:
    nickname: { max_length: 32 }   # 放宽
  prod:
    nickname: { required: true }    # 收紧成必填
```

## Generated Artifact Samples

TypeScript (`build/client/ts/Stage.ts`, excerpt):

```ts
export interface Stage {
  /** 关卡 Boss（可选对象组：部分行缺省） */
  boss?: Record<string, unknown>;
  /** 稀有度 → 掉落道具 id 列表（map<string, Array<Int32>>；空 map 合法）, required */
  drop_table: Map<string, number[]>;
  /** 关卡标签（Array<String>，1-4 个）, required */
  tags: string[];
}
```

TypeScript (`build/client/ts/Quest.ts`, excerpt — Excel source + cross-table references + row ordering):

```ts
export interface Quest {
  /** required, min: 1 */
  id: number;
  /** required, min: 1, max: 100 */
  min_level: number;
  /** required, min_length: 1, max_length: 64 */
  name: string;
  /** 完成奖励道具 —— 引用 Item.id（L5 跨表引用校验）, required, → Item.id */
  reward_item_id: number;
}
```

Go (`build/client/go/Stage.go`, excerpt):

```go
DropTable map[string][]int32 `json:"drop_table"`
```

C# (`build/client/cs/Stage.cs`, excerpt):

```csharp
public Dictionary<string, IReadOnlyList<int>> drop_table { get; init; } = new();
```

Lua (`build/client/lua/Stage.lua`, `M.fields` metadata excerpt — Map and Array are both tables, told apart by type tags):

```lua
-- 稀有度 → 掉落道具 id 列表…, required, 键值表（hash part），与 Array 的数组 table（sequence part）不同
{ name = "drop_table", key = "drop_table", type = "map<string, array<integer>>", required = true },
```

Python (`build/client/py/Item.py`, excerpt — `T | None` for optional fields and enum imports):

```python
from cage_enums import ItemType, Rarity

@dataclass(frozen=True)
class Item:
    id: int
    name: str
    rarity: Rarity
    type: ItemType
    attack_bonus: int | None = None
    desc: str | None = None
```

Data artifact (`build/client/json/Shop.json`, excerpt — MsgPack source table, fields output in schema name order):

```json
[
  { "discount": 0.25, "id": 1, "item_id": 1001 },
  { "discount": 0.4, "id": 2, "item_id": 2001 },
  { "discount": 0.15, "id": 3, "item_id": 3001 }
]
```

Standard JSON Schema (`build/client/jsonschema/Item.schema.json`, excerpt — enum member names inlined, constraints mapped to `minimum`/`maxLength`, consumed directly by non-cage toolchains):

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Item",
  "description": "道具表（YAML 源；attack_bonus/desc 可选 —— 部分行缺省，演示可空字段）",
  "type": "object",
  "properties": {
    "id": {
      "type": "integer",
      "minimum": 1
    },
    "type": {
      "type": "string",
      "enum": [
        "Weapon",
        "Armor",
        "Consumable"
      ]
    },
    "rarity": {
      "type": "string",
      "enum": [
        "common",
        "rare",
        "epic",
        "legendary"
      ]
    },
    "price": {
      "type": "integer",
      "minimum": 0,
      "maximum": 999999
    }
  },
  "required": [
    "id",
    "name",
    "type",
    "rarity",
    "price"
  ],
  "additionalProperties": false
}
```

## 19 Field Types × 8 Languages: Full Mapping Table

Field type mapping for code targets (Map is the 19th type, added in v0.2):

| FieldType | C# | Python | Lua | TS / JS | C++ | Go | Java |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Null | `object?` | `Any` | `any` | `unknown` | `std::any` | `any` | `Object` |
| Bool | `bool` | `bool` | `boolean` | `boolean` | `bool` | `bool` | `boolean` / `Boolean` |
| Int8 | `sbyte` | `int` | `integer` | `number` | `std::int8_t` | `int8` | `byte` / `Byte` |
| Int16 | `short` | `int` | `integer` | `number` | `std::int16_t` | `int16` | `short` / `Short` |
| Int32 | `int` | `int` | `integer` | `number` | `std::int32_t` | `int32` | `int` / `Integer` |
| Int64 | `long` | `int` | `integer` | `number` | `std::int64_t` | `int64` | `long` / `Long` |
| UInt8 | `byte` | `int` | `integer` | `number` | `std::uint8_t` | `uint8` | `short` / `Short` |
| UInt16 | `ushort` | `int` | `integer` | `number` | `std::uint16_t` | `uint16` | `int` / `Integer` |
| UInt32 | `uint` | `int` | `integer` | `number` | `std::uint32_t` | `uint32` | `long` / `Long` |
| UInt64 | `ulong` | `int` | `integer` | `number` | `std::uint64_t` | `uint64` | `long` / `Long` |
| Float32 | `float` | `float` | `number` | `number` | `float` | `float32` | `float` / `Float` |
| Float64 | `double` | `float` | `number` | `number` | `double` | `float64` | `double` / `Double` |
| String | `string` | `str` | `string` | `string` | `std::string` | `string` | `String` |
| Bytes | `byte[]` | `bytes` | `bytes` | `Uint8Array` | `std::vector<std::uint8_t>` | `[]byte` | `byte[]` |
| Array\<T\> | `IReadOnlyList<T>` | `list[T]` | `array<T>` | `T[]` | `std::vector<T>` | `[]T` | `List<T>` |
| Object | `IReadOnlyDictionary<string, object?>` | `dict[str, Any]` | `table` | `Record<string, unknown>` | `std::map<std::string, std::any>` | `map[string]any` | `Map<String, Object>` |
| **Map\<K,V\>** | `Dictionary<K, V>` | `dict[K, V]` | `table` (type tag `map<K, V>`) | `Map<K, V>` | `std::unordered_map<K, V>` | `map[K]V` | `HashMap<K, V>` |
| Enum | enum name (falls back to `string` when unresolved) | enum name | enum name | enum name | enum name (string bucket `std::string_view`) | enum name | enum name |
| Any | `object?` | `Any` | `any` | `unknown` | `std::any` | `any` | `Object` |

Mapping for Map key types K:

| key_type | C# | Python | Lua | TS / JS | C++ | Go | Java |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `string` | `string` | `str` | `string` | `string` | `std::string` | `string` | `String` |
| `int` | `long` | `int` | `int` | `number` | `std::int64_t` | `int64` | `Long` |

The Java column's `primitive / wrapper` choice depends on field nullability (primitive for required fields, promoted to the wrapper for non-required); C++'s `Map` goes through `std::unordered_map` (distinguished from Object's `std::map`), with `#include <unordered_map>` auto-injected by the generator when needed; in Lua both Map and Array are `table`, and the generated type tags and field comments spell out the hash part / sequence part distinction.

## CI Rot Prevention

`.github/workflows/ci.yml` has a dedicated `example` job: after checkout it runs `bash examples/run.sh`. If any part of the example project breaks — data, schema, or run.sh — CI goes red immediately.

`examples/game-config/ci/` holds three drop-in CI reference snippets, all following "install Rust → `bash examples/run.sh` → archive artifacts":

| File | Platform | Highlights |
| --- | --- | --- |
| `github-actions.yml` | GitHub Actions | `dtolnay/rust-toolchain` installs the toolchain, `upload-artifact` archives `build/` |
| `Jenkinsfile` | Jenkins (declarative) | `rustup` bootstraps the toolchain, `archiveArtifacts` archives |
| `gitlab-ci.yml` | GitLab CI | `rust:1-bookworm` image, `artifacts` for upload |

The three snippets map one-to-one onto `run.sh`'s 15 steps: check (including `--env` and bad-data assertions) → dual-profile builds → incremental → snapshot re-verification → gen → diff → migrate → registry → the full remote distribution chain (direct push / presigned / consumer resolve); a red step anywhere means real rot.

## Assets: Schema Management and Loading

The example's endpoint is not the artifact directory but **distributable, re-verifiable assets**: `cage snapshot` freezes a build (schema + artifacts + hash ledger), and `cage registry publish` packages it into the versioned `rpg-demo/0.1.0` entry. The schema itself is an asset too — source review governs the schema, snapshot hashes govern the build, and the registry catalog governs distribution.

The consumer side (another repository's builds / toolchain / CI) pulls registry entries, not source files: `schema_path = "game:0.1.0/schema.json"` + `[source_roots] game = "registry:0.1.0"` + `[dependencies]` version pins, passing the trust gate at build time before running the full pipeline. The standard JSON Schema form is produced by the `format = "jsonschema"` target (`build/client/jsonschema/`), for consumption by non-cage toolchains.

See the [Assets page](/en/assets) for details.
