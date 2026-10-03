//! Validation Pipeline - L0 through L7 validation levels
//! Each level can be run independently via `--level` flag

pub mod rules;

use crate::diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics, Severity};
use crate::error::codes::{parse, reference, schema, semantic, table, type_val, value};
use crate::schema::{
    ExpressionRule, FieldSchema, FieldType, ReferenceSchema, Schema, TableSchema, ValidatedSchema,
};
use crate::value::{Document, Row, TypedValue, Value};
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

/// Main validation entry point
pub fn validate(
    schema: &ValidatedSchema,
    document: &Document,
    max_level: ValidationLevel,
    warnings_as_errors: bool,
) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    let mut ctx = ValidationContext {
        schema,
        document,
        diagnostics: &mut diagnostics,
        max_level,
        warnings_as_errors,
        reference_cache: ReferenceCache::build(document, schema),
    };

    // Run validation levels up to max_level
    if max_level >= ValidationLevel::Parse {
        validate_parse(&mut ctx);
    }
    if max_level >= ValidationLevel::Schema {
        validate_schema(&mut ctx);
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
                            .hint(format!(
                                "Expected type: {:?}, got: {}",
                                field_schema.field_type,
                                typed_value.value.type_name()
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

/// L5: Reference validation - existence, type compatibility, predicates
fn validate_reference(ctx: &mut ValidationContext) {
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

                if let Some(ref_schema) = &field_schema.reference {
                    validate_single_reference(
                        ctx,
                        table_name,
                        row,
                        field_name,
                        typed_value,
                        ref_schema,
                    );
                }
            }
        }
    }
}

fn validate_single_reference(
    ctx: &mut ValidationContext,
    table_name: &str,
    row: &Row,
    field_name: &str,
    typed_value: &TypedValue,
    ref_schema: &ReferenceSchema,
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

    // E1410: Check predicate (semantic compatibility)
    if let Some(predicate) = &ref_schema.predicate {
        if !evaluate_predicate(predicate, &target_row, &ctx.schema.schema) {
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
                .hint(format!(
                    "Predicate '{}' not satisfied by referenced {}",
                    predicate.assert, ref_schema.table
                ))
                .related(
                    reference::E1410,
                    "Referenced row here",
                    target_row.location.clone(),
                )
                .build(),
            );
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

fn evaluate_predicate(predicate: &ExpressionRule, _target_row: &Row, _schema: &Schema) -> bool {
    // Simplified expression evaluation - in practice would use a proper expression engine
    // For now, just check simple field comparisons
    // This is a placeholder - real implementation would parse and evaluate expressions
    let _ = predicate;
    true
}

/// L6: Semantic validation - expression rules on rows
fn validate_semantic(ctx: &mut ValidationContext) {
    let schema = &ctx.schema.schema;
    let doc = ctx.document;

    for (table_name, table) in &doc.tables {
        let Some(table_schema) = schema.tables.get(table_name) else {
            continue;
        };

        for row in &table.rows {
            // Table-level semantic rules
            for rule in table_schema.fields.values().flat_map(|f| f.rules.iter()) {
                if !evaluate_expression(rule, row, table_schema, schema) {
                    let severity = if rule.warning_only {
                        Severity::Warning
                    } else {
                        Severity::Error
                    };
                    let mut diag =
                        DiagnosticBuilder::error(semantic::E1501, "Semantic rule violation")
                            .location(row.location.clone())
                            .table(table_name)
                            .row(format!("{}", row.index))
                            .hint(
                                rule.message.clone().unwrap_or_else(|| {
                                    format!("Assertion '{}' failed", rule.assert)
                                }),
                            )
                            .build();
                    diag.severity = severity;
                    ctx.diagnostics.add(diag);
                }
            }
        }
    }
}

fn evaluate_expression(
    _rule: &ExpressionRule,
    _row: &Row,
    _table_schema: &TableSchema,
    _schema: &Schema,
) -> bool {
    // Placeholder for expression evaluation
    // Real implementation would use a proper expression parser/evaluator
    true
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

    #[test]
    fn test_value_validation_catches_range() {
        let schema = ValidatedSchema {
            schema: make_test_schema(),
            dependency_graph: crate::schema::DependencyGraph::default(),
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
            dependency_graph: crate::schema::DependencyGraph::default(),
        };
        let diags = validate(&validated_schema, &doc, ValidationLevel::Table, false);

        assert!(diags.has_errors());
        let errors = diags.errors();
        assert!(errors.iter().any(|e| e.code == table::E1301));
    }
}
