//! Declarative L7 rule bodies: the five game-rule runners plus their shared
//! helpers (split out of rules.rs, LAY-012).

use crate::diagnostics::{Diagnostic, DiagnosticBuilder};
use crate::error::codes::gamerule;
use crate::reference::DependencyGraph;
use crate::schema::Schema;
use crate::value::{Document, Row, SourceLocation, TypedValue, Value};
use std::collections::{HashMap, HashSet};

use super::{
    as_f64, ClientServerFieldConfig, DropRewardConfig, DungeonStageConfig, ReferenceCycleConfig,
    RuleConfig, SkillItemCategoryConfig,
};
pub(crate) fn run_skill_item_category(
    config: &RuleConfig,
    _schema: &Schema,
    document: &Document,
    params: &SkillItemCategoryConfig,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(skill_table) = document.tables.get(&params.skill_table) else {
        return out;
    };
    let Some(item_table) = document.tables.get(&params.item_table) else {
        return out;
    };

    // item id -> category; the first row wins.
    let mut item_category: HashMap<String, String> = HashMap::new();
    for row in &item_table.rows {
        let Some(id) = row
            .fields
            .get(&params.item_id_field)
            .and_then(|tv| tv.value.coerce_to_string())
        else {
            continue;
        };
        let Some(category) = row
            .fields
            .get(&params.category_field)
            .and_then(|tv| tv.value.coerce_to_string())
        else {
            continue;
        };
        item_category.entry(id).or_insert(category);
    }

    for row in &skill_table.rows {
        let Some(reference_field) = row.fields.get(&params.item_field) else {
            continue;
        };
        let referenced: Vec<Value> = match &reference_field.value {
            Value::Array(items) => items.clone(),
            single => vec![single.clone()],
        };
        for item_ref in referenced {
            let Some(item_id) = item_ref.coerce_to_string() else {
                continue;
            };
            let Some(category) = item_category.get(&item_id) else {
                continue;
            };
            if params.allowed_categories.iter().any(|c| c == category) {
                continue;
            }
            out.push(
                DiagnosticBuilder::error(
                    gamerule::E1604,
                    "Skill references an item in a disallowed category",
                )
                .location(field_location(reference_field, row))
                .table(&params.skill_table)
                .row(format!("{}", row.index))
                .field(&params.item_field)
                .value(item_id.clone())
                .hint(format!(
                    "{}: item '{item_id}' has category '{category}', allowed: {}",
                    config.name,
                    params.allowed_categories.join(", ")
                ))
                .build(),
            );
        }
    }
    out
}

/// L7.2 declarative rule: the cross-table reference graph (built from schema
/// `reference` definitions) must be acyclic. Each detected cycle reports the
/// edge that closes it, located at the referencing field/row.
pub(crate) fn run_reference_cycle(
    config: &RuleConfig,
    schema: &Schema,
    document: &Document,
    params: &ReferenceCycleConfig,
) -> Vec<Diagnostic> {
    let in_scope = |table: &str| {
        params.table_filter.is_empty() || params.table_filter.iter().any(|t| t == table)
    };

    let mut graph = DependencyGraph::new();
    // (from, to) -> the first schema field (deterministic) that declares the reference.
    let mut edge_field: HashMap<(String, String), String> = HashMap::new();
    let mut table_source: HashMap<String, (String, Option<String>)> = HashMap::new();

    for (table_name, table_schema) in &schema.tables {
        if !in_scope(table_name) {
            continue;
        }
        if let Some(table) = document.tables.get(table_name) {
            table_source.insert(
                table_name.clone(),
                (table.source_file.clone(), table.sheet.clone()),
            );
        }
        for (field_name, field_schema) in &table_schema.fields {
            let Some(reference) = &field_schema.reference else {
                continue;
            };
            if !in_scope(&reference.table) {
                continue;
            }
            graph.add_edge(table_name, &reference.table);
            edge_field
                .entry((table_name.clone(), reference.table.clone()))
                .or_insert_with(|| field_name.clone());
        }
    }

    // Canonical form: rotate so the lexicographically smallest table is first,
    // then dedup and sort — deterministic diagnostic order.
    let mut cycles = graph.find_cycles();
    for cycle in &mut cycles {
        if cycle.is_empty() {
            continue;
        }
        let Some(min_index) = cycle
            .iter()
            .enumerate()
            .min_by_key(|(_, name)| name.as_str())
            .map(|(index, _)| index)
        else {
            continue;
        };
        cycle.rotate_left(min_index);
    }
    cycles.sort();
    cycles.dedup();

    let mut out = Vec::new();
    for cycle in cycles {
        // The edge from the rotated last table back to the first closes the
        // cycle — that is the point of detection with a precise location.
        let from = cycle.last().cloned().unwrap_or_default();
        let to = cycle.first().cloned().unwrap_or_default();
        let Some(field) = edge_field.get(&(from.clone(), to.clone())).cloned() else {
            continue;
        };
        let location = reference_edge_location(document, &from, &field, &table_source);
        let mut cycle_text = cycle.join(" -> ");
        cycle_text.push_str(" -> ");
        cycle_text.push_str(&to);
        out.push(
            DiagnosticBuilder::error(
                gamerule::E1605,
                "Cross-table reference cycle detected",
            )
            .location(location)
            .table(&from)
            .field(field.clone())
            .value(field.clone())
            .hint(format!(
                "{}: {from} -> {to} closes the cycle ({cycle_text}); break the ring or drop the reference",
                config.name,
            ))
            .build(),
        );
    }
    out
}

/// L7.3 declarative rule: every drop row must reference a known item and,
/// when it declares them, carry a non-negative whole-number quantity and a
/// finite probability weight within `[0, 1]`. Unknown item references are
/// reported here (the L5 check covers schema-declared references; a
/// data-declared drop relation needs its own cross-table resolution).
pub(crate) fn run_drop_reward(
    config: &RuleConfig,
    document: &Document,
    params: &DropRewardConfig,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(drop_table) = document.tables.get(&params.drop_table) else {
        return out;
    };
    let Some(item_table) = document.tables.get(&params.item_table) else {
        return out;
    };
    let item_ids = id_set(item_table, &params.item_id_field);

    for row in &drop_table.rows {
        let Some(item_field) = row.fields.get(&params.item_field) else {
            continue;
        };
        for item_ref in as_values(&item_field.value) {
            let Some(item_id) = item_ref.coerce_to_string() else {
                continue;
            };
            if item_ids.contains(&item_id) {
                continue;
            }
            out.push(
                DiagnosticBuilder::error(gamerule::E1606, "Drop rewards an unknown item")
                    .location(field_location(item_field, row))
                    .table(&params.drop_table)
                    .row(format!("{}", row.index))
                    .field(&params.item_field)
                    .value(item_id.clone())
                    .hint(format!(
                        "{}: item '{item_id}' is not in {}.{}",
                        config.name, params.item_table, params.item_id_field
                    ))
                    .build(),
            );
        }

        if let Some(quantity) = row.fields.get(&params.quantity_field) {
            let valid = as_f64(&quantity.value)
                .is_some_and(|f| f.is_finite() && f >= 0.0 && f.fract() == 0.0);
            if !valid {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1606,
                        "Drop quantity must be a non-negative whole number",
                    )
                    .location(field_location(quantity, row))
                    .table(&params.drop_table)
                    .row(format!("{}", row.index))
                    .field(&params.quantity_field)
                    .value(quantity.value.to_string())
                    .hint(format!("{}: got {:?}", config.name, quantity.value))
                    .build(),
                );
            }
        }

        if let Some(weight) = row.fields.get(&params.weight_field) {
            let valid =
                as_f64(&weight.value).is_some_and(|f| f.is_finite() && (0.0..=1.0).contains(&f));
            if !valid {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1606,
                        "Drop weight must be a finite number within [0, 1]",
                    )
                    .location(field_location(weight, row))
                    .table(&params.drop_table)
                    .row(format!("{}", row.index))
                    .field(&params.weight_field)
                    .value(weight.value.to_string())
                    .hint(format!("{}: got {:?}", config.name, weight.value))
                    .build(),
                );
            }
        }
    }
    out
}

/// L7.4 declarative rule: every dungeon stage must reference a monster group
/// and a reward that actually exist in their definition tables. Both edges
/// are data-declared relations (not schema references), so they are resolved
/// and enforced here.
pub(crate) fn run_dungeon_stage(
    config: &RuleConfig,
    document: &Document,
    params: &DungeonStageConfig,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(stage_table) = document.tables.get(&params.stage_table) else {
        return out;
    };
    let Some(group_table) = document.tables.get(&params.group_table) else {
        return out;
    };
    let Some(reward_table) = document.tables.get(&params.reward_table) else {
        return out;
    };
    let group_ids = id_set(group_table, &params.group_id_field);
    let reward_ids = id_set(reward_table, &params.reward_id_field);

    for row in &stage_table.rows {
        for (field_name, target_table, allowed) in [
            (&params.group_field, &params.group_table, &group_ids),
            (&params.reward_field, &params.reward_table, &reward_ids),
        ] {
            let Some(field) = row.fields.get(field_name) else {
                continue;
            };
            for reference in as_values(&field.value) {
                let Some(id) = reference.coerce_to_string() else {
                    continue;
                };
                if allowed.contains(&id) {
                    continue;
                }
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1607,
                        "Dungeon stage references an unknown monster group or reward",
                    )
                    .location(field_location(field, row))
                    .table(&params.stage_table)
                    .row(format!("{}", row.index))
                    .field(field_name)
                    .value(id.clone())
                    .hint(format!(
                        "{}: '{id}' is not a row of {}",
                        config.name, target_table
                    ))
                    .build(),
                );
            }
        }
    }
    out
}

/// L7.5 declarative rule: a field exposed to a profile must not reference an
/// entity (table or field) that the profile projection hides — otherwise the
/// profile build would silently dangle. Both the referenced table and the
/// referenced field are checked against the same visibility rule the profile
/// projection uses (empty targets = visible everywhere, `*` = wildcard).
pub(crate) fn run_client_server_field(
    config: &RuleConfig,
    schema: &Schema,
    document: &Document,
    params: &ClientServerFieldConfig,
) -> Vec<Diagnostic> {
    let in_scope = |table: &str| {
        params.table_filter.is_empty() || params.table_filter.iter().any(|t| t == table)
    };
    let mut table_source: HashMap<String, (String, Option<String>)> = HashMap::new();
    for (name, table) in &document.tables {
        table_source.insert(
            name.clone(),
            (table.source_file.clone(), table.sheet.clone()),
        );
    }

    let mut out = Vec::new();
    for (table_name, table_schema) in &schema.tables {
        if !in_scope(table_name) {
            continue;
        }
        for (field_name, field_schema) in &table_schema.fields {
            if !visible_to(&field_schema.targets, &params.profile) {
                continue;
            }
            let Some(reference) = &field_schema.reference else {
                continue;
            };
            // Unknown reference targets are the L5 reference check's report.
            let Some(target_schema) = schema.tables.get(&reference.table) else {
                continue;
            };
            if !visible_to(&target_schema.targets, &params.profile) {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1608,
                        "A field visible to the profile references a table hidden from it",
                    )
                    .location(reference_edge_location(
                        document,
                        table_name,
                        field_name,
                        &table_source,
                    ))
                    .table(table_name)
                    .field(field_name)
                    .value(format!("{}.{}", reference.table, reference.field))
                    .hint(format!(
                        "{}: {table_name}.{field_name} is visible to profile {:?} but \
                         references {}.{}, which is not (targets: {:?})",
                        config.name,
                        params.profile,
                        reference.table,
                        reference.field,
                        target_schema.targets,
                    ))
                    .build(),
                );
                continue;
            }
            let Some(target_field) = target_schema.fields.get(&reference.field) else {
                continue; // unknown referenced field: L5's job
            };
            if !visible_to(&target_field.targets, &params.profile) {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1608,
                        "A field visible to the profile references a field hidden from it",
                    )
                    .location(reference_edge_location(
                        document,
                        table_name,
                        field_name,
                        &table_source,
                    ))
                    .table(table_name)
                    .field(field_name)
                    .value(format!("{}.{}", reference.table, reference.field))
                    .hint(format!(
                        "{}: {table_name}.{field_name} is visible to profile {:?} but \
                         references {}.{}, which is not (targets: {:?})",
                        config.name,
                        params.profile,
                        reference.table,
                        reference.field,
                        target_field.targets,
                    ))
                    .build(),
                );
            }
        }
    }
    out
}

/// The set of string ids a table declares in one field (scalar or array
/// values, first row wins per id).
pub(crate) fn id_set(table: &crate::value::Table, id_field: &str) -> HashSet<String> {
    table
        .rows
        .iter()
        .filter_map(|row| row.fields.get(id_field))
        .flat_map(|tv| as_values(&tv.value))
        .filter_map(|value| value.coerce_to_string())
        .collect()
}

/// A scalar-or-array field's values, each value cloned (mirrors the
/// skill-item rule's handling of array item fields).
pub(crate) fn as_values(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        single => vec![single.clone()],
    }
}

/// Whether a target list exposes an entity to the given profile: empty
/// targets are visible everywhere and `*` is a wildcard — the same rule the
/// profile projection applies.
pub(crate) fn visible_to(targets: &[String], profile: &str) -> bool {
    targets.is_empty() || targets.iter().any(|t| t == profile || t == "*")
}

/// Choose the most precise location for a field: the typed value's own
/// location when it carries row/line/column info, otherwise the row location.
pub(crate) fn field_location(field: &TypedValue, row: &Row) -> SourceLocation {
    let location = &field.location;
    if location.row.is_some()
        || location.line.is_some()
        || location.col.is_some()
        || location.column.is_some()
    {
        location.clone()
    } else {
        row.location.clone()
    }
}

/// Locate a reference edge at the referencing field, falling back to the
/// table's source location when no row carries the field.
pub(crate) fn reference_edge_location(
    document: &Document,
    from_table: &str,
    field: &str,
    table_source: &HashMap<String, (String, Option<String>)>,
) -> SourceLocation {
    if let Some(table) = document.tables.get(from_table) {
        if let Some(row) = table.rows.iter().find(|row| row.fields.contains_key(field)) {
            return field_location(&row.fields[field], row);
        }
    }
    let (file, sheet) = table_source.get(from_table).cloned().unwrap_or_default();
    let mut location = SourceLocation::new(file);
    location.sheet = sheet;
    location.field = Some(field.to_string());
    location
}
