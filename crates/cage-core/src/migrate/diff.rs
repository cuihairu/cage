//! Schema diff → migration rule draft (design §46 deferred item).
//!
//! Given two schema documents (a *from* schema and a *to* schema), derive
//! what a `migrations/` rule moving data between them must contain. The
//! draft is deliberately conservative — a structural diff cannot tell a
//! rename from a remove-plus-add, so renames and enum-value remaps are
//! surfaced as `# TODO` comments for the author, never guessed:
//!
//! * mechanically safe transforms become real steps: `set_default` (a new
//!   or newly-required field whose to-schema declaration carries a
//!   default), `remove_field` (present in from, absent in to),
//!   `widen_type` (a type change the safe-widening direction table
//!   accepts — everything else is a TODO, apply-time E2003 would reject
//!   it anyway);
//! * ambiguous cases become `# TODO` notes: removed tables (migration has
//!   no table-drop step), new tables, new required fields without a
//!   default, non-widening type changes, inline-enum member set changes.
//!
//! The rendered file is meant for the author to finish (confirm TODOs
//! into steps, fill `from`/`to`), then hand to `cage migrate` — which
//! always dry-runs first, so a half-reviewed draft cannot touch data.

use super::Step;
use crate::schema::{FieldSchema, FieldType, Schema};
use std::fmt::Write as _;

use draft::{diff_table_fields, draft_field};

/// One field-level difference between two schemas.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldDelta {
    /// Present in *from*, absent in *to*.
    Removed,
    /// Absent in *from*, present in *to*.
    Added,
    /// Present in both with a different canonical type.
    Retyped {
        /// Type in the from-schema
        from: FieldType,
        /// Type in the to-schema
        to: FieldType,
    },
    /// Present in both, `required` flipped.
    RequiredChanged {
        /// `required` in the from-schema
        from: bool,
        /// `required` in the to-schema
        to: bool,
    },
    /// Present in both, the inline enum member set changed.
    EnumMembersChanged {
        /// Members only in the from-schema
        removed: Vec<String>,
        /// Members only in the to-schema
        added: Vec<String>,
    },
}

/// A field's delta plus the schema data needed to draft a step.
#[derive(Debug, Clone)]
pub struct FieldDiff {
    /// Field name (same in both schemas — renames are not inferred).
    pub name: String,
    /// What changed.
    pub delta: FieldDelta,
    /// The field as declared in *from* (absent when `Added`).
    pub from: Option<FieldSchema>,
    /// The field as declared in *to* (absent when `Removed`).
    pub to: Option<FieldSchema>,
}

/// A table-level difference.
#[derive(Debug, Clone)]
pub struct TableDiff {
    /// Table name.
    pub name: String,
    /// Whether the table exists only in *from*.
    pub removed: bool,
    /// Whether the table exists only in *to*.
    pub added: bool,
    /// Field-level differences (empty for added/removed tables).
    pub fields: Vec<FieldDiff>,
}

/// The full structural diff between two schemas.
#[derive(Debug, Clone)]
pub struct SchemaDiff {
    /// Differences grouped by table, in *from* order then *to*-only tables.
    pub tables: Vec<TableDiff>,
}

/// The output of a draft: mechanically safe steps plus the ambiguous
/// cases the author has to confirm.
#[derive(Debug, Clone, Default)]
pub struct MigrationDraft {
    /// Steps that are mechanically safe (`set_default` / `remove_field` /
    /// `widen_type`), ready to apply once reviewed.
    pub steps: Vec<Step>,
    /// Human-readable `# TODO` notes (rename and enum-remap candidates the
    /// diff refused to guess).
    pub todos: Vec<String>,
}

/// Compute the structural diff between two schemas.
///
/// Tables are matched by name; fields are matched by name within a table.
/// Nothing here infers renames — a renamed table or field shows up as a
/// remove plus an add, and [`draft_migration`] surfaces it for the author.
pub fn diff_schemas(from: &Schema, to: &Schema) -> SchemaDiff {
    let mut tables = Vec::new();
    for (name, from_table) in &from.tables {
        match to.tables.get(name) {
            Some(to_table) => tables.push(TableDiff {
                name: name.clone(),
                removed: false,
                added: false,
                fields: diff_table_fields(from_table, to_table),
            }),
            None => tables.push(TableDiff {
                name: name.clone(),
                removed: true,
                added: false,
                fields: Vec::new(),
            }),
        }
    }
    for name in to.tables.keys() {
        if !from.tables.contains_key(name) {
            tables.push(TableDiff {
                name: name.clone(),
                removed: false,
                added: true,
                fields: Vec::new(),
            });
        }
    }
    SchemaDiff { tables }
}

/// Draft a migration from a structural diff. Steps preserve deterministic
/// order (from-schema table order × field order), so the same schema pair
/// always drafts the same rule.
pub fn draft_migration(diff: &SchemaDiff) -> MigrationDraft {
    let mut draft = MigrationDraft::default();
    for table in &diff.tables {
        if table.removed {
            draft.todos.push(format!(
                "# TODO table `{}` was removed in the to-schema — the document still \
                 carries its rows and migration has no table-drop step; keep the table \
                 (restore it to the schema) or drop it from the source directly.",
                table.name
            ));
            continue;
        }
        if table.added {
            draft.todos.push(format!(
                "# TODO table `{}` is new in the to-schema — no existing rows move into \
                 it; author it in the source (migration steps are unnecessary for an \
                 empty new table).",
                table.name
            ));
            continue;
        }
        for field in &table.fields {
            draft_field(&table.name, field, &mut draft);
        }
    }
    draft
}

/// Render a draft as a rule file body. The caller writes the `from:` /
/// `to:` header around it (the versions are the author's labels, not
/// something a diff can know). TODO notes come first as a comment block;
/// `steps:` follows. A draft with no mechanical steps renders
/// `steps: []`, which `parse_spec` rejects (E2001, no steps) — by
/// design: an all-TODO draft must not run until the author adds steps.
pub fn render_draft(draft: &MigrationDraft) -> String {
    let mut out = String::new();
    if !draft.todos.is_empty() {
        out.push_str("# --- review before use ---\n");
        for todo in &draft.todos {
            out.push_str(todo);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("steps:\n");
    if draft.steps.is_empty() {
        out.push_str("  []\n");
    } else {
        for step in &draft.steps {
            match render_step(step) {
                Some(text) => out.push_str(&text),
                None => {
                    let _ = writeln!(
                        out,
                        "# TODO step {} carries a value with no rule-file form — write it \
                         by hand.",
                        step.describe()
                    );
                }
            }
        }
    }
    out
}

mod draft;
mod render;
#[cfg(test)]
mod tests;
mod values;

pub(crate) use draft::schema_default;
pub(crate) use render::render_step;
