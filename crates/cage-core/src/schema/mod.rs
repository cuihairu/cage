//! Schema definition - decoupled from Source, defines structure and constraints

use crate::value::SourceLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Root schema document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Table schemas by name
    pub tables: IndexMap<String, TableSchema>,
    /// Shared enum definitions
    pub enums: IndexMap<String, EnumSchema>,
    /// Schema-level metadata
    pub metadata: Option<SchemaMetadata>,
}

/// Metadata about the schema itself
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaMetadata {
    /// Schema version string
    pub version: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Author name
    pub author: Option<String>,
}

impl Schema {
    /// Every environment name declared by any table's `env_overrides`,
    /// sorted — the valid `--env` inputs for this schema.
    pub fn declared_envs(&self) -> std::collections::BTreeSet<String> {
        self.tables
            .values()
            .flat_map(|table| table.env_overrides.keys().cloned())
            .collect()
    }

    /// Structural check of every table's `env_overrides`: each override
    /// must name a field that exists in its table and set at least one
    /// constraint key. Errors carry the table/env/field path — the CLI
    /// renders them as plain load errors, before any `--env` is chosen.
    pub fn validate_env_overrides(&self) -> Result<(), String> {
        match self.lint_env_overrides().into_iter().next() {
            Some(problem) => Err(problem),
            None => Ok(()),
        }
    }

    /// Every `env_overrides` structural problem, all of them, not just the
    /// first — the base lane's lint. A schema with a broken override in an
    /// environment nobody selected would otherwise validate clean until
    /// someone runs `--env` on it; the CLI prints these as warnings on the
    /// base path (no `--env`) and hard-errors on the `--env` path.
    pub fn lint_env_overrides(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (table_name, table) in &self.tables {
            for (env, overrides) in &table.env_overrides {
                for (field_name, patch) in overrides {
                    if !table.fields.contains_key(field_name) {
                        problems.push(format!(
                            "env_overrides[{env}] names unknown field '{field_name}' in table \
                             '{table_name}'"
                        ));
                    }
                    if *patch == FieldOverride::default() {
                        problems.push(format!(
                            "env_overrides[{env}].{field_name} in table '{table_name}' sets no \
                             constraint"
                        ));
                    }
                }
            }
        }
        problems
    }

    /// The schema as it validates under `env`: every `env_overrides[env]`
    /// patch applied onto the base fields (later keys in one patch win —
    /// an override is a whole-constraint replacement, not a merge). The
    /// base schema is never mutated; an environment that declares no
    /// overrides for a table leaves that table untouched. The result is
    /// what every downstream stage — validation, codegen, hashing —
    /// consumes, so `--env` changes exactly the rules those stages see.
    pub fn resolve_env(&self, env: &str) -> Result<Schema, String> {
        let mut resolved = self.clone();
        for (table_name, table) in &mut resolved.tables {
            let Some(overrides) = table.env_overrides.get(env) else {
                continue;
            };
            for (field_name, patch) in overrides {
                let Some(field) = table.fields.get_mut(field_name) else {
                    return Err(format!(
                        "env_overrides[{env}] names unknown field '{field_name}' in table \
                         '{table_name}'"
                    ));
                };
                if let Some(v) = patch.required {
                    field.required = v;
                }
                if let Some(v) = patch.min {
                    field.min = Some(v);
                }
                if let Some(v) = patch.max {
                    field.max = Some(v);
                }
                if let Some(v) = patch.min_length {
                    field.min_length = Some(v);
                }
                if let Some(v) = patch.max_length {
                    field.max_length = Some(v);
                }
                if let Some(v) = patch.pattern.clone() {
                    field.pattern = Some(v);
                }
                if let Some(v) = patch.enum_values.clone() {
                    field.enum_values = Some(v);
                }
                if let Some(v) = patch.min_items {
                    field.min_items = Some(v);
                }
                if let Some(v) = patch.max_items {
                    field.max_items = Some(v);
                }
            }
        }
        Ok(resolved)
    }
}

/// Table schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    /// Table name
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Primary key field(s)
    pub primary_key: Vec<String>,
    /// Field definitions
    pub fields: IndexMap<String, FieldSchema>,
    /// Composite unique constraints
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub unique_constraints: Vec<UniqueConstraint>,
    /// Row ordering requirement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<Vec<String>>,
    /// Target profiles this table applies to
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub targets: Vec<String>,
    /// Per-environment constraint overrides (`--env` validation, design
    /// §48): environment name → field name → constraint deltas. Applied
    /// by [`Schema::resolve_env`] before validation; the base fields
    /// stay untouched.
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub env_overrides: IndexMap<String, IndexMap<String, FieldOverride>>,
}

/// One environment's constraint deltas for one field (design §48): a
/// partial field patch — only the constraint keys an environment may
/// tighten or loosen. Keys absent here keep the base value. Types stay
/// out on purpose: an environment never re-shapes a field, it re-tunes
/// how strictly the same shape is checked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldOverride {
    /// Override `required`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Override `min`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Override `max`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Override `min_length`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    /// Override `max_length`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// Override `pattern`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Override `enum_values`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// Override `min_items`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,
    /// Override `max_items`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
}

/// Unique constraint on multiple fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniqueConstraint {
    /// Constraint name
    pub name: String,
    /// Field names forming the unique key
    pub fields: Vec<String>,
}

/// Field schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Field name
    pub name: String,
    /// Field type
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Human-readable description
    pub description: Option<String>,
    /// Whether the field must be present
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// Whether a present Null (e.g. an Excel empty cell) is a valid value
    /// (回流-2): the nullable equivalent of Luban's `string?` / `int?` — the
    /// type check accepts Null without weakening the field to `Any`. A
    /// nullable reference field should also use `cardinality: optional`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub nullable: bool,
    /// Default value applied when absent
    pub default: Option<serde_json::Value>,
    // Numeric constraints
    /// Minimum numeric value (inclusive)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum numeric value (inclusive)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    // String constraints
    /// Minimum string length
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    /// Maximum string length
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// Regular expression the string must match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    // Enum constraint
    /// Allowed values (inline enum)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    // Array constraints
    /// Minimum number of items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,
    /// Maximum number of items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
    /// Element schema for arrays
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<FieldSchema>>,
    /// Split a scalar table cell (Excel/CSV cannot hold arrays) into the
    /// field's array elements on this separator (回流-3; the equivalent of
    /// Luban's `(list#sep=,)`). Each token is trimmed and coerced to the
    /// element type; a cell that fails to split cleanly stays as-is and the
    /// type check reports it. Only meaningful on `Array` fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
    // Object constraints
    /// Property schemas for objects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<IndexMap<String, FieldSchema>>,
    /// Whether keys outside `properties` are allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_properties: Option<bool>,
    // Reference constraint
    /// Cross-table reference definition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceSchema>,
    /// Target profiles this field appears in
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub targets: Vec<String>,
    /// Semantic validation rules (expressions)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rules: Vec<ExpressionRule>,
    /// Custom metadata
    #[serde(flatten)]
    pub metadata: IndexMap<String, serde_json::Value>,
}

/// Reference to another table/field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceSchema {
    /// Referenced table name
    pub table: String,
    /// Referenced field name
    pub field: String,
    /// Additional predicate: the referenced object must satisfy this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ExpressionRule>,
    /// Cardinality: "one" | "many" | "optional"
    #[serde(default = "default_cardinality", skip_serializing_if = "is_one")]
    pub cardinality: String,
    /// Compatibility check (for E1411)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_with: Option<Vec<String>>,
}

fn default_cardinality() -> String {
    "one".to_string()
}

fn is_one(s: &str) -> bool {
    s == "one"
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Expression rule for semantic validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionRule {
    /// Rule name
    pub name: String,
    /// Expression like "`min_level` <= `max_level`"
    pub assert: String,
    /// Custom diagnostic message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Whether a violation is reported as a warning
    #[serde(default, skip_serializing_if = "is_false")]
    pub warning_only: bool,
}

/// Enum schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumSchema {
    /// Enum name
    pub name: String,
    /// Allowed values
    pub values: Vec<EnumValue>,
    /// Human-readable description
    pub description: Option<String>,
}

/// One value of an enum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumValue {
    /// Value name
    pub name: String,
    /// Optional literal backing value
    pub value: Option<serde_json::Value>,
    /// Human-readable description
    pub description: Option<String>,
}

/// Validation context for error reporting
#[derive(Debug, Clone)]
pub struct ValidationContext<'a> {
    /// Schema being validated against
    pub schema: &'a Schema,
    /// Table currently being validated
    pub current_table: Option<&'a str>,
    /// Row currently being validated (0-based)
    pub current_row: Option<usize>,
    /// Field currently being validated
    pub current_field: Option<&'a str>,
    /// Source location of the value under validation
    pub source_location: Option<SourceLocation>,
}

/// Validated schema with resolved references
#[derive(Debug, Clone)]
pub struct ValidatedSchema {
    /// The validated schema
    pub schema: Schema,
    /// Table dependency graph (for build order)
    pub dependency_graph: crate::reference::DependencyGraph,
}

impl Schema {
    /// Create an empty schema
    pub fn new() -> Self {
        Self {
            tables: IndexMap::new(),
            enums: IndexMap::new(),
            metadata: None,
        }
    }

    /// Add (or replace) a table schema
    pub fn add_table(&mut self, table: TableSchema) {
        self.tables.insert(table.name.clone(), table);
    }

    /// Add (or replace) a shared enum definition
    pub fn add_enum(&mut self, enum_schema: EnumSchema) {
        self.enums.insert(enum_schema.name.clone(), enum_schema);
    }

    /// Look up a table schema by name
    pub fn get_table(&self, name: &str) -> Option<&TableSchema> {
        self.tables.get(name)
    }

    /// Look up a field schema by table and field name
    pub fn get_field(&self, table: &str, field: &str) -> Option<&FieldSchema> {
        self.tables.get(table)?.fields.get(field)
    }

    /// Look up a shared enum definition by name — the read side of
    /// [`Schema::add_enum`], so enum consumers (validation L2, target
    /// generators) don't reach into `self.enums` directly.
    pub fn get_enum(&self, name: &str) -> Option<&EnumSchema> {
        self.enums.get(name)
    }

    /// Validate schema internal consistency
    pub fn validate(&self) -> crate::diagnostics::Diagnostics {
        let mut diags = crate::diagnostics::Diagnostics::new();

        // FieldType nesting depth guard: recursion through Array / Map /
        // Object / TaggedUnion is bounded by MAX_FIELD_TYPE_DEPTH.
        for (table_name, table) in &self.tables {
            for (field_name, field) in &table.fields {
                if let Some(measure) = field_type_depth(&field.field_type) {
                    if measure.depth > MAX_FIELD_TYPE_DEPTH {
                        // A saturated probe means the real depth is far past
                        // the cap (e.g. a hand-built 200k-deep type): name
                        // the cap without pretending to know the depth. A
                        // non-saturated depth of exactly MAX+1 is a real,
                        // one-over-the-cap authoring mistake — report it so
                        // the operator can tell "one level over" from a
                        // pathological type.
                        let msg = if measure.saturated {
                            format!(
                                "Field '{field_name}' in table '{table_name}' exceeds the \
                                 maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                            )
                        } else {
                            format!(
                                "Field '{field_name}' in table '{table_name}' has nesting \
                                 depth {} which exceeds the maximum nesting depth limit of \
                                 {MAX_FIELD_TYPE_DEPTH}",
                                measure.depth
                            )
                        };
                        diags.add(
                            crate::diagnostics::Diagnostic::error("E1004", msg)
                                .with_source("schema"),
                        );
                    }
                }
            }
        }

        // Check primary keys exist
        for (table_name, table) in &self.tables {
            for pk in &table.primary_key {
                if !table.fields.contains_key(pk) {
                    diags.add(
                        crate::diagnostics::Diagnostic::error(
                            "E1004",
                            format!("Primary key '{pk}' not found in table '{table_name}' fields"),
                        )
                        .with_source("schema"),
                    );
                }
            }
            // Check unique constraints reference existing fields
            for uc in &table.unique_constraints {
                for f in &uc.fields {
                    if !table.fields.contains_key(f) {
                        diags.add(
                            crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!(
                                    "Unique constraint '{}' references unknown field '{}'",
                                    uc.name, f
                                ),
                            )
                            .with_source("schema"),
                        );
                    }
                }
            }
        }

        // Check enum references
        for (table_name, table) in &self.tables {
            for (field_name, field) in &table.fields {
                if let FieldType::Enum(enum_name) = &field.field_type {
                    if !self.enums.contains_key(enum_name) {
                        diags.add(
                            crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!(
                                    "Field '{field_name}' in table '{table_name}' references unknown enum '{enum_name}'"
                                ),
                            )
                            .with_source("schema"),
                        );
                    }
                }
            }
        }

        // Check reference targets exist
        for (table_name, table) in &self.tables {
            for (field_name, field) in &table.fields {
                if let Some(ref_schema) = &field.reference {
                    if !self.tables.contains_key(&ref_schema.table) {
                        diags.add(
                            crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!(
                                    "Field '{}' in table '{}' references unknown table '{}'",
                                    field_name, table_name, ref_schema.table
                                ),
                            )
                            .with_source("schema"),
                        );
                    } else if let Some(ref_table) = self.tables.get(&ref_schema.table) {
                        if !ref_table.fields.contains_key(&ref_schema.field) {
                            diags.add(crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!("Field '{}' in table '{}' references unknown field '{}' in table '{}'", field_name, table_name, ref_schema.field, ref_schema.table),
                            ).with_source("schema"));
                        }
                    }
                }
            }
        }

        diags
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

mod field_type;
mod field_type_serde;
#[cfg(test)]
mod tests;

pub(crate) use field_type::field_type_depth;
pub use field_type::{FieldType, MapField, MapKeyType, TaggedUnion, MAX_FIELD_TYPE_DEPTH};
