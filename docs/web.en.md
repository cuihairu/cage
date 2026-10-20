
# Web UI / Schema Editor (Stage 3)

First item of Stage 3: a web-based Schema editing entry point for designers and programmers. The red lines are unchanged: no Excel editor, no game-logic framework. The Web UI is only a front door to the Schema, not a data runtime.

## Positioning

Cage's CLI is the interface of record for the compile chain; the Web UI is **another front door to the same core**: the editor does not parse or render YAML, it only exchanges documents with the core and delegates validation. All editing semantics (the 19 field types, constraints, references, targets visibility, expression rules) come from `cage-core`; the front end replicates no rules.

## Architecture Decisions

### Swap model = the existing serde Canonical shape

The editor document is the serde shape of `Schema`—the same document as `schema.json` in a Configuration Snapshot and the HTTP API. One document, consumed in three places:

```text
snapshot/schema.json  ≡  cage-core::edit::to_editor_json  ≡  HTTP GET /api/schema
```

Editor JSON preserves insertion order (tables/fields in document order), omits the optional fields covered by `skip_serializing_if`, and emits `default` explicitly as `null` because it has no skip attribute (the front end can edit either).

### Save = write back to a single canonical YAML file

The editor works on the **merged canonical Schema** (the cli merges the multiple files under schemas/ into one `Schema` at load time, and the editor builds on that form). Save writes back to a single canonical YAML file (`to_canonical_yaml`): the same Schema → the same bytes (including a single trailing newline), and the round trip through `from_canonical_yaml` is idempotent. Splitting into multiple files is the author's persistence choice; the editor produces no split artifacts. Determinism contract: artifacts produced by `cage build` from editor-saved YAML are byte-identical to those built from the same hand-written Schema. The generator only ever sees `Schema` values, so any value-preserving edit round-trip is sufficient.

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

### W2 implemented: `cage web` local HTTP service (cage-cli/src/web.rs)

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
- [x] `POST /api/schema`: canonical YAML write-back. A single-file `schema_path`
  is overwritten in place; a directory (multi-file schema) → 409 refused (the editor
  does not rewrite the author's split); a `schema_path` starting with `registry:`
  is likewise 409 (the schema of a published entry is not edited here—the publishing
  project must republish); nothing configured → writes `schema.yaml` plus a note
  reminding to wire up cage.toml (the server never modifies configuration files)
- [x] Integration tests (crates/cage-cli/tests/web.rs): real subprocess + raw TCP requests,
  covering document serving / E1701+E1004 validation /
  reload after save and a `cage check` rerun / directory refusal / starting from an empty schema

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
- [x] Save: Ctrl/Cmd+S or the Save button → `POST /api/schema`;
  an unsaved marker + beforeunload interception; 409 (a directory or `registry:` schema_path)
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
  assert that `POST /api/schema` yields a 409 refusal (multiple files are an author-side organization,
  and the editor does not rewrite them)
- [x] CI integration: the `web-smoke` job mirrors the example job
  (checkout + toolchain + rust-cache + `bash examples/web-smoke.sh`)

### W5 implemented: project status panel (U1, see the roadmap in [UI facade](/en/ui))

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
- [x] Tests: 6 web.rs unit tests (status default profile / unknown profile 400 /
  fresh and up-to-date after a build / `changed_tables` naming after a source change /
  a build on bad data refused with diagnostics / a build for an unknown profile refused)
  + 3 integration tests (status page routing and the 301 / status-build end to end /
  bad data end to end) + web-smoke step 5 (the status page against a real server → fresh → up-to-date)

### W6 implemented: artifact browsing (U2a, roadmap in [UI facade](/en/ui))

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
- [x] Tests: 3 web.rs unit tests (the list across profiles and the empty case /
  serving + 404 for a non-ledger path + 409 for tampering / the stale marker)
  + 1 integration test (list → byte-exact serving → 404 → 409 end to end) + artifact assertions added to web-smoke step 5

### W7 implemented: targets badges + profile-scoped view (U2b, roadmap in [UI facade](/en/ui))

- [x] `GET /api/schema?profile=NAME`: also returns `projection` (a direct call to `filter_by_profile`—the same projection function as the build pipeline;
  unknown profile 400). Zero semantic duplication in the front end: it only takes the "full − projection" set difference to get the hidden set;
  visibility rules never enter the JS
- [x] Targets badges: whenever `field.targets` / `table.targets` is non-empty, the editor renders a chip (a table entry in the left column /
  a field row / a table title); the data source is the schema itself, with no new state
- [x] Scoped view: a "Scope" knob in the editor's top bar ((all) / each profile);
  tables/fields dropped by the projection are greyed out with a "not visible in this profile" badge rather than disappearing—what is hidden should be visible as hidden;
  the projection is computed from the saved schema,
  and the prompt states this explicitly while there are unsaved edits (saving is entirely author-driven, with no autosave)
- [x] Tests: +2 web.rs unit tests (projection gating tables and fields / unknown profile 400)
  + 1 integration test (no profile → no projection → client drops the server-only field → server keeps it → ghost 400)
  + web-smoke step 7 (game-config's `internal_note`: dropped by the client projection,
  kept by server, ghost 400)

### W8 implemented: snapshots / layered diff / registry browsing (U3, roadmap in [UI facade](/en/ui))

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
      and the sticking point (an incremental-sync need for "fetch only the changed subtree") is covered in [UI facade](/en/ui)
- [x] `GET /api/registry`: browses the packages and versions under the `[registry] path` (the same entry point as cage_core's registry);
  if nothing is configured, it returns ok + an explanation rather than an error
- [x] The status page: three new sections—"Snapshots" (a self-verify badge + a pack button),
  "Compare" (base/target knobs: the live build against each snapshot;
  three-level collapsing, with a verdict when the bytes match exactly), and "Registry"
- [x] CLI alignment: `cage snapshot --list` (lists packages + a self-verify
  verdict for each, mutually exclusive with `--verify`/`--env`); `cage diff`
  gains a layered "tables changed / targets affected" summary from the same
  data source as the status page (manifest `table_hashes` and artifact paths)
- [x] Tests: +3 web.rs unit tests (snapshot packing / list self-verification /
  bad body 400 / diff layering with missing arguments and an unknown base 400 /
  registry not-configured explanation + browsing when configured)
  + `cage snapshot` CLI tests (--list package listing and a tampering verdict, --list conflicting with `--verify`/`--env`)
  + 1 integration test (registry not configured → pack → list → cross-snapshot diff → ghost 400)
  + web-smoke step 5 (pack → self-verify → a zero-difference diff → the empty registry state)

### W9 implemented: data panel (planner interface P0, 2026-10-20, roadmap in [UI facade](/en/ui))

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
- [x] Tests: +2 web.rs unit tests (table list facts and ghost 400 /
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
      `cage snapshot` package), record in [UI facade](/en/ui)
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
      web unit 32 green / integration 14 green unchanged

### W12 implemented: table source upload (planner interface P0, 2026-10-21, roadmap in [UI facade](/en/ui))

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

## Roadmap

With this, the whole Stage 3 W series and the subsequent UI increments are complete (W1 swap model → W2 local service → W3 editor front end → W4 docs and example → W5 project status panel → W6 artifact browsing → W7 scoped view → W8 snapshots / layered diff / registry browsing; the planner-interface increments are in [UI facade](/en/ui) (the data panel, W9, has landed). Example entry point: `examples/web-smoke.sh` (the same one CI runs). The roadmap and decisions for later UI increments are in [UI facade](/en/ui).
