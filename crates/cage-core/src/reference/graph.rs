//! Dependency graph and incremental build planner: cross-table edges,
//! transitive closure, topological ordering and cycle detection, and the
//! affected-set / build-order computation (split out of mod.rs, LAY-024).

use crate::schema::Schema;
use std::collections::{HashMap, HashSet};

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

    /// Build the graph from all references in the schema. Every table is
    /// registered (isolated tables without reference edges included) so the
    /// topological build order always covers the whole schema.
    pub fn from_schema(schema: &Schema) -> Self {
        let mut graph = Self::new();

        for table_name in schema.tables.keys() {
            graph.forward.entry(table_name.clone()).or_default();
        }
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

    /// All forward edges (table -> its dependencies), table order arbitrary —
    /// callers that need deterministic order must sort.
    pub fn forward_edges(&self) -> Vec<(&String, &HashSet<String>)> {
        self.forward.iter().collect()
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

    /// Topological sort for build order. Deterministic: nodes are visited in
    /// sorted name order and dependencies in sorted order — required by the
    /// determinism contract (same schema → same build order bytes).
    pub fn topological_sort(&self) -> Result<Vec<String>, String> {
        let mut visited = HashSet::new();
        let mut temp = HashSet::new();
        let mut order = Vec::new();
        let mut all_nodes: Vec<&String> = self.forward.keys().chain(self.reverse.keys()).collect();
        all_nodes.sort();
        all_nodes.dedup();

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
            let mut deps: Vec<&String> = deps.iter().collect();
            deps.sort();
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
