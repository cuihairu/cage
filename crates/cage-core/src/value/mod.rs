//! Canonical Value types - the unified intermediate representation
//! All source adapters parse into these types, carrying source location metadata.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Source location information for precise diagnostics
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Source file path (relative to project root)
    pub file: String,
    /// Sheet name (for Excel) or logical section
    pub sheet: Option<String>,
    /// Row number (1-indexed, for tabular sources)
    pub row: Option<usize>,
    /// Column identifier (letter for Excel, name for CSV/JSON/YAML)
    pub column: Option<String>,
    /// Field name in the schema
    pub field: Option<String>,
    /// Byte offset in source file (for text formats)
    pub byte_offset: Option<usize>,
    /// Line number (for text formats)
    pub line: Option<usize>,
    /// Column number in line (for text formats)
    pub col: Option<usize>,
}

impl SourceLocation {
    /// Create a new source location for a file
    pub fn new(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            sheet: None,
            row: None,
            column: None,
            field: None,
            byte_offset: None,
            line: None,
            col: None,
        }
    }

    /// Set the sheet name (Excel) or logical section
    pub fn with_sheet(mut self, sheet: impl Into<String>) -> Self {
        self.sheet = Some(sheet.into());
        self
    }

    /// Set the row number (1-indexed, for tabular sources)
    pub fn with_row(mut self, row: usize) -> Self {
        self.row = Some(row);
        self
    }

    /// Set the column identifier (Excel letter, or field name)
    pub fn with_column(mut self, column: impl Into<String>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// Set the field name in the schema
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Set the byte offset in the source file
    pub fn with_byte_offset(mut self, offset: usize) -> Self {
        self.byte_offset = Some(offset);
        self
    }

    /// Set the line and column numbers (for text formats)
    pub fn with_line_col(mut self, line: usize, col: usize) -> Self {
        self.line = Some(line);
        self.col = Some(col);
        self
    }

    /// Human-readable display for diagnostics
    pub fn display(&self) -> String {
        let mut parts = vec![self.file.clone()];
        if let Some(sheet) = &self.sheet {
            parts.push(format!("Sheet: {sheet}"));
        }
        if let Some(row) = self.row {
            parts.push(format!("Row: {row}"));
        }
        if let Some(column) = &self.column {
            parts.push(format!("Col: {column}"));
        }
        if let Some(field) = &self.field {
            parts.push(format!("Field: {field}"));
        }
        if let Some(line) = self.line {
            parts.push(format!("Line: {line}"));
        }
        parts.join(" | ")
    }
}

impl Default for SourceLocation {
    fn default() -> Self {
        Self::new("<unknown>")
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// Canonical value types - the unified representation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Value {
    /// Null / absent value
    Null,
    /// Boolean value
    Bool(bool),
    /// Signed 64-bit integer
    Int(i64),
    /// Unsigned 64-bit integer
    UInt(u64),
    /// 64-bit floating point
    Float(f64),
    /// UTF-8 string
    String(String),
    /// Raw bytes (base64-encoded when serialized to JSON/CSV)
    Bytes(Vec<u8>),
    /// Ordered array of values
    Array(Vec<Value>),
    /// Insertion-ordered object / mapping
    Object(IndexMap<String, Value>),
}

impl Value {
    /// Get the type name for diagnostics
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::UInt(_) => "uint",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Bytes(_) => "bytes",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// Check if value is "empty" (null, empty string, empty array, empty object)
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            Value::Bytes(b) => b.is_empty(),
            _ => false,
        }
    }

    /// Coerce to a signed 64-bit integer (lossless only)
    pub fn coerce_to_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::UInt(u) => (*u).try_into().ok(),
            // 仅接受无损整数（100.0→100；100.5/越界→None）
            Value::Float(f)
                if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 =>
            {
                Some(*f as i64)
            }
            Value::Bool(b) => Some(i64::from(*b)),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Coerce to an unsigned 64-bit integer (lossless only)
    pub fn coerce_to_uint(&self) -> Option<u64> {
        match self {
            Value::UInt(u) => Some(*u),
            Value::Int(i) => (*i).try_into().ok(),
            // 仅接受无损非负整数（100.0→100；100.5/负数/越界→None）
            Value::Float(f) if f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64 => {
                Some(*f as u64)
            }
            Value::Bool(b) => Some(u64::from(*b)),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Coerce to a 64-bit floating point number
    pub fn coerce_to_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::UInt(u) => Some(*u as f64),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Coerce to a boolean (accepts true/yes/1/on/enabled, false/no/0/off/disabled)
    pub fn coerce_to_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Int(i) => Some(*i != 0),
            Value::UInt(u) => Some(*u != 0),
            Value::Float(f) => Some(*f != 0.0),
            Value::String(s) => {
                let s = s.trim().to_lowercase();
                match s.as_str() {
                    "true" | "yes" | "1" | "on" | "enabled" => Some(true),
                    "false" | "no" | "0" | "off" | "disabled" => Some(false),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Coerce to a string (scalar types only)
    pub fn coerce_to_string(&self) -> Option<String> {
        match self {
            Value::String(s) => Some(s.clone()),
            Value::Int(i) => Some(i.to_string()),
            Value::UInt(u) => Some(u.to_string()),
            Value::Float(f) => Some(f.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Bytes(b) => String::from_utf8(b.clone()).ok(),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::UInt(u) => write!(f, "{u}"),
            Value::Float(fl) => write!(f, "{fl}"),
            Value::String(s) => write!(f, "\"{s}\""),
            Value::Bytes(b) => write!(f, "bytes[{}]", b.len()),
            Value::Array(a) => {
                write!(f, "[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Object(o) => {
                write!(f, "{{")?;
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

/// A typed value with source location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedValue {
    /// The raw value
    pub value: Value,
    /// Where this value came from
    pub location: SourceLocation,
    /// Schema-inferred type (after validation)
    pub schema_type: Option<String>,
}

impl TypedValue {
    /// Create a typed value with its source location
    pub fn new(value: Value, location: SourceLocation) -> Self {
        Self {
            value,
            location,
            schema_type: None,
        }
    }

    /// Attach the schema-inferred type
    pub fn with_schema_type(mut self, schema_type: impl Into<String>) -> Self {
        self.schema_type = Some(schema_type.into());
        self
    }
}

/// A table row with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    /// Primary key value(s)
    pub primary_key: Vec<Value>,
    /// All field values
    pub fields: IndexMap<String, TypedValue>,
    /// Source location of this row
    pub location: SourceLocation,
    /// Row index (0-based within table)
    pub index: usize,
}

/// A table (collection of rows with same schema)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    /// Table name (unique within a document)
    pub name: String,
    /// Field names forming the primary key
    pub primary_key_fields: Vec<String>,
    /// All rows in source order
    pub rows: Vec<Row>,
    /// Source file this table came from
    pub source_file: String,
    /// Sheet name (for Excel)
    pub sheet: Option<String>,
}

/// Document - the top-level container for parsed sources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Tables by name, in insertion order
    pub tables: IndexMap<String, Table>,
    /// Source files contributing to this document
    pub source_files: Vec<String>,
    /// Document-level metadata
    pub metadata: DocumentMetadata,
}

/// Metadata about the document/source
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentMetadata {
    /// Original format (excel, csv, json, yaml)
    pub format: String,
    /// Parsing timestamp (for debugging, not for deterministic builds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed_at: Option<String>,
    /// Tool version that generated this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator_version: Option<String>,
    /// Custom metadata
    #[serde(flatten)]
    pub custom: IndexMap<String, Value>,
}

impl Document {
    /// Create an empty document
    pub fn new() -> Self {
        Self {
            tables: IndexMap::new(),
            source_files: Vec::new(),
            metadata: DocumentMetadata::default(),
        }
    }

    /// Add a table (registers its source file)
    pub fn add_table(&mut self, table: Table) {
        self.source_files.push(table.source_file.clone());
        self.tables.insert(table.name.clone(), table);
    }

    /// Look up a table by name
    pub fn get_table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }

    /// Look up a table by name, mutably
    pub fn get_table_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.get_mut(name)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_coercion() {
        let v = Value::String("42".to_string());
        assert_eq!(v.coerce_to_int(), Some(42));
        assert_eq!(v.coerce_to_uint(), Some(42));
        assert_eq!(v.coerce_to_float(), Some(42.0));

        let v = Value::String("true".to_string());
        assert_eq!(v.coerce_to_bool(), Some(true));

        let v = Value::Int(100);
        assert_eq!(v.coerce_to_string(), Some("100".to_string()));
    }

    #[test]
    fn test_source_location_display() {
        let loc = SourceLocation::new("items.xlsx")
            .with_sheet("Items")
            .with_row(27)
            .with_column("G")
            .with_field("DropItemID");
        let display = loc.display();
        assert!(display.contains("items.xlsx"));
        assert!(display.contains("Items"));
        assert!(display.contains("Row: 27"));
        assert!(display.contains("Col: G"));
        assert!(display.contains("Field: DropItemID"));
    }
}
