//! Rule execution and post-migration reverification (split out of
//! mod.rs, LAY-007): [`apply`] walks the step chain over a document,
//! [`reverify`] re-runs validation so a broken migration is refused.

use super::{MigrationSpec, Step};
use crate::error::codes::migration::{E2003, E2004};
use crate::schema::{FieldType, Schema};
use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
use indexmap::IndexMap;
use std::path::Path;

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
pub(crate) fn is_widening(from: &FieldType, to: &FieldType) -> bool {
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
pub(crate) fn widen_value(value: &Value, to: &FieldType) -> Option<Value> {
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
