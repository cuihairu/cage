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

use super::is_widening;
use super::Step;
use crate::schema::{FieldSchema, FieldType, Schema, TableSchema};
use crate::value::Value;
use indexmap::IndexMap;
use std::fmt::Write as _;

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

fn diff_table_fields(from: &TableSchema, to: &TableSchema) -> Vec<FieldDiff> {
    let mut fields = Vec::new();
    for (name, from_field) in &from.fields {
        match to.fields.get(name) {
            Some(to_field) => {
                if let Some(delta) = field_delta(from_field, to_field) {
                    fields.push(FieldDiff {
                        name: name.clone(),
                        delta,
                        from: Some(from_field.clone()),
                        to: Some(to_field.clone()),
                    });
                }
            }
            None => fields.push(FieldDiff {
                name: name.clone(),
                delta: FieldDelta::Removed,
                from: Some(from_field.clone()),
                to: None,
            }),
        }
    }
    for (name, to_field) in &to.fields {
        if !from.fields.contains_key(name) {
            fields.push(FieldDiff {
                name: name.clone(),
                delta: FieldDelta::Added,
                from: None,
                to: Some(to_field.clone()),
            });
        }
    }
    fields
}

/// Compare one field's structural shape, ignoring wire noise
/// (descriptions, display metadata). `None` when nothing structural
/// changed.
fn field_delta(from: &FieldSchema, to: &FieldSchema) -> Option<FieldDelta> {
    if from.field_type != to.field_type {
        return Some(FieldDelta::Retyped {
            from: from.field_type.clone(),
            to: to.field_type.clone(),
        });
    }
    if from.required != to.required {
        return Some(FieldDelta::RequiredChanged {
            from: from.required,
            to: to.required,
        });
    }
    let from_members = from.enum_values.clone().unwrap_or_default();
    let to_members = to.enum_values.clone().unwrap_or_default();
    if from_members != to_members {
        let removed: Vec<String> = from_members
            .iter()
            .filter(|m| !to_members.contains(m))
            .cloned()
            .collect();
        let added: Vec<String> = to_members
            .iter()
            .filter(|m| !from_members.contains(m))
            .cloned()
            .collect();
        return Some(FieldDelta::EnumMembersChanged { removed, added });
    }
    None
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

fn draft_field(table: &str, field: &FieldDiff, draft: &mut MigrationDraft) {
    match &field.delta {
        FieldDelta::Removed => draft.steps.push(Step::RemoveField {
            table: table.to_string(),
            field: field.name.clone(),
        }),
        FieldDelta::Added => {
            // A brand-new field needs a step only when the to-schema gives
            // it a default (rows should receive that value) or makes it
            // required (rows must carry it).
            if let Some(to_field) = &field.to {
                match (&to_field.default, schema_default(to_field)) {
                    (Some(raw), None) => draft.todos.push(format!(
                        "# TODO field `{table}.{}` is new and its default `{raw}` is not \
                         representable as a value of its own type — supply a \
                         `set_default` by hand.",
                        field.name
                    )),
                    (_, Some(value)) => draft.steps.push(Step::SetDefault {
                        table: table.to_string(),
                        field: field.name.clone(),
                        value,
                    }),
                    (None, None) if to_field.required => draft.todos.push(format!(
                        "# TODO field `{table}.{}` is new and required with no default — \
                         existing rows have no value for it; supply one (`set_default`) \
                         or relax `required`.",
                        field.name
                    )),
                    (None, None) => {}
                }
            }
        }
        FieldDelta::Retyped { from, to } => {
            if is_widening(from, to) {
                draft.steps.push(Step::WidenType {
                    table: table.to_string(),
                    field: field.name.clone(),
                    to: to.clone(),
                });
            } else {
                draft.todos.push(format!(
                    "# TODO field `{table}.{}` type changed {} → {} — not a safe \
                     widening, `widen_type` would refuse it at apply time (E2003); \
                     transform the values explicitly (`remap_values` for strings) or \
                     reconsider the schema change.",
                    field.name,
                    type_label(from),
                    type_label(to)
                ));
            }
        }
        FieldDelta::RequiredChanged { from, to } if !from && *to => {
            let to_field = field
                .to
                .as_ref()
                .expect("required-change fields exist in both");
            match (&to_field.default, schema_default(to_field)) {
                (Some(raw), None) => draft.todos.push(format!(
                    "# TODO field `{table}.{}` became required and its default `{raw}` is \
                     not representable as a value of its own type — supply a \
                     `set_default` by hand.",
                    field.name
                )),
                (_, Some(value)) => draft.steps.push(Step::SetDefault {
                    table: table.to_string(),
                    field: field.name.clone(),
                    value,
                }),
                (None, None) => draft.todos.push(format!(
                    "# TODO field `{table}.{}` became required with no default — \
                     existing rows may be missing it; add a `set_default` step or a \
                     value column in the source.",
                    field.name
                )),
            }
        }
        // required → optional needs no data change.
        FieldDelta::RequiredChanged { .. } => {}
        FieldDelta::EnumMembersChanged { removed, added } => {
            draft.todos.push(format!(
                "# TODO field `{table}.{}` enum members changed (removed: [{}]; added: \
                 [{}]) — map retired members with `remap_values`; the target value is a \
                 business decision the diff cannot make.",
                field.name,
                removed.join(", "),
                added.join(", ")
            ));
        }
    }
}

/// A to-schema field default, as a canonical value **of the field's own
/// type family**: a JSON `5` becomes `Value::UInt(5)` under a `UInt32`
/// column and `Value::Int(5)` under an `Int32` one — the generic
/// YAML/JSON bridges prefer `Int`, which would sail through `set_default`
/// and then die in the L2 family check (E1101) during reverification.
/// Unrepresentable combinations (bytes, mismatched literals) yield `None`
/// and surface as a `# TODO` instead of a broken step.
fn schema_default(field: &FieldSchema) -> Option<Value> {
    typed_json_value(field.default.as_ref()?, &field.field_type)
}

/// Plain JSON → canonical value, shaped by the expected field type.
fn typed_json_value(json: &serde_json::Value, expected: &FieldType) -> Option<Value> {
    match expected {
        FieldType::UInt8 | FieldType::UInt16 | FieldType::UInt32 | FieldType::UInt64 => {
            match json {
                serde_json::Value::Number(n) if n.is_u64() => Some(Value::UInt(n.as_u64()?)),
                _ => None,
            }
        }
        FieldType::Int8 | FieldType::Int16 | FieldType::Int32 | FieldType::Int64 => match json {
            serde_json::Value::Number(n) if n.is_i64() => Some(Value::Int(n.as_i64()?)),
            _ => None,
        },
        FieldType::Float32 | FieldType::Float64 => match json {
            serde_json::Value::Number(n) => Some(Value::Float(n.as_f64()?)),
            _ => None,
        },
        FieldType::Bool => match json {
            serde_json::Value::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        },
        FieldType::String | FieldType::Enum(_) => match json {
            serde_json::Value::String(s) => Some(Value::String(s.clone())),
            _ => None,
        },
        FieldType::Null => match json {
            serde_json::Value::Null => Some(Value::Null),
            _ => None,
        },
        FieldType::Array(inner) => match json {
            serde_json::Value::Array(items) => Some(Value::Array(
                items
                    .iter()
                    .map(|v| typed_json_value(v, inner))
                    .collect::<Option<Vec<_>>>()?,
            )),
            _ => None,
        },
        // Objects, maps, bytes and `Any` fall back to the generic bridge —
        // a schema default for these is rare and the family check on
        // objects is structural anyway.
        _ => json_to_value(json),
    }
}

/// Plain JSON (the shape schema files carry defaults in) → canonical
/// value. The canonical model is adjacently tagged for serde, so this is
/// a hand-rolled mapping rather than a serde round-trip; `Bytes` and
/// anything else a schema default cannot literally be, maps to `None`.
fn json_to_value(json: &serde_json::Value) -> Option<Value> {
    Some(match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else {
                Value::Float(n.as_f64()?)
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(items) => Value::Array(
            items
                .iter()
                .map(json_to_value)
                .collect::<Option<Vec<_>>>()?,
        ),
        serde_json::Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Some((k.clone(), json_to_value(v)?)))
                .collect::<Option<IndexMap<String, Value>>>()?,
        ),
    })
}

/// Short type label for `# TODO` prose (the serde `kind` tag).
fn type_label(t: &FieldType) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
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

/// Render one step in the single-key wire form the rule parser decodes.
/// `None` when the step carries a value with no inline-or-block YAML form
/// (a bytes default) — the caller turns that into a `# TODO`.
fn render_step(step: &Step) -> Option<String> {
    match step {
        Step::RenameField { table, from, to } => Some(format!(
            "  - rename_field:\n      table: {table}\n      from: {from}\n      to: {to}\n"
        )),
        Step::SetDefault {
            table,
            field,
            value,
        } => {
            // A bare scalar cannot carry the UInt family (plain `5` reads
            // back as `Int`): unsigned values ride the canonical tagged
            // form, which `yaml_to_value` decodes first.
            if has_unsigned(value) {
                let rendered = serde_yaml::to_string(value).ok()?;
                return Some(format!(
                    "  - set_default:\n      table: {table}\n      field: {field}\n      value:\n{}",
                    indent(8, rendered.trim_end())
                ));
            }
            let rendered = value_to_yaml_block(value, 0)?;
            let trimmed = rendered.trim_end();
            // Scalars ride inline; sequences and mappings need the block
            // form (`value: - 1` is not YAML).
            if trimmed.contains('\n') {
                Some(format!(
                    "  - set_default:\n      table: {table}\n      field: {field}\n      value:\n{}",
                    indent(8, trimmed)
                ))
            } else {
                Some(format!(
                    "  - set_default:\n      table: {table}\n      field: {field}\n      value: {trimmed}\n"
                ))
            }
        }
        Step::RemoveField { table, field } => Some(format!(
            "  - remove_field:\n      table: {table}\n      field: {field}\n"
        )),
        Step::WidenType { table, field, to } => {
            // FieldType is adjacently tagged for serde, so serde_yaml can
            // render it — block form, one `kind:`/`value:` pair per line.
            Some(format!(
                "  - widen_type:\n      table: {table}\n      field: {field}\n      to:\n{}",
                indent(8, &serde_yaml::to_string(to).unwrap_or_default())
            ))
        }
        Step::RemapValues { table, field, map } => {
            let mut out = format!(
                "  - remap_values:\n      table: {table}\n      field: {field}\n      map:\n"
            );
            for (k, v) in map {
                let _ = writeln!(out, "        {}: {}", quote_string(k)?, quote_string(v)?);
            }
            Some(out)
        }
        Step::RenameTable { from, to } => Some(format!(
            "  - rename_table:\n      from: {from}\n      to: {to}\n"
        )),
    }
}

fn indent(n: usize, text: &str) -> String {
    let pad = " ".repeat(n);
    let mut out = String::new();
    for line in text.lines() {
        let _ = writeln!(out, "{pad}{line}");
    }
    out
}

/// Quote a YAML scalar so it reparses as a string, always. `serde_yaml`
/// leaves `yes`/`5`-lookalikes bare in some paths, and a bare scalar
/// round-trips as a bool or number — a silent type flip. Single-quote
/// form (`'...'`, `''` escape) is valid for every one-line string; strings
/// that cannot live on one line have no inline form and yield `None`.
fn quote_string(s: &str) -> Option<String> {
    if s.chars()
        .any(|c| c == '\n' || c == '\r' || (c.is_control() && c != '\t'))
    {
        return None;
    }
    Some(format!("'{}'", s.replace('\'', "''")))
}

/// True when plain rendering would lose the value's family: a bare
/// unsigned integer reads back as `Int` (`yaml_to_value` is i64-first),
/// so `UInt` anywhere in the value forces the canonical tagged form.
fn has_unsigned(value: &Value) -> bool {
    match value {
        Value::UInt(_) => true,
        Value::Array(items) => items.iter().any(has_unsigned),
        Value::Object(map) => map.values().any(has_unsigned),
        _ => false,
    }
}

/// Render a canonical value as inline-or-block YAML at the given indent
/// (the inverse of the rule parser's `yaml_to_value`, with quoting the
/// generic serializer cannot be trusted for). `Bytes` has no plain YAML
/// form and yields `None`.
fn value_to_yaml_block(value: &Value, pad: usize) -> Option<String> {
    let ind = " ".repeat(pad);
    Some(match value {
        Value::Null => "null\n".to_string(),
        Value::Bool(true) => "true\n".to_string(),
        Value::Bool(false) => "false\n".to_string(),
        Value::Int(i) => format!("{i}\n"),
        Value::UInt(u) => format!("{u}\n"),
        // serde formats floats with a `.0`-style fractional part, so the
        // round-trip cannot land back in the integer family.
        Value::Float(f) => {
            let mut text =
                serde_yaml::to_string(&serde_yaml::Value::Number((*f).into())).unwrap_or_default();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text
        }
        Value::String(s) => format!("{}\n", quote_string(s)?),
        Value::Bytes(_) => return None,
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                match item {
                    Value::Array(_) | Value::Object(_) => {
                        let nested = value_to_yaml_block(item, pad + 2)?;
                        write!(out, "{ind}-\n{nested}").unwrap();
                    }
                    scalar => {
                        let one = value_to_yaml_block(scalar, pad)?;
                        write!(out, "{ind}- {}", one.trim_end()).unwrap();
                        out.push('\n');
                    }
                }
            }
            out
        }
        Value::Object(map) => {
            let mut out = String::new();
            for (k, v) in map {
                let key = quote_string(k)?;
                match v {
                    Value::Array(_) | Value::Object(_) => {
                        let nested = value_to_yaml_block(v, pad + 2)?;
                        write!(out, "{ind}{key}:\n{nested}").unwrap();
                    }
                    scalar => {
                        let one = value_to_yaml_block(scalar, pad)?;
                        write!(out, "{ind}{key}: {}", one.trim_end()).unwrap();
                        out.push('\n');
                    }
                }
            }
            out
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldType;
    use crate::value::Value as V;

    fn schema(yaml: &str) -> Schema {
        serde_yaml::from_str(yaml).expect("parse schema")
    }

    #[test]
    fn draft_covers_remove_widen_add_default_and_todo_paths() {
        let from = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      name:
        name: name
        type: { kind: String }
        required: true
      level:
        name: level
        type: { kind: Int32 }
  Boss:
    name: Boss
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
",
        );
        let to = schema(
            r#"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      level:
        name: level
        type: { kind: Int64 }
      rarity:
        name: rarity
        type: { kind: String }
        default: "common"
  NewTab:
    name: NewTab
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
"#,
        );
        let draft = draft_migration(&diff_schemas(&from, &to));
        assert_eq!(
            draft.steps,
            vec![
                Step::RemoveField {
                    table: "Item".into(),
                    field: "name".into(),
                },
                Step::WidenType {
                    table: "Item".into(),
                    field: "level".into(),
                    to: FieldType::Int64,
                },
                Step::SetDefault {
                    table: "Item".into(),
                    field: "rarity".into(),
                    value: V::String("common".into()),
                },
            ]
        );
        // The dropped table and the new table surface as TODOs, not steps.
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("table `Boss` was removed")));
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("table `NewTab` is new")));
    }

    #[test]
    fn typed_defaults_land_in_the_field_family() {
        let from = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
",
        );
        let to = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      weight:
        name: weight
        type: { kind: UInt32 }
        default: 5
      mana:
        name: mana
        type: { kind: Int32 }
        default: 5
      ratio:
        name: ratio
        type: { kind: Float64 }
        default: 5
      label:
        name: label
        type: { kind: String }
        required: true
        default: 7
enums: {}
",
        );
        let draft = draft_migration(&diff_schemas(&from, &to));
        let find_default = |field: &str| {
            draft.steps.iter().find_map(|s| match s {
                Step::SetDefault {
                    field: f, value, ..
                } if f == field => Some(value.clone()),
                _ => None,
            })
        };
        assert_eq!(find_default("weight"), Some(V::UInt(5)));
        assert_eq!(find_default("mana"), Some(V::Int(5)));
        assert_eq!(find_default("ratio"), Some(V::Float(5.0)));
        // A number default on a String field is not representable as a
        // value of the field's own type: TODO, no broken step.
        assert!(find_default("label").is_none());
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("field `Item.label`") && t.contains("not representable")));
    }

    #[test]
    fn enum_member_changes_and_non_widenings_stay_todo() {
        let from = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      grade:
        name: grade
        type: { kind: String }
        enum_values: [S, A, B]
      code:
        name: code
        type: { kind: Int32 }
enums: {}
",
        );
        let to = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      grade:
        name: grade
        type: { kind: String }
        enum_values: [S, A, C]
      code:
        name: code
        type: { kind: String }
enums: {}
",
        );
        let draft = draft_migration(&diff_schemas(&from, &to));
        assert_eq!(draft.steps, vec![]);
        assert!(draft.todos.iter().any(|t| t.contains("`Item.grade`")
            && t.contains("removed: [B]")
            && t.contains("added: [C]")));
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("`Item.code` type changed Int32 → String")));
    }

    #[test]
    fn required_flip_backfills_from_default_or_todo() {
        let from = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      a:
        name: a
        type: { kind: String }
      b:
        name: b
        type: { kind: String }
enums: {}
",
        );
        let to = schema(
            r#"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      a:
        name: a
        type: { kind: String }
        required: true
        default: "x"
      b:
        name: b
        type: { kind: String }
        required: true
enums: {}
"#,
        );
        let draft = draft_migration(&diff_schemas(&from, &to));
        assert_eq!(
            draft.steps,
            vec![Step::SetDefault {
                table: "Item".into(),
                field: "a".into(),
                value: V::String("x".into()),
            }]
        );
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("`Item.b` became required with no default")));
    }

    #[test]
    fn render_round_trips_through_parse_spec() {
        let draft = MigrationDraft {
            steps: vec![
                Step::RemoveField {
                    table: "Item".into(),
                    field: "name".into(),
                },
                Step::WidenType {
                    table: "Item".into(),
                    field: "level".into(),
                    to: FieldType::Int64,
                },
                Step::SetDefault {
                    table: "Item".into(),
                    field: "rarity".into(),
                    value: V::String("yes".into()),
                },
                Step::SetDefault {
                    table: "Item".into(),
                    field: "caps".into(),
                    // UInt rides the canonical tagged form: a bare `1`
                    // would read back as `Int` and trip E1101 at reverify.
                    value: V::Array(vec![V::UInt(1), V::UInt(2)]),
                },
            ],
            todos: vec!["# TODO rename name → title".into()],
        };
        let text = format!("from: \"1.0.0\"\nto: \"2.0.0\"\n{}", render_draft(&draft));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0001-draft.yaml");
        std::fs::write(&path, &text).unwrap();
        let parsed = super::super::parse_spec(&path).unwrap();
        assert_eq!(parsed.from, "1.0.0");
        assert_eq!(parsed.to, "2.0.0");
        assert_eq!(parsed.steps, draft.steps);
        assert!(text.contains("type: UInt"));
    }

    #[test]
    fn draft_render_is_deterministic_and_todo_only_drafts_render_empty_steps() {
        let from = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
enums: {}
",
        );
        let diff = diff_schemas(&from, &from);
        let draft = draft_migration(&diff);
        assert_eq!(render_draft(&draft), render_draft(&draft));
        // An all-quiet diff drafts nothing: the empty-steps marker, which
        // parse_spec refuses (E2001) until the author adds steps.
        assert!(render_draft(&draft).contains("steps:\n  []\n"));
        let empty = MigrationDraft::default();
        assert!(render_draft(&empty).contains("steps:\n  []\n"));
    }

    #[test]
    fn typed_json_value_shapes_defaults_into_their_family() {
        use serde_json::json;
        // Unsigned family demands a non-negative integer literal.
        assert_eq!(
            typed_json_value(&json!(5), &FieldType::UInt32),
            Some(V::UInt(5))
        );
        assert_eq!(typed_json_value(&json!(-1), &FieldType::UInt32), None);
        assert_eq!(typed_json_value(&json!("x"), &FieldType::UInt8), None);
        // Signed family demands an integer literal.
        assert_eq!(
            typed_json_value(&json!(-3), &FieldType::Int32),
            Some(V::Int(-3))
        );
        assert_eq!(typed_json_value(&json!(1.5), &FieldType::Int64), None);
        // Float, bool and string families.
        assert_eq!(
            typed_json_value(&json!(2.5), &FieldType::Float64),
            Some(V::Float(2.5))
        );
        assert_eq!(typed_json_value(&json!("x"), &FieldType::Float32), None);
        assert_eq!(
            typed_json_value(&json!(true), &FieldType::Bool),
            Some(V::Bool(true))
        );
        assert_eq!(typed_json_value(&json!(1), &FieldType::Bool), None);
        assert_eq!(
            typed_json_value(&json!("mage"), &FieldType::String),
            Some(V::String("mage".into()))
        );
        assert_eq!(typed_json_value(&json!(1), &FieldType::String), None);
        // Null accepts only a JSON null.
        assert_eq!(
            typed_json_value(&json!(null), &FieldType::Null),
            Some(V::Null)
        );
        assert_eq!(typed_json_value(&json!(1), &FieldType::Null), None);
        // Arrays recurse per element; one bad element kills the default.
        assert_eq!(
            typed_json_value(
                &json!([1, 2]),
                &FieldType::Array(Box::new(FieldType::UInt32))
            ),
            Some(V::Array(vec![V::UInt(1), V::UInt(2)]))
        );
        assert_eq!(
            typed_json_value(
                &json!([1, -1]),
                &FieldType::Array(Box::new(FieldType::UInt32))
            ),
            None
        );
        assert_eq!(
            typed_json_value(&json!("x"), &FieldType::Array(Box::new(FieldType::Int32))),
            None
        );
        // Everything else (bytes included) falls back to the generic bridge.
        assert_eq!(
            typed_json_value(&json!(5), &FieldType::Bytes),
            Some(V::Int(5))
        );
    }

    #[test]
    fn json_to_value_maps_every_json_shape() {
        use serde_json::json;
        assert_eq!(json_to_value(&json!(null)), Some(V::Null));
        assert_eq!(json_to_value(&json!(true)), Some(V::Bool(true)));
        assert_eq!(json_to_value(&json!(5)), Some(V::Int(5)));
        assert_eq!(json_to_value(&json!(u64::MAX)), Some(V::UInt(u64::MAX)));
        assert_eq!(json_to_value(&json!(1.5)), Some(V::Float(1.5)));
        assert_eq!(json_to_value(&json!("s")), Some(V::String("s".to_string())));
        assert_eq!(
            json_to_value(&json!([1, "a"])),
            Some(V::Array(vec![V::Int(1), V::String("a".into())]))
        );
        let obj = json_to_value(&json!({"k": [1]}));
        let V::Object(map) = obj.unwrap() else {
            panic!("expected an object value");
        };
        assert_eq!(map.get("k"), Some(&V::Array(vec![V::Int(1)])));
    }

    #[test]
    fn quote_string_always_reparses_as_a_string_or_refuses() {
        assert_eq!(quote_string("yes"), Some("'yes'".to_string()));
        assert_eq!(quote_string("5"), Some("'5'".to_string()));
        assert_eq!(quote_string("it's"), Some("'it''s'".to_string()));
        assert_eq!(quote_string("a\tb"), Some("'a\tb'".to_string()));
        // No inline form exists for multi-line or control-character text.
        assert_eq!(quote_string("a\nb"), None);
        assert_eq!(quote_string("a\rb"), None);
        assert_eq!(quote_string("a\u{1}b"), None);
    }

    #[test]
    fn has_unsigned_spots_the_family_through_containers() {
        assert!(has_unsigned(&V::UInt(1)));
        assert!(!has_unsigned(&V::Int(-1)));
        assert!(has_unsigned(&V::Array(vec![V::Int(1), V::UInt(2)])));
        assert!(!has_unsigned(&V::Array(vec![V::Int(1)])));
        let mut map = IndexMap::new();
        map.insert("k".to_string(), V::UInt(2));
        assert!(has_unsigned(&V::Object(map)));
    }

    #[test]
    fn render_step_forms_cover_every_variant_and_todo_fallback() {
        let mut remap = IndexMap::new();
        remap.insert("yes".to_string(), "true".to_string());
        let draft = MigrationDraft {
            todos: vec!["# TODO fix by hand".to_string()],
            steps: vec![
                Step::RenameField {
                    table: "Item".into(),
                    from: "desc".into(),
                    to: "description".into(),
                },
                Step::SetDefault {
                    table: "Item".into(),
                    field: "rarity".into(),
                    value: V::String("common".into()),
                },
                // Unsigned rides the canonical tagged form.
                Step::SetDefault {
                    table: "Item".into(),
                    field: "stack".into(),
                    value: V::UInt(99),
                },
                // Sequences need the block form.
                Step::SetDefault {
                    table: "Item".into(),
                    field: "ids".into(),
                    value: V::Array(vec![V::Int(1), V::Int(2)]),
                },
                Step::RemoveField {
                    table: "Item".into(),
                    field: "ghost".into(),
                },
                Step::WidenType {
                    table: "Item".into(),
                    field: "level".into(),
                    to: FieldType::Int64,
                },
                Step::RemapValues {
                    table: "Item".into(),
                    field: "rarity".into(),
                    map: remap,
                },
                Step::RenameTable {
                    from: "Mob".into(),
                    to: "Enemy".into(),
                },
                // Bytes have no rule-file form → rendered as a TODO line.
                Step::SetDefault {
                    table: "Item".into(),
                    field: "blob".into(),
                    value: V::Bytes(vec![1]),
                },
            ],
        };
        let text = render_draft(&draft);
        assert!(text.starts_with("# --- review before use ---\n# TODO fix by hand\n\nsteps:\n"));
        assert!(text.contains(
            "rename_field:\n      table: Item\n      from: desc\n      to: description\n"
        ));
        assert!(text.contains("value: 'common'\n"), "inline scalar form");
        assert!(
            text.contains("value:\n") && text.contains("99"),
            "unsigned default rides the tagged block form: {text}"
        );
        assert!(text.contains("- 1\n"), "sequence default in block form");
        assert!(text.contains("remove_field:\n      table: Item\n      field: ghost\n"));
        assert!(text.contains("widen_type:\n      table: Item\n      field: level\n"));
        assert!(text.contains("remap_values:\n      table: Item\n      field: rarity\n"));
        assert!(text.contains("'yes': 'true'\n"), "map keys stay quoted");
        assert!(text.contains("rename_table:\n      from: Mob\n      to: Enemy\n"));
        assert!(
            text.contains("# TODO step set_default(Item.blob) carries a value"),
            "bytes default degrades to a TODO: {text}"
        );
    }

    #[test]
    fn added_and_required_flip_todo_arms() {
        let from = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      old_req:
        name: old_req
        type: { kind: String }
        required: true
      bad_default:
        name: bad_default
        type: { kind: Int32 }
        default: true
enums: {}
",
        );
        let to = schema(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
      old_req:
        name: old_req
        type: { kind: String }
      new_req:
        name: new_req
        type: { kind: String }
        required: true
      new_opt:
        name: new_opt
        type: { kind: String }
      bad_default:
        name: bad_default
        type: { kind: Int32 }
        required: true
        default: true
enums: {}
",
        );
        let draft = draft_migration(&diff_schemas(&from, &to));
        // New + required + no default → TODO telling the author to backfill.
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("`Item.new_req` is new and required with no default")));
        // New + optional + no default → nothing to say.
        assert!(!draft.todos.iter().any(|t| t.contains("`Item.new_opt`")));
        // Became required with a default no rule can express → TODO.
        assert!(draft
            .todos
            .iter()
            .any(|t| t.contains("`Item.bad_default` became required and its default `true`")));
        // required → optional needs no data change: no step, no TODO.
        assert!(!draft.steps.iter().any(|s| matches!(
            s,
            Step::SetDefault { field, .. } if field == "old_req"
        )));
        assert!(!draft.todos.iter().any(|t| t.contains("`Item.old_req`")));
    }
}
