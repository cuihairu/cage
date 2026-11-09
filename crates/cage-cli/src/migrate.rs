//! `cage migrate` / `cage migrate --draft` / schema draft: read raw
//! sources, run the step chain against the target schema, and rewrite
//! files in place (or preview the rewrite). Split out of the CLI entry.

use indexmap::IndexMap;
use std::path::{Path, PathBuf};

use cage_core::convention::derive;
use cage_core::error::codes::registry::E1802;
use cage_core::error::codes::remote::E1905;
use cage_core::error::codes::source_convention::E2201;
use std::collections::HashSet;

use cage_core::manifest::ProjectConfig;
use cage_core::schema::Schema;
use cage_core::value::Document;

use crate::remote;
use crate::{excel_adapter, load_project_config, load_project_impl, load_schema};

/// How many affected-row location lines one step may render before the
/// rest collapse into a single deterministic summary line (always the
/// first N in source order — rows already are in source order).
pub(crate) const EXCEL_LOCATION_CAP: usize = 8;

/// Render one step's affected-row locations for the migrate report, in
/// source order under the step's row-count line (4-space indent), tail
/// summarized once past [`EXCEL_LOCATION_CAP`]. Core records locations
/// only for Excel-served tables, so an empty list renders nothing.
pub(crate) fn affected_location_lines(step: &cage_core::migrate::StepReport) -> Vec<String> {
    let locations = &step.affected_locations;
    if locations.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = locations
        .iter()
        .take(EXCEL_LOCATION_CAP)
        .map(|loc| format!("    {}", loc.display()))
        .collect();
    if locations.len() > EXCEL_LOCATION_CAP {
        lines.push(format!(
            "    … +{} more row(s)",
            locations.len() - EXCEL_LOCATION_CAP
        ));
    }
    lines
}

/// `cage migrate [--all | --to <ver|latest>] [--write]` — apply the
/// migration chain in `<root>/migrations/` (file-name order = version
/// step chain) to the loaded document, then reverify it under the
/// current schema. `--to latest` resolves to the whole chain, same
/// selection as `--all`.
///
/// The CLI always holds the post-migration schema (rules migrate data;
/// the schema was already edited), so no historical from-schema exists on
/// disk: `validate_spec`'s from-schema reference check stays a library
/// capability, and a bad reference surfaces here as E2003 at apply time
/// (missing table, rename target already taken) or E2004 at reverification.
/// Dry-run by default; `--write` rewrites the local text sources in place,
/// byte-identically skipping unchanged files.
pub(crate) fn run_migrate(path: &Path, all: bool, to: Option<&str>, write: bool) -> i32 {
    let project = match load_project_impl(path, false, false) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let migrations_dir = path.join("migrations");
    if !migrations_dir.is_dir() {
        println!(
            "cage migrate: nothing to migrate (no {} directory)",
            migrations_dir.display()
        );
        return 0;
    }
    let chain = match cage_core::migrate::parse_migration_dir(&migrations_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if chain.is_empty() {
        println!(
            "cage migrate: nothing to migrate (empty {} directory)",
            migrations_dir.display()
        );
        return 0;
    }
    // `latest` is the symbolic end of the chain — it resolves to the whole
    // chain, exactly like `--all`. A chain that literally contains a
    // segment ending at the version named "latest" resolves as that
    // version first (the concrete target wins over the symbol).
    let latest_symbol = to == Some("latest") && !chain.iter().any(|(_, s)| s.to == "latest");
    let selected: Vec<(String, cage_core::migrate::MigrationSpec)> = if all || latest_symbol {
        chain
    } else if let Some(target) = to {
        let mut picked = Vec::new();
        for (file, spec) in chain {
            let at_target = spec.to == target;
            picked.push((file, spec));
            if at_target {
                break;
            }
        }
        if picked.last().is_none_or(|(_, s)| s.to != target) {
            eprintln!(
                "error: --to {target}: no segment on the migration chain ends at that version"
            );
            return 2;
        }
        picked
    } else {
        // Explicit step-by-step evolution: one segment per run by default.
        vec![chain.into_iter().next().expect("non-empty chain")]
    };

    // Linkage gate (design §46, deferred item): the chain end and the
    // `[dependencies]` pin are two independent declarations of the schema
    // version the data must satisfy — migrating outside the pin leaves
    // the rewritten data no schema to validate under. Gate both dry-run
    // and write.
    if let Some(chain_end) = selected.last().map(|(_, spec)| spec.to.as_str()) {
        if let Err(message) = check_migrate_pin_linkage(&project.config, chain_end) {
            eprintln!("error: {message}");
            return 2;
        }
    }

    let mut doc = project.document;
    let mut migrated_rows = 0usize;
    for (file, spec) in &selected {
        let report = match cage_core::migrate::apply(spec, &mut doc, &project.schema) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        println!("cage migrate: segment {file} ({} → {})", spec.from, spec.to);
        for step in &report.steps {
            println!("  {}: {} row(s)", step.step, step.rows_changed);
            migrated_rows += step.rows_changed;
            // Excel sources are report-only (never written back), so the
            // affected rows' addresses are the one concrete record of what
            // the step moved. Plain-text sources stay file-level — their
            // rewritten file is the record, and per-row lines there would
            // be noise.
            for line in affected_location_lines(step) {
                println!("{line}");
            }
        }
    }

    // Post-migration reverification (L0-L6) under the current schema —
    // a failure means nothing is written.
    if let Err(e) = cage_core::migrate::reverify(&doc, &project.schema) {
        eprintln!("error: {e}");
        eprintln!("cage migrate: verification failed — nothing written");
        return 1;
    }

    let lines = match write_migrated_sources(&doc, path, &project.config, &project.schema, write) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    for line in &lines {
        println!("{line}");
    }
    println!(
        "cage migrate: OK ({} segment(s), {} row(s) migrated, schema {}){}",
        selected.len(),
        migrated_rows,
        project
            .schema
            .metadata
            .as_ref()
            .map_or("(unversioned)", |m| m.version.as_str()),
        if write {
            ""
        } else {
            " — dry run, nothing written"
        }
    );
    0
}

/// Gate the migration chain end against the `[dependencies]` pin of the
/// registry package providing the schema (design §46 linkage): the pin
/// declares which schema versions this project accepts, so a chain end
/// outside it would leave the migrated data no schema to validate under.
/// Local-schema projects have no pin and nothing to link; a registry
/// `schema_path` without a pin is governed by its explicit version alone.
pub(crate) fn check_migrate_pin_linkage(
    config: &ProjectConfig,
    chain_end: &str,
) -> Result<(), String> {
    let Some(rel) = config
        .schema_path
        .as_deref()
        .filter(|r| r.starts_with("registry:"))
    else {
        return Ok(());
    };
    let (package, _) = cage_core::registry::parse_spec(rel)?;
    let Some(pin) = config.dependencies.get(&package) else {
        return Ok(());
    };
    let req = cage_core::registry::parse_version_req(pin)
        .map_err(|e| format!("{e} (dependency '{package}')"))?;
    if cage_core::registry::satisfies(chain_end, &req) {
        Ok(())
    } else {
        Err(format!(
            "{E1802} migrate target version {chain_end} is outside the \
             [dependencies] pin '{pin}' for '{package}' — widen the pin or \
             migrate to a version it admits"
        ))
    }
}

/// `cage migrate-draft <from-schema> <to-schema> --from <ver> --to <ver>
/// [-o <file>]` — draft a migration rule file from two schema versions
/// (design §46 deferred item). Mechanically safe transforms become steps;
/// renames and enum remaps stay "# TODO" comments. The draft is never
/// applied by this command: the author finishes it, then `cage migrate`
/// dry-runs it. The draft skips `validate_spec` on purpose — a
/// `set_default` on a newly-added field references the to-schema, and a
/// `remove_field` references the from-schema, so neither schema alone
/// satisfies the reference check; the real gates are the diff itself and
/// apply's E2003防线.
pub(crate) fn run_migrate_draft(
    from_schema: &Path,
    to_schema: &Path,
    from_version: &str,
    to_version: &str,
    out: Option<&Path>,
) -> i32 {
    let from = match load_schema(from_schema) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let to = match load_schema(to_schema) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let diff = cage_core::migrate::diff::diff_schemas(&from, &to);
    let draft = cage_core::migrate::diff::draft_migration(&diff);
    // Versions are single-quoted so numeric-looking labels (`2.0`) cannot
    // reparse as floats when the rule is read back.
    let mut text = format!(
        "from: '{}'\nto: '{}'\n",
        from_version.replace('\'', "''"),
        to_version.replace('\'', "''"),
    );
    text.push_str(&cage_core::migrate::diff::render_draft(&draft));
    match out {
        Some(path) => {
            // `-o migrations/0001.yaml` with no `migrations/` yet is the
            // documented first-run shape — create the parent, not an error.
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("error: cannot create {}: {e}", parent.display());
                    return 1;
                }
            }
            if let Err(e) = std::fs::write(path, &text) {
                eprintln!("error: cannot write {}: {e}", path.display());
                return 1;
            }
            println!(
                "cage migrate-draft: {} step(s), {} TODO(s) → {}{}",
                draft.steps.len(),
                draft.todos.len(),
                path.display(),
                if draft.steps.is_empty() {
                    " — steps: [] must be filled before cage migrate can parse it"
                } else {
                    ""
                }
            );
        }
        None => {
            print!("{text}");
        }
    }
    0
}

/// `cage schema-draft <project> <spec>... [-o <file>]` — draft a schema
/// file from live database tables (design §45 deferred item). Each
/// `mysql:<table>` / `pg:<table>` spec is introspected read-only over
/// the project's `[remote.<scheme>]` settings and rendered into one
/// deterministic YAML draft; the author reviews the comments and owns
/// the semantic constraints. No project schema or data loads — cage.toml
/// alone settles the DSN discipline.
pub(crate) fn run_schema_draft(path: &Path, specs: &[String], out: Option<&Path>) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let mut tables = Vec::new();
    let mut db_specs = Vec::new();
    let mut excel_drafts = Vec::new();
    let mut excel_specs = Vec::new();
    for spec in specs {
        // X2 (design §51): `excel:<relative-path>` drafts the schema from a
        // convention Excel's meta rows — same parser, same `TableMeta`, so
        // the draft is exactly what the derived chain would validate against.
        if let Some(rel) = spec.strip_prefix("excel:") {
            let file = path.join(rel);
            let excel = config.source_excel().map(|c| (c, path));
            let metas = match excel_adapter(excel).parse_file_with_meta(&file) {
                Ok((_, metas)) => metas,
                Err(diags) => {
                    eprintln!("{}", diags.render(false));
                    return 2;
                }
            };
            if metas.is_empty() {
                eprintln!(
                    "error: {E2201} no convention metadata decoded from {} (is `[source.excel]` \
                     declared and does the sheet carry meta rows?)",
                    file.display()
                );
                return 2;
            }
            let derived = derive::derive_schema(&metas);
            let count = derived.tables.len();
            excel_drafts.push(derive::render_draft(spec, &derived));
            excel_specs.push(count);
            continue;
        }
        let Some((scheme, name)) = cage_source_db::parse_spec(spec) else {
            eprintln!("error: {E1905} not a mysql:/pg:/excel: source spec: {spec}");
            return 2;
        };
        let dsn = match cage_source_db::resolve_dsn(scheme, config.remote.get(scheme)) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        match cage_source_db::introspect(scheme, &dsn, name) {
            Ok(info) => {
                db_specs.push(spec.clone());
                tables.push(info);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    let mut text = String::new();
    if !tables.is_empty() {
        text.push_str(&cage_source_db::draft::render_draft(&db_specs, &tables));
    }
    for draft in &excel_drafts {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(draft);
    }
    let table_count = tables.len() + excel_specs.iter().sum::<usize>();
    match out {
        Some(path) => {
            // `-o schemas/draft.yaml` with no `schemas/` yet is the
            // documented first-run shape — create the parent, not an
            // error (same as migrate-draft).
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("error: cannot create {}: {e}", parent.display());
                    return 2;
                }
            }
            if let Err(e) = std::fs::write(path, &text) {
                eprintln!("error: cannot write {}: {e}", path.display());
                return 2;
            }
            println!(
                "cage schema-draft: {} table(s) → {} — review the comments before adopting",
                table_count,
                path.display()
            );
        }
        None => {
            print!("{text}");
        }
    }
    0
}

/// Group tables by source file and rewrite each local text source
/// (JSON/YAML/CSV) with its migrated tables. Excel sources and anything
/// outside the project's local source roots (registry entries, remote
/// caches) are never written — they are reported instead. Rows render in
/// schema field order (the stable contract order — the JSON reader loses
/// the file's original key order, so that would not be stable to re-render
/// against), which is what makes a second `--write` run byte-identical.
/// Byte-identical renders are skipped outright.
/// One source queued for the --write landing phase of
/// `write_migrated_sources`.
struct PendingWrite {
    line: usize,
    source: String,
    file: PathBuf,
    old_bytes: Vec<u8>,
    new_bytes: Vec<u8>,
    tables: usize,
}

pub(crate) fn write_migrated_sources(
    doc: &Document,
    root: &Path,
    config: &ProjectConfig,
    schema: &Schema,
    write: bool,
) -> Result<Vec<String>, String> {
    let writable: Vec<PathBuf> = config
        .source_roots
        .values()
        .filter(|rel| {
            !rel.starts_with("registry:")
                && !rel.starts_with("mysql:")
                && !rel.starts_with("pg:")
                && !rel.starts_with("gsheet:")
                && !remote::is_remote_root(rel)
        })
        .map(|rel| root.join(rel))
        .collect();
    let mut groups: std::collections::BTreeMap<String, Vec<&cage_core::value::Table>> =
        std::collections::BTreeMap::new();
    for table in doc.tables.values() {
        if !table.source_file.is_empty() {
            groups
                .entry(table.source_file.clone())
                .or_default()
                .push(table);
        }
    }
    let mut lines = Vec::new();
    // Two phases (review 2026-10-18 表一#4): first render every writable
    // text source and read back its current bytes, THEN touch the
    // filesystem. The old loop wrote each file as it went, so a render or
    // read failure further down the list left the earlier files already
    // rewritten — a half-migrated project. Placeholders keep the report
    // in source order; entries are rewritten to "wrote" as replacements
    // land.
    let mut pending: Vec<PendingWrite> = Vec::new();
    for (source, tables) in &groups {
        let file = PathBuf::from(source);
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "xlsx" | "xls" => {
                let rows: usize = tables.iter().map(|t| t.rows.len()).sum();
                lines.push(format!(
                    "  skip {source} (Excel source: report only — {} table(s), {rows} row(s))",
                    tables.len()
                ));
                continue;
            }
            "json" | "yaml" | "yml" | "csv" => {}
            _ => {
                lines.push(format!("  skip {source} (not a text source)"));
                continue;
            }
        }
        if !writable.iter().any(|w| file.starts_with(w)) {
            lines.push(format!(
                "  skip {source} (registry/remote source: read-only, {} table(s))",
                tables.len()
            ));
            continue;
        }
        let new_bytes = render_source_file(&ext, tables, source, schema)?;
        let old_bytes = std::fs::read(&file)
            .map_err(|e| format!("cannot read back source file {source}: {e}"))?;
        if old_bytes == new_bytes {
            lines.push(format!("  unchanged {source}"));
            continue;
        }
        let idx = lines.len();
        lines.push(format!(
            "  would write {source} ({} table(s))",
            tables.len()
        ));
        pending.push(PendingWrite {
            line: idx,
            source: source.clone(),
            file,
            old_bytes,
            new_bytes,
            tables: tables.len(),
        });
    }
    if write {
        // Second phase: replace each source atomically — write a sibling
        // temp file, rename it over the source. A replacement that fails
        // (locked directory, full disk, …) rolls back the earlier
        // replacements from their saved originals, so --write is
        // all-or-nothing across the whole run.
        let mut replaced: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        for item in &pending {
            let (source, file) = (&item.source, &item.file);
            // Honor the file's own write permission: rename alone would
            // bypass a read-only target that the old in-place write used
            // to refuse, so probe first and treat refusal as failure.
            if let Err(e) = std::fs::OpenOptions::new().write(true).open(file) {
                return Err(replace_failed(&replaced, source, &e));
            }
            let tmp = file.with_file_name(format!(
                ".{}.migrate-tmp-{}",
                file.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("source"),
                std::process::id()
            ));
            let done =
                std::fs::write(&tmp, &item.new_bytes).and_then(|()| std::fs::rename(&tmp, file));
            match done {
                Ok(()) => {
                    replaced.push((file.clone(), item.old_bytes.clone()));
                    lines[item.line] = format!("  wrote {source} ({} table(s))", item.tables);
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(replace_failed(&replaced, source, &e));
                }
            }
        }
    }
    Ok(lines)
}

/// Roll back the replacements landed so far and report the failure that
/// stopped the --write landing.
pub(crate) fn replace_failed(
    replaced: &[(PathBuf, Vec<u8>)],
    source: &str,
    e: &std::io::Error,
) -> String {
    let rolled = replaced.len();
    for (file, old_bytes) in replaced.iter().rev() {
        let _ = std::fs::write(file, old_bytes);
    }
    format!("cannot replace source file {source}: {e}; rolled back {rolled} earlier write(s)")
}

/// Render one source file's tables back into their original format:
/// JSON/YAML as `{ table: [row objects] }` in document order, CSV as a
/// single-table file (the reader's own convention). Row fields render in
/// schema declaration order when the table is defined in the schema —
/// the JSON source reader does not preserve the file's original key
/// order, so the schema order is the only stable one to re-render — with
/// any off-schema fields appended in load order.
pub(crate) fn render_source_file(
    ext: &str,
    tables: &[&cage_core::value::Table],
    source: &str,
    schema: &Schema,
) -> Result<Vec<u8>, String> {
    match ext {
        "json" => {
            let mut outer = IndexMap::new();
            for table in tables {
                outer.insert(table.name.clone(), rows_to_json(table, source, schema)?);
            }
            let mut buf = Vec::new();
            serde_json::to_writer_pretty(&mut buf, &outer)
                .map_err(|e| format!("cannot render {source}: {e}"))?;
            buf.push(b'\n');
            Ok(buf)
        }
        "yaml" | "yml" => {
            let mut outer = IndexMap::new();
            for table in tables {
                outer.insert(table.name.clone(), rows_to_json(table, source, schema)?);
            }
            let text = serde_yaml::to_string(&outer)
                .map_err(|e| format!("cannot render {source}: {e}"))?;
            Ok(text.into_bytes())
        }
        "csv" => render_csv(tables, source, schema),
        _ => Err(format!(
            "cannot render {source}: unsupported text format '{ext}'"
        )),
    }
}

/// Column order for one table: schema-declared fields that actually appear
/// in the data (in schema order), then any off-schema fields in
/// first-appearance order.
pub(crate) fn ordered_columns(table: &cage_core::value::Table, schema: &Schema) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    if let Some(defined) = schema.tables.get(&table.name) {
        for name in defined.fields.keys() {
            if table.rows.iter().any(|r| r.fields.contains_key(name)) {
                columns.push(name.clone());
            }
        }
    }
    let seen: HashSet<String> = columns.iter().cloned().collect();
    let mut extra: Vec<String> = Vec::new();
    for row in &table.rows {
        for name in row.fields.keys() {
            if !seen.contains(name) && !extra.contains(name) {
                extra.push(name.clone());
            }
        }
    }
    columns.extend(extra);
    columns
}

/// Rows of one table as ordered JSON objects (schema field order; the
/// nested payload goes through `serde_json`'s own map, which sorts deep
/// keys — order there carries no meaning, the top level does).
pub(crate) fn rows_to_json(
    table: &cage_core::value::Table,
    source: &str,
    schema: &Schema,
) -> Result<Vec<IndexMap<String, serde_json::Value>>, String> {
    let columns = ordered_columns(table, schema);
    let mut rows = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        let mut obj = IndexMap::with_capacity(row.fields.len());
        for name in &columns {
            if let Some(tv) = row.fields.get(name) {
                obj.insert(
                    name.clone(),
                    value_to_json(&tv.value, source, &table.name, name)?,
                );
            }
        }
        rows.push(obj);
    }
    Ok(rows)
}

pub(crate) fn value_to_json(
    value: &cage_core::value::Value,
    source: &str,
    table: &str,
    field: &str,
) -> Result<serde_json::Value, String> {
    use cage_core::value::Value;
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::UInt(u) => serde_json::Value::from(*u),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::String(s) => serde_json::Value::from(s.clone()),
        // A text source never carries bytes — a base64 write would not
        // read back the same, so refuse instead of mangling the value.
        Value::Bytes(_) => {
            return Err(format!(
                "cannot write {table}.{field}: bytes value has no text-source form ({source})"
            ));
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(value_to_json(item, source, table, field)?);
            }
            serde_json::Value::from(out)
        }
        Value::Object(entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                map.insert(k.clone(), value_to_json(v, source, table, field)?);
            }
            serde_json::Value::Object(map)
        }
    })
}

/// One CSV file = one table, headers in schema declaration order (see
/// [`ordered_columns`]), cells formatted so each value reads back as
/// itself.
pub(crate) fn render_csv(
    tables: &[&cage_core::value::Table],
    source: &str,
    schema: &Schema,
) -> Result<Vec<u8>, String> {
    if tables.len() != 1 {
        return Err(format!(
            "cannot render {source}: a CSV file carries exactly one table, found {}",
            tables.len()
        ));
    }
    let table = tables[0];
    let headers = ordered_columns(table, schema);
    let headers = if headers.is_empty() && !table.primary_key_fields.is_empty() {
        table.primary_key_fields.clone()
    } else {
        headers
    };
    let mut writer = csv::Writer::from_writer(Vec::new());
    if !headers.is_empty() {
        writer
            .write_record(&headers)
            .map_err(|e| format!("cannot render {source}: {e}"))?;
    }
    for row in &table.rows {
        let mut record = Vec::with_capacity(headers.len());
        for header in &headers {
            match row.fields.get(header) {
                Some(tv) => record.push(csv_cell(&tv.value, source, &table.name, header)?),
                None => record.push(String::new()),
            }
        }
        writer
            .write_record(&record)
            .map_err(|e| format!("cannot render {source}: {e}"))?;
    }
    writer
        .flush()
        .map_err(|e| format!("cannot render {source}: {e}"))?;
    writer
        .into_inner()
        .map_err(|e| format!("cannot render {source}: {e}"))
}

/// One CSV cell: the source reader's inference rules inverted, so a
/// written cell parses back as the same value — bool before number,
/// integral floats carry a `.0` to stay clear of the integer branches,
/// an empty cell is null.
pub(crate) fn csv_cell(
    value: &cage_core::value::Value,
    source: &str,
    table: &str,
    field: &str,
) -> Result<String, String> {
    use cage_core::value::Value;
    Ok(match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) if f.fract() == 0.0 => format!("{f}.0"),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        // No round-trippable cell form: refusing beats writing something
        // that silently reads back as a string.
        Value::Bytes(_) | Value::Array(_) | Value::Object(_) => {
            return Err(format!(
                "cannot write {table}.{field}: {} value has no CSV cell form ({source})",
                value.type_name()
            ));
        }
    })
}
