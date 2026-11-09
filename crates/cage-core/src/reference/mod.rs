//! Reference resolution and cross-table dependency tracking

use crate::schema::{ReferenceSchema, Schema};
use crate::value::{Document, Row, SourceLocation, Value};
use std::collections::{HashMap, HashSet};

/// Resolved reference information
#[derive(Debug, Clone)]
pub struct ResolvedReference {
    /// Table containing the reference
    pub source_table: String,
    /// Field containing the reference
    pub source_field: String,
    /// Row index containing the reference
    pub source_row: usize,
    /// Referenced table
    pub target_table: String,
    /// Referenced field
    pub target_field: String,
    /// Row index of the referenced row
    pub target_row: usize,
    /// Value of the referenced field
    pub target_value: Value,
}

/// Reference resolver for L5 validation
pub struct ReferenceResolver<'a> {
    schema: &'a Schema,
    document: &'a Document,
    /// Cache: table -> field -> `value_string` -> `row_index`
    index: HashMap<String, HashMap<String, HashMap<String, usize>>>,
    /// Resolved references
    resolved: Vec<ResolvedReference>,
    /// Unresolved references (for error reporting)
    unresolved: Vec<UnresolvedReference>,
}

/// A reference that could not be resolved
#[derive(Debug, Clone)]
pub struct UnresolvedReference {
    /// Table containing the reference
    pub source_table: String,
    /// Field containing the reference
    pub source_field: String,
    /// Row index containing the reference
    pub source_row: usize,
    /// Where the reference came from
    pub source_location: SourceLocation,
    /// Referenced table
    pub target_table: String,
    /// Referenced field
    pub target_field: String,
    /// The value that was looked up
    pub searched_value: String,
}

impl<'a> ReferenceResolver<'a> {
    /// Create a resolver and build the primary-key lookup index
    pub fn new(schema: &'a Schema, document: &'a Document) -> Self {
        let mut resolver = Self {
            schema,
            document,
            index: HashMap::new(),
            resolved: Vec::new(),
            unresolved: Vec::new(),
        };
        resolver.build_index();
        resolver
    }

    fn build_index(&mut self) {
        for (table_name, table) in &self.document.tables {
            if let Some(table_schema) = self.schema.tables.get(table_name) {
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
                    self.index
                        .entry(table_name.clone())
                        .or_default()
                        .extend(field_map);
                }
            }
        }
    }

    /// Resolve all references in the document
    pub fn resolve_all(&mut self) -> Vec<ResolvedReference> {
        self.resolved.clear();
        self.unresolved.clear();

        for (table_name, table) in &self.document.tables {
            let Some(table_schema) = self.schema.tables.get(table_name) else {
                continue;
            };

            for row in &table.rows {
                for (field_name, typed_value) in &row.fields {
                    if let Some(ref_schema) = table_schema
                        .fields
                        .get(field_name)
                        .and_then(|f| f.reference.as_ref())
                    {
                        self.resolve_reference(
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

        self.resolved.clone()
    }

    fn resolve_reference(
        &mut self,
        source_table: &str,
        row: &Row,
        field_name: &str,
        typed_value: &crate::value::TypedValue,
        ref_schema: &ReferenceSchema,
    ) {
        let value_str = typed_value.value.coerce_to_string().unwrap_or_default();

        // Look up in index
        if let Some(field_map) = self.index.get(&ref_schema.table) {
            if let Some(value_map) = field_map.get(&ref_schema.field) {
                if let Some(&target_row_idx) = value_map.get(&value_str) {
                    // Found!
                    let target_table = &self.document.tables[&ref_schema.table];
                    let target_row = &target_table.rows[target_row_idx];
                    let target_value = target_row
                        .fields
                        .get(&ref_schema.field)
                        .map_or(Value::Null, |v| v.value.clone());

                    self.resolved.push(ResolvedReference {
                        source_table: source_table.to_string(),
                        source_field: field_name.to_string(),
                        source_row: row.index,
                        target_table: ref_schema.table.clone(),
                        target_field: ref_schema.field.clone(),
                        target_row: target_row_idx,
                        target_value,
                    });
                    return;
                }
            }
        }

        // Not found
        self.unresolved.push(UnresolvedReference {
            source_table: source_table.to_string(),
            source_field: field_name.to_string(),
            source_row: row.index,
            source_location: typed_value.location.clone(),
            target_table: ref_schema.table.clone(),
            target_field: ref_schema.field.clone(),
            searched_value: value_str,
        });
    }

    /// All successfully resolved references
    pub fn get_resolved(&self) -> &[ResolvedReference] {
        &self.resolved
    }

    /// All references that failed to resolve
    pub fn get_unresolved(&self) -> &[UnresolvedReference] {
        &self.unresolved
    }

    /// Get all rows in `target_table` that reference the given source row
    pub fn get_referencing_rows(
        &self,
        target_table: &str,
        target_field: &str,
        target_value: &str,
    ) -> Vec<(String, String, usize)> {
        let mut results = Vec::new();

        for (source_table, table) in &self.document.tables {
            if let Some(table_schema) = self.schema.tables.get(source_table) {
                for (field_name, field_schema) in &table_schema.fields {
                    if let Some(ref_schema) = &field_schema.reference {
                        if ref_schema.table == target_table && ref_schema.field == target_field {
                            // This field references our target - check rows
                            for row in &table.rows {
                                if let Some(typed_value) = row.fields.get(field_name) {
                                    if typed_value.value.coerce_to_string().unwrap_or_default()
                                        == target_value
                                    {
                                        results.push((
                                            source_table.clone(),
                                            field_name.clone(),
                                            row.index,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        results
    }

    /// Build reverse dependency map: `target_table` -> set of (`source_table`, `source_field`)
    pub fn build_reverse_deps(&self) -> HashMap<String, HashSet<(String, String)>> {
        let mut reverse: HashMap<String, HashSet<(String, String)>> = HashMap::new();

        for (source_table, table_schema) in &self.schema.tables {
            for (field_name, field_schema) in &table_schema.fields {
                if let Some(ref_schema) = &field_schema.reference {
                    reverse
                        .entry(ref_schema.table.clone())
                        .or_default()
                        .insert((source_table.clone(), field_name.clone()));
                }
            }
        }
        reverse
    }
}

mod graph;
#[cfg(test)]
mod tests;

pub use graph::{DependencyGraph, IncrementalPlanner};
