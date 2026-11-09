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

mod l0_parse;
mod l1_schema;
mod l2_types;
mod l3_values;
mod l4_tables;
mod l5_refs;
mod l6_semantic;
#[cfg(test)]
mod tests;

pub(crate) use l0_parse::validate_parse;
pub(crate) use l1_schema::{validate_profile_visibility, validate_schema};
pub(crate) use l5_refs::validate_reference;

pub use l1_schema::check_profile_visibility;
pub(crate) use l2_types::validate_type;
pub(crate) use l3_values::validate_value;
pub(crate) use l4_tables::{
    parse_cardinality, report_cardinality_violation, validate_table, Cardinality,
};
pub(crate) use l6_semantic::{validate_game_rule, validate_semantic};
