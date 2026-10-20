
# UI Facade: Web Enhancement (Decisions and Roadmap)

User feedback: cage does not feel good to use, and people want a UI. This document settles
four things: the form-factor choice, interface coverage, the change-visualization policy,
and how dual-end differences are expressed. The positioning is unchanged (`web.md`): the
Web UI is **another facade over the same core**, and the frontend duplicates no rule
semantics; the three disciplines — zero new dependencies, single binary, no node editor
toolchain — all carry forward.

## Measured Baseline (at the 2026-10-12 Decision)

- release exe **10.6 MiB** (lto + strip + opt-level=z).
- editor page 48 KB (index.html 2.0 / app.js 37.6 / app.css 8.4 KB), embedded at compile
  time via `include_str!` (web.rs), so the single binary carries its own UI.
- the existing HTTP surface has 6 routes: `GET /`, `/app.js`, `/app.css`,
  `GET /api/schema`, `POST /api/validate`, `POST /api/schema`; bound to `127.0.0.1`
  only, no authentication (positioned as a local tool).
- manifest hashing is two-tier: `table_hashes` as a per-table blake3 row fingerprint → a
  per-target canonical fingerprint (sorted by `(format, output_dir)`) → a per-file hash →
  the root `content_hash`; `build_id = blake3(profile + schema_hash + source_hash +
  content_hash)` (profile is part of the input).
- incremental builds already consume this hash layer: `prev/cur table_hashes` compared
  table by table + dependency-graph propagation decides the skip set (main.rs).
- `cage diff` today: a flat comparison of the `artifacts` from two manifests
  (`+/-/~` + counts + a `content_hash` line); schema-level diff is handled separately
  by `migrate::diff_schemas`.

## Form Factor: Decision c (Enhance the Existing Web), Absorbing d, a / b Demoted to Alternatives

| Dimension | a) Tauri desktop shell | b) pure Rust GUI (egui/iced) | c) enhance cage web | d) polish the CLI only |
|---|---|---|---|---|
| Size | shell adds ~8–15 MB more (estimated, system WebView) | adds ~10–20 MB (estimated, GUI stack goes into the exe) | embedded strings + a thin JSON API, about ±0.3 MB, still ~11 MiB overall | unchanged |
| Distribution / packaging | NSIS/MSI or a portable exe; Linux cannot cross-build a Tauri exe, so **packaging must move to a Windows runner**, and the nightly zip layout would have to change | cross-building an MSVC exe on Linux needs a new toolchain (something like cargo-xwin), a large new CI surface | **completely unchanged**: same exe, same nightly zip + SHA256SUMS, install.sh/ps1 as-is | unchanged |
| cage-core reuse | in-process direct calls | in-process direct calls | in-process direct calls (the W2 exchange model already works) | direct calls |
| Escaping the terminal | desktop window | possible | the browser — and naturally cross-machine: `cage web` runs on a Linux server, opened from a Windows browser | not possible |
| Maintenance cost | high: a giant dependency tree (breaks zero new dependencies) + WebView2/webkit2gtk quirks on both ends + a packaging pipeline | high: rich data views (tables / trees / diffs) are all hand-rolled in immediate mode | **lowest**: vanilla JS with no build chain, reusing all of the existing W1–W4 assets | lowest, but it does not address the "want a UI" request |

**What was decided**: the UI form factor is direction c — enhance the existing `cage web`
(a direct extension of the W2 local server + W3 editor page), plugging the
**already-existing data** — manifest / artifacts / snapshot / registry / diff — into the
facade; direction d is not a separate project but is absorbed as a discipline (see
"Absorbing d"). The primary usage topology is a Linux server (running builds) + a Windows
client (viewing results, editing data), and the artifact must still ship as an exe.

**Why**: ① topology fit — the browser is the only surface that spans "headless server +
Windows desktop" at zero packaging cost, and a desktop shell cannot open a window on a
server; ② releaseable — the artifact is still that one exe, and the nightly rolling channel
carries it out as before (a daily build is a normal delivery surface), without touching the
version-release red line and without adding a Windows packaging surface to CI; ③ highest
reuse — the W1 exchange model, W2 local server, W3 editor, `/api/schema`,
`POST /api/validate`, E1701/E1004 are all already in place; what is missing is only facade
area, not foundations.

**Alternatives**: a) a Tauri shell (reuses the most UI assets, but at the cost of an entire
packaging pipeline + a dependency-tree explosion, and it is useless in the server
scenario); b) an egui/iced single-binary GUI (the dependency tree is somewhat more
controllable, but all rich data views are hand-rolled — the largest amount of work — and
it is equally a dead end on a headless server) — both are demoted to alternatives and are
not implemented.

**Boundaries**: no game-logic framework (the red line is unchanged); no multi-user /
remote-team collaboration (the web UI is a single-machine local tool); no standalone
frontend build chain beyond the editor (the vanilla ES module discipline is unchanged,
still embedded at compile time).

**Sticking-point condition (the only reason to reopen a/b)**: a real usage scenario arises
where you "must escape the browser" (a fully offline desktop environment, no browser
available), or the UI grows so large that embedded strings become unmaintainable — then
restart the form-factor evaluation.

**Remote access policy (decided)**: keep binding to `127.0.0.1` only with no
authentication — **untouched**. The server → Windows client remote scenario uses SSH port
forwarding (documenting the usage is enough). `--bind` / LAN exposure must have
authentication first (the registry already has the `--auth-env` precedent to follow); it is
registered as a sticking-point condition and is not done this phase — exposing an
unauthenticated local service onto a LAN is a security regression, and no exception is made
for convenience.

**How d is absorbed (not a separate project)**: CLI readability was already a first-class
citizen (E codes + location + hints), and it is absorbed as two disciplines: ① when a new
UI endpoint lands, the same parameter semantics are reflected in the CLI output (the text
surface and the UI share the same core function — the single-entry discipline, same as the
`validate_project` precedent); ② any CLI output an increment touches is re-reviewed
against the "errors are readable + summary is readable" standard. The one explicit
deliverable: when U3 lands, the `cage diff` text output also gets the layered summary
(see the change-visualization section).

## Interface Coverage (P0 → P1 → P2)

Endpoint discipline: every new surface = one thin JSON endpoint + frontend wiring. Every
endpoint is a direct call into an existing cage-core function, continuing the W2 exchange
model; the server holds no state and reloads the on-disk state on every request (same as
`/api/schema`). The frontend adds no build chain — it remains an extension of the
`docs/public/editor/` single page or a sibling static page, embedded at compile time.

### P0 (First Batch, All Wiring of Existing Data)

- **Project and build status**: profile list, validation status (shares the
  `validate_project` single entry with `check`, so UI diagnostics and CI diagnostics never
  diverge), build freshness (current source `table_hashes` vs the old values in the last
  manifest — the very numbers incremental builds already compare, telling the user directly
  "which tables changed and have not been rebuilt yet"), and the last build facts (profile,
  build_id, content_hash, target count).
- **Validation panel**: a diagnostics table (code / severity / file-Sheet-cell location /
  hint), sourced from `POST /api/validate`, and consumed in three places — LSP, CLI, and
  here — from the same `Diagnostics`.

### P1 (Second Batch)

- **profile × target artifact browsing**: a profile selector → target list → the
  `manifest.artifacts` tree (per-file hash / size / online preview / download); grouping by
  `output_dir` naturally layers by profile (build/client/*, build/server/*).
- **schema editor completion**: the existing W3 editor already mounts E1701/E1004
  diagnostics; add the field-level `targets` badge and the rest of the dual-end visibility
  expressions (see the dedicated section below).

### P2 (Third Batch, Policy First, Landing with Each Increment)

- **Snapshots**: three actions — list (the historical snapshot directory), verify (the
  same core call as `snapshot --verify`), and package. (implemented in U3)
- **registry entry browsing**: lists of local and remote entries (the remote list has
  already landed), with version, dependency pin, and hash visible. (implemented in U3)
- **diff and change visualization**: the semantic layered diff (dedicated section below).
  (implemented in U3)

## Change Visualization: No Merkle, a Semantic Layered Diff

**What was decided**: the semantic presentation of a diff has three layers, all data
coming from the existing manifest, with **zero format changes and zero hash
redefinitions**. The diff API returns:

```json
{
  "changed_tables": ["item", "stage"],
  "affected_targets": ["json:build/client/data", "..."],
  "files": { "added": [], "removed": [], "changed": [] },
  "roots": { "content_hash": "…", "build_id": "…" }
}
```

The UI collapses along three layers of **table → target → file**: which tables changed →
which targets are affected → which files are touched. `changed_tables` = a table-by-table
comparison of the two manifests' `table_hashes`; `affected_targets` = changed tables
propagated through the dependency graph (the same logic as incremental builds); `files` =
the existing run_diff's flat result hung under the corresponding subtree. The text surface
is in sync: `cage diff` gains a layered summary output (table / target lines + a file
list), sharing the same semantic diff function as the UI (single entry).

**Why**: today's `cage diff` is a flat file list, so "which part changed" has to be read
out of the paths by the user. But `table_hashes` **are already in hand per table**, so
semantic layering only needs one comparison + one dependency-graph propagation + one join —
all display-layer work, touching no load-bearing structure.

**Alternative (rejected): Merkle-ization** — add interior node digests so diff walks the
minimal subtrees that differ. Rejection reasons: ① the manifest root
(`content_hash` / `build_id`) is a **load-bearing wall** — deterministic builds, registry
version pins, snapshot verification, and incremental caching all hang on it, so redefining
it = total cache invalidation + old pins breaking; ② in a local diff both manifests are
already on disk and must be scanned O(manifest) anyway, so Merkle pruning **saves no local
work at all** — its benefit only exists in a "pull only the changed subtree from a remote"
incremental-sync scenario, which this repository does not have.

**Boundaries**: no versioned manifest format migration (no format change, so no migration);
the semantic layer of diff answers only "where it changed", not "how the content changed"
(a per-field value-level diff is a new capability surface, to be a separate project when
needed — the schema level already has `diff_schemas` to follow).

**Sticking-point condition (the only reason to reopen Merkle)**: reopen it if an
incremental-sync need emerges where "a consumer pulls only the changed subtree from a
remote registry / snapshot".

## Expressing Dual-End Differences (profile × target × Field Visibility)

Factual basis: a profile determines the target list and `output_dir` (build/client/*,
build/server/*); table-level and field-level `targets` (in the schema) cause
`filter_by_profile` to strip fields from that profile's schema and from **all artifacts**
(the example project's server-only field `internal_note` is exactly this); `build_id`
inputs include profile.

Four UI expressions:

- **Artifact ownership**: the manifest carries profile and build_id, and the artifact
  browser groups by `output_dir` + a global profile badge — every view is labeled as
  "which end's artifact this is".
- **Field-visibility badges**: whenever `field.targets` / `table.targets` is non-empty,
  render a badge in the editor and browsing views (`internal_note` → "server only"); the
  data source is the schema itself, with no new state. (implemented in U2b)
- **profile-scoped view**: `GET /api/schema?profile=` returns a projected schema (a direct
  call to the off-the-shelf `filter_by_profile` function); stripped fields show "not
  visible in this profile" instead of disappearing — things that are hidden must be
  visibly hidden. (implemented in U2b)
- **Cross-end comparison (P2)**: pick a table → a client / server two-column matrix, with
  field rows in three states: "server only", "client only", "visible on both ends"; the
  artifacts side shows the same table side by side, and the extra fields on the server are
  the mirror image of the visibility stripping.

## Roadmap

- [x] **U1 (P0)**: project and build status + validation panel (status endpoint + frontend
      wiring) — **implemented 2026-10-12**, landing record:
      `GET /api/status` (three blocks in one document: project facts / validation panel /
      per-profile build facts) + `POST /api/build` (the same `build_project` entry as
      `cage build`) + the `/status/` status page (validation panel + build cards, embedded
      at compile time, vanilla ES module with zero dependencies). **Two policy refinements**:
      ① freshness = the "reported version" of `build_project`'s first-layer comparison —
      same profile, same environment, same schema/source/target fingerprints, complete
      artifacts — plus `changed_tables` (the second layer's table-by-table `table_hashes`
      comparison) telling the user which tables changed and have not been rebuilt yet; the
      manifest is single-slot (`<output_dir>/manifest.json` records one profile at a time),
      `last_build_profile` marks ownership, and freshness across profiles is false outright.
      ② The validation panel goes through the `validate_project` single entry (same source
      as check / lsp), and the page has three knobs, `?profile=&env=&level=`, with defaults
      matching `cage check` (first profile, semantic, no environment). Bare `/status`
      301-redirects to `/status/` — so relative resource references do not collide with the
      editor's /app.js. Acceptance: 6 unit tests in web.rs + 3 integration tests in
      tests/web.rs + step 5 of web-smoke (real server: page → fresh → up-to-date).
- [x] **U2a (P1) artifact browsing**: `/api/artifacts` (the manifest ledger; across
      profiles the single slot returns an empty list + `manifest_profile`, and staleness
      reuses the first-layer verdict) + `/api/artifact` (accepts only manifest keys +
      recomputes blake3 before serving; tampering gives 409, a non-ledger path gives 404) +
      an "artifacts" area on the status page (grouped by output_dir, preview/download).
      Acceptance: 3 unit tests in web.rs + 1 integration test in tests/web.rs (list →
      byte-exact serving → 404 → 409) + the artifact assertions in step 5 of web-smoke
- [x] **U2b (P1) editor targets badges + profile-scoped view**:
      `GET /api/schema?profile=` returns a `filter_by_profile` projection (an unknown
      profile gives 400); the frontend only takes set differences, with zero visibility
      semantics copied into JS; non-empty targets always render a badge (left column /
      field row / table title), stripped entries are grayed out with "not visible in this
      profile" instead of disappearing; when there are unsaved edits the projection hint
      says "projected from the saved schema". Acceptance: +2 unit tests in web.rs + 1
      integration test + step 7 of web-smoke (the three-state assertion for game-config's
      `internal_note`)
- [x] **U3 (P2) snapshot list/verify/package + registry entry browsing + layered diff** —
      **implemented 2026-10-12**, landing record:
      ① `GET /api/snapshots` (lists every package under `<output_dir>/snapshot/`, each
      package **live-verified** by `verify_snapshot` — the list never claims a package it
      has not just verified is intact; plus manifest facts: profile / env / build_id /
      content_hash / artifact count) + `POST /api/snapshot` (the same pipeline as
      `cage snapshot`: full build → pack_snapshot; validation failure gives 200 +
      diagnostics, Io gives 400) + a "snapshots" area on the status page (verified badge +
      fact chips + a package button).
      ② `GET /api/diff?from=&to=` (the semantic layered diff of two manifests, in exactly
      the shape decided above: `changed_tables` / `affected_targets` /
      `files:{added,removed,changed:[path,table,target,from_hash,to_hash]}` /
      `roots:{content_hash,build_id}`; table/target metadata travels with each file entry,
      so the UI's three-layer collapse is pure grouping with no new computation) + a
      "compare" area on the status page (baseline/target knobs covering the live build and
      each snapshot, a three-layer table → target → file collapse, and a "the two builds
      are byte-identical" message when hashes match).
      ③ `GET /api/registry` (reads the `[registry] path`; when unconfigured it returns ok +
      an explanation rather than an error; per-version build_id / content_hash / files
      chips) + a "Registry" area on the status page.
      ④ CLI: `cage snapshot --list` (lists packages + a self-verification verdict for each;
      clap-conflicts with `--verify`/`--env`), and `cage diff` gains two layered-summary
      lines, "tables changed / targets affected". **The Merkle sticking point stands**:
      not done until a "pull only the changed subtree" incremental-sync need appears.
      Acceptance: +3 unit tests in web.rs (snapshot packaging/listing/bad body, diff
      layering and 400, registry browsing and unconfigured) + 5 CLI tests for
      `cage snapshot` (including --list package listing and a tampering verdict, plus
      describe-conflict assertions) + 1 integration test (registry unconfigured → package
      → list self-verify → cross-snapshot diff → ghost 400 end to end) + step 5 of
      web-smoke (real server: package → self-verify → zero-diff → registry empty state)

After each item lands, append a landing record to this section (the convention follows
`docs/design.md`): the implemented surface, policy refinements, and acceptance method.

## Planner Rule Interface (2026-10-20 user directive: design first, then build)

> The directive: planners configuring rules without any interface hurts. Positioning per the
> "Rule Configuration Interface" block in todo: planners must be able to configure rules, run
> validation and read results without touching the CLI or files, reusing existing assets
> (Diagnostics locations → table/row/column anchoring; the error-code table → a readable panel;
> one parsing convention, never two). This section is the design decision taken before any
> implementation.

**Entry shape (decided 2026-10-20)**: no new `cage serve` subcommand — everything lands in
the existing single `cage web` service. Basis: shape decision c (above) already settled
"enhance the existing cage web", and W5–W8 have already brought project-level validation
(the validation face of `/api/status` ships code/severity/location/hint diagnostics JSON),
builds, artifacts, diff and registry to the facade — a separate subcommand would fork the
security boundary (loopback checks, bounded reads, the 8 MiB cap; the 2026-10-18 表一#2
hardening) and the docs surface. `cage serve` in the todo was the queuing seat's shorthand,
not the user's own words.

**P0 delta inventory (against the implemented surface, to avoid rebuilding)**:

| todo wording | Current state | Real delta |
|---|---|---|
| One-click validate | `/api/status` already runs full `validate_project` diagnostics | No new validation — presentation-layer work only |
| Diagnostics panel | The status page panel exists (code/severity/location/hint) | Filter controls (by code/severity) |
| Click to jump to table row/column | The editor's `focusDiag` only jumps to schema fields | A **data table view** (data rows are not displayed anywhere today) plus mapping diagnostic Source locations (file/Sheet/Row/Col) to highlighted cells |
| Upload or select files | The server reads only in-repo `source_roots` | **Select** first (an inventory of the repo's source files); **upload** deferred (decided: writing arbitrary files into the repo is the same class as the 2026-10-18 表一#1 path traversal — it needs its own security decision covering where writes land, overwrite/traversal defense, and a size cap; not implemented until decided) |

**Endpoint discipline unchanged**: every new face = one thin JSON endpoint
(`GET /api/tables` listing + `GET /api/table?name=` with paginated, truncated rows,
calling the existing load-chain functions directly) + front-end wiring; the server stays
stateless; vanilla ES module, zero build chain, compiled in at build time; validation stays
single-entry (`validate_project`, the existing check / lsp / web same-source discipline).

**Phasing**:

- **P0 data panel**: table list → table view (pagination + row/column caps so a large
  table cannot blow up the browser) → diagnostics filters (code/severity) → click a
  diagnostic to jump and highlight the cell.
- **P1 rule forms + one-click export**: rule editing rides the existing W3 editor
  (form-ization = editor interaction work; saving goes through the existing
  `POST /api/schema` chain, serialization stays cage's); export reuses the existing
  `POST /api/build` + `/api/artifacts` downloads (whether to also zip is decided at
  implementation time; artifact bytes are never recomputed).
- **P2 version compare + template library**: compare reuses the existing `GET /api/diff`
  layered JSON; the template library is a front-end static template dictionary generating
  schema fragments (numeric range / enum / foreign-key reference to start), saved through
  the same schema save chain — zero new server surface.

**Acceptance**: per endpoint, web.rs unit tests + tests/web.rs integration + a web-smoke
step (the U-series convention); the table view gets pagination/truncation assertions; the
diagnostic anchoring gets a bad-data → click → highlighted-coordinate assertion; no upload
means no upload surface.

**Boundary**: no multi-user/remote (remote access still goes through SSH port forwarding,
per the decision above); no uploads (separate security decision first); no new rule
execution logic — validation stays the `validate_project` single entry.

**P0 landing record (2026-10-20)**: the data panel is implemented —
`GET /api/tables` + `GET /api/table` (web.rs; row window paginated at 200,
capped at 1000; the primary key taken from the schema definition); the status
page's "Data" section (table list + sticky-thead table view + pager); the
diagnostics panel's severity/error-code filters and click-to-jump row highlight
(jumpToDiag). Tests: +2 web unit tests (32 green total) + 1 integration test
(14 green) + web-smoke step 5 assertions folded in, full script run passing.
Recorded in docs/web.md W9. Remaining: uploads deferred (no security decision
yet, honestly carried as a blocker); P1 rule forms/export and P2
compare/templates untouched.
