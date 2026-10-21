[中文](/zh/audit-doc-consistency.md) | English

# Documentation/Source Consistency Audit (2026-10-08)

> Executed per the Documentation/Source Consistency Audit Specification: enumerate the documentation surfaces → extract verifiable technical claims and gather evidence against the code one by one (file:line) → put the three kinds of differences into a table → fix the documentation using the code as the truth (never change the code in reverse) → close the loop.
>
> **Audit surfaces**: README.md, all 12 pages under docs/ (index/architecture/source/schema/validation/target/cli/build/web/example/design/需求整理), examples/game-config/README.md, the docs site navigation configuration (.vitepress/config.mts), rustdoc (cargo doc with zero warnings, the `#![warn(missing_docs)]` gate in place).
> **Evidence-gathering method**: 8 parallel code-comparison tracks (one each for CLI/Schema/Validation/Source/Target/Architecture+Build/Web/Design) + empirical runs (`bash examples/run.sh`, a reproduction of `cage inspect` with no arguments, a reproduction of the type-handling claims against the release binary, and reproductions of E1101/E1601 in a minimal project).
> **Result**: all 73 differences were fixed (the documentation was fixed using the code as the truth, with zero code changes). P0×11 / P1×25 / P2×37.
> (Closing cross-check: the 3 intermediate drafts left in the root directory by the 8 evidence-gathering agents were compared one by one and merged — the 27 items re-checked against `docs/design.md` are all consistent with no differences; T9 from `docs/target.md` (the C# namespace example) is the only un-merged item, while T11/T12/T23 are already inside T2/T3/T6.)
>
> Three kinds of differences: **ahead of code** = documented but absent from the code (the most dangerous — auditing against the documentation would surface phantom features); **missing** = present in the code but undocumented; **inconsistent** = present in both but stated differently.

## Overall remediation principles

- Documentation ahead of code → delete it or relabel it "not implemented / reserved / planned / placeholder" (consistent with the existing "reserved" labelling convention);
- Code ahead of documentation → add documentation (the whole migrate family, registry export/import/push, the four remote-source crates, template target, JSON/CSV options, etc.);
- Inconsistent description → rewrite it to match the actual behavior of the code (wire format, defaults, exit codes, evaluator placeholder state, error-code trigger conditions).
- All 11 P0 items (substantive errors that would mislead usage) were cleared; all 25 P1 items (gaps/statement drift) were cleared; all 37 P2 items (wording/completeness) were cleared.

## P0 list (11 items, all fixed)

| ID | Doc:line | Claim | Source evidence | Category | Fix |
|----|---------|-------|-----------------|----------|-----|
| R1 | `README.md`:60 | `cage inspect Item` (Item is the only argument) | main.rs:106-115: the first positional argument is a required project root directory; reproduced on a real machine as exit 2 | inconsistent | Change to `cage inspect config/ Item`, noting "the project path is required, the table name is optional" |
| R2 | `docs/index.md`:55 | Same as R1 | Same as R1 | inconsistent | Same as R1 |
| SC1 | `docs/schema.md`:34-69 | DSL example: root-level `table:` + `primary_key: id` + fields without `name` | schema/mod.rs:9-16 (the root only has tables/enums/metadata), 31-49 (TableSchema requires name+primary_key), 61-67 (FieldSchema requires name); the example cannot be parsed | ahead of code | Rewrite the whole page to the wire format (`tables:` root key, `{ kind: X }` types, fields carrying name) |
| SC2 | `docs/schema.md`:75 | Type list `uint32`/`int64`/`float` (lowercase scalars) | schema/mod.rs:129-189: all 19 variants are PascalCase and adjacently tagged; lowercase always fails (tests at :832-849) | ahead of code | Change to the accurate names of the 19 kinds (`float` does not exist; add the 9 missing types such as Float32/Float64) |
| SC3 | `docs/schema.md`:81 | Constraint `regex` | schema/mod.rs:91: the key name is `pattern`; `regex` is silently swallowed by flatten | ahead of code | Change to `pattern`, noting that a misspelled key is silently ineffective |
| SC4 | `docs/schema.md`:78 | Constraint `enum` / `values` | schema/mod.rs:95: the key name is `enum_values`; named enums use top-level `enums:` + `{ kind: Enum, value: 名 }` | ahead of code | Change to `enum_values` + the named-enum shape |
| SC5 | `docs/schema.md`:82 | Field-level constraint `unique` | schema/mod.rs:41-42: uniqueness exists only as the table-level `unique_constraints`; a field-level `unique` silently lands in metadata | ahead of code | Change to the table-level `unique_constraints` and give its YAML shape |
| V4 | `docs/validation.md`:308-316 | "Type reference / Predicate are implemented (E1410)" | validation/mod.rs:946-952: the predicate evaluator is a placeholder that always returns true, so the only E1410 production site is unreachable (proved by the test name); E1411 is produced only by compatible_with | ahead of code | Change the inspection-surface table to "predicate reserved (the evaluator is a placeholder that always passes, E1410 is currently untriggerable)"; change the sample code to E1411; add a placeholder note to the code-table row |
| V5 | `docs/validation.md`:342 | "A violated assert reports E1501, located at row level" | validation/mod.rs:992-1001: the expression evaluator is a placeholder that always returns true, so E1501 is untriggerable | ahead of code | Change to "the evaluator is a placeholder that always passes; E1501 never actually occurs"; point cross-field semantic validation at the L7 plugin |
| W1 | `docs/需求整理.md`:15 (and `docs/source.md`) | The Excel adapter checkbox claims "comments (Comment)" | grep over the whole crate finds no comment reading; calamine itself does not expose it | ahead of code | Flip it to a not-implemented label (same treatment as Protobuf/MsgPack); update the capability list in `docs/source.md` |
| SR1 | `docs/source.md`:75 | "All cells are read as strings initially, with types calibrated by the Schema" | cage-source-csv/lib.rs:180,225-258 infers types during parsing; validation/mod.rs:473-516 L2 strictly matches by family; coerce_to_type has zero call sites in the pipeline; empirically: numeric text in a String column → E1101 | inconsistent | Change to "type inference during parsing + strict per-family validation at L2, no re-calibration" (the same sentence in `docs/需求整理.md` is updated in sync) |
| T1 | `docs/target.md`:7-34 | Normalize: `"100"/100/100.0 → UInt(100)`, `yes/true/1 → Bool(true)` | normalize_value only trims/normalizes -0.0/sorts keys (normalize/mod.rs:12-49); coerce_to_type has no call site in the pipeline; empirically against the release binary: CSV "100" vs uint32 → E1101 | ahead of code | Rewrite to the actual behavior (representation-layer normalization + source inference + L2 family check); label cross-type conversion as "the library API is not wired into the pipeline" (the same sentence in 需求整理.md is updated in sync) |

## Full difference table (P0×11 + P1×25 + P2×36, all fixed)

| ID | Doc:line | Claim excerpt | Source evidence (file:line) | Category | Severity | Fix |
|----|---------|---------------|---------------------------|----------|----------|-----|
| R1 | `README.md`:60 | `cage inspect Item` | main.rs:106-115 | inconsistent | P0 | Add the project path argument |
| R2 | `docs/index.md`:55 | Same as R1 | Same as R1 | inconsistent | P0 | Same as R1 |
| R3 | `README.md`:35 | "Databases and Google Sheets come later" | cage-source-db/sheets are implemented | missing | P1 | Move into the implemented list and add the http(s) remote sources |
| R4 | `docs/index.md`:21 | Same as R3 | Same as R3 | missing | P1 | Same as R3 |
| R5 | `README.md`:33-62 | Migration is not mentioned at all | main.rs:165-178 `Commands::Migrate` | missing | P1 | Add a migration bullet under core capabilities and add `cage migrate` to common commands |
| R6 | `docs/index.md`:19-36 | The nine features include no migration | Same as R5 | missing | P1 | Add a migration entry to the features |
| R7 | `README.md`:43 | The registry parenthetical lists only 5 of 8 subcommands | main.rs:182-309 has eight subcommands | missing | P1 | Add "bundle export/import/push straight to a remote" |
| R8 | `README.md`:43 | "Remote http(s) roots resolve read-only" | main.rs:278-309 `RegistryCmd::Push` pushes directly | inconsistent | P2 | Change to "read-only cache on the consumer side + direct push via push" |
| R9 | `docs/index.md`:64 | The Registry section mentions no A series | main.rs:251-309 | missing | P2 | Add the A series implementation note to that section row |
| R10 | `docs/index.md`:35 | The Target plugin list omits Template | cage-target-template is in place | missing | P2 | Add the Template Target |
| R11 | `docs/index.md`:63 | The Web UI section only states "W1, W2, W3 are implemented" | web-smoke.sh has six steps and ci.yml has a web-smoke job (W4 is in place) | missing | P2 | Add W4 |
| C1 | `docs/cli.md`:43-51 | The build section omits `--level` | main.rs:92-93 (semantic by default) | missing | P1 | Add a line (semantic by default, same as check) |
| C2 | `docs/cli.md`:67-68 | `cage inspect Item` | Same as R1 | inconsistent | P1 | Same as R1 |
| C3 | `docs/cli.md`:59 | The gen enumeration omits template | main.rs:2313-2326 the template branch | missing | P2 | Add "template custom-template targets are also supported" |
| C4 | `docs/cli.md`:175 | `registry publish --registry` is written as required | main.rs:199-201 `Option<PathBuf>` (falls back to cage.toml) | missing | P2 | Change to `[--registry <dir>]` and note the fallback when omitted |
| C5 | `docs/cli.md`:32-38 | The check section gives no `--level` value domain/default | main.rs:77-81 defaults to semantic/client; validation/mod.rs:59-66 the value domain | missing | P2 | Add the default and the eight-level value domain |
| C6 | `docs/cli.md`:88-92 | The diff parameters only show directories | main.rs:129-133, 2555-2560 also accept manifest.json | missing | P2 | Note that manifest.json paths are accepted |
| C7 | `docs/cli.md`:59-60 | gen "does not depend on configuration row data" | main.rs:2180-2186 still fully loads and parses source data | inconsistent | P2 | Change to "does not consume row data, but source files must still be loadable" |
| SC1-SC5 | the whole of `docs/schema.md` | Four items: wire format / type names / regex / values / unique | schema/mod.rs | ahead of code | P0 | Rewrite the whole page to the wire format (86→120 lines) |
| SC6 | `docs/schema.md`:71-84 | The constraint list omits implemented keys | schema/mod.rs:99-122 (min_items/max_items/items/properties/targets/rules/order_by) | missing | P1 | Complete the field and table-level constraint lists |
| SC7 | `docs/schema.md`:77 | `default` "fills in when absent" | There is no back-fill implementation; only an E9006 exemption + codegen consuming initial values | ahead of code | P1 | Describe it truthfully (a missing value in a data row is still E1001) |
| SC8 | `docs/schema.md`:86 | "Schema validation is executed by layers L1-L3" | Uniqueness is at L4, references at L5 | inconsistent | P1 | Change to "required L1, types L2, value ranges L3, uniqueness L4, references L5" |
| SC9 | `docs/schema.md`:3 | "DSL file (YAML)" | main.rs:500-507 also accepts yml/json, and merges multiple files in a directory | missing | P2 | Add JSON support and directory merging |
| SC10 | `docs/schema.md`:84 | The reference sub-keys are not documented | schema/mod.rs:280-294 (predicate/cardinality/compatible_with) | missing | P2 | Document the three sub-keys (labeling placeholders/reserved truthfully) |
| V1 | `docs/validation.md`:88-95 | The unknown-field example shows `ERROR E1002` | validation/mod.rs:276-277 defaults to Warning | inconsistent | P1 | Change to WARNING and note the escalation condition |
| V2 | `docs/validation.md`:185-196 | The L3 list includes regex/enum/unique/required | validation/mod.rs:583-712 actually has pattern/enum_values/min_items/max_items etc. | inconsistent | P1 | Change the list to the seven real keys and label where unique/required belong |
| V3 | `docs/validation.md`:222-229 | L4 "null-value rules" | validation/mod.rs:716-810 covers only primary key/uniqueness/ordering | ahead of code | P1 | Delete "null-value rules" and merge the composite constraints into one item |
| V4 | `docs/validation.md`:289-316 | E1410 is implemented | Evaluator placeholder (see the P0 table) | ahead of code | P0 | See the P0 table |
| V5 | `docs/validation.md`:342 | E1501 fires as soon as an assert is violated | Evaluator placeholder (see the P0 table) | ahead of code | P0 | See the P0 table |
| V6 | `docs/validation.md`:389-393 | `cage check # 执行完整验证` | main.rs:77 defaults to semantic (L6 upper bound) | inconsistent | P1 | Change to "L0–L6 by default" + a gamerule row |
| V7 | `docs/validation.md`:498 | E1004 "e.g. circular references" | schema/mod.rs:401-483 actually covers dangling references; only web.rs:167 calls it | inconsistent | P1 | Change to "dangling reference + the Web API loading path" |
| V8 | `docs/validation.md`:129-340 | Embedded schema snippets use the old format (bare scalar types) | Same evidence as SC1/SC2 | ahead of code | P1 | Replace the snippets in each section with the canonical `{ kind: … }` form |
| V9 | `docs/validation.md`:415-446 | The diagnostics example splits File:/Sheet:/Cell: into sections | diagnostics/mod.rs:370-430 actually renders a single-line Source | inconsistent | P2 | Change to the actual render form |
| V10 | `docs/validation.md`:486 | E0001 covers only syntax errors | validation/mod.rs:243-250 L1 reuses E0001 Warning for a missing table | missing | P2 | Add a note to the code-table row |
| V11 | `docs/validation.md`:99 | targets "empty = all profiles" | validation/mod.rs:319 the `"*"` wildcard is equivalent | missing | P2 | Add the `"*"` equivalence note |
| W1 | `docs/需求整理.md`:15 | Excel "comments" delivered | cage-source-excel has no comment reading | ahead of code | P0 | Flip it to a not-implemented label |
| W2 | `docs/web.md`:73-76 | The 409 case lists only the directory scenario | web.rs:202-217 `registry:` schema_path gives the same 409 (tests/web.rs:468) | missing | P1 | Add the registry: 409 scenario (including the toast row) |
| W3 | `docs/web.md`:48 | "The repository introduces no Node/npm build chain" | package.json vitepress is in place (the editor chain indeed has no node) | inconsistent | P2 | Change to "the editor chain introduces none" (excluding the documentation site) |
| W4 | `docs/web.md`:103-105 | The example "a legacy field such as fuzzyField" | Zero hits across repo-wide grep and the whole git log | inconsistent | P2 | Delete the invented example, keep the mechanism description |
| SR1 | `docs/source.md`:75 | CSV "read entirely as strings, calibrated by the Schema" | See the P0 table | inconsistent | P0 | See the P0 table |
| SR2 | `docs/source.md`:45-57 | The Excel capability list includes Comment/Sheet Metadata | cage-source-excel has no corresponding implementation | ahead of code | P2 | Delete and label as not implemented |
| SR3 | `docs/source.md`:53 | Merged cells carry no format restriction | lib.rs:134-137 is .xlsx only (.ods is skipped) | inconsistent | P2 | Label it ".xlsx only" |
| SR4 | `docs/source.md`:59 | The error-location example uses the `Cell G27` form | value/mod.rs:82-100 actually renders `file \ | Sheet: M \ | Row: N \ | Col: G` | inconsistent | P2 | Change to the actual render form |
| SR5 | `docs/source.md` | The source-directory extension loading range is undocumented | main.rs:600-605 (json/yaml/yml/csv/xlsx/xls) vs the adapter supported_extensions (.xlsx/.xlsm/.ods, excel/lib.rs:80) | missing | P2 | Add a "source directory loading range" section (xls is collected then rejected; xlsm/ods are ignored in directory mode, and are supported when a single file is given explicitly) |
| SR6 | `docs/source.md`:84-150 | Remote sources never mention the `registry:` source root | main.rs:435-437 the registry: dispatch | missing | P2 | Add registry: guidance to the loading-range section |
| SR7 | `docs/source.md`:77-80 | JSON/YAML root shapes/merge keys are undocumented | cage-source-yaml/lib.rs:84-95 (merge), 127-187 (root shapes) | missing | P2 | Add the root-shape detection and merge-key explanation |
| EX1 | `docs/example.md`:3 | "Covers every field type" | The instance schema uses only 9 of the 19 types | inconsistent | P2 | Change to "common field types and constraints" |
| EX2 | `docs/example.md`:184 | lua `class` → `class_` | cage-target-lua has no such escaping (LUA_KEYWORDS excludes class); the measured artifact is `key = "class"` | inconsistent | P1 | Remove the lua item, note that lua/ts/js do not escape |
| EX3 | `docs/example.md`:13 | run.sh "build → gen → check" | examples/run.sh:19-63 actually runs check → build → gen → bad data → inspect | inconsistent | P2 | Change to the actual order |
| EX4 | `docs/example.md`:105 | "3 data files per table + manifest" | Measured 42 = 6 data + 36 code; the manifest is listed separately | inconsistent | P2 | Change to the accurate breakdown |
| AR1 | `docs/architecture.md`:305-318 | The structure tree omits migrate | lib.rs:45 `pub mod migrate` | missing | P1 | Add the migrate/ row to the structure tree |
| AR2 | `docs/architecture.md`:337-338 | The CLI command list omits migrate | main.rs:165-178 | missing | P1 | Add migrate |
| AR3 | `docs/architecture.md`:124 | db/http/sheets are still labelled "coming later" | The three crates are implemented (contradicting lines 322-324 of the same document) | inconsistent | P1 | Change to "extended to …; XML/TOML/SQLite come later" |
| AR4 | `docs/architecture.md`:255 | "19 error-code families" | codes.rs actually has 15 families / 64 codes | inconsistent | P1 | Delete the phrase (19 is the field-type count miscopied) |
| AR5 | `docs/architecture.md`:46-47 | The Target list omits template | main.rs:2313-2331 the template branch | missing | P2 | Add the Template Target |
| AR6 | `docs/architecture.md`:258 | The ValidationContext fields omit warnings_as_errors | validation/mod.rs:96-112 seven fields | missing | P2 | Complete the list |
| AR7 | `docs/architecture.md`:136-160 | The Schema example uses the old wire format (unparseable) | Same evidence as SC1 | inconsistent | P1 | Replace it with an actual wire-format example |
| BU1 | `docs/build.md`:37 | "BTreeMap everywhere internally" | manifest/mod.rs:3-4 IndexMap + explicit sorting; BTreeMap is only for object keys | inconsistent | P2 | Change to "IndexMap for ordering + explicit stable sorting (BTreeMap for object keys)" |
| BU2 | `docs/build.md`:219 | Snapshot load is "lazy mapping" | snapshot/mod.rs:167-173 reads everything eagerly | inconsistent | P2 | Change to "fully loaded mapping" |
| BU3 | `docs/build.md`:140-144 | The second-layer condition omits "same profile" | main.rs:1394 `prev.profile == profile` | missing | P2 | Complete the condition |
| BU4 | `docs/build.md`:152-154 | "A missing file or a read failure" both fall back | main.rs:1408-1413 a missing file → targeted regeneration; 1427-1443 only read failures fall back | inconsistent | P2 | Split the two semantics |
| BU5 | `docs/build.md`:116,126 | "L5 validation produces the reference graph" | reference/mod.rs:256-266 builds the graph from_schema before L5, and L5 consumes it | inconsistent | P2 | Change to "the graph is built from schema reference declarations and consumed jointly by L5 and the second incremental layer" (architecture.md:224 updated in sync) |
| DS1 | `docs/design.md`:1130 | "Six languages `{lang}_default`" | All seven crates register filters (lib.rs in lua/cs/py/ts/cpp/go/java each calls register_filters) | inconsistent | P1 | Change to "seven languages"; move the lua type-side parenthetical back to `{lang}_type` |
| DS2 | `docs/design.md`:2227 | "`manifest::hash_source`" | It is actually a private associated function of ManifestGenerator (manifest/mod.rs:197) | inconsistent | P2 | Change to "`ManifestGenerator::input_hashes` → the private hash_source" |
| T1 | `docs/target.md`:7-34 | Normalize falsely claims unified cross-type conversion | See the P0 table | ahead of code | P0 | See the P0 table |
| T2 | `docs/target.md`:66-104 | The JSON target has no options | cage-target-json/lib.rs:81-88 (pretty/sort_keys) | missing | P1 | Add the options (including the generate_combined library API note) |
| T3 | `docs/target.md`:100-104 | The CSV target has no options | cage-target-csv/lib.rs:86-107 (delimiter/write_header/quote_style) | missing | P1 | Add the options table |
| T4 | `docs/target.md` | The combined-artifact capability is undocumented | cage-target-json/lib.rs:184-225 `generate_combined` (unreachable from the CLI) | missing | P2 | Mention the library API along with T2 |
| T5 | `docs/target.md`:345-387 | The Go/Java options omit `file_template` | cage-target-go/lib.rs:209, java/lib.rs:188 | missing | P2 | Add a file_template line to both examples |
| T6 | `docs/target.md`:446-447 | The nine fields are enumerated as a closed list | schema/mod.rs:61-126 serializes far more than nine keys | missing | P2 | Change to a non-exhaustive list + the semantics of omitting required |
| T7 | `docs/target.md`:513-551 | Field Visibility only shows field-level targets | main.rs:781-794 the `"*"` wildcard + whole-table removal via table-level targets | missing | P2 | Add the wildcard and table-level semantics |
| T8 | `docs/target.md`:396-398 | Out-of-range u64 is "noted in the field documentation" | java/lib.rs:1136-1160 comments exist only on enum members; field defaults that are out of range are skipped entirely | inconsistent | P2 | Change to "enum values are two's-complemented onto disk + end-of-line comments; field defaults are skipped" |
| T9 | `docs/target.md`:166 | The C# options example `namespace = "Game.Config"  # 默认 Cage.Generated` — the comment pattern in the same block is "value = default" (the enums_file row), but this example line uses a non-default value, which easily misleads anyone copying the default | cage-target-cs/src/lib.rs:69 defaults to `"Cage.Generated"`; the enums_file in the same block and the namespace line on the C++ page (:309) both follow the "value = default" pattern | inconsistent | P2 | Change the example value back to the default `Cage.Generated`, with the comment "this is the default" |
| N1 | `docs/.vitepress/config.mts` | The site nav/sidebar omits web.md | docs/web.md exists but is reachable only through cross-links | missing | P2 | Add the Web entry to the nav and the engineering sidebar |
| N2 | `docs/需求整理.md`:32 | Normalize `"100"/100/100.0 → UInt(100)` | Same as T1 | ahead of code | P0 | Merge into T1 and fix in sync |

(N2 is the same claim as T1, counted under T1's P0; the total count includes the N2 row, hence 72 rows / 11 P0 items.)

## Items found during the audit but deliberately **not** changed (archived)

1. **Code-side observation (not a documentation difference)**: ~~membership validation for `FieldType::Enum` (named enums) has no home in either L2/L3~~ — **closed (2026-10)**: L3 `validate_field_constraints` gained named-enum membership checks (same E1204 path, string values only; non-strings fall to L2 E1101, and dangling enum names fall to L1 E1004), and the L3 constraint table in `docs/validation.md` was updated to match.
2. **The code comment on `compatible_with`** (schema/mod.rs "for E1410") does not match the code it actually produces, E1411 — **fixed in passing (2026-10)**; the comment now says E1411.
3. **Historical counts** (such as "10 unit tests" and "545 workspace tests all green" in `docs/design.md`): these are delivery-time snapshots, the current values are higher because of later additions, so they are not differences.
4. **f42c83e** (an external-wording commit right after M3) removed the qualifier "declarative data migration" from the status line — this audit restored the complete wording as part of the R-series fixes; history is not rewritten.

## Closing the loop

- Gating after the fixes: fmt / clippy (0 warnings) / 580 tests all passing / rustdoc 0 warnings / docs:build succeeds.
- Incremental cadence: for new features, write the documentation first and drop the "reserved" label within 24h of the error-code wiring — folded into the self-check items of every subsequent delivery per the specification.

## Closing note (2026-10-10)

The two "ahead of code" items in this audit that were downgraded have since been superseded by real implementations, and the documentation has flipped back to "implemented":

- **V4 (E1410 predicate)**: the reference-predicate evaluator is implemented (`validation/expr.rs`, asserts are evaluated against the referenced target row) — the inspection-surface table and the code-table row in `docs/validation.md` were changed back to "implemented", with the evaluation semantics appended (a missing optional field takes no position, incomparable types report this code).
- **V5 (E1501 semantic rules)**: the L6 expression evaluator is implemented (single-comparison asserts `operand OP operand`; a false assert / incomparable types report `E1501`, `warning_only` downgrades to WARNING, and a malformed assert / an undeclared referenced field reports `E1004` once per rule, including on empty tables) — the L6 section in `docs/validation.md`, the `rules` row in `docs/schema.md` and the three-code comment in `docs/codes.rs` were updated in sync.
- Empirically: in the sample project the `hp <= attack` rule passes on good data, and the bad/ row violates it with E1501 (asserted in step 13 of `examples/run.sh`); 12 new tests on the core side (5 for expr + 6 for L6 + 1 for the L5 predicate) cover every branch.

## Closing note (2026-10-12)

The other two "ahead of code" items in this audit that were downgraded were closed per the [design §3.1
decision](https://github.com/cuihairu/cage/blob/main/docs/design.md#31-source):

- **W1 / SR2 (Excel Comment / Sheet Metadata)**: Sheet Metadata was scoped for
  implementation — hidden sheets (`hidden`/`veryHidden`) are skipped before parsing and do not
  enter the Canonical Model (cage-source-excel hand-written xlsx tests cover all three states);
  cell comments are explicitly not done (author annotations are not data and the pipeline has zero
  consumers), and the reopening condition is registered in the same decision. The capability list
  in `docs/source.md` and line 15 of `docs/需求整理.md` were rewritten in sync.
