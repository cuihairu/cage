//! Declarative data migration under schema evolution (design §46, M
//! series). Migration rules live in a project's `migrations/` directory —
//! the file-name order is the version step chain (`0001-…`, `0002-…`) —
//! and each segment explicitly declares its transforms against the
//! Canonical Model. Rules are data: they go into version control, the
//! same rules over the same data produce the same document, and nothing
//! is inferred from a schema diff.
//!
//! The module splits in two:
//! - [`parse_spec`] / [`parse_migration_dir`] read rule files into
//!   [`MigrationSpec`] values (E2001 for anything structurally wrong),
//! - [`validate_spec`] checks every step's references against the
//!   from-schema (E2002 — a rule that names a table or field the schema
//!   does not have is a pointing error, caught before any data moves).
//!
//! Execution ([`apply`]-style transforms, E2003) and post-migration
//! reverification (E2004) land with M2.
//!
//! The [`diff`] submodule drafts a rule file from two schema versions
//! (design §46 deferred item): mechanically safe transforms become steps,
//! renames and enum remaps stay `# TODO` comments for the author.

pub mod diff;

use crate::error::codes::migration::{E2001, E2002, E2003, E2004};
use crate::schema::{FieldType, Schema};
use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
use indexmap::IndexMap;
use serde::Deserialize;
use std::path::Path;

/// One migration segment: transforms taking a document from the `from`
/// schema version to the `to` schema version.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MigrationSpec {
    /// Schema version this segment starts from
    pub from: String,
    /// Schema version this segment produces
    pub to: String,
    /// Transforms applied in order — later steps see earlier steps' output
    pub steps: Vec<Step>,
}

/// One declarative transform. Wire format is a single-key mapping per
/// step (`- rename_field: { table: Item, from: name, to: title }`), so
/// an unknown transform name fails parsing as E2001 rather than being
/// silently ignored. Deserialization is hand-rolled: `serde_yaml` has no
/// externally-tagged enum support, so the single-key mapping is decoded
/// as a generic YAML mapping and dispatched by its one key.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Rename a field within a table
    RenameField {
        /// Table containing the field
        table: String,
        /// Current field name
        from: String,
        /// New field name
        to: String,
    },
    /// Give a field a default value (for rows where it is absent; also
    /// satisfies a newly-`required` field)
    SetDefault {
        /// Table containing the field
        table: String,
        /// Field receiving the default
        field: String,
        /// The default value, in canonical value form
        value: Value,
    },
    /// Drop a field from every row of a table
    RemoveField {
        /// Table containing the field
        table: String,
        /// Field to remove
        field: String,
    },
    /// Widen a field's type (e.g. Int32 → Int64). Narrowing is rejected
    /// at apply time (E2003) — parse accepts any well-formed type and
    /// the direction check happens where the data is.
    WidenType {
        /// Table containing the field
        table: String,
        /// Field to retype
        field: String,
        /// The new field type (same wire format as schema `type:`)
        to: FieldType,
    },
    /// Rewrite a field's string values through an explicit mapping
    /// (enum renames, value consolidation). Values not present in the
    /// mapping pass through unchanged.
    RemapValues {
        /// Table containing the field
        table: String,
        /// Field whose values are remapped
        field: String,
        /// `old → new` mapping
        map: IndexMap<String, String>,
    },
    /// Rename a table
    RenameTable {
        /// Current table name
        from: String,
        /// New table name
        to: String,
    },
}

// Per-variant payload shapes (serde derives only — used by the manual
// `Step` deserializer below via `serde_yaml::from_value`).
#[derive(serde::Deserialize)]
struct RenameFieldBody {
    table: String,
    from: String,
    to: String,
}
#[derive(serde::Deserialize)]
struct SetDefaultBody {
    table: String,
    field: String,
    /// Bare YAML scalar / array / mapping — converted via
    /// [`yaml_to_value`] (mappings are decoded canonically first, so
    /// adjacently-tagged values work; see [`yaml_to_value`]).
    value: serde_yaml::Value,
}

/// Convert a generic YAML value into the canonical [`Value`] model.
/// Mappings are decoded canonically first: the adjacently-tagged form
/// (`{type: UInt, value: 5}`) is how the rule-draft renderer writes
/// defaults whose family a bare scalar cannot carry (plain `5` reads
/// back as `Int`). A mapping that does not decode as a tagged value —
/// or anything that is not a mapping — falls through to the plain
/// reading. YAML tags are rejected; plain YAML cannot express `Bytes`.
fn yaml_to_value(yaml: serde_yaml::Value) -> Result<Value, String> {
    if let serde_yaml::Value::Mapping(_) = yaml {
        if let Ok(tagged) = serde_yaml::from_value::<Value>(yaml.clone()) {
            return Ok(tagged);
        }
    }
    Ok(match yaml {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else {
                Value::Float(
                    n.as_f64()
                        .ok_or_else(|| "unrepresentable number".to_string())?,
                )
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(yaml_to_value(item)?);
            }
            Value::Array(out)
        }
        serde_yaml::Value::Mapping(map) => {
            let mut out = IndexMap::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    other => return Err(format!("unsupported mapping key: {other:?}")),
                };
                out.insert(key, yaml_to_value(v)?);
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(tag) => {
            return Err(format!(
                "YAML tags are not supported in defaults: {}",
                tag.tag
            ));
        }
    })
}
#[derive(serde::Deserialize)]
struct RemoveFieldBody {
    table: String,
    field: String,
}
#[derive(serde::Deserialize)]
struct WidenTypeBody {
    table: String,
    field: String,
    to: FieldType,
}
#[derive(serde::Deserialize)]
struct RemapValuesBody {
    table: String,
    field: String,
    map: IndexMap<String, String>,
}
#[derive(serde::Deserialize)]
struct RenameTableBody {
    from: String,
    to: String,
}

/// Decode one step payload body, keeping the step kind in the error text.
fn payload<T: serde::de::DeserializeOwned>(kind: &str, b: serde_yaml::Value) -> Result<T, String> {
    serde_yaml::from_value(b).map_err(|e| format!("bad `{kind}` payload: {e}"))
}

impl<'de> serde::Deserialize<'de> for Step {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw: IndexMap<String, serde_yaml::Value> =
            serde::Deserialize::deserialize(deserializer)?;
        if raw.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "a step must be exactly one transform mapping, got {} keys",
                raw.len()
            )));
        }
        let (kind, body) = raw.into_iter().next().unwrap();
        match kind.as_str() {
            "rename_field" => {
                let b: RenameFieldBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RenameField {
                    table: b.table,
                    from: b.from,
                    to: b.to,
                })
            }
            "set_default" => {
                let b: SetDefaultBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                let value = yaml_to_value(b.value).map_err(serde::de::Error::custom)?;
                Ok(Step::SetDefault {
                    table: b.table,
                    field: b.field,
                    value,
                })
            }
            "remove_field" => {
                let b: RemoveFieldBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RemoveField {
                    table: b.table,
                    field: b.field,
                })
            }
            "widen_type" => {
                let b: WidenTypeBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::WidenType {
                    table: b.table,
                    field: b.field,
                    to: b.to,
                })
            }
            "remap_values" => {
                let b: RemapValuesBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RemapValues {
                    table: b.table,
                    field: b.field,
                    map: b.map,
                })
            }
            "rename_table" => {
                let b: RenameTableBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RenameTable {
                    from: b.from,
                    to: b.to,
                })
            }
            other => Err(serde::de::Error::custom(format!(
                "unknown step kind `{other}` (expected rename_field / set_default / \
                 remove_field / widen_type / remap_values / rename_table)"
            ))),
        }
    }
}

impl Step {
    /// Human-readable summary used in reports and E2002 messages, e.g.
    /// `rename_field(Item.name → title)`.
    pub fn describe(&self) -> String {
        match self {
            Step::RenameField { table, from, to } => {
                format!("rename_field({table}.{from} → {to})")
            }
            Step::SetDefault { table, field, .. } => {
                format!("set_default({table}.{field})")
            }
            Step::RemoveField { table, field } => format!("remove_field({table}.{field})"),
            Step::WidenType { table, field, to } => {
                format!("widen_type({table}.{field} → {to:?})")
            }
            Step::RemapValues { table, field, map } => {
                format!(
                    "remap_values({table}.{field}, {} entr{})",
                    map.len(),
                    if map.len() == 1 { "y" } else { "ies" }
                )
            }
            Step::RenameTable { from, to } => format!("rename_table({from} → {to})"),
        }
    }
}

/// Parse one migration rule file into a [`MigrationSpec`]. Any structural
/// problem — unreadable file, invalid YAML, missing `from`/`to`/`steps`,
/// an unknown step kind, a bad field type — is E2001. Reference validity
/// (does the schema actually have these tables and fields?) is a separate
/// [`validate_spec`] pass so parsing stays schema-independent.
pub fn parse_spec(path: &Path) -> Result<MigrationSpec, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "{E2001}: cannot read migration rule {}: {e}",
            path.display()
        )
    })?;
    let spec: MigrationSpec = serde_yaml::from_str(&text).map_err(|e| {
        format!(
            "{E2001}: migration rule {} is not a valid spec: {e}",
            path.display()
        )
    })?;
    validate_structure(&spec, &path.display().to_string())?;
    Ok(spec)
}

/// Structural self-checks beyond what serde enforces: non-empty version
/// strings and at least one step (a segment that transforms nothing is a
/// pointing error, not a no-op).
fn validate_structure(spec: &MigrationSpec, origin: &str) -> Result<(), String> {
    if spec.from.trim().is_empty() {
        return Err(format!(
            "{E2001}: migration rule {origin} has an empty `from` version"
        ));
    }
    if spec.to.trim().is_empty() {
        return Err(format!(
            "{E2001}: migration rule {origin} has an empty `to` version"
        ));
    }
    if spec.from == spec.to {
        return Err(format!(
            "{E2001}: migration rule {origin} moves `{}` to itself",
            spec.from
        ));
    }
    if spec.steps.is_empty() {
        return Err(format!(
            "{E2001}: migration rule {origin} ({}) declares no steps",
            spec.from
        ));
    }
    Ok(())
}

/// Check every step's references against the from-schema (E2002): the
/// named table and field must exist, a rename must not collide with an
/// existing name. `rename_table`'s target must also be free — rules run
/// in order against one evolving document, but names are resolved against
/// the from-schema, so a target colliding with any table the document
/// could carry is refused up front rather than discovered mid-chain.
pub fn validate_spec(spec: &MigrationSpec, schema: &Schema) -> Result<(), String> {
    for step in &spec.steps {
        validate_step(step, schema)?;
    }
    Ok(())
}

fn validate_step(step: &Step, schema: &Schema) -> Result<(), String> {
    let need_table = |schema: &Schema, table: &str| -> Result<(), String> {
        if schema.tables.contains_key(table) {
            Ok(())
        } else {
            Err(format!(
                "{E2002}: {step_desc} — schema has no table `{table}`",
                step_desc = step.describe()
            ))
        }
    };
    let need_field = |schema: &Schema, table: &str, field: &str| -> Result<(), String> {
        need_table(schema, table)?;
        let fields = &schema.tables[table].fields;
        if fields.contains_key(field) {
            Ok(())
        } else {
            Err(format!(
                "{E2002}: {} — table `{table}` has no field `{field}`",
                step.describe()
            ))
        }
    };
    match step {
        Step::RenameField { table, from, to } => {
            need_field(schema, table, from)?;
            if schema.tables[table].fields.contains_key(to) {
                return Err(format!(
                    "{E2002}: {} — table `{table}` already has a field `{to}`",
                    step.describe()
                ));
            }
            Ok(())
        }
        Step::SetDefault { table, field, .. }
        | Step::RemoveField { table, field }
        | Step::WidenType { table, field, .. }
        | Step::RemapValues { table, field, .. } => need_field(schema, table, field),
        Step::RenameTable { from, to } => {
            need_table(schema, from)?;
            if schema.tables.contains_key(to) {
                return Err(format!(
                    "{E2002}: {} — schema already has a table `{to}`",
                    step.describe()
                ));
            }
            Ok(())
        }
    }
}

/// Parse a `migrations/` directory into the version step chain: every
/// `*.yaml` / `*.yml` file, ordered by file name (the chain order), each
/// parsed as one segment (E2001 on any bad file). A missing directory is
/// E2001; an empty directory yields an empty chain — projects simply have
/// nothing to migrate.
pub fn parse_migration_dir(dir: &Path) -> Result<Vec<(String, MigrationSpec)>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        format!(
            "{E2001}: cannot read migrations directory {}: {e}",
            dir.display()
        )
    })?;
    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext == "yml")
        })
        .collect();
    // File-name byte order IS the step chain order — `0001-…` sorts
    // before `0002-…`, which is the whole naming convention.
    files.sort();
    let mut chain = Vec::with_capacity(files.len());
    for name in &files {
        let path = dir.join(name);
        let spec = parse_spec(&path)?;
        chain.push((name.clone(), spec));
    }
    Ok(chain)
}

/// Per-step outcome of an [`apply`] run.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    /// The step, in [`Step::describe`] form
    pub step: String,
    /// Rows touched by this step (table-level steps report the table's
    /// row count)
    pub rows_changed: usize,
    /// Source locations of the changed rows, in table then row (source)
    /// order — the data for cell-level localization, chiefly Excel
    /// sources whose rendered report carries `Sheet`/`Row` addressing.
    /// `rename_table` (a table-level step) reports the whole table's rows,
    /// matching its `rows_changed`. Rows whose bytes did not change are
    /// absent, so a re-run over an already-migrated document reports none
    /// — the same no-op semantics `rows_changed` follows.
    pub affected_locations: Vec<SourceLocation>,
}

/// What one migration segment did to a document.
#[derive(Debug, Clone, PartialEq)]
pub struct MigrateReport {
    /// Schema version the document started at
    pub from: String,
    /// Schema version the document now represents
    pub to: String,
    /// One entry per applied step, in order
    pub steps: Vec<StepReport>,
}

impl MigrateReport {
    /// Total rows touched across all steps (a row may be counted by
    /// several steps — this is an activity measure, not a distinct-row
    /// count).
    pub fn total_rows_changed(&self) -> usize {
        self.steps.iter().map(|s| s.rows_changed).sum()
    }
}

/// Apply one migration segment to a document in place, reporting per-step
/// activity. Steps run in declaration order, each seeing the previous
/// step's output. A step that the data cannot satisfy — a `widen_type`
/// whose direction is not a safe widening, or a value that would not fit
/// the widened type — fails the whole segment (E2003): migration never
/// silently drops or mangles rows.
///
/// `rows_changed` counts rows whose bytes actually changed: re-applying an
/// already-migrated document reports 0 for every step (rename targets that
/// are already in place, defaults already filled, values already remapped
/// are all no-ops), which is what makes `cage migrate --write` idempotent.
///
/// `from_schema` supplies the pre-migration field types for `widen_type`'s
/// direction check; reference validity against it is [`validate_spec`]'s
/// job (run before apply). The CLI loads the current (already-bumped)
/// schema, so a step whose target type equals the schema's type takes the
/// `current == to` path — the direction table has nothing to say about a
/// type that did not move; only the value-domain check runs.
pub fn apply(
    spec: &MigrationSpec,
    doc: &mut Document,
    from_schema: &Schema,
) -> Result<MigrateReport, String> {
    let mut report = MigrateReport {
        from: spec.from.clone(),
        to: spec.to.clone(),
        steps: Vec::with_capacity(spec.steps.len()),
    };
    for step in &spec.steps {
        let mut affected_locations = Vec::new();
        let rows_changed = apply_step(step, doc, from_schema, &mut affected_locations)?;
        report.steps.push(StepReport {
            step: step.describe(),
            rows_changed,
            affected_locations,
        });
    }
    Ok(report)
}

fn apply_step(
    step: &Step,
    doc: &mut Document,
    from_schema: &Schema,
    affected: &mut Vec<SourceLocation>,
) -> Result<usize, String> {
    match step {
        Step::RenameField { table, from, to } => {
            let excel = excel_sourced(doc, table);
            let rows = need_table_rows(doc, table, step)?;
            let mut changed = 0;
            for row in rows {
                // Rows already carrying the target name are a no-op — a
                // second run over a migrated document reports 0.
                if row.fields.contains_key(from) {
                    // Source and target present at once: renaming would
                    // collapse two values into one key (silent data loss).
                    // `validate_spec` catches this against a from-schema
                    // when the caller has one; here it is the guarantee.
                    if row.fields.contains_key(to) {
                        return Err(format!(
                            "{E2003}: {} — row {} already has target field `{to}`",
                            step.describe(),
                            row.index
                        ));
                    }
                    rename_row_field(&mut row.fields, from, to);
                    changed += 1;
                    if excel {
                        affected.push(row.location.clone());
                    }
                }
            }
            Ok(changed)
        }
        Step::SetDefault {
            table,
            field,
            value,
        } => {
            let excel = excel_sourced(doc, table);
            let rows = need_table_rows(doc, table, step)?;
            let mut changed = 0;
            for row in rows {
                match row.fields.get_mut(field) {
                    Some(existing) if existing.value != Value::Null => {} // keep
                    slot => {
                        let location = match slot {
                            Some(existing) => existing.location.clone(),
                            None => row.location.clone(),
                        };
                        let _ = slot; // borrow settled; re-borrow through map
                        row.fields.insert(
                            field.clone(),
                            TypedValue {
                                value: value.clone(),
                                location,
                                schema_type: None,
                            },
                        );
                        changed += 1;
                        if excel {
                            affected.push(row.location.clone());
                        }
                    }
                }
            }
            Ok(changed)
        }
        Step::RemoveField { table, field } => {
            let excel = excel_sourced(doc, table);
            let rows = need_table_rows(doc, table, step)?;
            let mut changed = 0;
            for row in rows {
                if row.fields.shift_remove(field).is_some() {
                    changed += 1;
                    if excel {
                        affected.push(row.location.clone());
                    }
                }
            }
            Ok(changed)
        }
        Step::WidenType { table, field, to } => {
            let current = from_schema
                .tables
                .get(table)
                .and_then(|t| t.fields.get(field))
                .map(|f| &f.field_type)
                .ok_or_else(|| {
                    format!(
                        "{E2003}: {} — from-schema has no type for `{table}.{field}`",
                        step.describe()
                    )
                })?;
            // The direction table only speaks when the type actually moves
            // (`current != to`). The CLI always loads the post-migration
            // schema — rules migrate data, the schema was already edited —
            // so `current == to` is the normal CLI path and would otherwise
            // be misread as a narrowing (X → X is not on the widening list).
            // The value-domain check below runs either way: it is the net
            // that catches a value the schema's own type cannot hold.
            if current != to && !is_widening(current, to) {
                return Err(format!(
                    "{E2003}: {} — {current:?} → {to:?} is not a safe widening",
                    step.describe()
                ));
            }
            let excel = excel_sourced(doc, table);
            let rows = need_table_rows(doc, table, step)?;
            let mut changed = 0;
            for row in rows {
                let slot = row.fields.get_mut(field).ok_or_else(|| {
                    format!(
                        "{E2003}: {} — row {} has no `{field}`",
                        step.describe(),
                        row.index
                    )
                })?;
                let widened = widen_value(&slot.value, to).ok_or_else(|| {
                    format!(
                        "{E2003}: {} — row {} value {} does not fit {to:?}",
                        step.describe(),
                        row.index,
                        slot.value.type_name()
                    )
                })?;
                if widened != slot.value {
                    slot.value = widened;
                    changed += 1;
                    if excel {
                        affected.push(row.location.clone());
                    }
                }
            }
            Ok(changed)
        }
        Step::RemapValues { table, field, map } => {
            let excel = excel_sourced(doc, table);
            let rows = need_table_rows(doc, table, step)?;
            let mut changed = 0;
            for row in rows {
                if let Some(existing) = row.fields.get_mut(field) {
                    if let Value::String(s) = &existing.value {
                        if let Some(new) = map.get(s.as_str()) {
                            existing.value = Value::String(new.clone());
                            changed += 1;
                            if excel {
                                affected.push(row.location.clone());
                            }
                        }
                    }
                }
            }
            Ok(changed)
        }
        Step::RenameTable { from, to } => {
            if !doc.tables.contains_key(from) {
                // Already renamed by a previous run: the target table is in
                // place, nothing to do. Neither name present is still an
                // error — the rule points at a table this document lacks.
                if doc.tables.contains_key(to) {
                    return Ok(0);
                }
                return Err(format!(
                    "{E2003}: {} — document has no table `{from}`",
                    step.describe()
                ));
            }
            // Both names present: the rebuild below would collapse the two
            // tables into one key (silent data loss) — refused.
            if doc.tables.contains_key(to) {
                return Err(format!(
                    "{E2003}: {} — document already has table `{to}`",
                    step.describe()
                ));
            }
            // Position-preserving rename: rebuild the table map with the
            // same order, only the one key changed.
            let rebuilt: IndexMap<String, Table> = doc
                .tables
                .iter()
                .map(|(name, table)| {
                    if name == from {
                        let mut renamed = table.clone();
                        renamed.name.clone_from(to);
                        (to.clone(), renamed)
                    } else {
                        (name.clone(), table.clone())
                    }
                })
                .collect();
            doc.tables = rebuilt;
            let renamed = doc.tables.get(to).expect("just inserted");
            if excel_sourced(doc, to) {
                affected.extend(renamed.rows.iter().map(|row| row.location.clone()));
            }
            Ok(renamed.rows.len())
        }
    }
}

/// Whether the named table is Excel-served (its source file carries an
/// `.xlsx` / `.xls` extension) — the only sources `cage migrate` never
/// rewrites, so per-row locations are the only record of which cells a
/// step moved. Read before the table's rows are borrowed for mutation.
fn excel_sourced(doc: &Document, table: &str) -> bool {
    doc.tables
        .get(table)
        .is_some_and(|t| is_excel_source(&t.source_file))
}

/// Excel-served source file: `.xlsx` or `.xls`, case-insensitive.
fn is_excel_source(source_file: &str) -> bool {
    Path::new(source_file)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx") || ext.eq_ignore_ascii_case("xls"))
}

/// Borrow every row of a table for mutation, E2003 if the document has no
/// such table.
fn need_table_rows<'a>(
    doc: &'a mut Document,
    table: &str,
    step: &Step,
) -> Result<impl Iterator<Item = &'a mut Row> + 'a, String> {
    doc.tables
        .get_mut(table)
        .map(|t| t.rows.iter_mut())
        .ok_or_else(|| {
            format!(
                "{E2003}: {} — document has no table `{table}`",
                step.describe()
            )
        })
}

/// Rename one row field in place, preserving field order.
fn rename_row_field(fields: &mut IndexMap<String, TypedValue>, from: &str, to: &str) {
    let rebuilt: IndexMap<String, TypedValue> = fields
        .iter()
        .map(|(name, value)| {
            if name == from {
                (to.to_string(), value.clone())
            } else {
                (name.clone(), value.clone())
            }
        })
        .collect();
    *fields = rebuilt;
}

/// Safe widening directions: integer ranks only grow, unsigned → signed
/// only when every value of the source type fits the target, and the
/// float step is Float32 → Float64 only (Int64/UInt64 → Float64 loses
/// precision above 2^53 and is refused). Everything else — narrowing,
/// cross-family moves, string/bool/bytes — is not a widening.
fn is_widening(from: &FieldType, to: &FieldType) -> bool {
    use FieldType as F;
    let int_rank = |t: &FieldType| match t {
        F::Int8 => Some(1u8),
        F::Int16 => Some(2),
        F::Int32 => Some(3),
        F::Int64 => Some(4),
        _ => None,
    };
    let uint_rank = |t: &FieldType| match t {
        F::UInt8 => Some(1u8),
        F::UInt16 => Some(2),
        F::UInt32 => Some(3),
        F::UInt64 => Some(4),
        _ => None,
    };
    match (from, to) {
        (f, t) if int_rank(f).is_some() && int_rank(t).is_some() => int_rank(t) > int_rank(f),
        (f, t) if uint_rank(f).is_some() && uint_rank(t).is_some() => uint_rank(t) > uint_rank(f),
        // unsigned → signed: 8→16, 16→32, 32→64 (one signed step per
        // unsigned rank keeps every value representable); every ≤32-bit
        // integer and Float32 fit Float64 exactly (2^53).
        (F::UInt8, F::Int16 | F::Int32 | F::Int64)
        | (F::UInt16, F::Int32 | F::Int64)
        | (F::UInt32, F::Int64)
        | (
            F::Int8 | F::Int16 | F::Int32 | F::UInt8 | F::UInt16 | F::UInt32 | F::Float32,
            F::Float64,
        ) => true,
        _ => false,
    }
}

/// Move one row value into the widened type's canonical representation.
/// Values keep their payload where the representation already matches
/// (Int stays Int); unsigned values moving into a signed type become
/// Int. Returns `None` when the value would not fit.
fn widen_value(value: &Value, to: &FieldType) -> Option<Value> {
    use FieldType as F;
    let fits_int = |i: i64, to: &F| match to {
        F::Int8 => i8::try_from(i).is_ok(),
        F::Int16 => i16::try_from(i).is_ok(),
        F::Int32 => i32::try_from(i).is_ok(),
        F::Int64 => true,
        _ => false,
    };
    let fits_uint = |u: u64, to: &F| match to {
        F::UInt8 => u8::try_from(u).is_ok(),
        F::UInt16 => u16::try_from(u).is_ok(),
        F::UInt32 => u32::try_from(u).is_ok(),
        F::UInt64 => true,
        _ => false,
    };
    match (value, to) {
        (Value::Null, _) => Some(Value::Null),
        (Value::Int(i), t) if fits_int(*i, t) => Some(Value::Int(*i)),
        (Value::UInt(u), F::Int8 | F::Int16 | F::Int32 | F::Int64) => {
            i64::try_from(*u).ok().map(Value::Int)
        }
        (Value::UInt(u), t) if fits_uint(*u, t) => Some(Value::UInt(*u)),
        (Value::Float(f), F::Float64) => Some(Value::Float(*f)),
        (Value::Int(_) | Value::UInt(_), F::Float64) => Some(value.clone()),
        _ => None,
    }
}

/// Re-check a migrated document under the new schema (E2004): the full
/// validation stack through L6 semantic must come back clean — a
/// migration that leaves the document failing validation is not done.
/// `GameRule` (L7) is out of scope here: it needs the plugin registry and
/// is run by the CLI check path after `--write`.
pub fn reverify(doc: &Document, schema: &Schema) -> Result<(), String> {
    let validated = crate::schema::ValidatedSchema {
        schema: schema.clone(),
        dependency_graph: crate::reference::DependencyGraph::from_schema(schema),
    };
    let diagnostics = crate::validation::validate(
        &validated,
        doc,
        crate::validation::ValidationLevel::Semantic,
        false,
    );
    if diagnostics.has_errors() {
        return Err(format!(
            "{E2004}: migrated document fails validation under schema `{}`\n{}",
            schema
                .metadata
                .as_ref()
                .map_or("(unversioned)", |m| m.version.as_str()),
            diagnostics.render(false)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_to_value_decodes_the_canonical_tagged_form() {
        // A bare unsigned scalar reads back Int; the tagged form keeps
        // the family, including nested inside plain structures.
        let tagged: serde_yaml::Value = serde_yaml::from_str("type: UInt\nvalue: 5").unwrap();
        assert_eq!(yaml_to_value(tagged).unwrap(), Value::UInt(5));

        let nested: serde_yaml::Value =
            serde_yaml::from_str("- 1\n- type: UInt\n  value: 7").unwrap();
        assert_eq!(
            yaml_to_value(nested).unwrap(),
            Value::Array(vec![Value::Int(1), Value::UInt(7)])
        );

        // A mapping that is not a valid tagged value keeps the plain
        // reading (Object), even with type/value-looking keys.
        let plain: serde_yaml::Value = serde_yaml::from_str("type: NotAVariant\nvalue: 5").unwrap();
        let mut expected = IndexMap::new();
        expected.insert("type".to_string(), Value::String("NotAVariant".into()));
        expected.insert("value".to_string(), Value::Int(5));
        assert_eq!(yaml_to_value(plain).unwrap(), Value::Object(expected));

        // Plain scalars keep the historical i64-first reading.
        let bare: serde_yaml::Value = serde_yaml::from_str("5").unwrap();
        assert_eq!(yaml_to_value(bare).unwrap(), Value::Int(5));
    }

    fn schema_with_table(name: &str, fields: &[&str]) -> Schema {
        let mut field_map = IndexMap::new();
        for field in fields {
            field_map.insert(
                (*field).to_string(),
                crate::schema::FieldSchema {
                    name: (*field).to_string(),
                    field_type: FieldType::String,
                    description: None,
                    required: false,
                    default: None,
                    min: None,
                    max: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                    enum_values: None,
                    min_items: None,
                    max_items: None,
                    items: None,
                    properties: None,
                    additional_properties: None,
                    reference: None,
                    targets: Vec::new(),
                    rules: Vec::new(),
                    metadata: IndexMap::new(),
                },
            );
        }
        let mut tables = IndexMap::new();
        tables.insert(
            name.to_string(),
            crate::schema::TableSchema {
                name: name.to_string(),
                description: None,
                primary_key: vec![fields.first().unwrap_or(&"id").to_string()],
                fields: field_map,
                unique_constraints: Vec::new(),
                order_by: None,
                targets: Vec::new(),
                env_overrides: IndexMap::new(),
            },
        );
        Schema {
            tables,
            enums: IndexMap::new(),
            metadata: None,
        }
    }

    const SPEC_YAML: &str = r#"
from: "1.0.0"
to: "1.1.0"
steps:
  - rename_field:
      table: Item
      from: name
      to: title
  - set_default:
      table: Item
      field: rarity
      value: "common"
  - remove_field:
      table: Item
      field: legacy_id
  - widen_type:
      table: Item
      field: level
      to: { kind: Int64 }
  - remap_values:
      table: Item
      field: grade
      map:
        "S": "legendary"
        "A": "epic"
  - rename_table:
      from: Mob
      to: Monster
"#;

    #[test]
    fn parse_spec_reads_all_six_step_kinds_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("0001-rename.yaml");
        std::fs::write(&file, SPEC_YAML).unwrap();
        let spec = parse_spec(&file).unwrap();
        assert_eq!(spec.from, "1.0.0");
        assert_eq!(spec.to, "1.1.0");
        assert_eq!(spec.steps.len(), 6);
        assert_eq!(
            spec.steps[0],
            Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "title".into()
            }
        );
        assert_eq!(
            spec.steps[1],
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: Value::String("common".into())
            }
        );
        assert_eq!(
            spec.steps[2],
            Step::RemoveField {
                table: "Item".into(),
                field: "legacy_id".into()
            }
        );
        assert_eq!(
            spec.steps[3],
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64
            }
        );
        assert_eq!(
            spec.steps[4],
            Step::RemapValues {
                table: "Item".into(),
                field: "grade".into(),
                map: IndexMap::from([
                    ("S".to_string(), "legendary".to_string()),
                    ("A".to_string(), "epic".to_string()),
                ])
            }
        );
        assert_eq!(
            spec.steps[5],
            Step::RenameTable {
                from: "Mob".into(),
                to: "Monster".into()
            }
        );
    }

    #[test]
    fn parse_migration_dir_orders_segments_by_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let later = r#"
from: "1.1.0"
to: "2.0.0"
steps:
  - remove_field:
      table: Item
      field: legacy_id
"#;
        std::fs::write(dir.path().join("0002-second.yaml"), later).unwrap();
        std::fs::write(dir.path().join("0001-first.yaml"), SPEC_YAML).unwrap();
        std::fs::write(dir.path().join("README.md"), "not a migration").unwrap();
        let chain = parse_migration_dir(dir.path()).unwrap();
        let names: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["0001-first.yaml", "0002-second.yaml"]);
        assert_eq!(chain[0].1.from, "1.0.0");
        assert_eq!(chain[1].1.from, "1.1.0");
    }

    #[test]
    fn parse_spec_reports_structural_problems_as_e2001() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let out = parse_spec(&root.join("missing.yaml")).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let bad_yaml = root.join("bad.yaml");
        std::fs::write(&bad_yaml, "from: [unclosed").unwrap();
        let out = parse_spec(&bad_yaml).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let no_steps = root.join("no-steps.yaml");
        std::fs::write(&no_steps, "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps: []\n").unwrap();
        let out = parse_spec(&no_steps).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let unknown = root.join("unknown-step.yaml");
        std::fs::write(
            &unknown,
            "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps:\n  - drop_database: { table: Item }\n",
        )
        .unwrap();
        let out = parse_spec(&unknown).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let no_to = root.join("no-to.yaml");
        std::fs::write(
            &no_to,
            "from: \"1.0.0\"\nsteps:\n  - remove_field: { table: Item, field: x }\n",
        )
        .unwrap();
        let out = parse_spec(&no_to).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let self_loop = root.join("self-loop.yaml");
        std::fs::write(
            &self_loop,
            "from: \"1.0.0\"\nto: \"1.0.0\"\nsteps:\n  - remove_field: { table: Item, field: x }\n",
        )
        .unwrap();
        let out = parse_spec(&self_loop).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");
    }

    #[test]
    fn validate_spec_checks_table_and_field_references_as_e2002() {
        let schema = schema_with_table("Item", &["id", "name"]);

        // Table does not exist.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RemoveField {
                table: "Weapon".into(),
                field: "id".into(),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");
        assert!(out.contains("Weapon"), "{out}");

        // Field does not exist.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: Value::String("common".into()),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");
        assert!(out.contains("rarity"), "{out}");

        // Rename collides with an existing field.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "id".into(),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");

        // rename_table to a free name is fine; to an existing one is not.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameTable {
                from: "Item".into(),
                to: "Weapon".into(),
            }],
        };
        assert!(validate_spec(&spec, &schema).is_ok());
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameTable {
                from: "Item".into(),
                to: "Item".into(),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");

        // A clean spec passes.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![
                Step::RenameField {
                    table: "Item".into(),
                    from: "name".into(),
                    to: "title".into(),
                },
                Step::RenameTable {
                    from: "Item".into(),
                    to: "Artifact".into(),
                },
            ],
        };
        assert!(validate_spec(&spec, &schema).is_ok());
    }

    use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value as V};

    fn tv(value: V) -> TypedValue {
        TypedValue {
            value,
            location: SourceLocation::new("config/item.json"),
            schema_type: None,
        }
    }

    fn make_row(index: usize, fields: Vec<(&str, V)>) -> Row {
        Row {
            primary_key: vec![],
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_string(), tv(value)))
                .collect(),
            location: SourceLocation::new("config/item.json").with_row(index + 1),
            index,
        }
    }

    fn make_document() -> Document {
        let items = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![
                make_row(
                    0,
                    vec![
                        ("id", V::Int(1)),
                        ("name", V::String("Sword".to_string())),
                        ("legacy_id", V::String("SW-1".to_string())),
                        ("level", V::Int(10)),
                        ("grade", V::String("S".to_string())),
                    ],
                ),
                make_row(
                    1,
                    vec![
                        ("id", V::Int(2)),
                        ("name", V::String("Shield".to_string())),
                        ("legacy_id", V::String("SH-1".to_string())),
                        ("level", V::Int(20)),
                        ("grade", V::String("A".to_string())),
                        ("rarity", V::String("rare".to_string())),
                    ],
                ),
            ],
            source_file: "config/item.json".to_string(),
            sheet: None,
        };
        let mobs = Table {
            name: "Mob".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![make_row(
                0,
                vec![("id", V::Int(1)), ("name", V::String("Slime".to_string()))],
            )],
            source_file: "config/mob.json".to_string(),
            sheet: None,
        };
        let mut tables = IndexMap::new();
        tables.insert("Item".to_string(), items);
        tables.insert("Mob".to_string(), mobs);
        Document {
            tables,
            source_files: vec!["config/item.json".to_string()],
            metadata: crate::value::DocumentMetadata::default(),
        }
    }

    fn full_spec() -> MigrationSpec {
        MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![
                Step::RenameField {
                    table: "Item".into(),
                    from: "name".into(),
                    to: "title".into(),
                },
                Step::SetDefault {
                    table: "Item".into(),
                    field: "rarity".into(),
                    value: V::String("common".into()),
                },
                Step::RemoveField {
                    table: "Item".into(),
                    field: "legacy_id".into(),
                },
                Step::WidenType {
                    table: "Item".into(),
                    field: "level".into(),
                    to: FieldType::Int64,
                },
                Step::RemapValues {
                    table: "Item".into(),
                    field: "grade".into(),
                    map: IndexMap::from([("S".to_string(), "legendary".to_string())]),
                },
                Step::RenameTable {
                    from: "Mob".into(),
                    to: "Monster".into(),
                },
            ],
        }
    }

    fn schema_for_document(version: &str) -> Schema {
        let mut schema = schema_with_table("Item", &[]);
        // Rebuild Item with the full from-schema field set.
        let fields: Vec<(&str, FieldType)> = vec![
            ("id", FieldType::Int32),
            ("name", FieldType::String),
            ("legacy_id", FieldType::String),
            ("level", FieldType::Int32),
            ("grade", FieldType::String),
        ];
        let mut field_map = IndexMap::new();
        for (name, field_type) in fields {
            field_map.insert(
                name.to_string(),
                crate::schema::FieldSchema {
                    name: name.to_string(),
                    field_type,
                    description: None,
                    required: false,
                    default: None,
                    min: None,
                    max: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                    enum_values: None,
                    min_items: None,
                    max_items: None,
                    items: None,
                    properties: None,
                    additional_properties: None,
                    reference: None,
                    targets: Vec::new(),
                    rules: Vec::new(),
                    metadata: IndexMap::new(),
                },
            );
        }
        schema.tables.get_mut("Item").unwrap().fields = field_map;
        schema.tables.insert(
            "Mob".to_string(),
            crate::schema::TableSchema {
                name: "Mob".to_string(),
                description: None,
                primary_key: vec!["id".to_string()],
                fields: {
                    let mut m = IndexMap::new();
                    for name in ["id", "name"] {
                        m.insert(
                            name.to_string(),
                            crate::schema::FieldSchema {
                                name: name.to_string(),
                                field_type: if name == "id" {
                                    FieldType::Int32
                                } else {
                                    FieldType::String
                                },
                                description: None,
                                required: false,
                                default: None,
                                min: None,
                                max: None,
                                min_length: None,
                                max_length: None,
                                pattern: None,
                                enum_values: None,
                                min_items: None,
                                max_items: None,
                                items: None,
                                properties: None,
                                additional_properties: None,
                                reference: None,
                                targets: Vec::new(),
                                rules: Vec::new(),
                                metadata: IndexMap::new(),
                            },
                        );
                    }
                    m
                },
                unique_constraints: Vec::new(),
                order_by: None,
                targets: Vec::new(),
                env_overrides: IndexMap::new(),
            },
        );
        schema.metadata = Some(crate::schema::SchemaMetadata {
            version: version.to_string(),
            description: None,
            author: None,
        });
        schema
    }

    #[test]
    fn apply_runs_all_six_step_semantics() {
        let mut doc = make_document();
        let from_schema = schema_for_document("1.0.0");
        let report = apply(&full_spec(), &mut doc, &from_schema).unwrap();

        // rename_field: every row renamed, order preserved (title stays
        // where name was).
        let items = &doc.tables["Item"];
        for row in &items.rows {
            let keys: Vec<&String> = row.fields.keys().collect();
            assert!(!keys.iter().any(|k| k.as_str() == "name"), "{keys:?}");
        }
        assert_eq!(
            items.rows[0].fields.keys().nth(1).unwrap(),
            "title",
            "renamed field keeps its position"
        );
        assert_eq!(
            items.rows[0].fields["title"].value,
            V::String("Sword".to_string())
        );

        // set_default: fills only rows where the field is absent; an
        // existing non-null value survives.
        assert_eq!(
            items.rows[0].fields["rarity"].value,
            V::String("common".to_string())
        );
        assert_eq!(
            items.rows[1].fields["rarity"].value,
            V::String("rare".to_string()),
            "existing value is kept"
        );

        // remove_field: gone from every row.
        assert!(items
            .rows
            .iter()
            .all(|r| !r.fields.contains_key("legacy_id")));

        // widen_type: values still present, representation unchanged.
        assert_eq!(items.rows[0].fields["level"].value, V::Int(10));

        // remap_values: mapped values rewritten, unmapped untouched.
        assert_eq!(
            items.rows[0].fields["grade"].value,
            V::String("legendary".to_string())
        );
        assert_eq!(
            items.rows[1].fields["grade"].value,
            V::String("A".to_string()),
            "unmapped value passes through"
        );

        // rename_table: new name carries the same rows, old name gone.
        assert!(!doc.tables.contains_key("Mob"));
        assert_eq!(doc.tables["Monster"].rows.len(), 1);
        assert_eq!(
            doc.tables.keys().take(2).cloned().collect::<Vec<_>>(),
            vec!["Item", "Monster"],
            "renamed table keeps its position"
        );

        // Report: one entry per step in order.
        assert_eq!(report.from, "1.0.0");
        assert_eq!(report.to, "1.1.0");
        assert_eq!(report.steps.len(), 6);
        assert_eq!(report.steps[0].rows_changed, 2, "both items renamed");
        assert_eq!(
            report.steps[1].rows_changed, 1,
            "only the row missing rarity"
        );
        assert_eq!(report.steps[2].rows_changed, 2);
        assert_eq!(report.steps[4].rows_changed, 1, "only S is mapped");
        assert_eq!(report.steps[5].rows_changed, 1, "the single mob row");

        // The document's tables are text-sourced: no per-row locations are
        // recorded for them (their rewritten file is the change record).
        assert!(report.steps.iter().all(|s| s.affected_locations.is_empty()));
    }

    /// An Item table served from an Excel workbook — sheet set, row
    /// locations carrying sheet-grid row numbers (header on row 1).
    fn excel_document() -> Document {
        let mut items = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: Vec::new(),
            source_file: "config/items.xlsx".to_string(),
            sheet: Some("Items".to_string()),
        };
        for (index, (id, grade)) in [(1, "S"), (2, "A"), (3, "B")].iter().enumerate() {
            items.rows.push(Row {
                primary_key: vec![V::Int(*id)],
                fields: [
                    ("id", V::Int(*id)),
                    ("grade", V::String((*grade).to_string())),
                ]
                .into_iter()
                .map(|(name, value)| (name.to_string(), tv(value)))
                .collect(),
                location: SourceLocation::new("config/items.xlsx")
                    .with_sheet("Items")
                    .with_row(index + 2),
                index,
            });
        }
        let mut tables = IndexMap::new();
        tables.insert("Item".to_string(), items);
        Document {
            tables,
            source_files: vec!["config/items.xlsx".to_string()],
            metadata: crate::value::DocumentMetadata::default(),
        }
    }

    #[test]
    fn excel_sourced_steps_record_affected_row_locations() {
        let mut doc = excel_document();
        let from_schema = schema_for_document("1.0.0");
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![
                Step::RemapValues {
                    table: "Item".into(),
                    field: "grade".into(),
                    map: IndexMap::from([
                        ("S".to_string(), "legendary".to_string()),
                        ("A".to_string(), "epic".to_string()),
                    ]),
                },
                Step::SetDefault {
                    table: "Item".into(),
                    field: "rarity".into(),
                    value: V::String("common".into()),
                },
            ],
        };
        let report = apply(&spec, &mut doc, &from_schema).unwrap();

        // Remap touched rows 1 and 2 (sheet rows 2 and 3) — in source
        // order, sheet-grid addressed, the step's field carried by the
        // step name.
        assert_eq!(report.steps[0].rows_changed, 2);
        assert_eq!(
            report.steps[0]
                .affected_locations
                .iter()
                .map(SourceLocation::to_string)
                .collect::<Vec<_>>(),
            vec![
                "config/items.xlsx | Sheet: Items | Row: 2",
                "config/items.xlsx | Sheet: Items | Row: 3",
            ]
        );
        // set_default filled all three rows.
        assert_eq!(report.steps[1].rows_changed, 3);
        assert_eq!(
            report.steps[1]
                .affected_locations
                .iter()
                .map(SourceLocation::to_string)
                .collect::<Vec<_>>(),
            vec![
                "config/items.xlsx | Sheet: Items | Row: 2",
                "config/items.xlsx | Sheet: Items | Row: 3",
                "config/items.xlsx | Sheet: Items | Row: 4",
            ]
        );
    }

    #[test]
    fn excel_remap_run_twice_records_no_locations_and_no_rows() {
        let mut doc = excel_document();
        let from_schema = schema_for_document("1.0.0");
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RemapValues {
                table: "Item".into(),
                field: "grade".into(),
                map: IndexMap::from([("S".to_string(), "legendary".to_string())]),
            }],
        };
        let first = apply(&spec, &mut doc, &from_schema).unwrap();
        assert_eq!(first.steps[0].rows_changed, 1);
        assert_eq!(first.steps[0].affected_locations.len(), 1);

        // Second run: nothing changed, so no rows and no locations — the
        // no-op semantics cover the location record too.
        let again = apply(&spec, &mut doc, &from_schema).unwrap();
        assert_eq!(again.steps[0].rows_changed, 0);
        assert_eq!(again.steps[0].affected_locations, [] as [SourceLocation; 0]);
    }

    #[test]
    fn rename_table_reports_whole_excel_table_and_text_tables_stay_location_free() {
        let mut doc = excel_document();
        // A second, text-sourced table sharing the workbook migration.
        let mut logs = Table {
            name: "Log".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![make_row(0, vec![("id", V::Int(1))])],
            source_file: "config/logs.json".to_string(),
            sheet: None,
        };
        logs.rows[0].location = SourceLocation::new("config/logs.json").with_row(1);
        doc.tables.insert("Log".to_string(), logs);

        let from_schema = schema_for_document("1.0.0");
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![
                Step::RenameTable {
                    from: "Item".into(),
                    to: "Goods".into(),
                },
                Step::RenameTable {
                    from: "Log".into(),
                    to: "Event".into(),
                },
            ],
        };
        let report = apply(&spec, &mut doc, &from_schema).unwrap();

        // Table-level steps count every row and locate every row — for
        // the Excel table only; the text-sourced table stays file-level.
        assert_eq!(report.steps[0].rows_changed, 3);
        assert_eq!(
            report.steps[0]
                .affected_locations
                .iter()
                .map(SourceLocation::to_string)
                .collect::<Vec<_>>(),
            vec![
                "config/items.xlsx | Sheet: Items | Row: 2",
                "config/items.xlsx | Sheet: Items | Row: 3",
                "config/items.xlsx | Sheet: Items | Row: 4",
            ]
        );
        assert_eq!(report.steps[1].rows_changed, 1);
        assert_eq!(
            report.steps[1].affected_locations,
            [] as [SourceLocation; 0]
        );
    }

    #[test]
    fn widen_type_rejects_unsafe_directions_and_unfit_values_as_e2003() {
        let mut doc = make_document();
        let from_schema = schema_for_document("1.0.0");

        // Int64 → Float32 is not a widening.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Float32,
            }],
        };
        let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
        assert!(out.contains("not a safe widening"), "{out}");

        // UInt8 → Int8 loses half the range — refused.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::WidenType {
                table: "Item".into(),
                field: "id".into(),
                to: FieldType::Int8,
            }],
        };
        let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");

        // A value outside the widened domain fails the segment without
        // touching the document: a rogue Int16 column carrying a value
        // beyond Int32's range (the schema type has been narrowed here to
        // make the row value provably out-of-domain for the target).
        let mut bad = make_document();
        bad.tables["Item"].rows[0].fields["level"].value = V::Int(5_000_000_000);
        let mut int16_schema = from_schema.clone();
        int16_schema.tables["Item"].fields["level"].field_type = FieldType::Int16;
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int32,
            }],
        };
        let out = apply(&spec, &mut bad, &int16_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
        assert!(out.contains("does not fit"), "{out}");

        // Missing field on a row.
        let mut short = make_document();
        short.tables["Item"].rows[1].fields.shift_remove("level");
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64,
            }],
        };
        let out = apply(&spec, &mut short, &from_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
        assert!(out.contains("has no `level`"), "{out}");

        // The safe direction on clean data succeeds: legal widenings run.
        let mut doc2 = make_document();
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::WidenType {
                table: "Item".into(),
                field: "id".into(),
                to: FieldType::Int64,
            }],
        };
        let report = apply(&spec, &mut doc2, &from_schema).unwrap();
        // rows_changed counts rows whose bytes moved: canonical Int has no
        // width, so Int32 → Int64 rewrites no value — 0 changed, value
        // intact.
        assert_eq!(report.steps[0].rows_changed, 0);
        assert_eq!(doc2.tables["Item"].rows[0].fields["id"].value, V::Int(1));

        // A document without the table fails as E2003 too.
        let mut empty = Document::new();
        let out = apply(&full_spec(), &mut empty, &from_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
    }

    #[test]
    fn reverify_passes_clean_and_fails_as_e2004() {
        let from_schema = schema_for_document("1.0.0");

        // Migrate a document, then build the to-schema (the migrated
        // shape: title / rarity / no legacy_id / level Int64) and check
        // it comes back clean.
        let mut doc = make_document();
        apply(&full_spec(), &mut doc, &from_schema).unwrap();
        let mut to_schema = schema_for_document("1.1.0");
        {
            let item = to_schema.tables.get_mut("Item").unwrap();
            // Rebuild fields to the migrated shape; `rarity` is required —
            // set_default in the spec is what makes the migrated rows
            // satisfy it, so the migrated document passes while the
            // pre-migration one (row 0 lacks rarity) must fail.
            let mut fields = IndexMap::new();
            for (name, field_type, required) in [
                ("id", FieldType::Int32, false),
                ("title", FieldType::String, false),
                ("level", FieldType::Int64, false),
                ("grade", FieldType::String, false),
                ("rarity", FieldType::String, true),
            ] {
                fields.insert(
                    name.to_string(),
                    crate::schema::FieldSchema {
                        name: name.to_string(),
                        field_type,
                        description: None,
                        required,
                        default: None,
                        min: None,
                        max: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                        enum_values: None,
                        min_items: None,
                        max_items: None,
                        items: None,
                        properties: None,
                        additional_properties: None,
                        reference: None,
                        targets: Vec::new(),
                        rules: Vec::new(),
                        metadata: IndexMap::new(),
                    },
                );
            }
            item.fields = fields;
        }
        to_schema
            .tables
            .insert("Monster".to_string(), to_schema.tables["Mob"].clone());
        to_schema.tables.shift_remove("Mob");
        assert!(reverify(&doc, &to_schema).is_ok());

        // Re-verifying the ORIGINAL document under the NEW schema fails
        // as E2004 — rows still carry `name`, which the new schema does
        // not know, and the old Mob table still exists.
        let original = make_document();
        let out = reverify(&original, &to_schema).unwrap_err();
        assert!(out.starts_with("E2004"), "{out}");
    }

    #[test]
    fn apply_is_deterministic_two_runs_identical() {
        let from_schema = schema_for_document("1.0.0");
        let mut first = make_document();
        let mut second = make_document();
        apply(&full_spec(), &mut first, &from_schema).unwrap();
        apply(&full_spec(), &mut second, &from_schema).unwrap();
        let a = serde_json::to_string(&first).unwrap();
        let b = serde_json::to_string(&second).unwrap();
        assert_eq!(a, b, "same rules over same data = same document");
    }

    #[test]
    fn apply_over_migrated_document_is_a_zero_change_noop() {
        // Second run over already-migrated data: every step finds its
        // target already in place (renamed fields, filled defaults, removed
        // fields, remapped values, renamed table) — 0 rows changed, bytes
        // untouched. This is the guarantee `cage migrate --write` leans on
        // for idempotence.
        let from_schema = schema_for_document("1.0.0");
        let mut doc = make_document();
        apply(&full_spec(), &mut doc, &from_schema).unwrap();
        let before = serde_json::to_string(&doc).unwrap();

        let again = apply(&full_spec(), &mut doc, &from_schema).unwrap();
        assert_eq!(
            again.total_rows_changed(),
            0,
            "every step is a no-op over migrated data: {:?}",
            again.steps
        );
        let after = serde_json::to_string(&doc).unwrap();
        assert_eq!(before, after, "second run writes nothing");
    }

    #[test]
    fn rename_collisions_fail_as_e2003_instead_of_collapsing() {
        // The CLI validates against the post-migration schema only (no
        // historical from-schema on disk), so `validate_spec` cannot run
        // there — the apply layer refuses a rename whose target is already
        // taken rather than collapsing two values into one key.
        let from_schema = schema_for_document("1.0.0");

        // Field collision: both `name` and `title` present on a row.
        let mut doc = make_document();
        doc.tables["Item"].rows[0]
            .fields
            .insert("title".into(), tv(V::String("Already".into())));
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "title".into(),
            }],
        };
        let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
        assert!(out.contains("already has target field `title`"), "{out}");

        // Table collision: `Monster` exists alongside `Mob`.
        let mut doc = make_document();
        doc.tables
            .insert("Monster".into(), doc.tables["Mob"].clone());
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameTable {
                from: "Mob".into(),
                to: "Monster".into(),
            }],
        };
        let out = apply(&spec, &mut doc, &from_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
        assert!(out.contains("already has table `Monster`"), "{out}");
    }

    #[test]
    fn widen_type_current_equals_to_skips_direction_table_keeps_domain_check() {
        // CLI shape: the loaded schema is already the post-migration one,
        // so the schema's type equals the step's target (`current == to`).
        // The direction table would refuse X → X as a non-widening; the
        // dual path skips it and still runs the value-domain net.
        let mut new_schema = schema_for_document("1.0.0");
        new_schema.tables["Item"].fields["level"].field_type = FieldType::Int64;
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64,
            }],
        };

        // Clean values: direction not consulted, domain holds, bytes
        // unchanged → 0 rows changed.
        let mut doc = make_document();
        let report = apply(&spec, &mut doc, &new_schema).unwrap();
        assert_eq!(report.steps[0].rows_changed, 0);

        // A value the target type cannot hold still fails as E2003 — the
        // domain check is the safety net on this path too.
        let mut bad = make_document();
        bad.tables["Item"].rows[0].fields["level"].value = V::String("ten".into());
        let out = apply(&spec, &mut bad, &new_schema).unwrap_err();
        assert!(out.starts_with("E2003"), "{out}");
        assert!(out.contains("does not fit"), "{out}");
    }
}
