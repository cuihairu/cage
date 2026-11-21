
# Web UI / Schema Editor (Stage 3)

First item of Stage 3: a web-based Schema editing entry point for designers and programmers. The red lines are unchanged: no Excel editor, no game-logic framework. The Web UI is only a front door to the Schema, not a data runtime.

## Positioning

Cage's CLI is the interface of record for the compile chain; the Web UI is **another front door to the same core**: the editor does not parse or render YAML, it only exchanges documents with the core and delegates validation. All editing semantics (the 19 editable field types—core defines 20 kinds, `TaggedUnion` is preserved verbatim with no dedicated editor—constraints, references, targets visibility, expression rules) come from `cage-core`; the front end replicates no rules.

## Architecture Decisions

### Swap model = the existing serde Canonical shape

The editor document is the serde shape of `Schema`—the same document as `schema.json` in a Configuration Snapshot and the HTTP API. One document, consumed in three places:

```text
snapshot/schema.json  ≡  cage-core::edit::to_editor_json  ≡  HTTP GET /api/schema
```

Editor JSON preserves insertion order (tables/fields in document order), omits the optional fields covered by `skip_serializing_if`, and emits `default` explicitly as `null` because it has no skip attribute (the front end can edit either).

### Save = canonical YAML write-back (whole-file / per-source incremental)

The editor works on the **merged canonical Schema** (the CLI merges the multiple files under schemas/ into one `Schema` at load time, and the editor builds on that shape). Write-back is canonical YAML (`to_canonical_yaml`); a `.json` schema file is written back as pretty-printed JSON (its format is honored, never clobbered into YAML): the same Schema → the same bytes (including a single trailing newline), and the round trip through `from_canonical_yaml` is idempotent. A single-file `schema_path` is overwritten whole (atomic replace: sibling temp file + rename). A multi-file (directory) `schema_path` saves incrementally: `GET /api/schema` carries a `sources` index (per-file sha256 revision + table/enum ownership), the save envelope `{"schema": …, "sources": …}` echoes it back — the index is conflict-checked (any file whose on-disk sha256 no longer matches its load-time revision → 409 refused, nothing written), only the files owning a changed table/enum/metadata are rewritten (each atomically), and the response returns the post-save index for the client's next save. `POST /api/schema/preview` is the same planning run without writing: it answers with the unified diff of exactly the files the save would rewrite, so the editor shows the changes before they land. Determinism contract: artifacts produced by `cage build` from editor-saved YAML are byte-identical to those built from the same hand-written Schema. The generator only ever sees `Schema` values, so any value-preserving edit round-trip is sufficient.

### The E1701 editing-state error family

When the editor document cannot be deserialized into `Schema` (syntax / shape / unknown kind), it reports **E1701** with the JSON path (`tables.Item.fields.id.type`) pinpointing the specific input (the front end's highlight anchor); internal schema consistency (a missing primary key, a dangling reference) still goes through L1's `E1004`. The two error classes are kept separate—the editor and the validator each report their own.

### No node front-end toolchain

The front end is a static single page under `docs/public` (vanilla ES modules), served directly by `cage web`. The editor path introduces no Node/npm build chain; cargo does it in one step (the docs site itself is built with VitePress, unrelated to the editor).

## Status

### W1 implemented: editor swap model (cage-core::edit)

- [x] `to_editor_json` / `from_editor_json`: `Schema` ↔ editor JSON;
  a parse failure → `E1701` (located by JSON path, root-level errors normalized to no path)
- [x] `to_canonical_yaml` / `from_canonical_yaml`: deterministic save write-back and read-back
- [x] Tests: editor JSON round-trip determinism,
  canonical YAML idempotence (re-rendering changes no bytes), `E1701` path location (including unknown kind /
  missing required member /
  root-level error / YAML syntax error),
  shape agreement with the snapshot's `schema.json`; `E1701` added to the error code table (crates/cage-core/src/error/codes.rs `editor` family + validation docs)

### W2 implemented: `cage web` local HTTP service (cage-cli/src/web/)

- [x] Project loading reuses the CLI load chain (`load_project_config` / `load_schema`),
  binds `127.0.0.1` only, no authentication (local tool)
- [x] Request boundary and resource caps (noted 2026-10-19): the Host header
  must name a loopback host (`127.0.0.1` / `localhost` / `[::1]`; the port does
  not participate in matching, a missing Host is refused outright), and a POST
  carrying `Origin` must name a loopback origin — violations get 403 (guards
  against DNS rebinding and cross-site writes); request bodies are capped at
  8 MiB (413) with a 30-second wall-clock read budget (408; the vendored
  tiny_http's connection-level 1s idle timeout bounds a single stalled read),
  an unreadable body gets 400
- [x] `GET /api/schema`: the editor document of the merged Schema + project facts (project /
  schema_path / profile_names / warnings_as_errors);
  reloaded per request, so saves are immediately visible
- [x] `POST /api/validate`: editing-state validation. E1701 (document cannot be deserialized,
  located by JSON path) → L1 consistency (the E1004 family);
  diagnostics are emitted as a bare array (the `Diagnostics` serde `{"items": [...]}` envelope is flattened at the API layer)
- [x] `POST /api/schema`: canonical YAML/JSON write-back (`.json` files stay JSON).
  A single-file `schema_path`
  is overwritten in place (atomic temp-file + rename). A directory (multi-file
  schema) is saved incrementally against the `sources` index `GET /api/schema`
  delivers: the envelope `{"schema": …, "sources": …}` echoes the index back,
  every listed file's sha256 must still match its load-time revision (a stale
  editor → 409 refused, nothing written), only the files owning a changed
  table/enum/metadata are rewritten, and the response carries the post-save
  index so the client's next save does not conflict with itself; a raw document
  without the index is still 409. A `schema_path` starting with `registry:`
  is likewise 409 (the schema of a published entry is not edited here—the publishing
  project must republish); nothing configured → writes `schema.yaml` plus a note
  reminding to wire up cage.toml (the server never modifies configuration files)
- [x] `POST /api/schema/preview`: the save dry-run. Same body, refusals and conflict
  check as the save, but nothing is written; the answer lists each file the save would
  rewrite with its unified diff (`files: [{file, action: "write"|"create", diff}]`,
  plus `unchanged`/`bytes`). The editor calls it before saving and shows the diff in a
  confirm dialog, so no write happens before the author has seen it
- [x] `POST /api/schema/rename`: the safe rename (P0-4). Body
  `{"schema": <editor document>, "target": {"kind": "table"|"field"|"enum", …}, "new_name": "…"}`;
  the rename runs in `cage_core::refactor` (Rust Core, not JavaScript) — the one place
  that knows every site a name appears in: the table/field/enum map key **and** the entity's
  own `name` member, primary-key entries, `unique_constraints[].fields`, `order_by`, the
  per-environment override keys, every cross-table `reference.table`/`reference.field`,
  `Enum(name)` field types anywhere in the type tree, and the field operands of rule
  expressions (row asserts of the owning table, plus the `reference.predicate` that filters
  rows of the referenced table). The answer is the renamed document plus the change report
  (`changes: [{path, kind, before, after}]` — each path is the editor's `data-path` anchor,
  and `notes` names a site the rename could not touch, such as a rule assert it could not
  parse, which is left byte-for-byte as written). A name that already exists, a ghost target,
  or a non-identifier new name (`1bad`) is 400 with the reason and the document unchanged;
  an unparseable document is 200 with the E1701 diagnostics. Nothing is written — the editor
  shows the change list, the author confirms, and only then does it adopt the returned
  document into its draft. Document order is preserved (a rename keeps its index, so the
  canonical YAML does not shuffle)
- [x] `POST /api/validate/data`: the draft-schema precheck (P0-3). The edited document
  stands in for the on-disk schema and is loaded against the project's real data through
  the exact `load → validate_with_profile` pipeline `cage check` runs — the editor sees the
  data consequences of a schema nobody has saved yet. The editor-state口径 (E1701 parse +
  the structural `Schema::validate` lint) and the data口径 merge into one diagnostic list;
  every item carries `scope` — `"data"` (the full pipeline over the real data) or
  `"schema"` (pure structural), and a finding both sides report appears once, keeping the
  `data` scope. Knobs mirror `/api/status` (`?profile=`/`?env=`/`?level=`) and default the
  way `cage check` does: the first configured profile, no environment, level `semantic`
  (the fast `POST /api/validate` above stays editor-state only — data loading cannot ride
  the editor's 700 ms auto-revalidate)
- [x] Integration tests (crates/cage-cli/tests/web/): real subprocess + raw TCP requests,
  covering document serving / E1701+E1004 validation /
  reload after save and a `cage check` rerun / directory refusal / starting from an empty schema /
  the save envelope over HTTP (only the changed fragment rewritten, the untouched one
  byte-identical, stale index → 409) / the save preview /
  the draft precheck (a clean draft against the real data, and a draft that breaks both the
  data and the structure — one merged list, one scope badge per side) /
  the safe rename (a field renamed across its primary key, row rule, cross-table reference
  and predicate — every site patched in the returned document, disk byte-identical, a
  collision refused) /
  the P0-6 acceptance loop (open Monster → add a field → rename it → relate Items →
  a range rule → Excel data validation that names the exact violating row → post-fix diff
  preview → incremental save back to the source files; no YAML is hand-written during the
  loop, and the CLI `check` reports the same single violation at the same Excel row the
  Web reported — see `p0_acceptance.rs`)

### W3 implemented: Schema editor front end (docs/public/editor/, compiled in at build time)

- [x] A static single page (vanilla ES module, zero dependencies, no node build chain): three columns—a table/enum tree on the left,
  an entity form in the middle, a diagnostics panel on the right; `cage web` embeds index.html / app.js /
  app.css into the binary at compile time and serves them directly (`docs/public` is the sole authored copy,
  and the docs site publishes it under /editor/), so the single cage binary ships the editor
- [x] Table/enum editing: add/delete, order-preserving rename (the fields/tables/enums keys follow the rename),
  primary key / sort / targets / unique constraints, enum values (name / literal value / description)
- [x] 19 field types: a recursive type editor (Array element /
  Map key_type + value / Object properties / Enum reference datalist);
  switching types rebuilds the payload (Array/Object/Map keep a compatible inner type); a nesting depth limit of 5
- [x] Constraint editing shown per type: numeric min/max, string min_length/max_length/pattern/allowlist,
  array min_items/max_items, default value (a JSON literal), required,
  references (target table/field/cardinality one|many|optional/compatible profile),
  expression rules (name/assert/message/warning_only), custom metadata (flatten extra keys,
  edit as a JSON object, reserved keys refuse to be overwritten)
- [x] An E-code diagnostics panel: the Validate button / Ctrl+Enter → `POST /api/validate`;
  counted and color-coded by severity; clicking a diagnostic → locates the field (`data-path` exact anchor + scroll highlight);
  diagnostics from a refused save (E1701) also go into the panel
- [x] The full precheck: the 完整预检 button / Ctrl+Shift+Enter → `POST /api/validate/data`.
  The merged list badges each finding 「结构」(structural, `scope: "schema"`) or
  「数据」(data-side, `scope: "data"`); clicking a data finding selects its table and
  column, an editor-state one its JSON-path anchor, exactly as before
- [x] Save: Ctrl/Cmd+S or the Save button → `POST /api/schema/preview` first, a
  confirm dialog showing the per-file unified diff (the exact bytes the save would
  write, computed without writing), then on confirm `POST /api/schema` (the sources
  index rides along when `GET /api/schema` delivered one, enabling the multi-file
  incremental save); an unsaved marker + beforeunload interception; 409 (a stale
  index or a `registry:` schema_path)
  / an unwired-up note / 500 all surface as error toasts
- [x] Surgical document editing: unknown/legacy keys are preserved as-is and only controllable keys are changed,
  consistent with the "editor JSON ≡ canonical shape" swap contract;
  saving never renders YAML in the front end
- [x] Integration tests: GET / returns the embedded editor page (including module/style references), /app.js, /app.css serving and 404s;
  the test's raw client adds chunked decoding (tiny_http uses chunked for large responses,
  and a chunk boundary can split a UTF-8 sequence)

### W4 implemented: finalized editing flow + example project smoke test + CI

- [x] `examples/web-demo/`: a single-file schema smoke project (`schema_path`
  pointing at `schema.yaml`; `[source_roots] main = "config"` so the example
  tables can be checked/built immediately). The minimal closed loop for the whole
  "open in the browser → edit → validate → save → `cage build` reproduces" path
- [x] `examples/web-smoke.sh`: an end-to-end smoke script that runs on a temporary copy (so it does not dirty the repository)
  and echoes every real command as it goes; any step failing exits non-zero. Seven steps: a baseline check;
  an editor session (GET the document → add a `stack_size` field (UInt8 /
  min 1 / default 1) → zero diagnostics from `/api/validate` → save via `/api/schema`);
  assert `schema.yaml` landed as canonical YAML containing the new field;
  full-chain `cage check` + `cage build` reproduction; a status-page session (U1,
  the `/status/` page → `/api/status` asserting client fresh → `POST /api/build` asserting up-to-date);
  two rebuilds of the same schema produce byte-identical artifact-tree hashes (determinism contract);
  against `examples/game-config` (the schemas/ directory),
  assert that the save envelope round-trips the `sources` index: a stale
  revision is 409-refused naming the changed file, a field edit rewrites
  exactly `schemas/item.yaml` (the other five files stay byte-identical),
  and the raw document without an index is still refused
- [x] CI integration: the `web-smoke` job mirrors the example job
  (checkout + toolchain + rust-cache + `bash examples/web-smoke.sh`)

### W5 implemented: project status panel (U1, see the roadmap in UI facade)

- [x] `GET /api/status[?profile=&env=&level=]`: one document,
  three blocks—project facts (name / schema_path / output_dir / profile list),
  the validation panel (**sharing the single `validate_project` entry point with `cage check` / `cage lsp`**: counts + full diagnostics + `visible_tables`,
  so the page, the CLI, and the LSP never disagree about diagnostics),
  and per-profile build facts (freshness = the first-layer comparison from `build_project`—same profile,
  same environment, same schema/source/target fingerprints, artifacts intact—**reported,
  not executed**; `changed_tables` = the second layer's per-table `table_hashes` comparison,
  telling the user which tables changed but have not been rebuilt;
  the manifest has a single slot, and `last_build_profile` marks the current owner)
- [x] `POST /api/build`: triggers a build through the same `build_project` pipeline as `cage build` (full validation + three-layer incremental + deterministic manifest). Validation failure → `{"ok": false, "diagnostics": [...]}` (the same shape as `/api/validate`,
  rendered straight into the panel by the page);
  already up to date → `up_to_date: true` with no bytes touched;
  unknown profile /
  I/O → 400 + error
- [x] The `/status/` status page (docs/public/status/, compiled in at build time, vanilla ES module,
  zero dependencies, the same dark theme as the editor): a validation panel (three knobs—level /
  environment / profile) + a build card (a freshness badge, changed-table chips,
  a target list, the first 12 characters of build_id / content_hash, a Build button);
  a bare `/status` 301-redirects to `/status/` (so relative asset references do not collide with the editor's /app.js);
  the editor's top bar gains a "Project status →" cross-link
- [x] Tests: 6 web/tests.rs unit tests (status default profile / unknown profile 400 /
  fresh and up-to-date after a build / `changed_tables` naming after a source change /
  a build on bad data refused with diagnostics / a build for an unknown profile refused)
  + 3 integration tests (status page routing and the 301 / status-build end to end /
  bad data end to end) + web-smoke step 5 (the status page against a real server → fresh → up-to-date)

### W6 implemented: artifact browsing (U2a, roadmap in UI facade)

- [x] `GET /api/artifacts[?profile=&env=]`: the artifact ledger = the build manifest. The manifest has a single slot,
  so **it never reads leftovers across profiles**—when the manifest belongs to another profile it returns `manifest_profile` plus an empty list (a profile that has not been built has no trustworthy artifacts;
  build it and it becomes browsable); `stale` reuses U1's first-layer verdict (artifacts are listed anyway,
  but the page marks them "stale");
  every entry carries path /
  blake3 hash / size / format / owning table /
  encoding, and grouping by `output_dir` in the front end is naturally layered by profile
- [x] `GET /api/artifact?path=REL`: serves a single file for preview/download. Two security rules: ① only manifest keys are accepted (a table lookup,
  no join-and-pray; path traversal is refused too); ② before serving,
  the blake3 hash is recomputed and compared with the recorded one—a mismatch is 409—the same verify-first posture as the snapshot loader,
  and a byte that was tampered with is never sent
- [x] The status page's "Artifacts" section: files listed in groups by each target's `output_dir` (path /
  size / hash prefix / preview / download); previews format JSON, show text as-is, and mark msgpack "binary,
  not pre-rendered"; an unbuilt or cross-profile case shows an explanation rather than an empty shell;
  stale artifacts show a staleness notice
- [x] Tests: 3 web/tests.rs unit tests (the list across profiles and the empty case /
  serving + 404 for a non-ledger path + 409 for tampering / the stale marker)
  + 1 integration test (list → byte-exact serving → 404 → 409 end to end) + artifact assertions added to web-smoke step 5

### W7 implemented: targets badges + profile-scoped view (U2b, roadmap in UI facade)

- [x] `GET /api/schema?profile=NAME`: also returns `projection` (a direct call to `filter_by_profile`—the same projection function as the build pipeline;
  unknown profile 400). Zero semantic duplication in the front end: it only takes the "full − projection" set difference to get the hidden set;
  visibility rules never enter the JS
- [x] Targets badges: whenever `field.targets` / `table.targets` is non-empty, the editor renders a chip (a table entry in the left column /
  a field row / a table title); the data source is the schema itself, with no new state
- [x] Scoped view: a "Scope" knob in the editor's top bar ((all) / each profile);
  tables/fields dropped by the projection are greyed out with a "not visible in this profile" badge rather than disappearing—what is hidden should be visible as hidden;
  the projection is computed from the saved schema,
  and the prompt states this explicitly while there are unsaved edits (saving is entirely author-driven, with no autosave)
- [x] Tests: +2 web/tests.rs unit tests (projection gating tables and fields / unknown profile 400)
  + 1 integration test (no profile → no projection → client drops the server-only field → server keeps it → ghost 400)
  + web-smoke step 7 (game-config's `internal_note`: dropped by the client projection,
  kept by server, ghost 400)

### W8 implemented: snapshots / layered diff / registry browsing (U3, roadmap in UI facade)

- [x] `GET /api/snapshots`: lists every package under `<output_dir>/snapshot/`,
  running `verify_snapshot` on each one right now—the list never claims a package is sound that it did not just verify;
  it also adds the manifest facts (profile / env / build_id / content_hash /
  artifact count; packages whose manifest cannot be read are still listed, marked broken)
- [x] `POST /api/snapshot`: the same pipeline as `cage snapshot` (a full `build_project` → `pack_snapshot`,
  same arguments including the environment);
  a validation failure is 200 + diagnostics (the same merge logic as the build panel); an Io error is 400
- [x] `GET /api/diff?from=&to=`: a semantic, layered diff of two manifests—
      `changed_tables` / `affected_targets` / `files:{added,removed,
      changed}` (each with its table and target owner) / `roots` (content_hash
      and build_id from → to). The table/target metadata rides along with the file entries,
      so the UI's three-level collapsing (table → target → file) is pure grouping; no Merkle-ification,
      and the sticking point (an incremental-sync need for "fetch only the changed subtree") is covered in UI facade
- [x] `GET /api/registry`: browses the packages and versions under the `[registry] path` (the same entry point as cage_core's registry);
  if nothing is configured, it returns ok + an explanation rather than an error
- [x] The status page: three new sections—"Snapshots" (a self-verify badge + a pack button),
  "Compare" (base/target knobs: the live build against each snapshot;
  three-level collapsing, with a verdict when the bytes match exactly), and "Registry"
- [x] CLI alignment: `cage snapshot --list` (lists packages + a self-verify
  verdict for each, mutually exclusive with `--verify`/`--env`); `cage diff`
  gains a layered "tables changed / targets affected" summary from the same
  data source as the status page (manifest `table_hashes` and artifact paths)
- [x] Tests: +3 web/tests.rs unit tests (snapshot packing / list self-verification /
  bad body 400 / diff layering with missing arguments and an unknown base 400 /
  registry not-configured explanation + browsing when configured)
  + `cage snapshot` CLI tests (--list package listing and a tampering verdict, --list conflicting with `--verify`/`--env`)
  + 1 integration test (registry not configured → pack → list → cross-snapshot diff → ghost 400)
  + web-smoke step 5 (pack → self-verify → a zero-difference diff → the empty registry state)

### W9 implemented: data panel (planner interface P0, 2026-10-20, roadmap in UI facade)

- [x] `GET /api/tables[?profile=&env=]`: the in-repo data table list
      (per table rows / source_file / sheet / primary_key; the primary
      key comes from the schema definition — the JSON load path leaves
      `Table.primary_key_fields` empty, so it is not used as the data
      source); an unspecified profile takes the first configured one,
      nothing configured is 500, an unknown profile is 400
- [x] `GET /api/table?name=NAME[&profile=&env=&offset=&limit=]`: a
      paginated row window (limit defaults to 200, capped at 1000,
      offset normalized), each row carrying index / pk / cells by
      column; the total JSON mapping covers the whole value domain
      (bytes become a `{"bytes": n}` marker object instead of failing
      the whole request); a ghost table is 404, a missing name 400
- [x] The status page's "Data" section: a table list on the left
      (rows / source / sheet), a table view on the right (sticky thead
      + pager knobs); the diagnostics panel gains severity / error-code
      filters (a filtered m/n count is shown); a diagnostic carrying a
      data location is clickable → jumps straight to the matching table
      row with a highlight (jumpToDiag: pk match → page change →
      scroll highlight)
- [x] Zero new validation: what the data panel consumes is still the
      same `/api/status` `validate_project` diagnostics (check / lsp /
      web / status page share the single entry point)
- [x] Tests: +2 web/tests.rs unit tests (table list facts and ghost 400 /
      pagination window and hidden-table 404) + 1 integration test
      (table list → row window → ghost 404 → page reference assertion)
      + web-smoke step 5 folds in data panel assertions (/api/tables →
      /api/table → ghost 404)

### W10 implemented: editor immediate re-validation + one-click artifact export (planner interface P1, 2026-10-20)

- [x] An "immediate re-validation" toggle in the editor's top bar
      (default on): every edit schedules a 700ms debounced silent
      re-validation (refreshes only the diagnostics panel and counts,
      no toast); the manual "Validate" / Ctrl+Enter is unchanged and
      cancels any pending auto run. The rule form itself reuses W3
      (zero new editing surface)
- [x] An "Export all" button on the status page's artifact cards (only
      for multi-file targets): triggers `/api/artifact` downloads one by
      one, each file hash-verified (blake3) before serving; decided
      against adding a zip/packaging endpoint (artifact bytes are never
      recomputed, zero new dependencies; bulk distribution rides the
      `cage snapshot` package), record in UI facade
- [x] Tests: integration assertions (the editor page carries the toggle
      and `scheduleAutoValidate`; the status page export rides
      `/api/artifact`) + `node --check` on both page scripts; P1 adds
      zero server surface, web unit/integration counts unchanged

### W11 implemented: diagnostics diff + field template library (planner interface P2, 2026-10-20)

- [x] Zero new endpoints: this whole entry is a front-end increment on
      existing pages, with unchanged validation/product data sources
- [x] Diagnostics diff in the validation panel: what two runs in the
      same session added or removed (summary "vs last: +n / m gone" plus a
      "new" badge on new diagnostics); runs are compared only at the same
      profile+env+level, and diagnostics swapped in by a refused build
      count too — covering the "change data → build → what errors
      appeared" main path. Historical cross-snapshot diagnostics are not
      done (they would have to be written into the snapshot package — a new
      format surface; the product diff already exists as `/api/diff`, see
      W8)
- [x] Field template library in the editor: an "add field from template"
      dropdown (numeric range Int+min/max, enum reference, foreign-key
      reference); a generated draft flows through the existing
      save/validate chain (and composes with W10's auto re-validation),
      zero new server surface
- [x] Tests: integration assertions (`FIELD_TEMPLATES` / foreign-key,
      `markDiagDiff` / `diag-new`) + `node --check` on both page scripts;
      web unit 73 green / integration 15 green unchanged

### W12 implemented: table source upload (planner interface P0, 2026-10-21, roadmap in UI facade)

- [x] `POST /api/upload?name=NAME[&profile=&env=]`: the raw-byte body is
      a whole-file replacement of the table's registered source file.
      Security policy delegated (docs/ui.md upload landing record): writes
      go only to the registered source file — `resolve_upload_target`
      double-guards after canonicalization (inside the project root AND
      inside at least one `source_roots`, symlink escapes refused); bytes
      land via tmp + rename, the original moves aside first
- [x] Validation and rollback: after the swap the same full
      `validate_project` runs (the single-entry discipline holds); errors
      or a load failure restore the original before responding — a refused
      upload never leaves bytes on disk; if a restore itself fails, the
      backup original is kept on disk and the error names its path
      (the UploadSwap guard disarms) instead of silently losing data
- [x] Refusal surface: tables spanning several files are 400 outright (a
      single-file replacement cannot avoid orphaning rows); empty body
      400, ghost 404, missing name 400, outside source_roots 400; size
      rides the existing 8 MiB body cap (non-UTF-8 bodies are accepted
      only by the upload route; the string-shaped routes keep their old
      "cannot read request body" refusal)
- [x] The status page's data section: an "upload replace…" button (shown
      once a table is selected, title carries the source path) → confirm
      → pick a file → raw-byte POST → accept/refused toast → refresh of
      the validation panel and the table view
- [x] Tests: +5 web unit tests (accept and land, no litter / bad data
      refused byte-identical / unparseable refused / ghost and missing
      name / double path guard incl. symlink escape) + 1 integration end
      to end (replace → visible in the panel → refusal rollback → no temp
      litter) + web-smoke step 5 folds in the upload round trip (accepted
      → new row in the panel → bad data refused → original restored)

### W13 implemented: three-tier editor modes (review P1-1, 2026-11-21)

- [x] Three editing modes sharing one Schema semantic model — 基础/规则/专家
      (basic/rules/expert) switch in the editor topbar; all three read and
      write the same `state.doc` and the same `field.rules` (`ExpressionRule`)
      array. The "no third rule format" guarantee is a node-testable pure
      contract in `modes.js` (`MODES` / `isMode` / `applyExpertText` /
      `ruleFormat` / `addRule`), served as its own embedded asset.
- [x] Basic mode = the existing field form (fields, types, defaults, ranges,
      enums, references — no code), unchanged.
- [x] Rules mode = a dedicated rules view editing the same `field.rules`
      arrays (reusing the ExpressionRule editor), with a field picker on the
      table overview; the visual condition builder (P1-2) and the nine
      templates (P1-3) land in this tier next.
- [x] Expert mode = a raw document editor (a JSON textarea over the Schema
      semantic model's serde shape); the editor never parses or renders YAML,
      save still writes canonical YAML. Apply is atomic (`applyExpertText`):
      a failed parse keeps the existing document untouched, structure errors
      surface via 校验 (E1701).
- [x] The mode choice persists in `localStorage` (`cage-editor-mode`); the
      author's last mode is restored on load. Switching back to basic/rules
      clears a now-ghost selection after a whole-document expert rewrite.
- [x] Tests: `modes.test.mjs` (node --test, 10 cases — round-trip, atomic
      reject, the one-rule-format contract, addRule sequencing) + integration
      assertions in the served page and the `/modes.js` asset; `node --check`
      on all edited scripts; web unit 80 green / integration 19 green;
      web-smoke green.

### W14 implemented: structured rule AST + visual condition builder (review P1-2, 2026-11-22)

- [x] `cage_core::rule_ast` — the authoring AST (`Compare{field, op, rhs}` /
      `And{items}` / `Unparseable{assert}`, rhs = literal or same-row field).
      It is an authoring intermediate only: `compile` turns it **down** into
      the Schema's one rule representation (`ExpressionRule` arrays on
      `field.rules` — `And` yields one rule per item, the Schema ANDs a
      field's rules), `decompile` parses an assert back into the builder's
      rows. The Schema never stores the AST, so W13's "no third rule format"
      holds.
- [x] The expressiveness boundary is enforced, not faked: a literal the
      engine cannot render (array/object/null, a string holding both quote
      kinds, a `UInt` beyond `i64`) refuses with `CompileError::Unsupported`
      carrying a human-readable reason — the compile endpoint answers
      `{ok: false, unsupported: true, error}` and the editor surfaces it as
      a boundary notice (P1-5 makes it prominent).
- [x] Web API: `POST /api/rule/compile` (AST + `RuleMeta` → `ExpressionRule[]`)
      and `POST /api/rule/decompile` (`ExpressionRule` → AST, `Unparseable`
      for asserts the engine cannot parse).
- [x] The visual condition builder lives in the editor's 规则模式 (rules
      tier) above the ExpressionRule editor: rows of `field OP literal/field`
      (operators `== != < <= > >=`, cross-field comparison picks another
      field of the same table), rule name / message / warning-only meta,
      write mode replace-all or append. The builder only produces an AST
      (`builderRowsToAst`, pure and node-tested; `parseLiteralInput` — `200`
      is a number, `hi` is a string); the assert text is always rendered by
      Rust Core. 「从既有规则装载」 decompiles the field's existing rules
      back into rows (unparseable ones are skipped with a notice).
- [x] Tests: rule_ast unit tests 8 (compile/decompile round trip, `And`
      naming, unrepresentable literals, unparseable asserts) + web unit 3
      (endpoint shapes incl. the unsupported refusal) + integration 1 (real
      HTTP: two-row `And` → two rules with suffixed names, decompile back,
      unsupported answer) + `modes.test.mjs` 12 green (the two new pure
      functions); workspace 1290 green; web-smoke green.

### W15 implemented: nine built-in rule templates (review P1-3, 2026-11-22)

- [x] `rule_templates.js` — a pure, node-tested catalog of the nine templates
      from the review table (主键唯一 / 数值范围 / 枚举选择 / 外键关系 /
      字段比较 / 条件必填 / 数值成长 / 跨表总量 / 配置可达性), each with
      its 底层实现 and a planner-facing hint.
- [x] `planTemplate` only lands rules on the Schema's existing
      representations — never a new rule format. The five expressible ones
      produce path patches (primary_key, min/max, enum type, reference) or an
      AST (字段比较, compiled by `/api/rule/compile` into the one
      `field.rules` array); the frontend still never writes an assert.
- [x] The four templates the expression engine cannot express (条件必填,
      数值成长, 跨表总量, 配置可达性) do **not** pretend to auto-convert:
      they answer `{kind:"boundary"}` with guidance toward the Python
      extension (P2-2) or a dedicated Rust validator (P3-2), surfaced in the
      rules tier as a prominent notice (P1-5 makes it more prominent).
- [x] The rules tier carries a template library section (pick a template →
      a small parameter form → apply). The parameter forms are built from the
      document (field/enum/table selects), so a template is never misapplied
      to an unknown field.
- [x] Tests: `rule_templates.test.mjs` (node --test, 8 cases — catalog
      id/engine/boundary flags, each patch shape, the boundary refusal, the
      no-self-compare and unknown-field errors) + integration asset
      assertions (the module serves with `RULE_TEMPLATES` / `planTemplate`,
      forms.js wires it) + `node --check` on the edited scripts; web
      integration 20 green; workspace tests green; web-smoke green.

### W16 implemented: rule human layer (review P1-4, 2026-11-22)

- [x] `ExpressionRule` gains the human layer as optional fields — `title`
      (中文标题), `description` (业务说明), `examples` (正反例). The error
      message was already there (`message`). All three skip serialization
      when absent, so an old Schema stays byte-identical (test-pinned), and
      the editor document (`to_editor_json`) carries them straight through —
      no second rule format anywhere.
- [x] 正反例 are executable: `cage_core::rule_examples::check_examples`
      runs each example row through the same expression engine as the
      validation main chain (`L6` semantics: a missing/null field counts as
      a pass, incomparable types as a violation) and answers per example
      期望 vs 实际, with a human note on mismatch. Purely advisory — it
      never touches the Schema or the data.
- [x] Web API: `POST /api/rule/examples` (a rule → one verdict per example:
      `{index, expected, actual, ok, note}`).
- [x] The editor (both the basic-tier field form and the rules tier) edits
      the human layer in the same ExpressionRule editor: 中文标题 /
      业务说明 inputs, a 正反例 sub-editor per rule (sample row JSON,
      expected pass/violate, note) and a 试例 button that posts the rule and
      shows `试算：N/M 例符合期望` with a per-example verdict line.
- [x] The visual builder (P1-2) carries 中文标题 / 业务说明 in its `RuleMeta`
      (compiled onto every produced rule), and a replace-write now merges
      the human layer of same-assert existing rules back in
      (`mergeHumanLayer`, pure and node-tested) — an unrelated recompile
      never drops what the planner wrote. 从既有规则装载 seeds the meta
      from the loaded rule.
- [x] Tests: rule_examples unit tests 8 (engine verdicts incl. mismatch
      notes, missing-field-as-pass, invalid example shapes, unparseable
      asserts, serde round trip, byte-exact absence without the layer) +
      web unit 1 (endpoint shapes) + integration 1 (real HTTP verdicts) +
      modes.test.mjs 15 (the three new mergeHumanLayer cases); workspace
      green; web-smoke green.

## Roadmap

With this, the whole Stage 3 W series and the subsequent UI increments are complete (W1 swap model → W2 local service → W3 editor front end → W4 docs and example → W5 project status panel → W6 artifact browsing → W7 scoped view → W8 snapshots / layered diff / registry browsing; the planner-interface increments are in UI facade (the data panel, W9, has landed). Example entry point: `examples/web-smoke.sh` (the same one CI runs). The roadmap and decisions for later UI increments are in UI facade.
