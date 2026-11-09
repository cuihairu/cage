//! L1 schema-structure validation pass plus profile-visibility checks.
//! Split out of the validation module (LAY-003).

use super::{
    build, parse, schema, Diagnostic, DiagnosticBuilder, Diagnostics, Schema, SourceLocation,
    ValidationContext,
};

pub(crate) fn validate_schema(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    // Check all tables in schema exist in document (if required)
    for (table_name, table_schema) in &schema.tables {
        let table = doc.tables.get(table_name);

        if table_schema.fields.iter().any(|(_, f)| f.required) && table.is_none() {
            // Table is required but missing - could be warning or error depending on config
            ctx.diagnostics.add(
                Diagnostic::warning(
                    parse::E0001,
                    format!("Expected table '{table_name}' not found in source"),
                )
                .with_source("schema")
                .with_table(table_name),
            );
            continue;
        }

        if let Some(table) = table {
            // Check required fields present in each row
            for row in &table.rows {
                for (field_name, field_schema) in &table_schema.fields {
                    if field_schema.required && !row.fields.contains_key(field_name) {
                        let loc = row.location.clone().with_field(field_name);
                        ctx.diagnostics.add(
                            DiagnosticBuilder::error(schema::E1001, "Missing required field")
                                .location(loc)
                                .table(table_name)
                                .row(format!("{}", row.index))
                                .field(field_name)
                                .hint(format!("Add required field '{field_name}' to this row"))
                                .build(),
                        );
                    }
                }

                // Check for unknown fields
                for field_name in row.fields.keys() {
                    if !table_schema.fields.contains_key(field_name) {
                        let loc = row.location.clone().with_field(field_name);
                        ctx.diagnostics.add(
                            DiagnosticBuilder::warning(
                                schema::E1002,
                                "Unknown field not defined in schema",
                            )
                            .location(loc)
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .field(field_name)
                            .hint("Remove this field or add it to the schema")
                            .build(),
                        );
                    }
                }
            }
        }
    }
}

/// E9006 — field-visibility conflicts in the active profile's projection
/// (Profile 语义化, v0.3). Empty `targets` means "visible in every profile";
/// stripping a field from the projection is a legitimate profile view only
/// while the remainder stays structurally valid. A projection that would
/// lose a required / primary-key / unique-constraint / reference-critical
/// field is a CONFLICT reported as E9006 — the review's "报冲突码而非静默
/// 过滤" — instead of silently producing an invalid view. Whole tables are
/// exempt: a table that excludes the profile disappears from the view
/// entirely, which is the table-level visibility semantic.
/// Schema-only profile-visibility check (E9006). Used by `cage gen`, which
/// skips data validation: the projection's structural validity is a schema
/// property and must gate code generation too.
pub fn check_profile_visibility(schema: &Schema, profile: &str) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    profile_visibility(schema, profile, &mut diagnostics);
    diagnostics
}

pub(crate) fn validate_profile_visibility(ctx: &mut ValidationContext) {
    let Some(profile) = ctx.profile else { return };
    profile_visibility(&ctx.schema.schema, profile, ctx.diagnostics);
}

pub(crate) fn profile_visibility(schema: &Schema, profile: &str, diagnostics: &mut Diagnostics) {
    let visible =
        |targets: &[String]| targets.is_empty() || targets.iter().any(|t| t == profile || t == "*");

    for (table_name, table) in &schema.tables {
        if !visible(&table.targets) {
            continue;
        }

        // Structurally required fields hidden by this profile → conflict.
        for (field_name, field) in &table.fields {
            if visible(&field.targets) {
                continue;
            }
            let loc = SourceLocation::new("schema").with_field(field_name);
            if field.required && field.default.is_none() {
                diagnostics.add(
                    DiagnosticBuilder::error(build::E9006, "Required field hidden by profile")
                        .location(loc.clone())
                        .table(table_name)
                        .field(field_name)
                        .hint(format!(
                            "field '{field_name}' is required and targets {:?}, which hides it \
                             from profile '{profile}'; remove 'targets', add '{profile}', or give \
                             the field a default",
                            field.targets
                        ))
                        .build(),
                );
            }
            if table.primary_key.iter().any(|k| k == field_name) {
                diagnostics.add(
                    DiagnosticBuilder::error(build::E9006, "Primary key field hidden by profile")
                        .location(loc.clone())
                        .table(table_name)
                        .field(field_name)
                        .hint(format!(
                            "primary key '{field_name}' of table '{table_name}' would be missing \
                             from profile '{profile}'"
                        ))
                        .build(),
                );
            }
            for constraint in &table.unique_constraints {
                if constraint.fields.iter().any(|f| f == field_name) {
                    diagnostics.add(
                        DiagnosticBuilder::error(
                            build::E9006,
                            "Unique-constraint field hidden by profile",
                        )
                        .location(loc.clone())
                        .table(table_name)
                        .field(field_name)
                        .hint(format!(
                            "unique constraint '{}' of table '{table_name}' would be incomplete \
                             in profile '{profile}'",
                            constraint.name
                        ))
                        .build(),
                    );
                }
            }
        }

        // Reference targets of visible fields must stay visible: a dangling
        // in-view reference is a projection conflict, not a silent drop.
        for (field_name, field) in &table.fields {
            if !visible(&field.targets) {
                continue;
            }
            let Some(reference) = &field.reference else {
                continue;
            };
            let Some(target_table) = schema.tables.get(&reference.table) else {
                continue;
            };
            let loc = SourceLocation::new("schema").with_field(field_name);
            if !visible(&target_table.targets) {
                diagnostics.add(
                    DiagnosticBuilder::error(
                        build::E9006,
                        "Reference target table hidden by profile",
                    )
                    .location(loc.clone())
                    .table(table_name)
                    .field(field_name)
                    .hint(format!(
                        "field '{field_name}' references table '{}', which is hidden from \
                         profile '{profile}'",
                        reference.table
                    ))
                    .build(),
                );
            } else if let Some(target_field) = target_table.fields.get(&reference.field) {
                if !visible(&target_field.targets) {
                    diagnostics.add(
                        DiagnosticBuilder::error(
                            build::E9006,
                            "Reference target field hidden by profile",
                        )
                        .location(loc)
                        .table(table_name)
                        .field(field_name)
                        .hint(format!(
                            "field '{field_name}' references '{}.{}', which is hidden from \
                             profile '{profile}'",
                            reference.table, reference.field
                        ))
                        .build(),
                    );
                }
            }
        }
    }
}
