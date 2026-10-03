//! Diagnostics framework - first-class citizen for error reporting
//! Each diagnostic carries full context: code, severity, source location, message, hint

use crate::error::codes::error_title;
pub use crate::error::codes::Severity;
use crate::value::SourceLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A single diagnostic message (error, warning, info)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Error code (e.g., E0001, E1001, E1401)
    pub code: String,
    /// Severity level
    pub severity: Severity,
    /// Human-readable title (derived from code if not provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Source file path
    pub source: String,
    /// Detailed source location
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
    /// Table name (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Row identifier (primary key or index)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<String>,
    /// Column/field name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// Field name in schema
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The problematic value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Human-readable message
    pub message: String,
    /// Actionable hint for fixing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Related diagnostics (e.g., the referenced item location)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub related: Vec<RelatedDiagnostic>,
    /// Custom metadata
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub metadata: IndexMap<String, serde_json::Value>,
}

/// Related diagnostic for cross-references
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedDiagnostic {
    /// Error code of the related diagnostic
    pub code: String,
    /// Short description of the related issue
    pub message: String,
    /// Location of the related item
    pub location: SourceLocation,
}

/// Collection of diagnostics with rendering capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Create an empty collection
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a diagnostic
    pub fn add(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    /// Append an error diagnostic with a source location
    pub fn add_error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        location: SourceLocation,
    ) {
        let code = code.into();
        self.add(Diagnostic {
            code: code.clone(),
            severity: Severity::Error,
            title: error_title(&code).map(str::to_string),
            source: location.file.clone(),
            location: Some(location),
            table: None,
            row: None,
            column: None,
            field: None,
            value: None,
            message: message.into(),
            hint: None,
            related: Vec::new(),
            metadata: IndexMap::new(),
        });
    }

    /// Append a warning diagnostic with a source location
    pub fn add_warning(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        location: SourceLocation,
    ) {
        let code = code.into();
        self.add(Diagnostic {
            code: code.clone(),
            severity: Severity::Warning,
            title: error_title(&code).map(str::to_string),
            source: location.file.clone(),
            location: Some(location),
            table: None,
            row: None,
            column: None,
            field: None,
            value: None,
            message: message.into(),
            hint: None,
            related: Vec::new(),
            metadata: IndexMap::new(),
        });
    }

    /// Append an info diagnostic with a source location
    pub fn add_info(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        location: SourceLocation,
    ) {
        let code = code.into();
        self.add(Diagnostic {
            code: code.clone(),
            severity: Severity::Info,
            title: error_title(&code).map(str::to_string),
            source: location.file.clone(),
            location: Some(location),
            table: None,
            row: None,
            column: None,
            field: None,
            value: None,
            message: message.into(),
            hint: None,
            related: Vec::new(),
            metadata: IndexMap::new(),
        });
    }

    /// Merge another collection into this one
    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    /// Whether any diagnostic has error severity
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }

    /// Whether any diagnostic has warning severity
    pub fn has_warnings(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Warning)
    }

    /// `可变迭代（warnings_as_errors` 升级用）
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Diagnostic> {
        self.items.iter_mut()
    }

    /// All error-severity diagnostics
    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    /// All warning-severity diagnostics
    pub fn warnings(&self) -> Vec<&Diagnostic> {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect()
    }

    /// All info-severity diagnostics
    pub fn infos(&self) -> Vec<&Diagnostic> {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Info)
            .collect()
    }

    /// Total number of diagnostics
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the collection is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate over all diagnostics
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.items.iter()
    }

    /// Render diagnostics in human-readable format (for CLI)
    pub fn render(&self, color: bool) -> String {
        let mut out = String::new();
        for diag in &self.items {
            out.push_str(&diag.render(color));
            out.push('\n');
        }
        out
    }

    /// Render as JSON for IDE/CI consumption
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(&self.items)
    }

    /// Sort by source location for consistent output
    pub fn sort_by_location(&mut self) {
        self.items.sort_by(|a, b| {
            let a_loc = a.location.as_ref();
            let b_loc = b.location.as_ref();
            match (a_loc, b_loc) {
                (Some(a), Some(b)) => a
                    .file
                    .cmp(&b.file)
                    .then_with(|| a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
                    .then_with(|| a.col.unwrap_or(0).cmp(&b.col.unwrap_or(0))),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.code.cmp(&b.code),
            }
        });
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'a> IntoIterator for &'a mut Diagnostics {
    type Item = &'a mut Diagnostic;
    type IntoIter = std::slice::IterMut<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

impl Diagnostic {
    /// Create a new diagnostic with minimal fields
    pub fn new(code: impl Into<String>, severity: Severity, message: impl Into<String>) -> Self {
        let code = code.into();
        Self {
            code: code.clone(),
            severity,
            title: error_title(&code).map(str::to_string),
            source: String::new(),
            location: None,
            table: None,
            row: None,
            column: None,
            field: None,
            value: None,
            message: message.into(),
            hint: None,
            related: Vec::new(),
            metadata: IndexMap::new(),
        }
    }

    /// Create an error-severity diagnostic
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, message)
    }

    /// Create a warning-severity diagnostic
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, message)
    }

    /// Create an info-severity diagnostic
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Info, message)
    }

    /// Set the source file / component name
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Set the detailed source location (also updates `source`)
    pub fn with_location(mut self, location: SourceLocation) -> Self {
        self.source.clone_from(&location.file);
        self.location = Some(location);
        self
    }

    /// Set the table name
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Set the row identifier (primary key or index)
    pub fn with_row(mut self, row: impl Into<String>) -> Self {
        self.row = Some(row.into());
        self
    }

    /// Set the column identifier
    pub fn with_column(mut self, column: impl Into<String>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// Set the schema field name
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Attach the problematic value
    pub fn with_value(mut self, value: serde_json::Value) -> Self {
        self.value = Some(value);
        self
    }

    /// Set the actionable fix hint
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Attach a related diagnostic (e.g., the referenced item)
    pub fn with_related(mut self, related: RelatedDiagnostic) -> Self {
        self.related.push(related);
        self
    }

    /// Attach custom metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Render a single diagnostic
    pub fn render(&self, color: bool) -> String {
        use console::style;
        use std::fmt::Write;
        let mut out = String::new();

        let severity_str = self.severity.as_str();
        let (severity_display, code_display) = if color {
            let sev = match self.severity {
                Severity::Error => style(severity_str).red().bold(),
                Severity::Warning => style(severity_str).yellow().bold(),
                Severity::Info => style(severity_str).blue().bold(),
            };
            let code = style(&self.code).cyan().bold();
            (sev.to_string(), code.to_string())
        } else {
            (severity_str.to_string(), self.code.clone())
        };

        // Header: ERROR E0001
        let _ = write!(out, "{severity_display} {code_display}");
        if let Some(title) = &self.title {
            let _ = write!(out, " — {title}");
        }
        out.push('\n');

        // Source location
        if let Some(loc) = &self.location {
            let _ = writeln!(out, "  Source: {}", loc.display());
        } else if !self.source.is_empty() {
            let _ = writeln!(out, "  Source: {}", self.source);
        }

        // Table/Row/Field context
        if let Some(table) = &self.table {
            let _ = writeln!(out, "  Table: {table}");
        }
        if let Some(row) = &self.row {
            let _ = writeln!(out, "  Row: {row}");
        }
        if let Some(field) = &self.field {
            let _ = writeln!(out, "  Field: {field}");
        }

        // Value
        if let Some(value) = &self.value {
            let _ = writeln!(out, "  Value: {value}");
        }

        // Message
        let _ = writeln!(out, "  Message: {}", self.message);

        // Hint
        if let Some(hint) = &self.hint {
            if color {
                let _ = write!(out, "{}", style(format!("  Hint: {hint}")).dim());
            } else {
                let _ = write!(out, "  Hint: {hint}");
            }
            out.push('\n');
        }

        // Related
        for rel in &self.related {
            let _ = writeln!(out, "  Related: {} at {}", rel.code, rel.location.display());
        }

        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render(false))
    }
}

/// Builder for creating structured diagnostics with fluent API
pub struct DiagnosticBuilder {
    diagnostic: Diagnostic,
}

impl DiagnosticBuilder {
    /// Start building an error-severity diagnostic
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            diagnostic: Diagnostic::error(code, message),
        }
    }

    /// Start building a warning-severity diagnostic
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            diagnostic: Diagnostic::warning(code, message),
        }
    }

    /// Start building an info-severity diagnostic
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            diagnostic: Diagnostic::info(code, message),
        }
    }

    /// Set the source file / component name
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.diagnostic.source = source.into();
        self
    }

    /// Set the detailed source location
    pub fn location(mut self, location: SourceLocation) -> Self {
        self.diagnostic = self.diagnostic.with_location(location);
        self
    }

    /// Set the table name
    pub fn table(mut self, table: impl Into<String>) -> Self {
        self.diagnostic = self.diagnostic.with_table(table);
        self
    }

    /// Set the row identifier
    pub fn row(mut self, row: impl Into<String>) -> Self {
        self.diagnostic = self.diagnostic.with_row(row);
        self
    }

    /// Set the column identifier
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.diagnostic = self.diagnostic.with_field(field);
        self
    }

    /// Attach the problematic value (any serializable payload)
    pub fn value(mut self, value: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.diagnostic = self.diagnostic.with_value(v);
        }
        self
    }

    /// Set the actionable fix hint
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.diagnostic = self.diagnostic.with_hint(hint);
        self
    }

    /// Attach a related diagnostic (code, message, location)
    pub fn related(
        mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        location: SourceLocation,
    ) -> Self {
        self.diagnostic = self.diagnostic.with_related(RelatedDiagnostic {
            code: code.into(),
            message: message.into(),
            location,
        });
        self
    }

    /// Attach custom metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.diagnostic = self.diagnostic.with_metadata(key, v);
        }
        self
    }

    /// Finish building and return the diagnostic
    pub fn build(self) -> Diagnostic {
        self.diagnostic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::SourceLocation;

    #[test]
    fn test_diagnostics_collection() {
        let mut diags = Diagnostics::new();
        let loc = SourceLocation::new("test.xlsx")
            .with_sheet("Sheet1")
            .with_row(5);
        diags.add_error("E1001", "Missing required field", loc.clone());
        diags.add_warning("E1201", "Value near max", loc);

        assert_eq!(diags.len(), 2);
        assert!(diags.has_errors());
        assert!(diags.has_warnings());
        assert_eq!(diags.errors().len(), 1);
        assert_eq!(diags.warnings().len(), 1);
    }

    #[test]
    fn test_diagnostic_builder() {
        let diag = DiagnosticBuilder::error("E1401", "Reference not found")
            .source("monster.xlsx")
            .table("Monster")
            .row("20003")
            .field("DropItemID")
            .value(99999)
            .hint("Add Item[99999] or change DropItemID")
            .build();

        assert_eq!(diag.code, "E1401");
        assert_eq!(diag.severity, Severity::Error);
        assert!(diag.hint.is_some());
    }
}
