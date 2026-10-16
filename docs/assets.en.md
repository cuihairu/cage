
# Assets: Managing and Loading Schemas

The endpoint of configuration compilation is **assets** — deterministic
artifacts that runtimes, toolchains, and other systems can consume directly.
A schema is not just an input; it is itself an asset. This page spells out
what "assets" covers in cage, how they are managed (versioned, trusted), and
how they are loaded (consumer-side integration).

## The Layers of Assets

| Asset | What it is | Produced by | Consumed by |
| --- | --- | --- | --- |
| Configuration data (json/csv/msgpack) | The row set of every table; deterministic bytes | Build | Runtime loading |
| Code bindings (cs/py/lua/ts/js/cpp/go/java/proto) | Schema-driven strongly typed definitions | `cage gen` | Server-side / client-side projects |
| Schema (`schema.yaml` + each source file's table definitions) | The single source of truth for structure, types, and constraints | Authors | Validation, generation, editors |
| Snapshot (`manifest.json` + `schema.json` + artifacts + `HASHES.json`) | A **self-verifying frozen view** of one build: the profile-projected schema, all artifacts, and a per-file hash ledger | `cage snapshot` | Distribution, rollback, audit |
| Registry entry (`pkg@version`) | A snapshot packaged and versioned — self-verifying, addressable, reclaimable on a rolling window | `cage registry publish` | Consumer-side projects (cross-repo / cross-team) |

## Schema as an Asset: Management

Schema evolution follows an **explicit version chain**, not "edit the file and
that is the new version":

1. **The source is text**: schema files travel with the code repository;
   review, diff, and rollback treat them as ordinary code. `metadata.version`
   is the version it presents to the outside world.
2. **Frozen into a snapshot**: `cage snapshot <project>` writes the
   **canonical schema after the current profile projection** into
   `snapshot/<profile>[-<env>]-<build_id[..12]>/schema.json` (the form the
   build was based on, not the source file itself; with `--env`, the resolved
   form for that environment, and the directory name carries the env
   segment). The snapshot ships its own `HASHES.json` per-file ledger and
   `--verify` re-verification — asset integrity rests on hashes, not
   promises.
3. **Published into the registry**: `cage registry publish` packs the snapshot
   into an entry shaped like `rpg-demo/0.1.0` (a self-verifying bundle) and
   registers it. The registry (R3) is the authoritative catalog of assets:
   `list` finds them, `verify` audits the whole catalog, `gc` applies the
   rolling window, `remove` performs explicit removal and re-publish.

In one sentence: **code review governs schemas, snapshot hashes govern
builds, the registry catalog governs distribution** — three ledgers, each
doing its own job, over one chain of bytes.

## Consumer Side: How to Load

Consumers (the build, toolchain, or CI of another repository) do not pull
source files; they pull **registry entries**:

```toml
# 消费端 cage.toml
schema_path = "game:0.1.0/schema.json"   # 版本化的规范 schema

[source_roots]
game = "registry:0.1.0"                 # 数据条目（自校验快照）

[dependencies]
game = "0.1.0"                            # 版本 pin：构建期解析并校验
```

At build time, `registry:<version>` resolves to the entry directory inside
the registry (a local multi-version repository or an http(s) remote root):
first the `verify_snapshot` trust gate (signatures / hashes), then the full
L0–L7 pipeline runs against the same schema + data — **nothing enters the
registry unverified, nothing gets loaded unverified**, and that holds for
remote bytes too.

Local HTTP API sources (`http(s) URL`) and MySQL/PostgreSQL / Google Sheets
are another "fetch data online" path (design §45), well suited to feeding
system data that has not yet landed in a database into the compilation chain;
the validation bar is exactly the same for both.

## Asset Classification and Backup Strategy (design §50)

By content, the assets of a cage project split into three kinds of "input
assets" plus one kind of "artifact asset":

| Kind | What it is | Where it lives | `cage assets` category |
| --- | --- | --- | --- |
| Rule assets | What the data should look like: validation rules, enums, cross-table constraints, migration chains | `schemas/` + `migrations/` | `rule` |
| Source-data assets | The data itself: the tables designers fill in | The set of files `[source_roots]` points to | `source` |
| Build rules | How to build: profiles / targets / registry | `cage.toml` | `build` |
| Artifacts & traceability | What was compiled at the time, and whether it can be reproduced | `build/`, snapshots, registry entries, bundles | (Artifacts are not inventoried — they can be deterministically rebuilt from the first three kinds) |

Two rulings (design §50): **no separate rules/ directory** — the source of
truth for rules is the schema file set + the migration chain, and splitting
out a directory would mean keeping two ledgers; **artifacts stay out of
version control** — the same inputs must yield the same bytes, and committing
them only creates false diffs (`.gitignore` the `build/` directory).

### `cage assets`: Asset Inventory

```bash
cage assets <project>                    # 文本清单：路径 / 类别 / blake3 / 大小
cage assets <project> --json             # 同形 JSON
cage assets <project> --write assets.json  # 基线随仓入库（提交进 Git）
cage assets <project> --check assets.json  # 复核：缺失 / 篡改 / 多余逐条点名，退出码 1
```

The three kinds of input assets are hashed per file with blake3 and sorted
deterministically by path; remote specs (`registry:` / `gsheet:` / `mysql:` /
`pg:` / `http(s)://`) are not inventoried — they are version pins or remotes
with their own contracts, covered by the manifest's `dependencies` +
`source_hash`. `--check` is the integrity gate before a release / after a
restore.

### The Four Layers of Backup Defense

| Layer | Means | What it protects against |
| --- | --- | --- |
| 1. Git version control | Source assets + the `assets.json` baseline committed with every change | Bad edits can be rolled back; what changed is traceable |
| 2. Off-site remote repository | Push to remote hosting | Local disk failure |
| 3. Release snapshot reproduction | `cage snapshot` + `registry publish` + `registry export` (signed bundle + `.commit` sidecar) | Needing to recover exactly which version of the rules was in use, and what was compiled |
| 4. Independent backup for disaster recovery | The bundle trio sent to cold storage + regular restore drills | The repository host going down entirely — **GitHub is not disaster recovery** |

**Backup ≠ version control ≠ reproducible build**: backup rescues loss,
versioning rescues reverting, reproducibility rescues "why does this differ
from history". Commits do not go into the manifest (a determinism contract:
the same content can be reproduced under different commits — a commit is
history, not content); the release workflow records the `<bundle>.commit`
sidecar instead.

For a complete runnable demo project (inventory → baseline → tamper re-check
→ dual-profile build → snapshot → publish → signed export → clean-directory
recovery drill), see
[`examples/asset-workspace/`](https://github.com/cuihairu/cage/tree/main/examples/asset-workspace);
one command runs the whole thing: `bash examples/asset-workspace.sh`.

## The JSON Schema (Standard) Form

The cage schema is a homegrown DSL (YAML; see the [Schema docs](/en/schema))
— it is the source of truth for rules, not a standard JSON Schema document.
The standard JSON Schema form consumed by non-cage toolchains (editors, ajv,
front-end forms, and the like) is produced by the **`format = "jsonschema"`
target**: one self-contained `{table}.schema.json` per table (draft-07 by
default, 2020-12 optional; enums inlined, closed-shape objects),
schema-driven, deterministic bytes, evolving with the source schema — see
[Target · JSON Schema](/en/target). The `schema.json` inside a snapshot (the
canonical structure projected by the profile) is cage's own form, consumed
as-is by editors / front ends; the two each do their own job: the homegrown
form serves consumption inside the compilation chain, the standard form
serves tools outside it.
