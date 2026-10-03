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

/// Dependency graph for build ordering and incremental builds
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// table -> set of tables it depends on (references)
    forward: HashMap<String, HashSet<String>>,
    /// table -> set of tables that depend on it
    reverse: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    /// Create an empty graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the graph from all references in the schema
    pub fn from_schema(schema: &Schema) -> Self {
        let mut graph = Self::new();

        for (table_name, table_schema) in &schema.tables {
            for field_schema in table_schema.fields.values() {
                if let Some(ref_schema) = &field_schema.reference {
                    graph.add_edge(table_name, &ref_schema.table);
                }
            }
        }
        graph
    }

    /// Record that `from` references `to`
    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.forward
            .entry(from.to_string())
            .or_default()
            .insert(to.to_string());
        self.reverse
            .entry(to.to_string())
            .or_default()
            .insert(from.to_string());
    }

    /// Get direct dependencies (tables this table references)
    pub fn dependencies(&self, table: &str) -> Vec<&String> {
        self.forward
            .get(table)
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    /// Get direct dependents (tables that reference this table)
    pub fn dependents(&self, table: &str) -> Vec<&String> {
        self.reverse
            .get(table)
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    /// Get all transitive dependencies (for build order)
    pub fn all_dependencies(&self, table: &str) -> HashSet<String> {
        let mut visited = HashSet::new();
        let mut stack = vec![table.to_string()];

        while let Some(current) = stack.pop() {
            if visited.insert(current.clone()) {
                if let Some(deps) = self.forward.get(&current) {
                    for dep in deps {
                        stack.push(dep.clone());
                    }
                }
            }
        }
        visited.remove(table); // Don't include self
        visited
    }

    /// Get all transitive dependents (for incremental build impact)
    pub fn all_dependents(&self, table: &str) -> HashSet<String> {
        let mut visited = HashSet::new();
        let mut stack = vec![table.to_string()];

        while let Some(current) = stack.pop() {
            if visited.insert(current.clone()) {
                if let Some(deps) = self.reverse.get(&current) {
                    for dep in deps {
                        stack.push(dep.clone());
                    }
                }
            }
        }
        visited.remove(table);
        visited
    }

    /// Topological sort for build order
    pub fn topological_sort(&self) -> Result<Vec<String>, String> {
        let mut visited = HashSet::new();
        let mut temp = HashSet::new();
        let mut order = Vec::new();
        let all_nodes: HashSet<_> = self.forward.keys().chain(self.reverse.keys()).collect();

        for node in all_nodes {
            if !visited.contains(node) {
                Self::visit_topo(self, node, &mut visited, &mut temp, &mut order)?;
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
        if let Some(deps) = graph.forward.get(node) {
            for dep in deps {
                Self::visit_topo(graph, dep, visited, temp, order)?;
            }
        }
        temp.remove(node);
        visited.insert(node.to_string());
        order.push(node.to_string());
        Ok(())
    }

    /// Check for cycles
    pub fn has_cycles(&self) -> bool {
        self.topological_sort().is_err()
    }

    /// Find all cycles
    pub fn find_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        let all_nodes: HashSet<_> = self.forward.keys().chain(self.reverse.keys()).collect();

        for node in all_nodes {
            if !visited.contains(node) {
                Self::dfs_cycles(self, node, &mut visited, &mut path, &mut cycles);
            }
        }
        cycles
    }

    fn dfs_cycles(
        graph: &DependencyGraph,
        node: &str,
        visited: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        if path.contains(&node.to_string()) {
            // Found cycle
            let idx = path.iter().position(|n| n == node).unwrap();
            cycles.push(path[idx..].to_vec());
            return;
        }
        if visited.contains(node) {
            return;
        }
        visited.insert(node.to_string());
        path.push(node.to_string());

        if let Some(deps) = graph.forward.get(node) {
            for dep in deps {
                Self::dfs_cycles(graph, dep, visited, path, cycles);
            }
        }

        path.pop();
    }
}

/// Incremental build planner
pub struct IncrementalPlanner<'a> {
    graph: &'a DependencyGraph,
}

impl<'a> IncrementalPlanner<'a> {
    /// Create a planner over a dependency graph
    pub fn new(graph: &'a DependencyGraph) -> Self {
        Self { graph }
    }

    /// Given a set of changed tables, compute which tables need rebuilding
    pub fn compute_affected(&self, changed_tables: &[String]) -> HashSet<String> {
        let mut affected = HashSet::new();

        for table in changed_tables {
            affected.insert(table.clone());
            // All dependents (transitive) need rebuild
            for dep in self.graph.all_dependents(table) {
                affected.insert(dep);
            }
        }

        affected
    }

    /// Get build order for affected tables
    pub fn build_order(&self, affected: &HashSet<String>) -> Result<Vec<String>, String> {
        let full_order = self.graph.topological_sort()?;
        Ok(full_order
            .into_iter()
            .filter(|t| affected.contains(t))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldSchema, FieldType, ReferenceSchema, Schema, TableSchema};
    use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
    use indexmap::IndexMap;

    fn make_ref_schema() -> Schema {
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
        schema
    }

    fn make_ref_doc() -> Document {
        let mut doc = Document::new();
        let mut item_table = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "items.json".to_string(),
            sheet: None,
        };
        item_table.rows.push(Row {
            primary_key: vec![Value::UInt(1)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(
                        Value::UInt(1),
                        SourceLocation::new("items.json").with_row(1),
                    ),
                );
                f
            },
            location: SourceLocation::new("items.json").with_row(1),
            index: 0,
        });
        item_table.rows.push(Row {
            primary_key: vec![Value::UInt(2)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(
                        Value::UInt(2),
                        SourceLocation::new("items.json").with_row(2),
                    ),
                );
                f
            },
            location: SourceLocation::new("items.json").with_row(2),
            index: 1,
        });
        doc.add_table(item_table);

        let mut monster_table = Table {
            name: "Monster".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "monsters.json".to_string(),
            sheet: None,
        };
        monster_table.rows.push(Row {
            primary_key: vec![Value::UInt(100)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(
                        Value::UInt(100),
                        SourceLocation::new("monsters.json").with_row(1),
                    ),
                );
                f.insert(
                    "drop_item_id".to_string(),
                    TypedValue::new(
                        Value::UInt(1),
                        SourceLocation::new("monsters.json")
                            .with_row(1)
                            .with_field("drop_item_id"),
                    ),
                );
                f
            },
            location: SourceLocation::new("monsters.json").with_row(1),
            index: 0,
        });
        monster_table.rows.push(Row {
            primary_key: vec![Value::UInt(101)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(
                        Value::UInt(101),
                        SourceLocation::new("monsters.json").with_row(2),
                    ),
                );
                f.insert(
                    "drop_item_id".to_string(),
                    TypedValue::new(
                        Value::UInt(999),
                        SourceLocation::new("monsters.json")
                            .with_row(2)
                            .with_field("drop_item_id"),
                    ),
                ); // invalid ref
                f
            },
            location: SourceLocation::new("monsters.json").with_row(2),
            index: 1,
        });
        doc.add_table(monster_table);
        doc
    }

    #[test]
    fn test_reference_resolution() {
        let schema = make_ref_schema();
        let doc = make_ref_doc();
        let mut resolver = ReferenceResolver::new(&schema, &doc);
        let resolved = resolver.resolve_all();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].source_table, "Monster");
        assert_eq!(resolved[0].target_table, "Item");
        assert_eq!(resolved[0].target_row, 0);

        let unresolved = resolver.get_unresolved();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].searched_value, "999");
    }

    #[test]
    fn test_dependency_graph() {
        let schema = make_ref_schema();
        let graph = DependencyGraph::from_schema(&schema);

        let deps = graph.dependencies("Monster");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "Item");

        let dependents = graph.dependents("Item");
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0], "Monster");

        let order = graph.topological_sort().unwrap();
        assert_eq!(order[0], "Item");
        assert_eq!(order[1], "Monster");
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("A", "B");
        graph.add_edge("B", "C");
        graph.add_edge("C", "A");

        assert!(graph.has_cycles());
        let cycles = graph.find_cycles();
        assert_ne!(cycles, [] as [std::vec::Vec<std::string::String>; 0]);
    }

    #[test]
    fn test_incremental_planner() {
        let schema = make_ref_schema();
        let graph = DependencyGraph::from_schema(&schema);
        let planner = IncrementalPlanner::new(&graph);

        // Change Item -> Monster affected
        let affected = planner.compute_affected(&["Item".to_string()]);
        assert!(affected.contains("Item"));
        assert!(affected.contains("Monster"));

        // Change Monster -> only Monster affected
        let affected = planner.compute_affected(&["Monster".to_string()]);
        assert!(affected.contains("Monster"));
        assert!(!affected.contains("Item"));
    }
}
