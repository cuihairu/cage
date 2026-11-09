//! L6 semantic and L7 game-rule validation passes.
//! Split out of the validation module (LAY-003).

use super::{
    expr, rules, schema, semantic, DiagnosticBuilder, ExpressionRule, HashMap, Severity,
    ValidationContext,
};

pub(crate) fn validate_semantic(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    // Pre-resolve every rule once per table: a malformed assert or a
    // reference to an undeclared field is a schema defect (E1004) reported
    // once per rule, whether or not the table carries rows. Rules that
    // resolve cleanly are evaluated per row below.
    let mut active: HashMap<&str, Vec<(&ExpressionRule, expr::Comparison)>> = HashMap::new();
    for (table_name, table_schema) in &schema.tables {
        for (field_name, field_schema) in &table_schema.fields {
            for rule in &field_schema.rules {
                let cmp = match expr::parse_assert(&rule.assert) {
                    Ok(cmp) => cmp,
                    Err(err) => {
                        ctx.diagnostics.add(
                            DiagnosticBuilder::error(
                                schema::E1004,
                                format!(
                                    "semantic rule '{}' on {}.{} has a malformed assert: {}",
                                    rule.name, table_name, field_name, err
                                ),
                            )
                            .source("schema")
                            .table(table_name)
                            .field(field_name)
                            .hint("Asserts are `operand OP operand` — e.g. min_level <= max_level")
                            .build(),
                        );
                        continue;
                    }
                };
                let unknown = cmp
                    .field_refs()
                    .into_iter()
                    .find(|name| !table_schema.fields.contains_key(*name));
                if let Some(name) = unknown {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(
                            schema::E1004,
                            format!(
                                "semantic rule '{}' on {}.{} references undeclared field '{}'",
                                rule.name, table_name, field_name, name
                            ),
                        )
                        .source("schema")
                        .table(table_name)
                        .field(field_name)
                        .hint("Asserts may only reference fields declared in the same table")
                        .build(),
                    );
                    continue;
                }
                active.entry(table_name).or_default().push((rule, cmp));
            }
        }
    }

    for (table_name, table) in &doc.tables {
        let Some(rules) = active.get(table_name.as_str()) else {
            continue;
        };

        for row in &table.rows {
            for (rule, cmp) in rules {
                let hint = match expr::evaluate(cmp, row) {
                    expr::Outcome::Holds(true) | expr::Outcome::MissingField(_) => continue,
                    expr::Outcome::Holds(false) => rule
                        .message
                        .clone()
                        .unwrap_or_else(|| format!("Assertion '{}' failed", rule.assert)),
                    expr::Outcome::Incomparable { lhs, rhs } => format!(
                        "Assertion '{}' is not evaluable: operands are {lhs} and {rhs}",
                        rule.assert
                    ),
                };
                let severity = if rule.warning_only {
                    Severity::Warning
                } else {
                    Severity::Error
                };
                let mut diag = DiagnosticBuilder::error(semantic::E1501, "Semantic rule violation")
                    .location(row.location.clone())
                    .table(table_name)
                    .row(format!("{}", row.index))
                    .hint(hint)
                    .build();
                diag.severity = severity;
                ctx.diagnostics.add(diag);
            }
        }
    }
}

/// L7: Game Rule validation - plugin validators (see [`rules`]).
///
/// Runs the built-in registry today; the trait + registry are the stable
/// extension point for embedder-supplied and (phase 2) dynamic-library
/// validators — see design §17「插件沙箱方案定稿」.
pub(crate) fn validate_game_rule(ctx: &mut ValidationContext) {
    let registry = rules::GameRuleRegistry::with_builtins();
    for diagnostic in registry.run(&ctx.schema.schema, ctx.document) {
        ctx.diagnostics.add(diagnostic);
    }
}
