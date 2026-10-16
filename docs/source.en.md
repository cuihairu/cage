[中文](source.md) | English

# Source: Input Sources

A Source Adapter is Cage's input plugin. One Adapter per input format, and all of them produce the [Canonical Model](/architecture.en) uniformly.

```text
                  Cage Core
                     |
        +------------+-------------+
        |            |             |
     Excel         JSON          YAML
        |            |             |
        +------------+-------------+
                     |
              Canonical Model
```

Key principle:

> **The Source Adapter is only responsible for reading and parsing, not for game business logic.**

Text-based sources perform basic type inference during parsing (e.g. `100` in CSV is inferred as an integer, `true` as a boolean); the Schema then **strictly** validates at L2 against the target type (a type-family mismatch yields `E1101`, with no string↔number recalibration — writing numeric text into a String column reports a type error). The Source performs no business validation and makes no conversion decisions.

## First-Phase Support

```text
Excel
CSV
JSON
YAML
MessagePack
```

### Excel

Excel is a very common Authoring Source in the game industry, but it is not Cage's core abstraction.

For example:

```text
Item.xlsx
Monster.xlsx
Skill.xlsx
Quest.xlsx
```

The Excel Adapter (based on [calamine](https://crates.io/crates/calamine) to read `.xlsx`/`.xlsm`/`.ods`) can handle:

- Workbook
- Worksheet
- Header
- Cell
- Row
- Column
- Merged Cells (merged-cell backfill; `.xlsx` only, not done for `.ods`)
- Formula (cached formula values, not recalculated)
- Cell Type

Sheet-level metadata (Sheet Metadata): hidden sheets (`hidden` /
`veryHidden`) are the author's draft/staging surface — they are skipped
before parsing and never enter the Canonical Model (decided 2026-10-12,
`docs/design.md` §3.1). Cell comments (Comment) are explicitly not supported — comments are author
annotations, not data, and the pipeline has zero consumers; the reopen
conditions and rationale are in the same decision.

Error locations are precise down to the cell (the diagnostic `Source` field
renders like `monster.xlsx | Sheet: Monster | Row: 27 | Col: G`).

Final conversion to:

```text
Document
  |
  +-- Table: Item
  +-- Table: Monster
  +-- Table: Skill
```

### CSV

- The header row defines the column names; each row is a data row
- Locations resolve to header name + row number
- Type inference at parse time: `true`/`false`/`yes`/`no` → Bool, integer and
  float text → Int/UInt/Float, empty cells → Null, everything else → String;
  the Schema then strictly validates at L2 against the target type (numeric
  text into a String column reports `E1101`; quotes are CSV syntax and do not
  change the content type)

### JSON / YAML

- Naturally tree-structured, mapping directly onto the Canonical Model
- YAML root shapes: a top-level Sequence (elements are Mappings) reads as
  multiple tables, a top-level Mapping reads as a single-row table, null is
  skipped; `<<: *anchor` merge-key expansion is supported
- Path locations are preserved: text-format errors (YAML syntax errors, etc.)
  are precise down to the line number (error code `E0001` family)

### MessagePack

- The file shape is exactly the msgpack target's wire format: a bare array of
  row objects `[{"id": 1, "name": "Sword"}, …]`; the binary carries no table
  name, so the file name stem names the table (CSV convention)
- Floats are restored bit-for-bit, `bin` payloads are restored as `Bytes`, and
  integers beyond `i64::MAX` keep the `UInt` family — three kinds of values a
  JSON round trip cannot express; non-negative integers are normalized to
  `Int` (same as the JSON source: the wire format's positive markers carry no
  signedness)
- An empty array produces an empty table (not skipped), keeping the
  target → source round trip complete
- Shape violations (root not an array, row not an object, trailing bytes,
  truncated frames) report `E0001` / `E0004`

### Source Directory Loading Scope

`[source_roots]` directory discovery collects `json` / `yaml` / `yml` / `csv` /
`xlsx` / `xls` / `msgpack` extensions; of these, the legacy `.xls` format is
collected and then rejected with `E0001`, while `.xlsm`/`.ods` are not in the
directory collection list (silently ignored in directory mode — the adapter
itself supports `.xlsx`/`.xlsm`/`.ods` when a single file is explicitly
specified via `schema_path`/`source`). A remote source root (`registry:`, see
[CLI · registry](/cli.en)) can likewise be a `[source_roots]` value.

### HTTP API (Remote Source, S1)

Writing an http(s) URL directly as a `[source_roots]` value makes it a remote
source: the GET response bytes land as-is in `.cage-cache/source/<URL fingerprint>/`,
then go through exactly the same parsing as local JSON
(`{table name: row array}` / single object → `Root` / row array → `Data`), with
the full L0-L7 validation and no bypass. Connection-class failures get bounded
retries; a fetch failure reports `E1901`, 401/403 report `E1902`, and malformed
JSON reports `E0001` (same diagnostics as local files). The finalized shape of
the four sources is in design §45.

Pagination (S7): when a response carries an RFC 8288 `Link: <...>; rel="next"`
header, pages are fetched along the chain — each page must be a JSON row array,
and the pages are concatenated in order into one merged document (bare array →
`Data` table) that lands in the cache; a relative next target is resolved
against the page URL, and both loops and exceeding 1000 pages are hard errors
(`E1901`). A response without a next header keeps the single-GET contract
byte-for-byte unchanged. Rate-limit negotiation: a 429 carrying `Retry-After`
(seconds or HTTP-date) and ≤ 30s backs off and retries by that amount (sharing
the budget with transport retries); a missing, unparseable, or over-cap value
is an immediate `E1901` error. Pagination-contract violations and rate-limit
errors do not participate in offline fallback — only transport-class failures
(server unreachable) fall back to the cache (`E1906` WARNING).

Conditional-GET incremental fetch (S8, document level): when a response carries
`ETag` / `Last-Modified` headers, the validator is stored alongside the bytes in
a sidecar meta next to the cache
(`.cage-cache/source/<URL fingerprint>/<URL fingerprint>.meta.json`); fetching the same URL
again first sends a conditional request with `If-None-Match` /
`If-Modified-Since`, and a server reply of **304 reuses the cached bytes
as-is** (not even rewriting the meta), while a 200 replaces both the new bytes
and the new validator. Both meta and payload must be present for revalidation —
missing either falls back to a plain GET rebuild as if there were no cache.
Only the first-page GET is conditional (the first page's validator describes the
whole collection, so on 304 the pagination chain need not be walked again);
`--no-cache` (strict) still revalidates — a 304 is the server confirming the
cache, not the cache answering itself. Transport failures still follow the S4
offline-fallback semantics; 304 does not participate. Row-level incrementals
(fetching changed rows by revision / updated_at) remain left to the
implementation phase.

### MySQL / PostgreSQL (Remote Source, S2)

`[source_roots]` takes `mysql:<table|named query>` / `pg:<table|named query>`:
named queries come from `[remote.<scheme>].queries`, otherwise the name must be a
safe table name and is expanded to `SELECT * FROM <name>`. Loading has dual
read-only insurance — a static SELECT whitelist (single statement, SELECT-prefixed;
rejects semicolons / comments / row locks / `INTO` / CTE, `E1905`) runs first,
and only then is the environment variable named by `[remote.<scheme>].dsn_env`
resolved (undeclared / unset reports `E1904`; the DSN never enters cage.toml),
and after connecting the session is pinned read-only (MySQL
`SESSION TRANSACTION READ ONLY`, PG `default_transaction_read_only`).
Connections are pooled at process level per DSN (design §45, delivered 2026-10-13): N sources on the same server share one handshake, each session
acquired is still pinned read-only, and a session whose fetch fails is discarded
rather than returned to the pool. Type policy: DECIMAL / NUMERIC keep textual
fidelity and do not go through Float, NULL → Null, binary columns are base64,
and everything else maps to a JSON scalar by column type. The row set is
serialized into canonical JSON and lands in
`.cage-cache/source/<scheme+DSN+SQL fingerprint>/`, then goes through standard JSON parsing
— the same cache anchor and parsing chain as the HTTP source. Row order: `ORDER BY`
is respected as-is, otherwise rows are sorted by their serialized form (the
build does not depend on server return order). Connection / statement failures
report `E1901`.

### Database Table Introspection Schema Draft (S9, design §45)

`cage schema-draft <project> <spec>... [-o <file>]` turns the **shape** of
database tables into a reviewable schema draft (see [cli](/cli.en)): each
`mysql:<table>` / `pg:<table>` spec introspects `information_schema` through a
read-only session (the table name is bound as a parameter, never concatenated
into SQL), rendering the table, its primary key, and
`NOT NULL → required: true`; DECIMAL / datetime / JSON / unknown types are
flagged in an inline comment as "needs an author decision", and tables with no
primary key get an empty `primary_key: []` plus a comment reminder. The type
policy matches the adapter's — DECIMAL / temporal / JSON draft as `String`,
which is exactly their actual shape in the cached JSON. Rendering determinism:
the same introspection always yields the same bytes (no timestamp in the
header). Credentials and read-only discipline are the same as S2: the DSN is
read from `[remote.<scheme>].dsn_env` (`E1904`), and a table that cannot be
found reports `E1901`.

### Google Sheets (Remote Source, S3)

`[source_roots]` takes `gsheet:<spreadsheet_id>/<tab>`, with an API key as the
credential, read from the environment variable named by
`[remote.gsheets].credential_env` (undeclared / unset reports `E1904`; the key
never enters cage.toml and never lands in any log — error diagnostics only cite
the spec). Fetching goes through the Sheets API v4 `values` endpoint with
`valueRenderOption=UNFORMATTED_VALUE` (cached formula values, not recalculated —
the same policy as the Excel adapter). Mapping follows Excel convention: tab =
table, first row = header (empty header cells fall back to `col<i>`), empty rows
are skipped, short rows are padded with null to the header width, and the tab's
natural row order is preserved exactly as the author-committed order. Cell
initial values are strings (numbers / booleans keep their JSON text); the Schema
then calibrates the types. Response-shape gates (not a row set / empty header /
majorDimension not ROWS) report `E1903`; 401/403 report `E1902`, and a fetch
failure reports `E1901`. The canonical JSON lands in
`.cage-cache/source/<gsheets+id+tab fingerprint>/` (the API key does not enter the fingerprint —
it does not change byte semantics), then goes through standard JSON parsing.

### Offline Fallback and `--no-cache` (All Remote Sources, S4)

All three remote sources share one offline semantic: on a **transport-class**
fetch failure (connection refused / DNS / timeout / service unreachable), if the
previous cached copy is still in `.cage-cache/source/`, the build does not fail
— it falls back to parsing that copy and prints an `E1906` WARNING to stderr
(`warning: E1906 remote source unreachable, serving the previous cached copy: <spec> (cache: <path>)`).
Two classes of failure **never fall back**: 404 (the remote has been deleted)
and 401/403 (access may have been revoked) — stale bytes would silently go
wrong, so they stay hard failures (`E1901` / `E1902`). DB source connection /
statement failures fall back as transport class; credentials (`E1904`) and the
query whitelist (`E1905`) happen before any network contact, so stale bytes
cannot mask a configuration error. A cache file served from fallback goes
through the same standard JSON parsing and full validation as the online path,
with no bypass.

`--no-cache` (on the four commands `check` / `build` / `gen` / `inspect`)
disables the fallback: if the remote cannot be fetched, the build fails even
when a cached copy exists. Suited to a pre-release check that "must build
against the latest remote bytes".

## Later Extensions (Not First Phase)

```text
XML
TOML
SQLite
Custom Binary
```

These formats are outside the first-phase delivery scope; the four first-phase remote sources
(HTTP / MySQL / PostgreSQL / Google Sheets) have been fully delivered along with S1–S5 —
see the respective sections above for details.

## Rust Implementation

| Input Source | crate | Key Dependencies |
| --- | --- | --- |
| Excel | `cage-source-excel` | calamine |
| CSV | `cage-source-csv` | csv |
| JSON | `cage-source-json` | serde_json |
| YAML | `cage-source-yaml` | serde_yaml |
| MessagePack | `cage-source-msgpack` | rmpv |
| HTTP API | `cage-source-http` | `cage_core::remote` (fetching / retry / cache) + cage-source-json |
| MySQL / PostgreSQL | `cage-source-db` | mysql / postgres (pure Rust protocol clients) + `cage_core::remote` (cache key) + cage-source-json |
| Google Sheets | `cage-source-sheets` | `cage_core::remote` (fetching / retry / cache) + cage-source-json |

All of them uniformly produce the Canonical Model of `cage-core`: Null / Bool / Int / UInt / Float / String / Bytes / Array / Object, carrying Source Location and metadata.
