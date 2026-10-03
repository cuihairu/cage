//! YAML Source Adapter - parses YAML files into Cage's Canonical Model

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: the Value/normalize layer is a numeric coercion engine —
// float<->int casts are its job, not a defect.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::approx_constant,
    clippy::checked_conversions
)]
// Stage: crate-prefixed type names (JsonTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: Diagnostics is the deliberate first-class error type,
// returned by value from every pipeline stage.
#![allow(clippy::result_large_err)]
// Restriction lints kept off for a 0.1.0 codebase.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names
)]
use cage_core::{
    diagnostics::{Diagnostic, Diagnostics},
    error::codes::{internal, parse},
    value::{Document, Row, SourceLocation, Table, TypedValue, Value},
};
use indexmap::IndexMap;
use std::path::Path;

/// YAML Source Adapter
pub struct YamlSourceAdapter;

impl YamlSourceAdapter {
    /// Parse a YAML file into a Document
    pub fn parse_file(path: &Path) -> Result<Document, Diagnostics> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            let mut diags = Diagnostics::new();
            diags.add(
                Diagnostic::error(internal::E9902, format!("Failed to read file: {e}"))
                    .with_source(path.display().to_string()),
            );
            diags
        })?;

        Self::parse_str(&content, path)
    }

    /// Parse YAML string into a Document
    pub fn parse_str(content: &str, path: &Path) -> Result<Document, Diagnostics> {
        let mut diags = Diagnostics::new();
        let file_path = path.display().to_string();

        // Parse YAML with position tracking
        let mut value: serde_yaml::Value = match serde_yaml::from_str(content) {
            Ok(v) => v,
            Err(e) => {
                let loc = if let Some(location) = e.location() {
                    SourceLocation::new(&file_path)
                        .with_line_col(location.line(), location.column())
                } else {
                    SourceLocation::new(&file_path)
                };

                diags.add(Diagnostic::error(parse::E0001, format!("YAML syntax error: {e}"))
                    .with_location(loc)
                    .with_hint("Check YAML syntax - common issues: indentation, missing colons, unquoted special characters"));
                return Err(diags);
            }
        };

        // Expand YAML merge keys (`<<: *anchor`) — serde_yaml does not do this
        // automatically when deserializing into Value
        if let Err(e) = value.apply_merge() {
            diags.add(
                Diagnostic::error(parse::E0001, format!("YAML merge key error: {e}"))
                    .with_source(file_path.clone())
                    .with_hint(
                        "The value of a `<<` merge key must be a mapping or a sequence of mappings",
                    ),
            );
            return Err(diags);
        }

        let mut doc = Document::new();
        doc.metadata.format = "yaml".to_string();
        doc.source_files.push(file_path.clone());

        match Self::yaml_to_document(value, &file_path, &mut diags) {
            Ok(tables) => {
                for table in tables {
                    doc.add_table(table);
                }
                if diags.has_errors() {
                    Err(diags)
                } else {
                    Ok(doc)
                }
            }
            Err(e) => {
                diags.add(e);
                Err(diags)
            }
        }
    }

    fn yaml_to_document(
        value: serde_yaml::Value,
        file_path: &str,
        diags: &mut Diagnostics,
    ) -> Result<Vec<Table>, Diagnostic> {
        let mut tables = Vec::new();

        match value {
            serde_yaml::Value::Mapping(map) => {
                // Multi-table format: at least one top-level value is a
                // sequence of mappings (table rows). A bare list of scalars
                // (e.g. a `tags:` field) does not signal multi-table.
                let has_table_values = map.values().any(|v| {
                    matches!(
                        v,
                        serde_yaml::Value::Sequence(seq) if matches!(seq.first(), Some(serde_yaml::Value::Mapping(_)))
                    )
                });

                if has_table_values {
                    // Multi-table format: each top-level key is a table.
                    // Sequences become row tables; mappings (e.g. anchor
                    // definitions like `defaults: &defaults {...}`) become
                    // single-row tables; nulls are skipped.
                    for (table_name, table_value) in map {
                        let table_name_str = Self::yaml_value_to_string(&table_name);
                        match table_value {
                            serde_yaml::Value::Sequence(rows) => {
                                if let Some(table) = Self::sequence_to_table(
                                    &table_name_str,
                                    rows,
                                    file_path,
                                    0,
                                    diags,
                                ) {
                                    tables.push(table);
                                }
                            }
                            serde_yaml::Value::Mapping(single) => {
                                tables.push(Self::mapping_to_table(
                                    &table_name_str,
                                    &single,
                                    file_path,
                                    0,
                                    diags,
                                ));
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Single mapping - treat as single-row table
                    tables.push(Self::mapping_to_table("Root", &map, file_path, 0, diags));
                }
            }
            serde_yaml::Value::Sequence(rows) => {
                // Top-level sequence - single table
                if let Some(table) = Self::sequence_to_table("Data", rows, file_path, 0, diags) {
                    tables.push(table);
                }
            }
            _ => {
                return Err(Diagnostic::error(
                    parse::E0004,
                    "YAML root must be a mapping or sequence",
                )
                .with_source(file_path));
            }
        }

        Ok(tables)
    }

    fn sequence_to_table(
        name: &str,
        rows: Vec<serde_yaml::Value>,
        file_path: &str,
        sheet_index: usize,
        diags: &mut Diagnostics,
    ) -> Option<Table> {
        if rows.is_empty() {
            return None;
        }

        let mut table = Table {
            name: name.to_string(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.to_string(),
            sheet: if sheet_index > 0 {
                Some(format!("Sheet{sheet_index}"))
            } else {
                None
            },
        };

        for (row_idx, row_value) in rows.into_iter().enumerate() {
            match row_value {
                serde_yaml::Value::Mapping(obj) => {
                    let row = Self::mapping_to_row(name, &obj, file_path, row_idx, diags);
                    table.rows.push(row);
                }
                _ => {
                    diags.add(
                        Diagnostic::warning(
                            parse::E0001,
                            format!("Row {row_idx} is not a mapping, skipping"),
                        )
                        .with_source(file_path)
                        .with_table(name)
                        .with_row(row_idx.to_string()),
                    );
                }
            }
        }

        if table.rows.is_empty() {
            None
        } else {
            Some(table)
        }
    }

    fn mapping_to_table(
        name: &str,
        map: &serde_yaml::Mapping,
        file_path: &str,
        sheet_index: usize,
        diags: &mut Diagnostics,
    ) -> Table {
        let mut table = Table {
            name: name.to_string(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.to_string(),
            sheet: if sheet_index > 0 {
                Some(format!("Sheet{sheet_index}"))
            } else {
                None
            },
        };

        let row = Self::mapping_to_row(name, map, file_path, 0, diags);
        table.rows.push(row);
        table
    }

    fn mapping_to_row(
        _table_name: &str,
        map: &serde_yaml::Mapping,
        file_path: &str,
        row_idx: usize,
        _diags: &mut Diagnostics,
    ) -> Row {
        let mut fields = IndexMap::new();
        let mut primary_key = Vec::new();

        // Sort keys for deterministic ordering
        let mut keys: Vec<_> = map.keys().collect();
        keys.sort_by(|a, b| Self::yaml_value_to_string(a).cmp(&Self::yaml_value_to_string(b)));

        for key in keys {
            let value = map[key].clone();
            let cage_value = Self::yaml_value_to_cage(value);
            let key_str = Self::yaml_value_to_string(key);
            let loc = SourceLocation::new(file_path)
                .with_row(row_idx + 1)
                .with_column(key_str.clone())
                .with_field(key_str.clone());

            fields.insert(key_str.clone(), TypedValue::new(cage_value, loc));

            // Heuristic: first field named "id" or "ID" is primary key
            if primary_key.is_empty()
                && (key_str.eq_ignore_ascii_case("id") || key_str.eq_ignore_ascii_case("ID"))
            {
                primary_key.push(fields[&key_str].value.clone());
            }
        }

        // If no explicit ID field, use first field as primary key
        if primary_key.is_empty() {
            if let Some((_first_key, first_value)) = fields.iter().next() {
                primary_key.push(first_value.value.clone());
            }
        }

        Row {
            primary_key,
            fields,
            location: SourceLocation::new(file_path).with_row(row_idx + 1),
            index: row_idx,
        }
    }

    fn yaml_value_to_cage(value: serde_yaml::Value) -> Value {
        match value {
            serde_yaml::Value::Null => Value::Null,
            serde_yaml::Value::Bool(b) => Value::Bool(b),
            serde_yaml::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Value::UInt(u)
                } else if let Some(f) = n.as_f64() {
                    Value::Float(f)
                } else {
                    Value::String(n.to_string())
                }
            }
            serde_yaml::Value::String(s) => Value::String(s),
            serde_yaml::Value::Sequence(seq) => {
                Value::Array(seq.into_iter().map(Self::yaml_value_to_cage).collect())
            }
            serde_yaml::Value::Mapping(map) => {
                let mut obj = IndexMap::new();
                for (k, v) in map {
                    obj.insert(Self::yaml_value_to_string(&k), Self::yaml_value_to_cage(v));
                }
                Value::Object(obj)
            }
            serde_yaml::Value::Tagged(tagged) => {
                // Handle tagged values by extracting the value
                Self::yaml_value_to_cage(tagged.value.clone())
            }
        }
    }

    fn yaml_value_to_string(value: &serde_yaml::Value) -> String {
        match value {
            serde_yaml::Value::Null => "null".to_string(),
            serde_yaml::Value::Bool(b) => b.to_string(),
            serde_yaml::Value::Number(n) => n.to_string(),
            serde_yaml::Value::String(s) => s.clone(),
            serde_yaml::Value::Sequence(_) => "[sequence]".to_string(),
            serde_yaml::Value::Mapping(_) => "{mapping}".to_string(),
            serde_yaml::Value::Tagged(tagged) => Self::yaml_value_to_string(&tagged.value),
        }
    }
}

/// Trait for source adapters (for future plugin system)
pub trait SourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics>;
    fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics>;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn format_name(&self) -> &'static str;
}

impl SourceAdapter for YamlSourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        Self::parse_file(path)
    }

    fn parse_str(&self, content: &str, path: &Path) -> Result<Document, Diagnostics> {
        Self::parse_str(content, path)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["yaml", "yml"]
    }

    fn format_name(&self) -> &'static str {
        "YAML"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::value::Value;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_yaml_sequence() {
        let yaml = r"
- id: 1
  name: Item 1
  price: 100
- id: 2
  name: Item 2
  price: 200
";

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let doc = YamlSourceAdapter::parse_file(file.path()).unwrap();
        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("Data").unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].fields["id"].value, Value::Int(1));
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("Item 1".to_string())
        );
    }

    #[test]
    fn test_parse_yaml_multi_table() {
        let yaml = r"
Item:
  - id: 1
    name: Sword
  - id: 2
    name: Shield
Monster:
  - id: 100
    name: Goblin
    hp: 50
";

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let doc = YamlSourceAdapter::parse_file(file.path()).unwrap();
        assert_eq!(doc.tables.len(), 2);
        assert!(doc.tables.contains_key("Item"));
        assert!(doc.tables.contains_key("Monster"));
    }

    #[test]
    fn test_parse_yaml_single_mapping() {
        let yaml = r"
id: 42
name: Single Item
tags:
  - weapon
  - rare
";

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let doc = YamlSourceAdapter::parse_file(file.path()).unwrap();
        assert_eq!(doc.tables.len(), 1);
        let table = doc.tables.get("Root").unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(
            table.rows[0].fields["tags"].value,
            Value::Array(vec![
                Value::String("weapon".to_string()),
                Value::String("rare".to_string()),
            ])
        );
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let yaml = r"
id: 1
  name: Bad indent
";

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let result = YamlSourceAdapter::parse_file(file.path());
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(diags.has_errors());
        assert_eq!(diags.errors()[0].code, parse::E0001);
    }

    #[test]
    fn test_yaml_anchors_and_aliases() {
        let yaml = r"
defaults: &defaults
  hp: 100
  mp: 50

monsters:
  - name: Goblin
    <<: *defaults
  - name: Orc
    <<: *defaults
    hp: 200
";

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(yaml.as_bytes()).unwrap();

        let doc = YamlSourceAdapter::parse_file(file.path()).unwrap();
        // Should have "defaults" table and "monsters" table
        assert!(doc.tables.contains_key("monsters"));
        let monsters = doc.tables.get("monsters").unwrap();
        assert_eq!(monsters.rows.len(), 2);
        // Anchor expansion handled by serde_yaml
        assert_eq!(monsters.rows[0].fields["hp"].value, Value::Int(100));
        assert_eq!(monsters.rows[1].fields["hp"].value, Value::Int(200));
    }

    #[test]
    fn test_source_adapter_trait() {
        let adapter = YamlSourceAdapter;
        assert_eq!(adapter.supported_extensions(), &["yaml", "yml"]);
        assert_eq!(adapter.format_name(), "YAML");
    }
}
