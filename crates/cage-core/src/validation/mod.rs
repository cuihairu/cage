//! Validation Pipeline - L0 through L7 validation levels
//! Each level can be run independently via `--level` flag

pub mod rules;

mod expr;

use crate::diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics, Severity};
use crate::error::codes::{build, parse, reference, schema, semantic, table, type_val, value};
use crate::schema::{
    ExpressionRule, FieldSchema, FieldType, MapKeyType, ReferenceSchema, Schema, ValidatedSchema,
};
use crate::value::{Document, Row, SourceLocation, TypedValue, Value};
use std::collections::HashMap;

/// Validation level enumeration
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ValidationLevel {
    /// L0: Parse/Syntax validation
    Parse,
    /// L1: Schema structure validation
    Schema,
    /// L2: Type validation
    Type,
    /// L3: Value constraint validation
    Value,
    /// L4: Table-level validation
    Table,
    /// L5: Cross-reference validation
    Reference,
    /// L6: Semantic expression validation
    Semantic,
    /// L7: Game rule validator plugins
    #[default]
    GameRule,
}

impl ValidationLevel {
    /// All levels, in pipeline order
    pub fn all() -> Vec<ValidationLevel> {
        vec![
            ValidationLevel::Parse,
            ValidationLevel::Schema,
            ValidationLevel::Type,
            ValidationLevel::Value,
            ValidationLevel::Table,
            ValidationLevel::Reference,
            ValidationLevel::Semantic,
            ValidationLevel::GameRule,
        ]
    }

    /// Parse a level name / alias ("parse", "l0", "0", "ref", …)
    pub fn parse_level(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "parse" | "l0" | "0" => Some(ValidationLevel::Parse),
            "schema" | "l1" | "1" => Some(ValidationLevel::Schema),
            "type" | "l2" | "2" => Some(ValidationLevel::Type),
            "value" | "l3" | "3" => Some(ValidationLevel::Value),
            "table" | "l4" | "4" => Some(ValidationLevel::Table),
            "reference" | "ref" | "l5" | "5" => Some(ValidationLevel::Reference),
            "semantic" | "l6" | "6" => Some(ValidationLevel::Semantic),
            "gamerule" | "game-rule" | "l7" | "7" => Some(ValidationLevel::GameRule),
            _ => None,
        }
    }

    /// Canonical lowercase name used by the CLI
    pub fn as_str(&self) -> &'static str {
        match self {
            ValidationLevel::Parse => "parse",
            ValidationLevel::Schema => "schema",
            ValidationLevel::Type => "type",
            ValidationLevel::Value => "value",
            ValidationLevel::Table => "table",
            ValidationLevel::Reference => "reference",
            ValidationLevel::Semantic => "semantic",
            ValidationLevel::GameRule => "gamerule",
        }
    }
}

impl std::str::FromStr for ValidationLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_level(s).ok_or_else(|| format!("Unknown validation level: {s}"))
    }
}

/// Validation context passed through pipeline
#[derive(Debug)]
pub struct ValidationContext<'a> {
    /// Schema being validated against
    pub schema: &'a ValidatedSchema,
    /// Document being validated
    pub document: &'a Document,
    /// Accumulated diagnostics
    pub diagnostics: &'a mut Diagnostics,
    /// Highest level to run
    pub max_level: ValidationLevel,
    /// Whether warnings are escalated to errors
    pub warnings_as_errors: bool,
    /// Active profile (E9006 field-visibility checks); None = no profile
    /// semantics: every field is visible
    pub profile: Option<&'a str>,
    /// Resolved references cache for L5
    pub reference_cache: ReferenceCache,
}

/// Cache for resolved references to avoid repeated lookups
#[derive(Debug, Default)]
pub struct ReferenceCache {
    /// `table_name` -> `field_name` -> value -> `row_index`
    pub index: HashMap<String, HashMap<String, HashMap<String, usize>>>,
}

impl ReferenceCache {
    /// Build the primary-key index for the whole document
    pub fn build(document: &Document, schema: &ValidatedSchema) -> Self {
        let mut cache = Self::default();
        for (table_name, table) in &document.tables {
            if let Some(table_schema) = schema.schema.tables.get(table_name) {
                for pk_field in &table_schema.primary_key {
                    let mut field_map: HashMap<String, HashMap<String, usize>> = HashMap::new();
                    for (row_idx, row) in table.rows.iter().enumerate() {
                        if let Some(pk_value) = row.fields.get(pk_field) {
                            let key = pk_value.value.to_string();
                            field_map
                                .entry(pk_field.clone())
                                .or_default()
                                .insert(key, row_idx);
                        }
                    }
                    cache
                        .index
                        .entry(table_name.clone())
                        .or_default()
                        .extend(field_map);
                }
            }
        }
        cache
    }

    /// Look up the row index matching a field value
    pub fn find_row(&self, table: &str, field: &str, value: &str) -> Option<usize> {
        self.index.get(table)?.get(field)?.get(value).copied()
    }
}

/// Main validation entry point (no active profile — the full schema view)
pub fn validate(
    schema: &ValidatedSchema,
    document: &Document,
    max_level: ValidationLevel,
    warnings_as_errors: bool,
) -> Diagnostics {
    validate_with_profile(schema, document, max_level, warnings_as_errors, None)
}

/// Profile-aware entry point. `profile` gates the E9006 field-visibility
/// checks (L1) and is carried in the validation context.
pub fn validate_with_profile(
    schema: &ValidatedSchema,
    document: &Document,
    max_level: ValidationLevel,
    warnings_as_errors: bool,
    profile: Option<&str>,
) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    let mut ctx = ValidationContext {
        schema,
        document,
        diagnostics: &mut diagnostics,
        max_level,
        warnings_as_errors,
        profile,
        reference_cache: ReferenceCache::build(document, schema),
    };

    // Run validation levels up to max_level
    if max_level >= ValidationLevel::Parse {
        validate_parse(&mut ctx);
    }
    if max_level >= ValidationLevel::Schema {
        validate_schema(&mut ctx);
        validate_profile_visibility(&mut ctx);
    }
    if max_level >= ValidationLevel::Type {
        validate_type(&mut ctx);
    }
    if max_level >= ValidationLevel::Value {
        validate_value(&mut ctx);
    }
    if max_level >= ValidationLevel::Table {
        validate_table(&mut ctx);
    }
    if max_level >= ValidationLevel::Reference {
        validate_reference(&mut ctx);
    }
    if max_level >= ValidationLevel::Semantic {
        validate_semantic(&mut ctx);
    }
    if max_level >= ValidationLevel::GameRule {
        validate_game_rule(&mut ctx);
    }

    // Convert warnings to errors if requested
    if ctx.warnings_as_errors {
        for diag in ctx.diagnostics.iter_mut() {
            if diag.severity == Severity::Warning {
                diag.severity = Severity::Error;
            }
        }
    }

    ctx.diagnostics.sort_by_location();
    diagnostics
}

/// L0: Parse validation - already done by source adapters, but check for any residual issues
fn validate_parse(_ctx: &mut ValidationContext) {
    // Source adapters should have caught parse errors
    // This level is a no-op in core, but exists for completeness
}

/// L1: Schema validation - check structure matches schema
fn validate_schema(ctx: &mut ValidationContext) {
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

fn validate_profile_visibility(ctx: &mut ValidationContext) {
    let Some(profile) = ctx.profile else { return };
    profile_visibility(&ctx.schema.schema, profile, ctx.diagnostics);
}

fn profile_visibility(schema: &Schema, profile: &str, diagnostics: &mut Diagnostics) {
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

/// L2: Type validation - check values match declared types
fn validate_type(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue; // Unknown table, already warned in L1
        };

        for row in &table.rows {
            for (field_name, typed_value) in &row.fields {
                let Some(field_schema) = table_schema.fields.get(field_name) else {
                    continue; // Unknown field, already warned in L1
                };

                if value_matches_type(&typed_value.value, &field_schema.field_type) {
                    // Store resolved type for later stages
                    // (in practice, we'd mutate typed_value, but it's borrowed)
                } else {
                    let loc = typed_value.location.clone();
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(type_val::E1101, "Type mismatch")
                            .location(loc)
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .field(field_name)
                            .value(serde_json::to_value(&typed_value.value).unwrap_or_default())
                            .hint(type_mismatch_hint(
                                &field_schema.field_type,
                                typed_value.value.type_name(),
                            ))
                            .build(),
                    );
                }
            }
        }
    }
}

fn value_matches_type(value: &Value, expected: &FieldType) -> bool {
    match (value, expected) {
        (Value::Array(arr), FieldType::Array(inner)) => {
            arr.iter().all(|v| value_matches_type(v, inner))
        }
        (Value::Object(obj), FieldType::Object(fields)) => {
            obj.keys().all(|k| fields.contains_key(k))
                && obj
                    .iter()
                    .all(|(k, v)| fields.get(k).is_some_and(|ft| value_matches_type(v, ft)))
        }
        // Map: an open-ended object — key_type gates which keys are legal
        // (int keys are numeric strings in the data model), value_type gates
        // every value recursively. An empty object is always a valid map.
        (Value::Object(obj), FieldType::Map(map)) => {
            obj.iter().all(|(k, _)| match map.key_type {
                MapKeyType::String => true, // Value::Object keys are String by construction
                MapKeyType::Int => k.parse::<i64>().is_ok(),
            }) && obj
                .iter()
                .all(|(_, v)| value_matches_type(v, &map.value_type))
        }
        // Enum membership itself is checked in L3 (value constraints)
        (Value::String(_), FieldType::Enum(_)) => true,
        // Scalars: only the family must match; width checking happens in L3
        _ if matches!(
            (value, expected),
            (Value::Null, FieldType::Null)
                | (Value::Bool(_), FieldType::Bool)
                | (
                    Value::Int(_),
                    FieldType::Int8 | FieldType::Int16 | FieldType::Int32 | FieldType::Int64
                )
                | (
                    Value::UInt(_),
                    FieldType::UInt8 | FieldType::UInt16 | FieldType::UInt32 | FieldType::UInt64
                )
                | (Value::Float(_), FieldType::Float32 | FieldType::Float64)
                | (Value::String(_), FieldType::String)
                | (Value::Bytes(_), FieldType::Bytes)
                | (_, FieldType::Any)
        ) =>
        {
            true
        }
        _ => false,
    }
}

/// Hint text for an E1101 type mismatch.
///
/// Non-map expectations keep the plain `Debug` rendering byte-for-byte
/// (`Expected type: Int32, got: string`); a map expectation is spelled
/// `map<string, Array(Int32)>`-style because `Debug` on the payload struct
/// (`Map(MapField { key_type: String, value_type: ... })`) reads poorly.
fn type_mismatch_hint(expected: &FieldType, got: &str) -> String {
    match expected {
        FieldType::Map(map) => format!(
            "Expected type: map<{}, {:?}>, got: {got}",
            match map.key_type {
                MapKeyType::String => "string",
                MapKeyType::Int => "int",
            },
            map.value_type
        ),
        _ => format!("Expected type: {expected:?}, got: {got}"),
    }
}

/// L3: Value constraint validation - min/max, length, pattern, enum
fn validate_value(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue;
        };

        for row in &table.rows {
            for (field_name, typed_value) in &row.fields {
                let Some(field_schema) = table_schema.fields.get(field_name) else {
                    continue;
                };

                validate_field_constraints(
                    ctx,
                    table_name,
                    row,
                    field_name,
                    typed_value,
                    field_schema,
                );
            }
        }
    }
}

fn validate_field_constraints(
    ctx: &mut ValidationContext,
    table_name: &str,
    row: &Row,
    field_name: &str,
    typed_value: &TypedValue,
    field_schema: &FieldSchema,
) {
    let value = &typed_value.value;
    let loc = typed_value.location.clone();

    // Numeric range
    if let Some(min) = field_schema.min {
        if let Some(num) = value.coerce_to_float() {
            if num < min {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1201, "Value below minimum")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Minimum allowed: {min}"))
                        .build(),
                );
            }
        }
    }
    if let Some(max) = field_schema.max {
        if let Some(num) = value.coerce_to_float() {
            if num > max {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1201, "Value exceeds maximum")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Maximum allowed: {max}"))
                        .build(),
                );
            }
        }
    }

    // String length
    if let Value::String(s) = value {
        if let Some(min_len) = field_schema.min_length {
            if s.len() < min_len {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1202, "String too short")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Minimum length: {min_len}"))
                        .build(),
                );
            }
        }
        if let Some(max_len) = field_schema.max_length {
            if s.len() > max_len {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1202, "String too long")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Maximum length: {max_len}"))
                        .build(),
                );
            }
        }
        // Pattern
        if let Some(pattern) = &field_schema.pattern {
            if let Ok(re) = regex::Regex::new(pattern) {
                if !re.is_match(s) {
                    ctx.diagnostics.add(
                        DiagnosticBuilder::error(value::E1203, "String does not match pattern")
                            .location(loc.clone())
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .field(field_name)
                            .value(serde_json::to_value(value).unwrap_or_default())
                            .hint(format!("Pattern: {pattern}"))
                            .build(),
                    );
                }
            }
        }
    }

    // Enum values
    if let Some(enum_values) = &field_schema.enum_values {
        let val_str = value.coerce_to_string().unwrap_or_default();
        if !enum_values.contains(&val_str) {
            ctx.diagnostics.add(
                DiagnosticBuilder::error(value::E1204, "Value not in allowed enum")
                    .location(loc.clone())
                    .table(table_name)
                    .row(format!("{}", row.index))
                    .field(field_name)
                    .value(serde_json::to_value(value).unwrap_or_default())
                    .hint(format!("Allowed values: {}", enum_values.join(", ")))
                    .build(),
            );
        }
    }

    // Named enum membership (`type: {kind: Enum, value: X}`): the value
    // must be one of the declared members of the schema-level enum. This
    // is a disjoint surface from inline `enum_values` (above) and
    // previously had no check at all — L2 passes any String for Enum
    // (see `value_matches_type`). Only well-typed strings are checked:
    // a non-string value is an L2 type mismatch (E1101), already
    // reported there — no double diagnosis. A dangling enum name is an
    // L1 schema check (E1004), so an unknown name falls through here.
    if let FieldType::Enum(enum_name) = &field_schema.field_type {
        if let (Some(enum_schema), Value::String(val_str)) =
            (ctx.schema.schema.enums.get(enum_name), value)
        {
            let members: Vec<&str> = enum_schema.values.iter().map(|v| v.name.as_str()).collect();
            if !members.contains(&val_str.as_str()) {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1204, "Value not in allowed enum")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Allowed values: {}", members.join(", ")))
                        .build(),
                );
            }
        }
    }

    // Array length
    if let Value::Array(arr) = value {
        if let Some(min_items) = field_schema.min_items {
            if arr.len() < min_items {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1205, "Array too short")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Minimum items: {min_items}"))
                        .build(),
                );
            }
        }
        if let Some(max_items) = field_schema.max_items {
            if arr.len() > max_items {
                ctx.diagnostics.add(
                    DiagnosticBuilder::error(value::E1205, "Array too long")
                        .location(loc.clone())
                        .table(table_name)
                        .row(format!("{}", row.index))
                        .field(field_name)
                        .value(serde_json::to_value(value).unwrap_or_default())
                        .hint(format!("Maximum items: {max_items}"))
                        .build(),
                );
            }
        }
    }
}

/// L4: Table-level validation - primary key uniqueness, composite unique, ordering
fn validate_table(ctx: &mut ValidationContext) {
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
enum Cardinality {
    /// Exactly one referenced target (a present null violates).
    One,
    /// An array of references, each checked against the target.
    Many,
    /// A single reference that may be absent or null.
    Optional,
}

fn parse_cardinality(raw: &str) -> Option<Cardinality> {
    match raw {
        "one" => Some(Cardinality::One),
        "many" => Some(Cardinality::Many),
        "optional" => Some(Cardinality::Optional),
        _ => None,
    }
}

/// E1404: the value's shape does not match the declared source-side
/// cardinality of the reference.
fn report_cardinality_violation(
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

/// L5: Reference validation - existence, type compatibility, predicates
fn validate_reference(ctx: &mut ValidationContext) {
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
                                        ctx,
                                        table_name,
                                        row,
                                        field_name,
                                        &element,
                                        ref_schema,
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
fn report_reference_cycles(ctx: &mut ValidationContext, schema: &Schema) {
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
fn validate_single_reference(
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

/// L6: Semantic validation - expression rules on rows
fn validate_semantic(ctx: &mut ValidationContext) {
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
fn validate_game_rule(ctx: &mut ValidationContext) {
    let registry = rules::GameRuleRegistry::with_builtins();
    for diagnostic in registry.run(&ctx.schema.schema, ctx.document) {
        ctx.diagnostics.add(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldSchema, FieldType, Schema, TableSchema};
    use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
    use indexmap::IndexMap;

    fn make_test_schema() -> Schema {
        let mut schema = Schema::new();
        let mut item = TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        item.fields.insert(
            "id".to_string(),
            FieldSchema {
                name: "id".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
                default: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
                enum_values: None,
                min_items: None,
                max_items: None,
                items: None,
                properties: None,
                additional_properties: None,
                reference: None,
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        item.fields.insert(
            "price".to_string(),
            FieldSchema {
                name: "price".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
                default: None,
                min: Some(0.0),
                max: Some(10000.0),
                min_length: None,
                max_length: None,
                pattern: None,
                enum_values: None,
                min_items: None,
                max_items: None,
                items: None,
                properties: None,
                additional_properties: None,
                reference: None,
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        schema.add_table(item);
        schema
    }

    fn make_test_doc() -> Document {
        let mut doc = Document::new();
        let mut table = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "items.json".to_string(),
            sheet: None,
        };
        table.rows.push(Row {
            primary_key: vec![Value::UInt(1)],
            fields: {
                let mut fields = IndexMap::new();
                fields.insert(
                    "id".to_string(),
                    TypedValue::new(
                        Value::UInt(1),
                        SourceLocation::new("items.json")
                            .with_row(1)
                            .with_field("id"),
                    ),
                );
                fields.insert(
                    "price".to_string(),
                    TypedValue::new(
                        Value::UInt(100),
                        SourceLocation::new("items.json")
                            .with_row(1)
                            .with_field("price"),
                    ),
                );
                fields
            },
            location: SourceLocation::new("items.json").with_row(1),
            index: 0,
        });
        table.rows.push(Row {
            primary_key: vec![Value::UInt(2)],
            fields: {
                let mut fields = IndexMap::new();
                fields.insert(
                    "id".to_string(),
                    TypedValue::new(
                        Value::UInt(2),
                        SourceLocation::new("items.json")
                            .with_row(2)
                            .with_field("id"),
                    ),
                );
                fields.insert(
                    "price".to_string(),
                    TypedValue::new(
                        Value::UInt(99999),
                        SourceLocation::new("items.json")
                            .with_row(2)
                            .with_field("price"),
                    ),
                ); // exceeds max
                fields
            },
            location: SourceLocation::new("items.json").with_row(2),
            index: 1,
        });
        doc.add_table(table);
        doc
    }

    /// Field with no constraints; tests override the slots they exercise.
    fn plain_field(name: &str, field_type: FieldType) -> FieldSchema {
        FieldSchema {
            name: name.to_string(),
            field_type,
            description: None,
            required: false,
            default: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            enum_values: None,
            min_items: None,
            max_items: None,
            items: None,
            properties: None,
            additional_properties: None,
            reference: None,
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        }
    }

    /// Table with no unique constraints / ordering; PK list may be empty.
    fn plain_table(name: &str, primary_key: &[&str], fields: Vec<FieldSchema>) -> TableSchema {
        let mut table = TableSchema {
            name: name.to_string(),
            description: None,
            primary_key: primary_key.iter().map(|pk| (*pk).to_string()).collect(),
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        for field in fields {
            table.fields.insert(field.name.clone(), field);
        }
        table
    }

    fn validated(schema: Schema) -> ValidatedSchema {
        ValidatedSchema {
            schema,
            dependency_graph: crate::reference::DependencyGraph::new(),
        }
    }

    fn str_val(s: &str) -> Value {
        Value::String(s.to_string())
    }

    fn row(index: usize, fields: &[(&str, Value)]) -> Row {
        let mut map = IndexMap::new();
        for (name, value) in fields {
            map.insert(
                name.to_string(),
                TypedValue::new(
                    value.clone(),
                    SourceLocation::new("sheet.json")
                        .with_row(index + 1)
                        .with_field(*name),
                ),
            );
        }
        Row {
            primary_key: vec![],
            fields: map,
            location: SourceLocation::new("sheet.json").with_row(index + 1),
            index,
        }
    }

    fn doc_with_tables(tables: &[(&str, Vec<Row>)]) -> Document {
        let mut doc = Document::new();
        for (name, rows) in tables {
            doc.add_table(Table {
                name: (*name).to_string(),
                primary_key_fields: vec![],
                rows: rows.clone(),
                source_file: "sheet.json".to_string(),
                sheet: None,
            });
        }
        doc
    }

    fn doc_with(name: &str, rows: Vec<Row>) -> Document {
        doc_with_tables(&[(name, rows)])
    }

    fn object_val(pairs: &[(&str, Value)]) -> Value {
        Value::Object(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
        )
    }

    fn object_type(pairs: &[(&str, FieldType)]) -> FieldType {
        FieldType::Object(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
        )
    }

    fn array_type(inner: FieldType) -> FieldType {
        FieldType::Array(Box::new(inner))
    }

    fn map_type(key_type: MapKeyType, value_type: FieldType) -> FieldType {
        FieldType::Map(crate::schema::MapField {
            key_type,
            value_type: Box::new(value_type),
        })
    }

    #[test]
    fn level_all_lists_pipeline_order_and_as_str_matches() {
        let names: Vec<&str> = ValidationLevel::all()
            .iter()
            .map(ValidationLevel::as_str)
            .collect();
        assert_eq!(
            names,
            vec![
                "parse",
                "schema",
                "type",
                "value",
                "table",
                "reference",
                "semantic",
                "gamerule",
            ]
        );
    }

    #[test]
    fn parse_level_accepts_every_alias_and_rejects_unknown() {
        for (input, expected) in [
            ("parse", ValidationLevel::Parse),
            ("L0", ValidationLevel::Parse),
            ("0", ValidationLevel::Parse),
            ("schema", ValidationLevel::Schema),
            ("l1", ValidationLevel::Schema),
            ("1", ValidationLevel::Schema),
            ("type", ValidationLevel::Type),
            ("l2", ValidationLevel::Type),
            ("2", ValidationLevel::Type),
            ("value", ValidationLevel::Value),
            ("l3", ValidationLevel::Value),
            ("3", ValidationLevel::Value),
            ("table", ValidationLevel::Table),
            ("l4", ValidationLevel::Table),
            ("4", ValidationLevel::Table),
            ("reference", ValidationLevel::Reference),
            ("REF", ValidationLevel::Reference),
            ("l5", ValidationLevel::Reference),
            ("5", ValidationLevel::Reference),
            ("semantic", ValidationLevel::Semantic),
            ("l6", ValidationLevel::Semantic),
            ("6", ValidationLevel::Semantic),
            ("gamerule", ValidationLevel::GameRule),
            ("game-rule", ValidationLevel::GameRule),
            ("l7", ValidationLevel::GameRule),
            ("7", ValidationLevel::GameRule),
        ] {
            assert_eq!(
                ValidationLevel::parse_level(input),
                Some(expected),
                "alias {input}"
            );
        }
        assert_eq!(ValidationLevel::parse_level("gamerules"), None);
        assert_eq!(ValidationLevel::parse_level("l8"), None);
        assert_eq!(ValidationLevel::parse_level(""), None);
    }

    #[test]
    fn from_str_delegates_to_parse_level() {
        assert_eq!(
            "ref".parse::<ValidationLevel>(),
            Ok(ValidationLevel::Reference)
        );
        assert_eq!(
            "nope".parse::<ValidationLevel>(),
            Err("Unknown validation level: nope".to_string())
        );
    }

    #[test]
    fn reference_cache_indexes_pks_and_reports_misses() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        let vs = validated(schema);
        let mut doc = doc_with(
            "Item",
            vec![
                row(0, &[("id", Value::UInt(7))]),
                row(1, &[("id", Value::UInt(8))]),
                row(2, &[]), // row missing its pk field is skipped while indexing
            ],
        );
        // Tables unknown to the schema are not indexed at all.
        doc.add_table(Table {
            name: "Orphan".to_string(),
            primary_key_fields: vec![],
            rows: vec![row(0, &[("id", Value::UInt(1))])],
            source_file: "orphan.json".to_string(),
            sheet: None,
        });

        let cache = ReferenceCache::build(&doc, &vs);
        assert_eq!(cache.find_row("Item", "id", "7"), Some(0));
        assert_eq!(cache.find_row("Item", "id", "8"), Some(1));
        assert_eq!(cache.find_row("Item", "id", "9"), None);
        assert_eq!(cache.find_row("Orphan", "id", "1"), None);
        assert_eq!(cache.find_row("Item", "name", "7"), None);
    }

    #[test]
    fn l1_warns_when_required_table_missing_from_document() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![FieldSchema {
                required: true,
                ..plain_field("id", FieldType::UInt32)
            }],
        ));
        let vs = validated(schema);

        let diags = validate(&vs, &Document::new(), ValidationLevel::Schema, false);
        assert!(diags.errors().is_empty());
        let warnings = diags.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, parse::E0001);
        assert_eq!(warnings[0].table.as_deref(), Some("Item"));
        assert!(warnings[0]
            .message
            .contains("Expected table 'Item' not found in source"));
    }

    #[test]
    fn l1_ignores_missing_table_when_no_field_is_required() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Tag",
            &[],
            vec![plain_field("label", FieldType::String)],
        ));
        let vs = validated(schema);

        let diags = validate(&vs, &Document::new(), ValidationLevel::Schema, false);
        assert!(diags.is_empty());
    }

    #[test]
    fn l1_errors_on_missing_required_field() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                FieldSchema {
                    required: true,
                    ..plain_field("id", FieldType::UInt32)
                },
                FieldSchema {
                    required: true,
                    ..plain_field("name", FieldType::String)
                },
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with("Item", vec![row(0, &[("id", Value::UInt(1))])]);

        let diags = validate(&vs, &doc, ValidationLevel::Schema, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, schema::E1001);
        assert_eq!(errors[0].field.as_deref(), Some("name"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Add required field 'name'")));
    }

    #[test]
    fn l1_warns_on_unknown_field_and_escalation_makes_it_an_error() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(0, &[("id", Value::UInt(1)), ("bonus", Value::UInt(5))])],
        );

        let as_warning = validate(&vs, &doc, ValidationLevel::Schema, false);
        assert!(as_warning.errors().is_empty());
        let warnings = as_warning.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, schema::E1002);
        assert_eq!(warnings[0].field.as_deref(), Some("bonus"));
        assert_eq!(warnings[0].severity, Severity::Warning);

        let escalated = validate(&vs, &doc, ValidationLevel::Schema, true);
        assert!(escalated.warnings().is_empty());
        let errors = escalated.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, schema::E1002);
        assert_eq!(errors[0].severity, Severity::Error);
    }

    #[test]
    fn l2_reports_type_mismatch_e1101() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("count", FieldType::Int32),
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[("id", Value::UInt(1)), ("count", str_val("many"))],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Type, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, type_val::E1101);
        assert_eq!(errors[0].field.as_deref(), Some("count"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Int32") && h.contains("got: string")));
    }

    /// Table used by the map-focused L2 tests: `weights` is `map<int, Int32>`,
    /// `drops` is `map<string, Array<Int32>>`.
    fn map_schema() -> Schema {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("weights", map_type(MapKeyType::Int, FieldType::Int32)),
                plain_field(
                    "drops",
                    map_type(MapKeyType::String, array_type(FieldType::Int32)),
                ),
            ],
        ));
        schema
    }

    #[test]
    fn l2_map_non_object_reports_e1101_with_row_location() {
        let vs = validated(map_schema());
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("weights", str_val("not a map")),
                    ("drops", object_val(&[])),
                ],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Type, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1, "diags: {diags:?}");
        let err = errors[0];
        assert_eq!(err.code, type_val::E1101);
        assert_eq!(err.table.as_deref(), Some("Item"));
        assert_eq!(err.row.as_deref(), Some("0"));
        assert_eq!(err.field.as_deref(), Some("weights"));
        let loc = err.location.as_ref().expect("E1101 carries a location");
        assert_eq!(loc.file, "sheet.json");
        assert_eq!(loc.row, Some(1));
        assert_eq!(loc.field.as_deref(), Some("weights"));
        assert!(err
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("map<int, Int32>") && h.contains("got: string")));
    }

    #[test]
    fn l2_map_int_keys_must_be_numeric_strings() {
        let vs = validated(map_schema());
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("weights", object_val(&[("foo", Value::Int(1))])),
                    ("drops", object_val(&[])),
                ],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Type, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1, "diags: {diags:?}");
        assert_eq!(errors[0].code, type_val::E1101);
        assert_eq!(errors[0].table.as_deref(), Some("Item"));
        assert_eq!(errors[0].row.as_deref(), Some("0"));
        assert_eq!(errors[0].field.as_deref(), Some("weights"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("map<int, Int32>")));
    }

    #[test]
    fn l2_map_accepts_valid_keys_and_well_typed_nested_values() {
        let vs = validated(map_schema());
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    (
                        "weights",
                        object_val(&[("1", Value::Int(10)), ("-2", Value::Int(3))]),
                    ),
                    (
                        "drops",
                        object_val(&[
                            ("sword", Value::Array(vec![Value::Int(1), Value::Int(2)])),
                            // A string-keyed map accepts any key, including
                            // spaces and empty arrays as values.
                            ("any key", Value::Array(vec![])),
                        ]),
                    ),
                ],
            )],
        );

        // Integration: through the public validate() entry at Type level …
        let diags = validate(&vs, &doc, ValidationLevel::Type, false);
        assert!(
            diags.is_empty(),
            "well-typed maps must not diagnose: {diags:?}"
        );
        // … and through the whole L0–L7 pipeline: no level chokes on maps.
        let full = validate(&vs, &doc, ValidationLevel::GameRule, false);
        assert!(full.is_empty(), "full pipeline must stay quiet: {full:?}");
    }

    #[test]
    fn l2_map_rejects_nested_value_mismatch_and_accepts_empty_maps() {
        let vs = validated(map_schema());
        // One bad element nested inside map<string, Array<Int32>> fails the row.
        let bad_doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("weights", object_val(&[])),
                    (
                        "drops",
                        object_val(&[("sword", Value::Array(vec![Value::Int(1), str_val("x")]))]),
                    ),
                ],
            )],
        );
        let diags = validate(&vs, &bad_doc, ValidationLevel::Type, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1, "diags: {diags:?}");
        assert_eq!(errors[0].code, type_val::E1101);
        assert_eq!(errors[0].field.as_deref(), Some("drops"));
        assert_eq!(errors[0].row.as_deref(), Some("0"));
        assert!(errors[0].hint.as_deref().is_some_and(|h| {
            // The hint names the field's expected type and the top-level
            // value type (the map itself is an object) — same machinery as
            // a single bad element inside a plain array.
            h.contains("map<string, Array(Int32)>") && h.contains("got: object")
        }));

        // Empty maps (no entries) are always valid for every key type.
        let empty_doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("weights", object_val(&[])),
                    ("drops", object_val(&[])),
                ],
            )],
        );
        let diags = validate(&vs, &empty_doc, ValidationLevel::Type, false);
        assert!(diags.is_empty(), "empty maps must pass: {diags:?}");
    }

    #[test]
    fn l2_object_schema_still_rejects_extra_keys() {
        // Maps are open-ended; schema Objects keep their closed-key semantics.
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("stats", object_type(&[("hp", FieldType::Int32)])),
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    (
                        "stats",
                        object_val(&[("hp", Value::Int(10)), ("mp", Value::Int(5))]),
                    ),
                ],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Type, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1, "diags: {diags:?}");
        assert_eq!(errors[0].code, type_val::E1101);
        assert_eq!(errors[0].field.as_deref(), Some("stats"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Object")));
    }

    #[test]
    fn value_matches_type_covers_every_family() {
        // Array: every element must match the declared inner type.
        assert!(value_matches_type(
            &Value::Array(vec![Value::Int(1), Value::Int(2)]),
            &array_type(FieldType::Int64)
        ));
        assert!(!value_matches_type(
            &Value::Array(vec![Value::Int(1), Value::Bool(true)]),
            &array_type(FieldType::Int64)
        ));
        // Object: known keys with matching value types pass; unknown keys and
        // mismatched value types fall through.
        let object = object_type(&[("hp", FieldType::Int32)]);
        assert!(value_matches_type(
            &object_val(&[("hp", Value::Int(10))]),
            &object
        ));
        assert!(!value_matches_type(
            &object_val(&[("atk", Value::Int(1))]),
            &object
        ));
        assert!(!value_matches_type(
            &object_val(&[("hp", str_val("10"))]),
            &object
        ));
        // Map: keys are gated by key_type, values by value_type; an empty
        // object is always a valid map, and any string key is legal for a
        // string-keyed map.
        let map = map_type(MapKeyType::String, FieldType::Int32);
        assert!(value_matches_type(&object_val(&[]), &map));
        assert!(value_matches_type(
            &object_val(&[("any key/空间", Value::Int(1))]),
            &map
        ));
        assert!(!value_matches_type(
            &object_val(&[("k", str_val("1"))]),
            &map
        ));
        assert!(!value_matches_type(&str_val("not an object"), &map));
        assert!(!value_matches_type(&Value::Int(1), &map));
        let int_map = map_type(MapKeyType::Int, FieldType::Int32);
        assert!(value_matches_type(
            &object_val(&[("-7", Value::Int(1))]),
            &int_map
        ));
        assert!(!value_matches_type(
            &object_val(&[("foo", Value::Int(1))]),
            &int_map
        ));
        // Nested value types are checked recursively.
        let nested = map_type(MapKeyType::String, array_type(FieldType::Int32));
        assert!(value_matches_type(
            &object_val(&[("a", Value::Array(vec![Value::Int(1)]))]),
            &nested
        ));
        assert!(!value_matches_type(
            &object_val(&[("a", Value::Array(vec![str_val("x")]))]),
            &nested
        ));
        // Enum membership itself is deferred to L3, so strings always pass.
        assert!(value_matches_type(
            &str_val("rare"),
            &FieldType::Enum("Rarity".to_string())
        ));
        // Scalar families only check the family; widths land in L3.
        assert!(value_matches_type(&Value::Null, &FieldType::Null));
        assert!(value_matches_type(&Value::Bool(true), &FieldType::Bool));
        assert!(value_matches_type(&Value::Int(1), &FieldType::Int8));
        assert!(value_matches_type(&Value::UInt(1), &FieldType::UInt64));
        assert!(value_matches_type(&Value::Float(1.5), &FieldType::Float32));
        assert!(value_matches_type(&str_val("s"), &FieldType::String));
        assert!(value_matches_type(
            &Value::Bytes(vec![1]),
            &FieldType::Bytes
        ));
        assert!(value_matches_type(&str_val("any"), &FieldType::Any));
        // Family mismatches fall through to false.
        assert!(!value_matches_type(&str_val("10"), &FieldType::Int32));
        assert!(!value_matches_type(&Value::Int(10), &FieldType::String));
        assert!(!value_matches_type(
            &Value::Array(vec![]),
            &FieldType::String
        ));
    }

    #[test]
    fn pipeline_skips_unknown_tables_and_fields_across_levels() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        let vs = validated(schema);
        let mut doc = doc_with(
            "Item",
            vec![row(0, &[("id", Value::UInt(1)), ("bonus", Value::UInt(9))])],
        );
        doc.add_table(Table {
            name: "Orphan".to_string(),
            primary_key_fields: vec![],
            rows: vec![row(0, &[("id", Value::UInt(1))])],
            source_file: "orphan.json".to_string(),
            sheet: None,
        });

        // Runs every level: only L1's unknown-field warning survives; each
        // later level skips both the orphan table and the unknown field.
        let diags = validate(&vs, &doc, ValidationLevel::GameRule, false);
        assert!(diags.errors().is_empty());
        let warnings = diags.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, schema::E1002);
        assert_eq!(warnings[0].field.as_deref(), Some("bonus"));
    }

    #[test]
    fn l3_enforces_numeric_bounds() {
        let mut schema = Schema::new();
        let low = FieldSchema {
            min: Some(10.0),
            ..plain_field("low", FieldType::UInt32)
        };
        let high = FieldSchema {
            max: Some(10.0),
            ..plain_field("high", FieldType::UInt32)
        };
        let ratio = FieldSchema {
            min: Some(0.0),
            max: Some(1.0),
            ..plain_field("ratio", FieldType::Any)
        };
        schema.add_table(plain_table(
            "Stat",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), low, high, ratio],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Stat",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("low", Value::UInt(5)),   // below min 10
                    ("high", Value::UInt(50)), // above max 10
                    ("ratio", Value::Null),    // not coercible → range checks skip
                ],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Value, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|e| e.code == value::E1201));
        assert!(errors.iter().any(|e| {
            e.field.as_deref() == Some("low")
                && e.hint
                    .as_deref()
                    .is_some_and(|h| h.contains("Minimum allowed: 10"))
        }));
        assert!(errors.iter().any(|e| {
            e.field.as_deref() == Some("high")
                && e.hint
                    .as_deref()
                    .is_some_and(|h| h.contains("Maximum allowed: 10"))
        }));
    }

    #[test]
    fn l3_enforces_string_length_and_pattern() {
        let mut schema = Schema::new();
        let code = FieldSchema {
            min_length: Some(3),
            max_length: Some(5),
            pattern: Some("^[a-z]+$".to_string()),
            ..plain_field("code", FieldType::String)
        };
        let broken = FieldSchema {
            pattern: Some("[".to_string()), // invalid regex → constraint skipped
            ..plain_field("broken", FieldType::String)
        };
        schema.add_table(plain_table(
            "Code",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), code, broken],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Code",
            vec![
                row(
                    0,
                    &[
                        ("id", Value::UInt(1)),
                        ("code", str_val("ab")),
                        ("broken", str_val("x")),
                    ],
                ),
                row(
                    1,
                    &[
                        ("id", Value::UInt(2)),
                        ("code", str_val("abcdef")),
                        ("broken", str_val("y")),
                    ],
                ),
                row(
                    2,
                    &[
                        ("id", Value::UInt(3)),
                        ("code", str_val("ABC1")),
                        ("broken", str_val("z")),
                    ],
                ),
                row(
                    3,
                    &[
                        ("id", Value::UInt(4)),
                        ("code", str_val("abc")),
                        ("broken", str_val("w")),
                    ],
                ),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Value, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(|e| {
            e.code == value::E1202
                && e.field.as_deref() == Some("code")
                && e.message.contains("too short")
        }));
        assert!(errors.iter().any(|e| {
            e.code == value::E1202
                && e.field.as_deref() == Some("code")
                && e.message.contains("too long")
        }));
        assert!(errors.iter().any(|e| {
            e.code == value::E1203
                && e.hint
                    .as_deref()
                    .is_some_and(|h| h.contains("Pattern: ^[a-z]+$"))
        }));
    }

    #[test]
    fn l3_enforces_enum_membership() {
        let mut schema = Schema::new();
        let rarity = FieldSchema {
            enum_values: Some(vec!["common".to_string(), "rare".to_string()]),
            ..plain_field("rarity", FieldType::Any)
        };
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), rarity],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Drop",
            vec![
                row(0, &[("id", Value::UInt(1)), ("rarity", str_val("epic"))]),
                // Not string-coercible → coerces to "" → also outside the enum.
                row(
                    1,
                    &[("id", Value::UInt(2)), ("rarity", Value::Array(vec![]))],
                ),
                row(2, &[("id", Value::UInt(3)), ("rarity", str_val("rare"))]),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Value, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|e| e.code == value::E1204));
        assert!(errors.iter().any(|e| e.row.as_deref() == Some("0")));
        assert!(errors.iter().any(|e| {
            e.row.as_deref() == Some("1")
                && e.hint
                    .as_deref()
                    .is_some_and(|h| h.contains("Allowed values: common, rare"))
        }));
    }

    #[test]
    fn l3_enforces_named_enum_membership() {
        use crate::schema::{EnumSchema, EnumValue};

        let named = |name: &str| EnumValue {
            name: name.to_string(),
            value: None,
            description: None,
        };
        let mut schema = Schema::new();
        schema.enums.insert(
            "Rarity".to_string(),
            EnumSchema {
                name: "Rarity".to_string(),
                values: vec![named("common"), named("rare")],
                description: None,
            },
        );
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("rarity", FieldType::Enum("Rarity".to_string())),
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Drop",
            vec![
                row(0, &[("id", Value::UInt(1)), ("rarity", str_val("epic"))]),
                row(1, &[("id", Value::UInt(2)), ("rarity", str_val("rare"))]),
                // A non-string value is an L2 type mismatch (E1101), not a
                // membership failure — no double diagnosis.
                row(
                    2,
                    &[("id", Value::UInt(3)), ("rarity", Value::Array(vec![]))],
                ),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Value, false);
        let errors = diags.errors();
        // L2 (type) runs before L3 (constraints), so the E1101 lands first.
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].code, type_val::E1101);
        assert_eq!(errors[1].code, value::E1204);
        assert_eq!(errors[1].row.as_deref(), Some("0"));
        assert!(errors[1]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Allowed values: common, rare")));
        assert_eq!(errors[0].row.as_deref(), Some("2"));
    }

    #[test]
    fn l3_enforces_array_length_bounds() {
        let mut schema = Schema::new();
        let tags = FieldSchema {
            min_items: Some(1),
            max_items: Some(2),
            ..plain_field("tags", array_type(FieldType::String))
        };
        schema.add_table(plain_table(
            "Unit",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), tags],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Unit",
            vec![
                row(0, &[("id", Value::UInt(1)), ("tags", Value::Array(vec![]))]),
                row(
                    1,
                    &[
                        ("id", Value::UInt(2)),
                        (
                            "tags",
                            Value::Array(vec![str_val("a"), str_val("b"), str_val("c")]),
                        ),
                    ],
                ),
                row(
                    2,
                    &[
                        ("id", Value::UInt(3)),
                        ("tags", Value::Array(vec![str_val("a")])),
                    ],
                ),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Value, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|e| e.code == value::E1205));
        assert!(errors.iter().any(|e| {
            e.hint
                .as_deref()
                .is_some_and(|h| h.contains("Minimum items: 1"))
        }));
        assert!(errors.iter().any(|e| {
            e.hint
                .as_deref()
                .is_some_and(|h| h.contains("Maximum items: 2"))
        }));
    }

    #[test]
    fn l4_detects_row_ordering_violations() {
        let mut schema = Schema::new();
        let mut tier = plain_table(
            "Tier",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("level", FieldType::Int32),
            ],
        );
        tier.order_by = Some(vec!["level".to_string()]);
        schema.add_table(tier);
        let vs = validated(schema);
        let doc = doc_with(
            "Tier",
            vec![
                row(0, &[("id", Value::UInt(1)), ("level", Value::Int(2))]),
                row(1, &[("id", Value::UInt(2)), ("level", Value::Int(1))]), // out of order
                row(2, &[("id", Value::UInt(3)), ("level", Value::Int(3))]),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Table, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, table::E1304);
        assert_eq!(errors[0].row.as_deref(), Some("1"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Rows must be ordered by: level")));
    }

    #[test]
    fn l4_skips_pk_check_when_table_has_no_primary_key() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Tag",
            &[],
            vec![plain_field("label", FieldType::String)],
        ));
        let vs = validated(schema);
        // Two identical labels would collide if a PK were enforced.
        let doc = doc_with(
            "Tag",
            vec![
                row(0, &[("label", str_val("a"))]),
                row(1, &[("label", str_val("a"))]),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Table, false);
        assert!(diags.is_empty());
    }

    #[test]
    fn l5_reports_missing_reference_target_e1401() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        let item_id = FieldSchema {
            reference: Some(ReferenceSchema {
                table: "Item".to_string(),
                field: "id".to_string(),
                predicate: None,
                cardinality: "one".to_string(),
                compatible_with: None,
            }),
            ..plain_field("item_id", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), item_id],
        ));
        let vs = validated(schema);
        let doc = doc_with_tables(&[
            (
                "Item",
                vec![
                    row(0, &[("id", Value::UInt(1))]),
                    row(1, &[("id", Value::UInt(2))]),
                ],
            ),
            (
                "Drop",
                vec![
                    row(0, &[("id", Value::UInt(1)), ("item_id", Value::UInt(99))]),
                    row(1, &[("id", Value::UInt(2)), ("item_id", Value::UInt(1))]),
                ],
            ),
        ]);

        let diags = validate(&vs, &doc, ValidationLevel::Reference, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, reference::E1401);
        assert_eq!(errors[0].field.as_deref(), Some("item_id"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("No Item with id=99 found")));
    }

    #[test]
    fn l5_predicate_gate_accepts_when_target_row_satisfies_it() {
        let mut schema = Schema::new();
        let tradable = plain_field("tradable", FieldType::Bool);
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), tradable],
        ));
        let item_id = FieldSchema {
            reference: Some(ReferenceSchema {
                table: "Item".to_string(),
                field: "id".to_string(),
                predicate: Some(ExpressionRule {
                    name: "tradable".to_string(),
                    assert: "tradable == true".to_string(),
                    message: None,
                    warning_only: false,
                }),
                cardinality: "one".to_string(),
                compatible_with: None,
            }),
            ..plain_field("item_id", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), item_id],
        ));
        let vs = validated(schema);
        let doc = doc_with_tables(&[
            (
                "Item",
                vec![
                    row(
                        0,
                        &[("id", Value::UInt(1)), ("tradable", Value::Bool(true))],
                    ),
                    row(
                        1,
                        &[("id", Value::UInt(2)), ("tradable", Value::Bool(false))],
                    ),
                ],
            ),
            (
                "Drop",
                vec![
                    row(0, &[("id", Value::UInt(1)), ("item_id", Value::UInt(1))]),
                    row(1, &[("id", Value::UInt(2)), ("item_id", Value::UInt(2))]),
                ],
            ),
        ]);

        // Row 0 targets a tradable item (holds); row 1 targets a
        // non-tradable item (E1410). An absent optional field would say
        // nothing and pass, but here the target field is declared Bool and
        // present in both rows.
        let diags = validate(&vs, &doc, ValidationLevel::Reference, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, reference::E1410);
        assert_eq!(errors[0].table.as_deref(), Some("Drop"));
        assert_eq!(errors[0].row.as_deref(), Some("1"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("not satisfied")));
    }

    /// `UInt32` field with a cross-table reference of the given cardinality.
    fn ref_field_card(name: &str, table: &str, field: &str, cardinality: &str) -> FieldSchema {
        let mut f = ref_field(name, table, field);
        if let Some(r) = f.reference.as_mut() {
            r.cardinality = cardinality.to_string();
        }
        f
    }

    #[test]
    fn l5_many_cardinality_checks_each_array_element() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        let mut item_ids = ref_field_card("item_ids", "Item", "id", "many");
        item_ids.field_type = FieldType::Array(Box::new(FieldType::UInt32));
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), item_ids],
        ));
        let vs = validated(schema);
        let doc = doc_with_tables(&[
            (
                "Item",
                vec![row(0, &[("id", Value::UInt(1))]), row(1, &[("id", Value::UInt(2))])],
            ),
            (
                "Drop",
                vec![
                    row(
                        0,
                        &[
                            ("id", Value::UInt(1)),
                            ("item_ids", Value::Array(vec![Value::UInt(1), Value::UInt(99)])),
                        ],
                    ),
                    // Empty list is vacuously fine; every element resolving
                    // reports nothing.
                    row(
                        1,
                        &[("id", Value::UInt(2)), ("item_ids", Value::Array(vec![]))],
                    ),
                    row(
                        2,
                        &[("id", Value::UInt(3)), ("item_ids", Value::Array(vec![Value::UInt(2)]))],
                    ),
                ],
            ),
        ]);

        let diags = validate(&vs, &doc, ValidationLevel::Reference, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, reference::E1401);
        assert_eq!(errors[0].row.as_deref(), Some("0"));
        assert_eq!(errors[0].field.as_deref(), Some("item_ids"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("No Item with id=99 found")));
    }

    #[test]
    fn l5_cardinality_shape_mismatches_report_e1404() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                // Fields here are `Any`-typed so L2 lets every shape
                // through and the test isolates the L5 contract.
                {
                    let mut f = ref_field_card("tag_ids", "Item", "id", "many");
                    f.field_type = FieldType::Any;
                    f
                },
                // Array value under the default `one` → the reverse shape.
                {
                    let mut f = ref_field("item_id", "Item", "id");
                    f.field_type = FieldType::Any;
                    f
                },
                // Nullable single reference under `one` → violated; the
                // `optional` spelling is the way to express nullability.
                {
                    let mut f = ref_field("maybe_item", "Item", "id");
                    f.field_type = FieldType::Any;
                    f
                },
                {
                    let mut f = ref_field_card("spare_item", "Item", "id", "optional");
                    f.field_type = FieldType::Any;
                    f
                },
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with_tables(&[
            ("Item", vec![row(0, &[("id", Value::UInt(1))])]),
            (
                "Drop",
                vec![
                    row(
                        0,
                        &[
                            ("id", Value::UInt(1)),
                            ("tag_ids", Value::UInt(1)),
                            (
                                "item_id",
                                Value::Array(vec![Value::UInt(1), Value::UInt(1)]),
                            ),
                            ("maybe_item", Value::Null),
                            ("spare_item", Value::Null),
                        ],
                    ),
                ],
            ),
        ]);

        let diags = validate(&vs, &doc, ValidationLevel::Reference, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 3, "optional null passes, the rest violate");
        assert!(errors.iter().all(|d| d.code == reference::E1404));
        let many_scalar = errors.iter().find(|d| d.field.as_deref() == Some("tag_ids"));
        assert!(many_scalar.is_some_and(|d| d
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("array of Item references")
                && h.contains("a single uint"))));
        let one_array = errors.iter().find(|d| d.field.as_deref() == Some("item_id"));
        assert!(one_array.is_some_and(|d| d
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("expects a single Item reference")
                && h.contains("use cardinality: many"))));
        let one_null = errors.iter().find(|d| d.field.as_deref() == Some("maybe_item"));
        assert!(one_null.is_some_and(|d| d
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("the value is null")
                && h.contains("use cardinality: optional"))));
    }

    #[test]
    fn l5_unknown_cardinality_spelling_is_a_schema_defect() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        schema.add_table(plain_table(
            "Drop",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field_card("item_id", "Item", "id", "banana"),
            ],
        ));
        // Empty tables: the defect is a schema problem, not a row problem.
        let doc = doc_with_tables(&[("Item", vec![]), ("Drop", vec![])]);

        let diags = validate(&validated(schema), &doc, ValidationLevel::Reference, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, schema::E1004);
        assert_eq!(errors[0].field.as_deref(), Some("item_id"));
        assert!(errors[0]
            .message
            .contains("unknown cardinality 'banana'"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("source-side")));
    }

    #[test]
    fn l5_compatible_with_rejects_mismatched_enum_on_target() {
        let mut schema = Schema::new();
        let kind = FieldSchema {
            enum_values: Some(vec!["weapon".to_string(), "armor".to_string()]),
            ..plain_field("kind", FieldType::String)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), kind],
        ));
        let item_id = FieldSchema {
            reference: Some(ReferenceSchema {
                table: "Item".to_string(),
                field: "id".to_string(),
                predicate: None,
                cardinality: "one".to_string(),
                compatible_with: Some(vec!["weapon".to_string()]),
            }),
            ..plain_field("item_id", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Slot",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), item_id],
        ));
        let vs = validated(schema);
        let doc = doc_with_tables(&[
            (
                "Item",
                vec![
                    // "legacy" is absent from the Item schema: the compatibility
                    // scan must skip it while walking the target row.
                    row(
                        0,
                        &[
                            ("id", Value::UInt(1)),
                            ("kind", str_val("armor")),
                            ("legacy", str_val("old")),
                        ],
                    ),
                    row(1, &[("id", Value::UInt(2)), ("kind", str_val("weapon"))]),
                ],
            ),
            (
                "Slot",
                vec![
                    row(0, &[("id", Value::UInt(1)), ("item_id", Value::UInt(1))]),
                    row(1, &[("id", Value::UInt(2)), ("item_id", Value::UInt(2))]),
                ],
            ),
        ]);

        let diags = validate(&vs, &doc, ValidationLevel::Reference, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, reference::E1411);
        assert_eq!(errors[0].row.as_deref(), Some("0"));
        assert!(errors[0].hint.as_deref().is_some_and(|h| {
            h.contains("Item.kind = 'armor'") && h.contains("allowed: weapon")
        }));
        // The stray "legacy" field is only reported by L1's unknown-field check.
        let warnings = diags.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, schema::E1002);
        assert_eq!(warnings[0].field.as_deref(), Some("legacy"));
    }

    #[test]
    fn l4_detects_duplicate_composite_unique_constraint() {
        let mut schema = Schema::new();
        let mut item = plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("slot", FieldType::String),
            ],
        );
        item.unique_constraints = vec![crate::schema::UniqueConstraint {
            name: "slot_unique".to_string(),
            fields: vec!["slot".to_string()],
        }];
        schema.add_table(item);
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![
                row(0, &[("id", Value::UInt(1)), ("slot", str_val("a"))]),
                row(1, &[("id", Value::UInt(2)), ("slot", str_val("a"))]), // duplicate
                row(2, &[("id", Value::UInt(3)), ("slot", str_val("b"))]),
            ],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Table, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, table::E1302);
        assert_eq!(errors[0].row.as_deref(), Some("1"));
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Fields: slot, first at row 0")));
    }

    #[test]
    fn l7_gamerule_level_surfaces_builtin_e1601_end_to_end() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Monster",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                plain_field("level", FieldType::UInt32),
                plain_field("attack", FieldType::UInt32),
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Monster",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("level", Value::UInt(1)),
                    ("attack", Value::UInt(500)), // level 1 → cap 150
                ],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::GameRule, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::error::codes::gamerule::E1601);
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("power_curve: attack 500")));
    }

    #[test]
    fn l6_rejects_violating_rule_with_e1501() {
        let mut schema = Schema::new();
        let price = FieldSchema {
            rules: vec![ExpressionRule {
                name: "price_bounds".to_string(),
                assert: "price <= 10000".to_string(),
                message: Some("price must stay under 10000".to_string()),
                warning_only: false,
            }],
            ..plain_field("price", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), price],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[("id", Value::UInt(1)), ("price", Value::UInt(20000))],
            )],
        );

        let diags = validate(&vs, &doc, ValidationLevel::Semantic, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, semantic::E1501);
        assert_eq!(errors[0].table.as_deref(), Some("Item"));
        assert_eq!(errors[0].row.as_deref(), Some("0"));
        assert_eq!(
            errors[0].hint.as_deref(),
            Some("price must stay under 10000")
        );
    }

    #[test]
    fn l6_satisfying_rule_and_absent_optional_field_pass() {
        let mut schema = Schema::new();
        let price = FieldSchema {
            rules: vec![ExpressionRule {
                name: "price_bounds".to_string(),
                assert: "price <= bonus".to_string(),
                message: None,
                warning_only: false,
            }],
            ..plain_field("price", FieldType::UInt32)
        };
        let bonus = FieldSchema {
            rules: vec![ExpressionRule {
                name: "bonus_pos".to_string(),
                assert: "bonus >= 0".to_string(),
                message: None,
                warning_only: true,
            }],
            ..plain_field("bonus", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), price, bonus],
        ));
        let vs = validated(schema);
        // price=5 <= bonus=9 holds; the bonus_pos rule references the
        // declared field, which is optional and absent here → passes.
        let doc = doc_with(
            "Item",
            vec![row(0, &[("id", Value::UInt(1)), ("price", Value::UInt(5))])],
        );
        let diags = validate(&vs, &doc, ValidationLevel::Semantic, false);
        assert!(diags.is_empty());
    }

    #[test]
    fn l6_warning_only_rule_reports_a_warning_not_an_error() {
        let mut schema = Schema::new();
        let price = FieldSchema {
            rules: vec![ExpressionRule {
                name: "price_bounds".to_string(),
                assert: "price <= 10".to_string(),
                message: None,
                warning_only: true,
            }],
            ..plain_field("price", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), price],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[("id", Value::UInt(1)), ("price", Value::UInt(50))],
            )],
        );
        let diags = validate(&vs, &doc, ValidationLevel::Semantic, false);
        assert!(diags.errors().is_empty());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.warnings().len(), 1);
        assert_eq!(diags.warnings()[0].code, semantic::E1501);
        assert_eq!(diags.warnings()[0].severity, Severity::Warning);
        assert!(diags.warnings()[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Assertion 'price <= 10' failed")));
    }

    #[test]
    fn l6_incomparable_operands_report_e1501_with_types() {
        let mut schema = Schema::new();
        let kind = FieldSchema {
            rules: vec![ExpressionRule {
                name: "kind_num".to_string(),
                assert: "kind <= price".to_string(),
                message: None,
                warning_only: false,
            }],
            ..plain_field("kind", FieldType::String)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                kind,
                plain_field("price", FieldType::UInt32),
            ],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("kind", Value::String("weapon".into())),
                    ("price", Value::UInt(10)),
                ],
            )],
        );
        let diags = validate(&vs, &doc, ValidationLevel::Semantic, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, semantic::E1501);
        assert!(errors[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("not evaluable") && h.contains("string and uint")));
    }

    #[test]
    fn l6_malformed_assert_and_unknown_ref_are_schema_defects() {
        let mut schema = Schema::new();
        let broken = FieldSchema {
            rules: vec![ExpressionRule {
                name: "broken".to_string(),
                assert: "price <=".to_string(),
                message: None,
                warning_only: false,
            }],
            ..plain_field("price", FieldType::UInt32)
        };
        let dangling = FieldSchema {
            rules: vec![ExpressionRule {
                name: "dangling".to_string(),
                assert: "ghost >= 1".to_string(),
                message: None,
                warning_only: false,
            }],
            ..plain_field("desc", FieldType::String)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), broken, dangling],
        ));
        let vs = validated(schema);
        let doc = doc_with(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("price", Value::UInt(50)),
                    ("desc", Value::String("x".into())),
                ],
            )],
        );

        // Defects are reported once per rule as E1004 regardless of row
        // count, and the violating rows emit nothing (the rules never run).
        let diags = validate(&vs, &doc, ValidationLevel::Semantic, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|d| d.code == schema::E1004));
        assert!(errors
            .iter()
            .any(|d| d.message.contains("malformed assert")));
        assert!(errors
            .iter()
            .any(|d| d.message.contains("undeclared field 'ghost'")));
    }

    #[test]
    fn l6_schema_defects_report_even_when_the_table_has_no_rows() {
        let mut schema = Schema::new();
        let broken = FieldSchema {
            rules: vec![ExpressionRule {
                name: "broken".to_string(),
                assert: "price ==".to_string(),
                message: None,
                warning_only: false,
            }],
            ..plain_field("price", FieldType::UInt32)
        };
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![plain_field("id", FieldType::UInt32), broken],
        ));
        let vs = validated(schema);
        let doc = doc_with("Item", vec![]);

        let diags = validate(&vs, &doc, ValidationLevel::Semantic, false);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, schema::E1004);
    }

    #[test]
    fn test_value_validation_catches_range() {
        let schema = ValidatedSchema {
            schema: make_test_schema(),
            dependency_graph: crate::reference::DependencyGraph::new(),
        };
        let doc = make_test_doc();
        let diags = validate(&schema, &doc, ValidationLevel::Value, false);

        assert!(diags.has_errors());
        let errors = diags.errors();
        assert!(errors.iter().any(|e| e.code == value::E1201));
    }

    #[test]
    fn test_duplicate_pk_caught() {
        let schema = make_test_schema();
        let mut doc = make_test_doc();

        // Add duplicate PK
        if let Some(table) = doc.tables.get_mut("Item") {
            table.rows.push(Row {
                primary_key: vec![Value::UInt(1)], // duplicate
                fields: {
                    let mut fields = IndexMap::new();
                    fields.insert(
                        "id".to_string(),
                        TypedValue::new(
                            Value::UInt(1),
                            SourceLocation::new("items.json")
                                .with_row(3)
                                .with_field("id"),
                        ),
                    );
                    fields.insert(
                        "price".to_string(),
                        TypedValue::new(
                            Value::UInt(50),
                            SourceLocation::new("items.json")
                                .with_row(3)
                                .with_field("price"),
                        ),
                    );
                    fields
                },
                location: SourceLocation::new("items.json").with_row(3),
                index: 2,
            });
        }

        let validated_schema = ValidatedSchema {
            schema,
            dependency_graph: crate::reference::DependencyGraph::new(),
        };
        let diags = validate(&validated_schema, &doc, ValidationLevel::Table, false);

        assert!(diags.has_errors());
        let errors = diags.errors();
        assert!(errors.iter().any(|e| e.code == table::E1301));
    }

    /// `UInt32` field with a cross-table reference (`table.field`).
    fn ref_field(name: &str, table: &str, field: &str) -> FieldSchema {
        let mut f = plain_field(name, FieldType::UInt32);
        f.reference = Some(ReferenceSchema {
            table: table.to_string(),
            field: field.to_string(),
            predicate: None,
            cardinality: "one".to_string(),
            compatible_with: None,
        });
        f
    }

    #[test]
    fn e1403_two_table_cycle_warns_canonically() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("part_of", "Kit", "id"),
            ],
        ));
        schema.add_table(plain_table(
            "Kit",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("best_item", "Item", "id"),
            ],
        ));
        let doc = doc_with_tables(&[("Item", vec![]), ("Kit", vec![])]);

        let diags = validate(&validated(schema), &doc, ValidationLevel::Reference, false);
        assert!(!diags.has_errors(), "cycles warn, they do not error");
        let warnings: Vec<_> = diags
            .warnings()
            .into_iter()
            .filter(|d| d.code == reference::E1403)
            .collect();
        assert_eq!(warnings.len(), 1, "one canonical cycle, not per-start");
        assert_eq!(warnings[0].message, "Circular reference: Item → Kit → Item");
        assert_eq!(warnings[0].table.as_deref(), Some("Item"));
        assert_eq!(warnings[0].source, "schema");
    }

    #[test]
    fn e1403_three_table_cycle_is_one_warning() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "A",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("b", "B", "id"),
            ],
        ));
        schema.add_table(plain_table(
            "B",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("c", "C", "id"),
            ],
        ));
        schema.add_table(plain_table(
            "C",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("a", "A", "id"),
            ],
        ));
        let doc = doc_with_tables(&[("A", vec![]), ("B", vec![]), ("C", vec![])]);

        let diags = validate(&validated(schema), &doc, ValidationLevel::Reference, false);
        let cycles: Vec<_> = diags
            .warnings()
            .into_iter()
            .filter(|d| d.code == reference::E1403)
            .collect();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].message, "Circular reference: A → B → C → A");
    }

    #[test]
    fn e1403_self_loop_warns_and_acyclic_is_silent() {
        // Self-reference (unlock chains etc.): the graph contract counts it
        // as a cycle, so it warns too.
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Stage",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("next", "Stage", "id"),
            ],
        ));
        let doc = doc_with_tables(&[("Stage", vec![])]);
        let diags = validate(&validated(schema), &doc, ValidationLevel::Reference, false);
        let cycles: Vec<_> = diags
            .warnings()
            .into_iter()
            .filter(|d| d.code == reference::E1403)
            .collect();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].message, "Circular reference: Stage → Stage");

        // Diamond A→B→D, A→C→D is acyclic: silent.
        let mut diamond = Schema::new();
        diamond.add_table(plain_table(
            "A",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("b", "B", "id"),
                ref_field("c", "C", "id"),
            ],
        ));
        diamond.add_table(plain_table(
            "B",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("d", "D", "id"),
            ],
        ));
        diamond.add_table(plain_table(
            "C",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("d", "D", "id"),
            ],
        ));
        diamond.add_table(plain_table(
            "D",
            &["id"],
            vec![plain_field("id", FieldType::UInt32)],
        ));
        let doc = doc_with_tables(&[("A", vec![]), ("B", vec![]), ("C", vec![]), ("D", vec![])]);
        let diags = validate(&validated(diamond), &doc, ValidationLevel::Reference, false);
        assert!(diags.is_empty());
    }

    #[test]
    fn e1403_escalates_with_warnings_as_errors() {
        let mut schema = Schema::new();
        schema.add_table(plain_table(
            "Item",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("part_of", "Kit", "id"),
            ],
        ));
        schema.add_table(plain_table(
            "Kit",
            &["id"],
            vec![
                plain_field("id", FieldType::UInt32),
                ref_field("best_item", "Item", "id"),
            ],
        ));
        let doc = doc_with_tables(&[("Item", vec![]), ("Kit", vec![])]);

        let diags = validate(&validated(schema), &doc, ValidationLevel::Reference, true);
        assert!(diags.has_errors());
        assert!(
            diags.errors().iter().any(|e| e.code == reference::E1403),
            "warnings_as_errors escalates the cycle warning"
        );
    }

    /// Table helper with profile targets on the table and/or fields.
    fn profiled_table(
        name: &str,
        table_targets: &[&str],
        fields: Vec<(FieldSchema, Vec<&str>)>,
    ) -> (String, TableSchema) {
        let mut table = plain_table(name, &[], vec![]);
        table.targets = table_targets.iter().map(|t| (*t).to_string()).collect();
        for (mut field, targets) in fields {
            field.targets = targets.iter().map(|t| (*t).to_string()).collect();
            table.fields.insert(field.name.clone(), field);
        }
        (name.to_string(), table)
    }

    #[test]
    fn e9006_required_field_hidden_by_profile_is_conflict() {
        let mut schema = Schema::new();
        let mut id = plain_field("id", FieldType::Int32);
        id.required = true;
        let mut secret = plain_field("secret", FieldType::String);
        secret.required = true;
        let (_name, table) =
            profiled_table("Account", &[], vec![(id, vec![]), (secret, vec!["server"])]);
        schema.add_table(table);

        let diags = check_profile_visibility(&schema, "client");
        let errors = diags.errors();
        assert_eq!(errors.len(), 1, "only the hidden required field conflicts");
        assert_eq!(errors[0].code, build::E9006);
        assert_eq!(errors[0].table.as_deref(), Some("Account"));
        assert_eq!(errors[0].field.as_deref(), Some("secret"));
        assert!(
            errors[0].hint.as_deref().unwrap_or("").contains("client"),
            "hint names the active profile"
        );

        // Serving profile sees nothing to complain about.
        let diags_server = check_profile_visibility(&schema, "server");
        assert!(!diags_server.has_errors());
    }

    #[test]
    fn e9006_required_field_with_default_may_be_hidden() {
        let mut schema = Schema::new();
        let mut id = plain_field("id", FieldType::Int32);
        id.required = true;
        let mut secret = plain_field("secret", FieldType::String);
        secret.required = true;
        secret.default = Some(serde_json::json!("redacted"));
        let (_name, table) =
            profiled_table("Account", &[], vec![(id, vec![]), (secret, vec!["server"])]);
        schema.add_table(table);

        let diags = check_profile_visibility(&schema, "client");
        assert_eq!(
            diags.errors().len(),
            0,
            "default backfills the projected view"
        );
    }

    #[test]
    fn e9006_primary_key_and_unique_members_hidden_are_conflicts() {
        let mut schema = Schema::new();
        let mut id = plain_field("id", FieldType::Int32);
        id.required = true;
        let mut secret = plain_field("secret", FieldType::String);
        secret.required = true;
        let (_name, table) =
            profiled_table("Account", &[], vec![(id, vec![]), (secret, vec!["server"])]);
        schema.add_table(table);

        // PK + unique constraint on the hidden field → two more conflicts.
        let mut vs = validated(schema);
        let table_mut = vs.schema.tables.get_mut("Account").unwrap();
        table_mut.primary_key = vec!["secret".to_string()];
        table_mut.unique_constraints = vec![crate::schema::UniqueConstraint {
            name: "uq_secret".to_string(),
            fields: vec!["secret".to_string()],
        }];

        let diags = check_profile_visibility(&vs.schema, "client");
        let errors = diags.errors();
        assert_eq!(errors.len(), 3, "pk + unique + required, one code each");
        assert!(errors.iter().all(|e| e.code == build::E9006));
        assert_eq!(
            errors
                .iter()
                .filter(|e| e.field.as_deref() == Some("secret"))
                .count(),
            3
        );
    }

    #[test]
    fn e9006_optional_field_may_be_hidden_silently() {
        let mut schema = Schema::new();
        let mut id = plain_field("id", FieldType::Int32);
        id.required = true;
        let debug = plain_field("debug_log", FieldType::Bool);
        let (_name, table) =
            profiled_table("Account", &[], vec![(id, vec![]), (debug, vec!["server"])]);
        schema.add_table(table);

        let diags = check_profile_visibility(&schema, "client");
        assert!(
            !diags.has_errors(),
            "optional fields are a legitimate view, not a conflict"
        );
    }

    #[test]
    fn e9006_reference_target_hidden_by_profile_is_conflict() {
        let mut schema = Schema::new();
        let mut item_id = plain_field("id", FieldType::Int32);
        item_id.required = true;
        let (_item_name, mut item_table) = profiled_table(
            "Item",
            &[],
            vec![
                (item_id.clone(), vec![]),
                (
                    plain_field("internal_note", FieldType::String),
                    vec!["server"],
                ),
            ],
        );
        item_table.primary_key = vec!["id".to_string()];
        schema.add_table(item_table);

        let mut drop_id = plain_field("id", FieldType::Int32);
        drop_id.required = true;
        let mut link = plain_field("item", FieldType::Int32);
        link.required = true;
        link.reference = Some(ReferenceSchema {
            table: "Item".to_string(),
            field: "internal_note".to_string(),
            predicate: None,
            cardinality: "one".to_string(),
            compatible_with: None,
        });
        let (_drop_name, drop_table) =
            profiled_table("Drop", &[], vec![(drop_id, vec![]), (link, vec![])]);

        schema.add_table(drop_table);

        // The referencing side is visible in client, the target field is not.
        let diags = check_profile_visibility(&schema, "client");
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, build::E9006);
        assert_eq!(errors[0].field.as_deref(), Some("item"));

        // Hiding the whole target TABLE also conflicts.
        let mut schema2 = Schema::new();
        let mut table2 = plain_table("Item", &["id"], vec![item_id.clone()]);
        table2.targets = vec!["server".to_string()];
        schema2.add_table(table2);
        let mut link2 = plain_field("item", FieldType::Int32);
        link2.reference = Some(ReferenceSchema {
            table: "Item".to_string(),
            field: "id".to_string(),
            predicate: None,
            cardinality: "one".to_string(),
            compatible_with: None,
        });
        let (_, drop_table2) = profiled_table("Drop", &[], vec![(link2, vec![])]);
        schema2.add_table(drop_table2);

        let diags2 = check_profile_visibility(&schema2, "client");
        let errs2 = diags2.errors();
        assert_eq!(errs2.len(), 1, "hidden target table conflicts");
        assert_eq!(errs2[0].code, build::E9006);
    }

    #[test]
    fn e9006_requires_an_active_profile() {
        let mut schema = Schema::new();
        let mut secret = plain_field("secret", FieldType::String);
        secret.required = true;
        let (_name, table) = profiled_table("Account", &[], vec![(secret, vec!["server"])]);
        schema.add_table(table);

        // Plain validate() has no profile → no E9006 semantics.
        let vs = validated(schema);
        let diags = validate(&vs, &Document::new(), ValidationLevel::Schema, false);
        assert!(
            !diags.has_errors(),
            "no profile means everything is visible"
        );
    }
}
