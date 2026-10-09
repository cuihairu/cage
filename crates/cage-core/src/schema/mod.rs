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
    /// Map with typed keys and values: `map<K, V>`.
    ///
    /// Canonical YAML/JSON schema syntax (adjacently tagged like every other
    /// kind):
    ///
    /// ```yaml
    /// type:
    ///   kind: Map
    ///   value:
    ///     key_type: string              # string | int
    ///     value_type:
    ///       kind: Array
    ///       value: { kind: Int32 }
    /// ```
    ///
    /// So `map<string, Array<Int32>>` =
    /// `{ kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }`.
    /// The payload is a struct rather than a bare `Box<FieldType>` because the
    /// key type declaration has to live inside the type — nested maps carry
    /// their own `key_type` that way.
    Map(MapField),
    /// Reference to a named enum
    Enum(String),
    /// Any value
    Any,
}

/// Map payload: key type plus (possibly nested) value type — `map<K, V>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapField {
    /// Key type: string keys or signed-integer keys.
    pub key_type: MapKeyType,
    /// Value type — arbitrary, may nest (Array / Map / Object / ...).
    pub value_type: Box<FieldType>,
}

/// Key kinds accepted by [`MapField::key_type`] (wire format: `string` | `int`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapKeyType {
    /// String keys — every `Value::Object` key is a string, so all pass.
    String,
    /// Signed-integer keys, stored as numeric strings in the data model
    /// (`"42"`, `"-7"`); non-numeric keys are rejected at L2.
    Int,
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
            FieldType::Map(m) => match m.key_type {
                MapKeyType::String => "HashMap<String, _>",
                MapKeyType::Int => "HashMap<i64, _>",
            },
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
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// `map<key_type, value_type>` — shared by the tests below.
    fn map_type(key_type: MapKeyType, value_type: FieldType) -> FieldType {
        FieldType::Map(MapField {
            key_type,
            value_type: Box::new(value_type),
        })
    }

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

        let graph = crate::reference::DependencyGraph::from_schema(&schema);
        let deps = graph.dependencies("Monster");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "Item");

        let order = graph.topological_sort().unwrap();
        assert_eq!(order[0], "Item");
        assert_eq!(order[1], "Monster");
    }

    #[test]
    fn field_type_classification_helpers() {
        for t in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
            FieldType::Float32,
            FieldType::Float64,
        ] {
            assert!(t.is_numeric());
        }
        for t in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
        ] {
            assert!(t.is_integer());
            assert!(!t.is_float());
        }
        assert!(FieldType::Float32.is_float());
        assert!(FieldType::Float64.is_float());
        assert!(FieldType::Float32.is_numeric());
        assert!(FieldType::Float64.is_numeric());
        assert!(!FieldType::Float32.is_integer());
        assert!(!FieldType::Float64.is_integer());
        for t in [
            FieldType::Null,
            FieldType::Bool,
            FieldType::String,
            FieldType::Bytes,
            FieldType::Any,
            FieldType::Enum("E".to_string()),
            FieldType::Array(Box::new(FieldType::Int32)),
            FieldType::Object(IndexMap::new()),
            map_type(MapKeyType::String, FieldType::Int32),
            map_type(MapKeyType::Int, FieldType::Int32),
        ] {
            assert!(!t.is_numeric());
            assert!(!t.is_integer());
            assert!(!t.is_float());
        }
    }

    #[test]
    fn rust_type_names_for_every_variant() {
        assert_eq!(FieldType::Null.rust_type(), "()");
        assert_eq!(FieldType::Bool.rust_type(), "bool");
        assert_eq!(FieldType::Int8.rust_type(), "i8");
        assert_eq!(FieldType::Int16.rust_type(), "i16");
        assert_eq!(FieldType::Int32.rust_type(), "i32");
        assert_eq!(FieldType::Int64.rust_type(), "i64");
        assert_eq!(FieldType::UInt8.rust_type(), "u8");
        assert_eq!(FieldType::UInt16.rust_type(), "u16");
        assert_eq!(FieldType::UInt32.rust_type(), "u32");
        assert_eq!(FieldType::UInt64.rust_type(), "u64");
        assert_eq!(FieldType::Float32.rust_type(), "f32");
        assert_eq!(FieldType::Float64.rust_type(), "f64");
        assert_eq!(FieldType::String.rust_type(), "String");
        assert_eq!(FieldType::Bytes.rust_type(), "Vec<u8>");
        assert_eq!(
            FieldType::Array(Box::new(FieldType::Int32)).rust_type(),
            "Vec<_>"
        );
        assert_eq!(
            FieldType::Object(IndexMap::new()).rust_type(),
            "IndexMap<String, _>"
        );
        assert_eq!(
            map_type(MapKeyType::String, FieldType::Int32).rust_type(),
            "HashMap<String, _>"
        );
        assert_eq!(
            map_type(MapKeyType::Int, FieldType::Int32).rust_type(),
            "HashMap<i64, _>"
        );
        assert_eq!(FieldType::Enum("Rarity".to_string()).rust_type(), "Enum");
        assert_eq!(FieldType::Any.rust_type(), "serde_json::Value");
    }

    #[test]
    fn field_type_yaml_parsing_all_kinds_and_edges() {
        // Every one of the 19 FieldType kinds must round-trip through YAML.
        let cases = [
            ("{ kind: Null }", FieldType::Null),
            ("{ kind: Bool }", FieldType::Bool),
            ("{ kind: Int8 }", FieldType::Int8),
            ("{ kind: Int16 }", FieldType::Int16),
            ("{ kind: Int32 }", FieldType::Int32),
            ("{ kind: Int64 }", FieldType::Int64),
            ("{ kind: UInt8 }", FieldType::UInt8),
            ("{ kind: UInt16 }", FieldType::UInt16),
            ("{ kind: UInt32 }", FieldType::UInt32),
            ("{ kind: UInt64 }", FieldType::UInt64),
            ("{ kind: Float32 }", FieldType::Float32),
            ("{ kind: Float64 }", FieldType::Float64),
            ("{ kind: String }", FieldType::String),
            ("{ kind: Bytes }", FieldType::Bytes),
            ("{ kind: Any }", FieldType::Any),
            (
                "{ kind: Array, value: { kind: Int32 } }",
                FieldType::Array(Box::new(FieldType::Int32)),
            ),
            (
                "{ kind: Object, value: { hp: { kind: Int32 }, tag: { kind: String } } }",
                FieldType::Object(IndexMap::from([
                    ("hp".to_string(), FieldType::Int32),
                    ("tag".to_string(), FieldType::String),
                ])),
            ),
            (
                "{ kind: Enum, value: Rarity }",
                FieldType::Enum("Rarity".to_string()),
            ),
            // The 19th kind: map<K, V> — string keys ...
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }",
                map_type(MapKeyType::String, FieldType::Int32),
            ),
            // ... signed-integer keys ...
            (
                "{ kind: Map, value: { key_type: int, value_type: { kind: String } } }",
                map_type(MapKeyType::Int, FieldType::String),
            ),
            // ... and arbitrary nesting in both directions.
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } }",
                map_type(
                    MapKeyType::String,
                    map_type(MapKeyType::String, FieldType::Int32),
                ),
            ),
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }",
                map_type(MapKeyType::String, FieldType::Array(Box::new(FieldType::Int32))),
            ),
        ];
        let mut kinds = HashSet::new();
        for (yaml, expected) in cases {
            let parsed: FieldType =
                serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("parse {yaml}: {e}"));
            assert_eq!(parsed, expected, "yaml: {yaml}");
            // serializing and re-parsing keeps the same shape
            let emitted = serde_yaml::to_string(&parsed).expect("serialize kind");
            let back: FieldType = serde_yaml::from_str(&emitted).expect("re-parse kind");
            assert_eq!(back, expected, "roundtrip: {emitted}");
            // Debug starts with the variant name (e.g. `Map(MapField { ... })`)
            let debug = format!("{expected:?}");
            kinds.insert(
                debug
                    .split('(')
                    .next()
                    .expect("Debug rendering starts with the variant name")
                    .to_string(),
            );
        }
        assert_eq!(kinds.len(), 19, "kinds covered: {kinds:?}");

        // unknown kind, missing content for data variants, missing tag, not a map
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Quadruple }").is_err());
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Array }").is_err());
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Enum }").is_err());
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Map }").is_err());
        assert!(
            serde_yaml::from_str::<FieldType>(
                "{ kind: Map, value: { key_type: bool, value_type: { kind: Int32 } } }"
            )
            .is_err(),
            "unknown key_type must fail"
        );
        assert!(
            serde_yaml::from_str::<FieldType>("{ kind: Map, value: { key_type: string } }")
                .is_err(),
            "missing value_type must fail"
        );
        assert!(serde_yaml::from_str::<FieldType>("{}").is_err());
        assert!(serde_yaml::from_str::<FieldType>("just-a-string").is_err());
    }

    #[test]
    fn map_key_type_wire_format_is_lowercase() {
        // Wire format is the lowercase rename — "string" / "int", nothing else.
        assert_eq!(
            serde_yaml::to_string(&MapKeyType::String).expect("yaml string"),
            "string\n"
        );
        assert_eq!(
            serde_yaml::to_string(&MapKeyType::Int).expect("yaml int"),
            "int\n"
        );
        assert_eq!(
            serde_json::to_string(&MapKeyType::String).expect("json string"),
            "\"string\""
        );
        assert_eq!(
            serde_json::to_string(&MapKeyType::Int).expect("json int"),
            "\"int\""
        );
        for (wire, expected) in [
            ("string", MapKeyType::String),
            ("int", MapKeyType::Int),
            ("\"string\"", MapKeyType::String),
            ("\"int\"", MapKeyType::Int),
        ] {
            let key: MapKeyType = if wire.starts_with('"') {
                serde_json::from_str(wire).expect("json key type")
            } else {
                serde_yaml::from_str(wire).expect("yaml key type")
            };
            assert_eq!(key, expected, "wire: {wire}");
        }
        assert!(serde_yaml::from_str::<MapKeyType>("u32").is_err());
        assert!(serde_yaml::from_str::<MapKeyType>("String").is_err());
        assert!(serde_json::from_str::<MapKeyType>("\"u32\"").is_err());
    }

    #[test]
    fn field_schema_yaml_parses_map_field() {
        let yaml = r"
name: drops
type:
  kind: Map
  value:
    key_type: string
    value_type:
      kind: Array
      value: { kind: Int32 }
description: drop table keyed by rarity
";
        let field: FieldSchema = serde_yaml::from_str(yaml).expect("map field parses");
        assert_eq!(field.name, "drops");
        assert_eq!(
            field.field_type,
            map_type(
                MapKeyType::String,
                FieldType::Array(Box::new(FieldType::Int32))
            )
        );
        assert_eq!(
            field.description.as_deref(),
            Some("drop table keyed by rarity")
        );
        // JSON carries the same shape
        let json = serde_json::to_value(&field.field_type).expect("map type to json");
        assert_eq!(json["kind"], "Map");
        assert_eq!(json["value"]["key_type"], "string");
        assert_eq!(json["value"]["value_type"]["kind"], "Array");
        let back: FieldType = serde_json::from_value(json).expect("map type from json");
        assert_eq!(back, field.field_type);
    }

    #[test]
    fn field_schema_yaml_parsing_rules_constraints_and_metadata() {
        let yaml = r#"
name: level
type: { kind: Int32 }
description: skill level
required: true
default: 1
min: 1
max: 100
targets: [server]
rules:
  - name: level_bounds
    assert: "`level` >= 1"
    message: level must be positive
    warning_only: true
  - name: no_message
    assert: "`level` <= 100"
custom_note: keep
"#;
        let field: FieldSchema = serde_yaml::from_str(yaml).expect("field parses");
        assert_eq!(field.name, "level");
        assert_eq!(field.field_type, FieldType::Int32);
        assert_eq!(field.description.as_deref(), Some("skill level"));
        assert!(field.required);
        assert_eq!(field.default, Some(serde_json::json!(1)));
        assert_eq!(field.min, Some(1.0));
        assert_eq!(field.max, Some(100.0));
        assert_eq!(field.targets, ["server"]);
        assert_eq!(field.rules.len(), 2);
        assert_eq!(field.rules[0].name, "level_bounds");
        assert_eq!(field.rules[0].assert, "`level` >= 1");
        assert_eq!(
            field.rules[0].message.as_deref(),
            Some("level must be positive")
        );
        assert!(field.rules[0].warning_only);
        // message absent and warning_only defaults to false
        assert_eq!(field.rules[1].message, None);
        assert!(!field.rules[1].warning_only);
        // unknown keys flatten into the metadata bag
        assert_eq!(
            field.metadata.get("custom_note"),
            Some(&serde_json::json!("keep"))
        );
        assert!(field.min_length.is_none());
        assert!(field.max_length.is_none());
        assert!(field.pattern.is_none());

        // required keys: `name` and `type` must both be present
        assert!(serde_yaml::from_str::<FieldSchema>("type: { kind: String }\n").is_err());
        assert!(serde_yaml::from_str::<FieldSchema>("name: x\n").is_err());
        // an expression rule requires its `assert`
        assert!(serde_yaml::from_str::<ExpressionRule>("name: r\n").is_err());
        let bare: ExpressionRule =
            serde_yaml::from_str("name: r\nassert: 'true'\n").expect("minimal rule parses");
        assert!(!bare.warning_only);
        assert!(bare.message.is_none());
    }

    #[test]
    fn table_schema_yaml_parsing_primary_key_unique_and_order() {
        let yaml = r"
name: Item
description: an item
primary_key: [id, version]
fields:
  id: { name: id, type: { kind: UInt32 } }
  version: { name: version, type: { kind: UInt16 } }
unique_constraints:
  - { name: uniq_pair, fields: [id, version] }
order_by: [id]
targets: [server, client]
";
        let table: TableSchema = serde_yaml::from_str(yaml).expect("table parses");
        assert_eq!(table.name, "Item");
        assert_eq!(table.description.as_deref(), Some("an item"));
        assert_eq!(table.primary_key, ["id", "version"]);
        assert_eq!(table.fields.len(), 2);
        assert_eq!(table.unique_constraints.len(), 1);
        assert_eq!(table.unique_constraints[0].name, "uniq_pair");
        assert_eq!(table.unique_constraints[0].fields, ["id", "version"]);
        assert_eq!(table.order_by, Some(vec!["id".to_string()]));
        assert_eq!(table.targets, ["server", "client"]);

        // primary_key has no default → required
        assert!(
            serde_yaml::from_str::<TableSchema>("name: T\nfields: {}\n").is_err(),
            "missing primary_key must fail"
        );
        // a unique constraint must name its fields
        assert!(serde_yaml::from_str::<TableSchema>(
            "name: T\nprimary_key: [a]\nfields: {}\nunique_constraints: [{ name: c }]\n"
        )
        .is_err());
    }

    #[test]
    fn reference_schema_cardinality_serde_edges() {
        let bare: ReferenceSchema =
            serde_yaml::from_str("table: Item\nfield: id\n").expect("minimal reference");
        assert_eq!(bare.cardinality, "one");
        assert!(bare.predicate.is_none());
        assert!(bare.compatible_with.is_none());
        // cardinality "one" is the default and is skipped when serializing
        let emitted = serde_yaml::to_string(&bare).expect("serialize reference");
        assert!(!emitted.contains("cardinality"));

        let many: ReferenceSchema = serde_yaml::from_str(
            "table: Item\nfield: id\ncardinality: many\ncompatible_with: [server]\n",
        )
        .expect("many reference");
        assert_eq!(many.cardinality, "many");
        assert_eq!(many.compatible_with, Some(vec!["server".to_string()]));
        let emitted = serde_yaml::to_string(&many).expect("serialize many");
        assert!(emitted.contains("cardinality"));
        let back: ReferenceSchema = serde_yaml::from_str(&emitted).expect("re-parse reference");
        assert_eq!(back.cardinality, "many");

        // required keys: table and field
        assert!(serde_yaml::from_str::<ReferenceSchema>("table: Item\n").is_err());
        assert!(serde_yaml::from_str::<ReferenceSchema>("field: id\n").is_err());
    }

    #[test]
    fn schema_default_and_enum_registration() {
        let mut schema = Schema::default();
        assert!(schema.tables.is_empty());
        assert!(schema.enums.is_empty());
        assert!(schema.metadata.is_none());
        assert!(schema.get_table("Item").is_none());

        schema.add_enum(EnumSchema {
            name: "Rarity".to_string(),
            values: vec![EnumValue {
                name: "Common".to_string(),
                value: None,
                description: None,
            }],
            description: None,
        });
        assert!(schema.enums.contains_key("Rarity"));
        // adding the same name replaces the previous definition
        schema.add_enum(EnumSchema {
            name: "Rarity".to_string(),
            values: vec![],
            description: None,
        });
        assert!(schema.enums["Rarity"].values.is_empty());
    }

    #[test]
    fn schema_lookup_helpers_hit_and_miss() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 } }
enums:
  Rarity:
    name: Rarity
    values:
      - { name: Common }
",
        )
        .expect("schema parses");

        let item = schema.get_table("Item").expect("Item exists");
        assert_eq!(item.primary_key, ["id"]);
        assert!(schema.get_table("Ghost").is_none());

        let id = schema.get_field("Item", "id").expect("id exists");
        assert_eq!(id.field_type, FieldType::UInt32);
        assert!(schema.get_field("Item", "ghost").is_none());
        assert!(schema.get_field("Ghost", "id").is_none());
    }

    #[test]
    fn schema_validate_reports_every_error_kind() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [missing_pk]
    fields:
      id: { name: id, type: { kind: UInt32 } }
      rarity: { name: rarity, type: { kind: Enum, value: Rarity } }
      element: { name: element, type: { kind: Enum, value: Present } }
      drop: { name: drop, type: { kind: UInt32 }, reference: { table: Ghost, field: id } }
      sub: { name: sub, type: { kind: UInt32 }, reference: { table: Item, field: nope } }
    unique_constraints:
      - { name: uc_missing, fields: [ghost_field] }
enums:
  Present:
    name: Present
    values:
      - { name: A }
",
        )
        .expect("schema parses");

        let diags = schema.validate();
        assert!(diags.has_errors());
        let messages: Vec<String> = diags.errors().iter().map(|d| d.message.clone()).collect();
        assert_eq!(messages.len(), 5, "messages: {messages:?}");
        let expect = |needle: &str| {
            assert!(
                messages.iter().any(|m| m.contains(needle)),
                "no message contains {needle:?} in {messages:?}"
            );
        };
        expect("Primary key 'missing_pk' not found in table 'Item' fields");
        expect("Unique constraint 'uc_missing' references unknown field 'ghost_field'");
        expect("Field 'rarity' in table 'Item' references unknown enum 'Rarity'");
        expect("Field 'drop' in table 'Item' references unknown table 'Ghost'");
        expect("Field 'sub' in table 'Item' references unknown field 'nope' in table 'Item'");
        for d in &diags {
            assert_eq!(d.code, "E1004");
            assert_eq!(d.source, "schema");
        }
        // a schema with only resolvable references validates cleanly
        let clean: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 } }
enums: {}
",
        )
        .expect("clean schema parses");
        assert!(!clean.validate().has_errors());
    }

    #[test]
    fn dependency_graph_sort_handles_revisits_and_cycles() {
        // A → B → C: C is pulled in as a dependency before the loop reaches B,
        // so the second visit of B skips the recursive call entirely.
        let mut graph = crate::reference::DependencyGraph::new();
        graph.add_edge("A", "B");
        graph.add_edge("B", "C");
        assert_eq!(graph.dependencies("A"), ["B"]);
        assert_eq!(graph.dependencies("B"), ["C"]);
        assert_eq!(graph.dependencies("C"), [] as [&String; 0]);
        let order = graph.topological_sort().expect("acyclic");
        assert_eq!(order, ["C", "B", "A"]);

        // circular dependencies are reported instead of looping forever
        let mut cyclic = crate::reference::DependencyGraph::new();
        cyclic.add_edge("X", "Y");
        cyclic.add_edge("Y", "X");
        let err = cyclic.topological_sort().expect_err("cycle detected");
        assert!(
            err.contains("Circular dependency detected involving:"),
            "unexpected error: {err}"
        );
    }
}
