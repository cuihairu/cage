//! Rule-file parsing and schema-reference validation (split out of
//! mod.rs, LAY-007): `parse_spec` / `parse_migration_dir` read rule
//! files into [`MigrationSpec`] values, `validate_spec` checks every
//! step against the from-schema.

use super::{MigrationSpec, Step};
use crate::error::codes::migration::{E2001, E2002};
use crate::schema::Schema;
use std::path::Path;

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

pub(crate) fn validate_step(step: &Step, schema: &Schema) -> Result<(), String> {
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
