//! L5 cross-reference validation pass (single + cycle).
//! Split out of the validation module (LAY-003).

use super::{
    expr, parse_cardinality, reference, report_cardinality_violation, schema, Cardinality,
    DiagnosticBuilder, HashMap, ReferenceSchema, Row, Schema, TypedValue, ValidationContext, Value,
};

pub(crate) fn validate_reference(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    // Parse every reference predicate once up front: a malformed assert or a
    // reference to a field the target table does not declare is a schema
    // defect (E1004) reported once, independent of how many rows consult
    // it. `None` marks a defective predicate so the row loop skips it.
    // The cardinality spelling is validated in the same pass: an unknown
    // value is a schema defect (E1004) and the row loop skips the field.
    let mut predicates: HashMap<(&str, &str), Option<expr::Comparison>> = HashMap::new();
    let mut cardinalities: HashMap<(&str, &str), Cardinality> = HashMap::new();
    for (table_name, table_schema) in &schema.tables {
        for (field_name, field_schema) in &table_schema.fields {
            let Some(ref_schema) = &field_schema.reference else {
                continue;
            };
            match parse_cardinality(&ref_schema.cardinality) {
                Some(cardinality) => {
                    cardinalities.insert((table_name.as_str(), field_name.as_str()), cardinality);
                }
                None => {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(
                            schema::E1004,
                            format!(
                                "reference on {}.{} has unknown cardinality '{}' \
                                 (expected one | many | optional)",
                                table_name, field_name, ref_schema.cardinality
                            ),
                        )
                        .source("schema")
                        .table(table_name)
                        .field(field_name)
                        .hint(
                            "Cardinality is source-side: 'one' (default) and 'optional' \
                             take a single value, 'many' takes an array of references",
                        )
                        .build(),
                    );
                }
            }
            let Some(predicate) = &ref_schema.predicate else {
                continue;
            };
            let parsed = match expr::parse_assert(&predicate.assert) {
                Err(err) => {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(
                            schema::E1004,
                            format!(
                                "reference predicate '{}' on {}.{} has a malformed assert: {}",
                                predicate.name, table_name, field_name, err
                            ),
                        )
                        .source("schema")
                        .table(table_name)
                        .field(field_name)
                        .hint("Asserts are `operand OP operand` — e.g. min_level <= max_level")
                        .build(),
                    );
                    None
                }
                Ok(cmp) => {
                    let unknown = schema.tables.get(&ref_schema.table).and_then(|target| {
                        cmp.field_refs()
                            .into_iter()
                            .find(|name| !target.fields.contains_key(*name))
                    });
                    match unknown {
                        Some(name) => {
                            ctx.diagnostics.add(
                                DiagnosticBuilder::error(
                                    schema::E1004,
                                    format!(
                                        "reference predicate '{}' on {}.{} references \
                                         undeclared field '{}' of table '{}'",
                                        predicate.name,
                                        table_name,
                                        field_name,
                                        name,
                                        ref_schema.table
                                    ),
                                )
                                .source("schema")
                                .table(table_name)
                                .field(field_name)
                                .hint(
                                    "Predicates may only reference fields declared in the \
                                     referenced table",
                                )
                                .build(),
                            );
                            None
                        }
                        None => Some(cmp),
                    }
                }
            };
            predicates.insert((table_name.as_str(), field_name.as_str()), parsed);
        }
    }

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue;
        };

        for row in &table.rows {
            for (field_name, typed_value) in &row.fields {
                let Some(field_schema) = table_schema.fields.get(field_name) else {
                    continue;
                };

                if let Some(ref_schema) = &field_schema.reference {
                    let parsed = predicates
                        .get(&(table_name.as_str(), field_name.as_str()))
                        .and_then(|p| p.as_ref());
                    // An unknown cardinality spelling was already reported
                    // as E1004 in the pre-pass; the row loop skips the field.
                    let Some(cardinality) =
                        cardinalities.get(&(table_name.as_str(), field_name.as_str()))
                    else {
                        continue;
                    };
                    let value = &typed_value.value;
                    match cardinality {
                        Cardinality::One | Cardinality::Optional => match value {
                            // `optional` tolerates a present null; `one`
                            // demands exactly one referenced target.
                            Value::Null if *cardinality == Cardinality::Optional => {}
                            Value::Null => report_cardinality_violation(
                                ctx,
                                table_name,
                                row,
                                field_name,
                                typed_value,
                                format!(
                                    "cardinality 'one' on {table_name}.{field_name} expects a \
                                     single {} reference; the value is null — use \
                                     cardinality: optional if the reference may be absent",
                                    ref_schema.table
                                ),
                            ),
                            Value::Array(_) => report_cardinality_violation(
                                ctx,
                                table_name,
                                row,
                                field_name,
                                typed_value,
                                format!(
                                    "cardinality '{}' on {table_name}.{field_name} expects a \
                                     single {} reference; the value is an array — use \
                                     cardinality: many to reference a list",
                                    ref_schema.cardinality, ref_schema.table
                                ),
                            ),
                            _ => validate_single_reference(
                                ctx,
                                table_name,
                                row,
                                field_name,
                                typed_value,
                                ref_schema,
                                parsed,
                            ),
                        },
                        Cardinality::Many => match value {
                            Value::Array(items) => {
                                for item in items {
                                    let element = TypedValue {
                                        value: item.clone(),
                                        location: typed_value.location.clone(),
                                        schema_type: typed_value.schema_type.clone(),
                                    };
                                    validate_single_reference(
                                        ctx, table_name, row, field_name, &element, ref_schema,
                                        parsed,
                                    );
                                }
                            }
                            Value::Null => report_cardinality_violation(
                                ctx,
                                table_name,
                                row,
                                field_name,
                                typed_value,
                                format!(
                                    "cardinality 'many' on {table_name}.{field_name} expects \
                                     an array of {} references; the value is null",
                                    ref_schema.table
                                ),
                            ),
                            _ => report_cardinality_violation(
                                ctx,
                                table_name,
                                row,
                                field_name,
                                typed_value,
                                format!(
                                    "cardinality 'many' on {table_name}.{field_name} expects \
                                     an array of {} references; the value is a single {}",
                                    ref_schema.table,
                                    value.type_name()
                                ),
                            ),
                        },
                    }
                }
            }
        }
    }

    report_reference_cycles(ctx, schema);
}

/// Table-level reference cycles → E1403 warnings (one per unique cycle).
///
/// The graph module's own contract counts self-loops as cycles
/// (`find_cycles` includes them). A cycle cannot break check/build:
/// generation is single-pass from the in-memory Document and incremental
/// propagation is visited-set safe — but it makes the library build-order
/// API (`topological_sort` / `build_order`) fail, so it is reported for
/// authors instead of rejected. ERROR severity would over-reject
/// legitimate mutual-reference schemas (drop tables ↔ monster tables).
pub(crate) fn report_reference_cycles(ctx: &mut ValidationContext, schema: &Schema) {
    let graph = crate::reference::DependencyGraph::from_schema(schema);

    // `find_cycles` reports a cycle once per starting table ([A,B] and
    // [B,A] for a 2-cycle); rotate each cycle so the lexicographically
    // smallest table leads, then dedupe — rendering stays canonical and
    // deterministic regardless of HashMap iteration order.
    let mut canonical: std::collections::BTreeSet<Vec<String>> = std::collections::BTreeSet::new();
    for cycle in graph.find_cycles() {
        if cycle.is_empty() {
            continue;
        }
        let lead = cycle
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.cmp(b.1))
            .map_or(0, |(idx, _)| idx);
        let rotated: Vec<String> = cycle[lead..]
            .iter()
            .chain(cycle[..lead].iter())
            .cloned()
            .collect();
        canonical.insert(rotated);
    }

    for cycle in canonical {
        let chain = cycle
            .iter()
            .cloned()
            .chain(std::iter::once(cycle[0].clone()))
            .collect::<Vec<_>>()
            .join(" → ");
        ctx.diagnostics.add(
            DiagnosticBuilder::warning(reference::E1403, format!("Circular reference: {chain}"))
                .source("schema")
                .table(cycle[0].clone())
                .hint(
                    "Remove one reference edge to break the cycle; cycles do not \
                     block builds but make the topological build order impossible",
                )
                .build(),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_single_reference(
    ctx: &mut ValidationContext,
    table_name: &str,
    row: &Row,
    field_name: &str,
    typed_value: &TypedValue,
    ref_schema: &ReferenceSchema,
    parsed_predicate: Option<&expr::Comparison>,
) {
    let value = &typed_value.value;
    let loc = typed_value.location.clone();
    let ref_value_str = value.coerce_to_string().unwrap_or_default();

    // Check existence
    let target_row_idx =
        ctx.reference_cache
            .find_row(&ref_schema.table, &ref_schema.field, &ref_value_str);

    if target_row_idx.is_none() {
        ctx.diagnostics.add(
            DiagnosticBuilder::error(reference::E1401, "Referenced target does not exist")
                .location(loc.clone())
                .table(table_name)
                .row(format!("{}", row.index))
                .field(field_name)
                .value(serde_json::to_value(value).unwrap_or_default())
                .hint(format!(
                    "No {} with {}={} found",
                    ref_schema.table, ref_schema.field, ref_value_str
                ))
                .build(),
        );
        return;
    }

    let target_row_idx = target_row_idx.unwrap();
    let target_row = ctx.document.tables[&ref_schema.table].rows[target_row_idx].clone();

    // E1410: Check predicate (semantic compatibility). The parsed form comes
    // from the caller's pre-pass; a defective predicate was already reported
    // as E1004 and arrives here as None. An absent (optional) field on the
    // target row says nothing about the predicate — pass.
    if let Some(predicate) = &ref_schema.predicate {
        if let Some(cmp) = parsed_predicate {
            let hint = match expr::evaluate(cmp, &target_row) {
                expr::Outcome::Holds(true) | expr::Outcome::MissingField(_) => None,
                expr::Outcome::Holds(false) => Some(format!(
                    "Predicate '{}' not satisfied by referenced {}",
                    predicate.assert, ref_schema.table
                )),
                expr::Outcome::Incomparable { lhs, rhs } => Some(format!(
                    "Predicate '{}' is not evaluable on the referenced {}: operands are {lhs} and {rhs}",
                    predicate.assert, ref_schema.table
                )),
            };
            if let Some(hint) = hint {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(
                        reference::E1410,
                        "Referenced object exists but fails semantic predicate",
                    )
                    .location(loc.clone())
                    .table(table_name)
                    .row(format!("{}", row.index))
                    .field(field_name)
                    .value(serde_json::to_value(value).unwrap_or_default())
                    .hint(hint)
                    .related(
                        reference::E1410,
                        "Referenced row here",
                        target_row.location.clone(),
                    )
                    .build(),
                );
            }
        }
    }

    // Check compatible_with (field value constraints on referenced object)
    if let Some(compatible_values) = &ref_schema.compatible_with {
        // Find a field in target that indicates "type" or similar
        let target_table_schema = &ctx.schema.schema.tables[&ref_schema.table];
        // This is a simplified check - in reality would be more flexible
        for (target_field_name, target_typed_value) in &target_row.fields {
            if let Some(target_field_schema) = target_table_schema.fields.get(target_field_name) {
                if target_field_schema.enum_values.is_some() {
                    let target_val_str = target_typed_value
                        .value
                        .coerce_to_string()
                        .unwrap_or_default();
                    if !compatible_values.contains(&target_val_str) {
                        ctx.diagnostics.add(
                            DiagnosticBuilder::error(
                                reference::E1411,
                                "Referenced object field value not compatible",
                            )
                            .location(loc.clone())
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .field(field_name)
                            .value(serde_json::to_value(value).unwrap_or_default())
                            .hint(format!(
                                "Referenced {}.{} = '{}', allowed: {}",
                                ref_schema.table,
                                target_field_name,
                                target_val_str,
                                compatible_values.join(", ")
                            ))
                            .build(),
                        );
                    }
                }
            }
        }
    }
}
