//! Schema definition - decoupled from Source, defines structure and constraints

use crate::value::SourceLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

/// Field type system
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum FieldType {
    /// Null type
    Null,
    /// Boolean type
    Bool,
    /// 8-bit signed integer
    Int8,
    /// 16-bit signed integer
    Int16,
    /// 32-bit signed integer
    Int32,
    /// 64-bit signed integer
    Int64,
    /// 8-bit unsigned integer
    UInt8,
    /// 16-bit unsigned integer
    UInt16,
    /// 32-bit unsigned integer
    UInt32,
    /// 64-bit unsigned integer
    UInt64,
    /// 32-bit floating point
    Float32,
    /// 64-bit floating point
    Float64,
    /// UTF-8 string
    String,
    /// Raw bytes
    Bytes,
    /// Array with element type
    Array(Box<FieldType>),
    /// Object with typed properties
    Object(IndexMap<String, FieldType>),
    /// Reference to a named enum
    Enum(String),
    /// Any value
    Any,
}

impl FieldType {
    /// Whether this is any numeric type
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            FieldType::Int8
                | FieldType::Int16
                | FieldType::Int32
                | FieldType::Int64
                | FieldType::UInt8
                | FieldType::UInt16
                | FieldType::UInt32
                | FieldType::UInt64
                | FieldType::Float32
                | FieldType::Float64
        )
    }

    /// Whether this is any integer type
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            FieldType::Int8
                | FieldType::Int16
                | FieldType::Int32
                | FieldType::Int64
                | FieldType::UInt8
                | FieldType::UInt16
                | FieldType::UInt32
                | FieldType::UInt64
        )
    }

    /// Whether this is a floating point type
    pub fn is_float(&self) -> bool {
        matches!(self, FieldType::Float32 | FieldType::Float64)
    }

    /// Corresponding Rust type name (for code generation)
    pub fn rust_type(&self) -> &'static str {
        match self {
            FieldType::Null => "()",
            FieldType::Bool => "bool",
            FieldType::Int8 => "i8",
            FieldType::Int16 => "i16",
            FieldType::Int32 => "i32",
            FieldType::Int64 => "i64",
            FieldType::UInt8 => "u8",
            FieldType::UInt16 => "u16",
            FieldType::UInt32 => "u32",
            FieldType::UInt64 => "u64",
            FieldType::Float32 => "f32",
            FieldType::Float64 => "f64",
            FieldType::String => "String",
            FieldType::Bytes => "Vec<u8>",
            FieldType::Array(_) => "Vec<_>",
            FieldType::Object(_) => "IndexMap<String, _>",
            FieldType::Enum(_) => "Enum",
            FieldType::Any => "serde_json::Value",
        }
    }
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
    /// Compatibility check (for E1410)
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
    pub dependency_graph: DependencyGraph,
}

/// Dependency graph between tables
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// table -> set of tables it depends on
    edges: IndexMap<String, HashSet<String>>,
}

impl DependencyGraph {
    /// Create an empty dependency graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `from` depends on `to`
    pub fn add_dependency(&mut self, from: &str, to: &str) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .insert(to.to_string());
    }

    /// Tables that `table` depends on (direct edges)
    pub fn dependencies_of(&self, table: &str) -> Vec<&String> {
        self.edges
            .get(table)
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    /// Topologically sort all tables; errors on circular dependencies
    pub fn topological_sort(&self) -> Result<Vec<String>, String> {
        let mut visited = HashSet::new();
        let mut temp = HashSet::new();
        let mut order = Vec::new();

        for node in self.edges.keys() {
            if !visited.contains(node) {
                Self::visit_topo(self, node, &mut visited, &mut temp, &mut order)?;
            }
        }

        // Also include tables with no dependencies
        for table in self.edges.keys() {
            if !order.contains(table) {
                order.push(table.clone());
            }
        }

        Ok(order)
    }

    fn visit_topo(
        graph: &DependencyGraph,
        node: &str,
        visited: &mut HashSet<String>,
        temp: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if temp.contains(node) {
            return Err(format!("Circular dependency detected involving: {node}"));
        }
        if visited.contains(node) {
            return Ok(());
        }
        temp.insert(node.to_string());
        for dep in graph.dependencies_of(node) {
            Self::visit_topo(graph, dep, visited, temp, order)?;
        }
        temp.remove(node);
        visited.insert(node.to_string());
        order.push(node.to_string());
        Ok(())
    }
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

    /// Validate schema internal consistency
    pub fn validate(&self) -> crate::diagnostics::Diagnostics {
        let mut diags = crate::diagnostics::Diagnostics::new();

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

    /// Build dependency graph from references
    pub fn build_dependency_graph(&self) -> DependencyGraph {
        let mut graph = DependencyGraph::new();
        for table_name in self.tables.keys() {
            // Register every table so isolated tables (no references in or
            // out) still appear in the topological build order.
            graph.edges.entry(table_name.clone()).or_default();
        }
        for (table_name, table) in &self.tables {
            for field in table.fields.values() {
                if let Some(ref_schema) = &field.reference {
                    graph.add_dependency(table_name, &ref_schema.table);
                }
            }
        }
        graph
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_validation() {
        let mut schema = Schema::new();
        let mut table = TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
        };
        table.fields.insert(
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
        schema.add_table(table);

        let diags = schema.validate();
        assert!(!diags.has_errors());
    }

    #[test]
    fn test_dependency_graph() {
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
        schema.add_table(item);

        let mut monster = TableSchema {
            name: "Monster".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
        };
        monster.fields.insert(
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
        monster.fields.insert(
            "drop_item_id".to_string(),
            FieldSchema {
                name: "drop_item_id".to_string(),
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
                reference: Some(ReferenceSchema {
                    table: "Item".to_string(),
                    field: "id".to_string(),
                    predicate: None,
                    cardinality: "many".to_string(),
                    compatible_with: None,
                }),
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        schema.add_table(monster);

        let graph = schema.build_dependency_graph();
        let deps = graph.dependencies_of("Monster");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "Item");

        let order = graph.topological_sort().unwrap();
        assert_eq!(order[0], "Item");
        assert_eq!(order[1], "Monster");
    }
}
