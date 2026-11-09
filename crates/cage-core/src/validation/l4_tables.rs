//! L4 table-level validation pass (cardinality).
//! Split out of the validation module (LAY-003).

use super::{reference, table, DiagnosticBuilder, HashMap, Row, TypedValue, ValidationContext};

pub(crate) fn validate_table(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue;
        };

        // Primary key uniqueness
        if !table_schema.primary_key.is_empty() {
            let mut seen = HashMap::new();
            for row in &table.rows {
                let pk_values: Vec<String> = table_schema
                    .primary_key
                    .iter()
                    .filter_map(|pk| row.fields.get(pk).map(|v| v.value.to_string()))
                    .collect();
                let pk_key = pk_values.join("|");

                if let Some(first_row) = seen.get(&pk_key) {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(table::E1301, "Duplicate primary key")
                            .location(row.location.clone())
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .value(serde_json::to_value(&pk_values).unwrap_or_default())
                            .hint(format!("First occurrence at row {first_row}"))
                            .build(),
                    );
                } else {
                    seen.insert(pk_key, row.index);
                }
            }
        }

        // Composite unique constraints
        for uc in &table_schema.unique_constraints {
            let mut seen = HashMap::new();
            for row in &table.rows {
                let uc_values: Vec<String> = uc
                    .fields
                    .iter()
                    .filter_map(|f| row.fields.get(f).map(|v| v.value.to_string()))
                    .collect();
                let uc_key = uc_values.join("|");

                if let Some(first_row) = seen.get(&uc_key) {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(
                            table::E1302,
                            format!("Duplicate unique constraint '{}'", uc.name),
                        )
                        .location(row.location.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .value(serde_json::to_value(&uc_values).unwrap_or_default())
                        .hint(format!(
                            "Fields: {}, first at row {}",
                            uc.fields.join(", "),
                            first_row
                        ))
                        .build(),
                    );
                } else {
                    seen.insert(uc_key, row.index);
                }
            }
        }

        // Row ordering (if specified)
        if let Some(order_by) = &table_schema.order_by {
            let mut prev_key: Option<Vec<String>> = None;
            for row in &table.rows {
                let key: Vec<String> = order_by
                    .iter()
                    .filter_map(|f| row.fields.get(f).map(|v| v.value.to_string()))
                    .collect();
                if let Some(prev) = &prev_key {
                    if key < *prev {
                        ctx.diagnostics.add(
                            DiagnosticBuilder::error(table::E1304, "Row ordering violation")
                                .location(row.location.clone())
                                .table(table_name)
                                .row(format!("{}", row.index))
                                .hint(format!("Rows must be ordered by: {}", order_by.join(", ")))
                                .build(),
                        );
                    }
                }
                prev_key = Some(key);
            }
        }
    }
}

/// Source-side reference cardinality (the `cardinality:` key of a
/// `reference:` block, E1404): `one` (default) and `optional` take a single
/// value, `many` takes an array checked element by element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cardinality {
    /// Exactly one referenced target (a present null violates).
    One,
    /// An array of references, each checked against the target.
    Many,
    /// A single reference that may be absent or null.
    Optional,
}

pub(crate) fn parse_cardinality(raw: &str) -> Option<Cardinality> {
    match raw {
        "one" => Some(Cardinality::One),
        "many" => Some(Cardinality::Many),
        "optional" => Some(Cardinality::Optional),
        _ => None,
    }
}

/// E1404: the value's shape does not match the declared source-side
/// cardinality of the reference.
pub(crate) fn report_cardinality_violation(
    ctx: &mut ValidationContext,
    table_name: &str,
    row: &Row,
    field_name: &str,
    typed_value: &TypedValue,
    hint: String,
) {
    ctx.diagnostics.add(
        DiagnosticBuilder::error(reference::E1404, "Reference cardinality violation")
            .location(typed_value.location.clone())
            .table(table_name)
            .row(format!("{}", row.index))
            .field(field_name)
            .value(serde_json::to_value(&typed_value.value).unwrap_or_default())
            .hint(hint)
            .build(),
    );
}
