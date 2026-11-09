//! Mechanical-safe diff → step drafting: field-by-field deltas, the
//! conservative `# TODO` policy for ambiguous cases, and the to-schema
//! default shaper (split out of diff.rs, LAY-025).

use super::values::typed_json_value;
use super::{FieldDelta, FieldDiff, MigrationDraft};
use crate::migrate::apply::is_widening;
use crate::migrate::Step;
use crate::schema::{FieldSchema, FieldType, TableSchema};
use crate::value::Value;

pub(crate) fn diff_table_fields(from: &TableSchema, to: &TableSchema) -> Vec<FieldDiff> {
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
pub(crate) fn field_delta(from: &FieldSchema, to: &FieldSchema) -> Option<FieldDelta> {
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

pub(crate) fn draft_field(table: &str, field: &FieldDiff, draft: &mut MigrationDraft) {
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
pub(crate) fn schema_default(field: &FieldSchema) -> Option<Value> {
    typed_json_value(field.default.as_ref()?, &field.field_type)
}

/// Plain JSON → canonical value, shaped by the expected field type.
fn type_label(t: &FieldType) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}
