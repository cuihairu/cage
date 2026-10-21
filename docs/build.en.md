
# Build: Deterministic Build

Determinism is a hard requirement of the configuration build system: **the same inputs must produce the same artifacts**.

## Deterministic Build

Same:

```text
Source
Schema
Cage Version
Profile
```

Should produce the same:

```text
Artifact
Hash
Manifest
```

That is:

```text
Build(A) == Build(A)
```

Avoid:

- Timestamps causing file changes
- Non-deterministic Map ordering
- Random IDs
- Unstable sorting

Implementation stance: `IndexMap` for ordering + explicit stable sorting (object keys normalized via `BTreeMap`), byte-level reproducible artifacts (golden test: two builds from the same input are byte-identical), and timestamps and random IDs are forbidden from entering artifacts.

## Build Manifest

Every build produces a manifest (cage-core::manifest, `BuildManifest` / `ArtifactInfo`):

```json
{
  "project": "game",
  "profile": "client",
  "environment": "prod",
  "cage_version": "0.1.0",
  "generator_version": "1.2.0",
  "build_id": "...",
  "schema_hash": "...",
  "source_hash": "...",
  "content_hash": "...",
  "dependencies": { "Monster": ["DropTable"], "DropTable": ["Item"] },
  "table_hashes": { "Item": "...", "DropTable": "...", "Monster": "..." },
  "targets": [
    { "format": "json", "output_dir": "build/client", "hash": "..." }
  ],
  "artifacts": {
    "item.json": { "path": "item.json", "hash": "...", "size": 123, "format": "json", "table": "Item", "encoding": "utf-8" }
  }
}
```

Current fields versus the ledger target (the landing point for the Manifest's gaps in "Compiler Core: The Boundaries of Eight Concepts"):

| Field | Status | Notes |
| --- | --- | --- |
| `project` / `profile` | implemented | build identity |
| `environment` | implemented | the environment the build is in (`--env`, Schema `env_overrides`, design §48); a baseline build (without `--env`) does not write this key. Changed environment → resolved Schema changes → `schema_hash`/`build_id` rotate; the incremental guard compares the environment explicitly, so artifacts are never reused across environments |
| `cage_version` | implemented | cage compiler version |
| `generator_version` | implemented | Manifest structure version ("1.2.0"; bumps when the layout evolves, independent of the cage version) |
| `build_id` | implemented | **deterministic fingerprint**: blake3(profile + schema_hash + source_hash + content_hash), first 24 characters. Same input, same ID (reproducible / matchable / rollback-capable); a semantic change rotates it. No timestamps—they would break the determinism contract "same input → same Manifest bytes" |
| `schema_hash` | implemented | Blake3, covering the full schema |
| `source_hash` | implemented | Blake3, covering all source content |
| `content_hash` | implemented | Blake3, covering all artifact bytes |
| `artifacts` | implemented | per artifact: path / hash / size / format / table / encoding |
| `dependencies` | implemented | cross-table reference ledger: `table -> ordered list of referenced table names` (ManifestGenerator writes the ledger once the L5 dependency graph is wired, D2; older manifests missing the field are tolerated via serde(default)) |
| `table_hashes` | implemented | per-table row-level fingerprint (incremental layer 2 change detection) |
| `targets` | implemented | per-target configuration fingerprint: a list of `TargetRecord`s sorted by (format, output_dir), hash being blake3 (covers format/output_dir/file_template/options, option keys sorted); identity is (format, output_dir) (incremental layer 3 change detection; older manifests missing the field are tolerated via serde(default)) |
| `ir_hash` | omitted as of the IR scope decision | IR is isomorphic to Canonical (see the architecture doc), covered by schema_hash + source_hash |

Uses:

- Version tracking
- Deployment
- Rollback
- Client/server version matching
- CI
- Caching
- Incremental builds

## Configuration Dependency Graph

Cross-table references form a configuration dependency graph (cage-core::reference implements `DependencyGraph`, including cycle detection and topological sorting). For example:

```text
Monster
   |
   | DropTableID
   v
DropTable
   |
   | ItemID
   v
Item
```

Ultimately:

```text
Monster
   |
   +----> DropTable
              |
              +----> Item
```

This dependency graph can be used for:

- Reference checking (L5 validation consumes the reference topology)
- Build ordering
- Incremental builds (layer 2: table-level dependency-graph propagation; layer 3: target-configuration propagation)
- Change impact analysis
- Deletion checks
- Circular dependency detection
- Debugging

Status: the core types and algorithms are implemented and tested (reference/mod.rs: `DependencyGraph` / `IncrementalPlanner`, cycle detection, topology, incremental planning), and the build path is wired up—the reference graph is built from the schema reference declarations via `DependencyGraph::from_schema` (running before and independently of L5 validation), L5 validation and the second incremental layer in the cli consume it together, and the manifest records the `dependencies` reference ledger. Core-to-cli wiring is covered in tests/incremental.rs (including layers 2/3 end to end: change propagation + carry-over + target-configuration add/edit/remove + manifest convergence + older-manifest migration). `cage graph` (implemented 2026-10-12) renders the same graph for humans: edges carry the reference field label, in three forms—text / Graphviz dot / json—plus a topological build order and a cycle list (see CLI).

## Incremental Build

`cage build --incremental` has three layers.

**Layer 1: hash-comparison skip.** At build time the current schema/source/target fingerprints are compared against the values recorded in the last `manifest.json`; when the profile, the environment, and the fingerprints all match and every artifact is present on disk, regeneration is skipped outright (validation still runs in full). Any input change, an environment switch, or a missing artifact drops to layer 2/3 or a full rebuild. An older manifest (without a `targets` record) migrates in one full build, and then takes part in all three layers.

**Layer 2: dependency-graph propagation (implemented in v0.3).** When the schema hash is unchanged (the schema-driven code-target shape; a schema change falls back to a full rebuild wholesale), the previous and current profile and environment are the same, `table_hashes` is non-empty, and no table was deleted, the changed tables are found by table hash, propagated through the dependency graph (`IncrementalPlanner::compute_affected`) into an affected-table set, and only those tables are rebuilt:

- Generation inputs are trimmed to the affected tables (code-target keeps the full enum set: the shared enums unit is regenerated wholesale,
  never carried over);
- Artifacts of unaffected tables that are intact on disk are carried from disk (generator determinism ⇒ carried bytes are byte-identical to a full build),
  artifact order is normalized by sorting on path,
  and the merged manifest is byte-identical to a full build (determinism contract).

**Layer 3: target propagation (implemented in v0.4).** The manifest records a configuration fingerprint per target (`targets`: a list of `TargetRecord`s sorted by (format, output_dir), hash covering format/output_dir/file_template/options). When the schema is unchanged but target configuration changes:

- Targets whose fingerprint changed regenerate only their own artifacts (generation
  input = the full document—a target's bytes depend on every table it serializes);
  unchanged targets keep layer 2's table-level propagation and carry-over;
- A target removed from the profile: its old on-disk artifacts are deleted (only paths recorded in the previous manifest,
  and only after generation succeeds); a moved old `output_dir` works the same way;
  old artifacts left at changed paths by a renamed file_template are cleaned up too;
- Identity is `(format, output_dir)`—when two targets share
  an `output_dir` they are conservatively regenerated together;
- The merged manifest remains byte-identical to a full build (determinism contract).

Conditions for falling back to a full rebuild: the previous manifest has no `table_hashes` (an older manifest), a table was deleted (the previous table set is not a superset of the current one—so stale artifacts are never wrongly carried over), a carried-over artifact fails to read (a warning, then a full rebuild), or the schema changed. A missing carried-over file is not a fallback: a table with a missing file is already treated as changed before the affected set is computed, and is regenerated on purpose along with the affected tables.

For example, editing `Item.xlsx` affects:

```text
Item
 |
 +--> DropTable
 |
 +--> Monster
```

So `Skill / Quest / Map` need not be regenerated:

```text
Changed Sources
      |
      v
table_hashes 比对 → 变更表
      |
      v
Dependency Graph 传播 → 受影响表
      |
      v
只重建受影响表 + 携带其余（字节一致）
```

The `drop-table → full fallback` semantics: the keys of `table_hashes` are every table from the previous build; if the current schema is missing any of them (the deletion scenario), layer 2 falls back to a full rebuild wholesale, so stale artifacts are never wrongly carried over.

## Configuration Snapshot (implemented in v0.3)

Build output is more than loose JSON: `cage snapshot` packs the profile view into a self-contained, self-verifying **Configuration Snapshot** (eight concepts #7, format defined in cage_core::snapshot):

```text
build/snapshot/<profile>[-<env>]-<build_id[..12]>/
├── manifest.json      # 构建账本（profile / hashes / artifacts，与 build 逐字节一致；--env 时记 environment）
├── schema.json        # profile 视图的规范 schema（构建所依据的形态；--env 时即 resolved 形态）
├── data/…             # 数据类产物（json / csv / msgpack）
├── generated/…        # 代码类产物
└── HASHES.json        # 逐文件 blake3 账本（trust root，不自我哈希）+ build_id/content_hash
```

Differences from the planning draft: the directory name uses the **profile + build_id fingerprint** rather than a date—timestamp-based naming would break the determinism contract (same input → same snapshot bytes, same directory name, a rebuild simply overwrites). Data and code artifacts are partitioned by target format, with the `output_dir` prefix stripped from paths but the target subdirectory kept (`build/client/Item.json` → `data/client/Item.json`). `--env` produces an environment-specific package: the directory name carries an env segment (`client-prod-<id>`), the in-package manifest records `environment`, and schema.json is that environment's resolved rules.

Usage:

```text
cage snapshot <project> --profile <p>   # 构建 + 打包 + 自校验
cage snapshot <project> --env prod      # 环境化出包（env_overrides 先解析再打包）
cage snapshot <snapshot-dir> --verify   # 载入前校验（内核入口同服务器）
```

Server startup flow (cage_core::snapshot::{verify_snapshot, load}):

```text
server
   ↓
verify HASHES.json（逐文件重哈希比对，缺/错/多文件均报）
   ↓
load（manifest + schema + data/generated 产物全量读入映射）
   ↓
load configuration
```

On strictness: the per-file ledger comparison checks the full "the file is the ledger and the ledger is the file" integrity; `content_hash` already covers the artifact fingerprint as a whole and is carried in the ledger as a cross-check field. Whole-directory integrity is transitive: tampering with any file, or adding or removing any file, is exposed at verification time.

The snapshot carries its own verification information and does not depend on the build machine's environment. Its division of labor with the runtime-side [Configuration Registry](#configuration-registry) is clear: the former governs the artifact shape, the latter multi-version distribution and rollback.

## Configuration Registry (R1–R4 implemented, design §29)

The registry elevates the independently loadable snapshots above into a **multi-version configuration repository**: one package can hold several versions of a self-verifying snapshot (`<registry>/<package>/<version>/` is one Snapshot, with a per-file blake3 ledger); the package directory carries a deterministic `index.json` (version order / build_id / content_hash / file count); consumers load an entry via `registry:<package>[@<version>]`—nothing enters the registry without verification, and nothing loads without verification.

After the initial R1 (local publish/resolve), the R series kept advancing:

- R2 `[dependencies]` version pinning (comparator ranges / `^` / `~`)
  and `schema_path: registry:` resolution;
- R3 read-only resolution from a remote http(s)
  root (anonymous GET of the three resources + a project-local `.cage-cache` cache, reusable offline);
- R4 whole-registry `verify` audit, rolling-window `gc`, explicit `remove`,
  and republishing the same package (idempotence decided by the whole-package fingerprint `build_id + content_hash`—a changed schema or environment is a different package,
  and republishing the same version reports E1801).

Commands, configuration syntax, and rollback discipline: see CLI: registry; protocol and error codes: see [design §29](https://github.com/cuihairu/cage/blob/main/docs/design.md#29-configuration-registry) and validation.md E180x.

## CI/CD

Cage should be a natural fit for CI:

```text
Git Push
    |
    v
CI
    |
    v
cage check
    |
    +---- Error ---> Build Failed
    |
    v
cage build
    |
    v
Artifacts
    |
    v
Package / Deploy
```

For example:

```bash
cage check --profile client
cage check --profile server

cage build --profile client
cage build --profile server
```

Configuration errors fail in CI first, so they never reach the game. This repository is itself guarded by a CI gate (all tests green) and daily builds; the `warnings_as_errors` policy is covered in Validation: Warning Policy.
