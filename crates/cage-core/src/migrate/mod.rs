//! Declarative data migration under schema evolution (design §46, M
//! series). Migration rules live in a project's `migrations/` directory —
//! the file-name order is the version step chain (`0001-…`, `0002-…`) —
//! and each segment explicitly declares its transforms against the
//! Canonical Model. Rules are data: they go into version control, the
//! same rules over the same data produce the same document, and nothing
//! is inferred from a schema diff.
//!
//! The module splits in two:
//! - [`parse_spec`] / [`parse_migration_dir`] read rule files into
//!   [`MigrationSpec`] values (E2001 for anything structurally wrong),
//! - [`validate_spec`] checks every step's references against the
//!   from-schema (E2002 — a rule that names a table or field the schema
//!   does not have is a pointing error, caught before any data moves).
//!
//! Execution ([`apply`]-style transforms, E2003) and post-migration
//! reverification (E2004) land with M2.

use crate::error::codes::migration::{E2001, E2002};
use crate::schema::{FieldType, Schema};
use crate::value::Value;
use indexmap::IndexMap;
use serde::Deserialize;
use std::path::Path;

/// One migration segment: transforms taking a document from the `from`
/// schema version to the `to` schema version.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MigrationSpec {
    /// Schema version this segment starts from
    pub from: String,
    /// Schema version this segment produces
    pub to: String,
    /// Transforms applied in order — later steps see earlier steps' output
    pub steps: Vec<Step>,
}

/// One declarative transform. Wire format is a single-key mapping per
/// step (`- rename_field: { table: Item, from: name, to: title }`), so
/// an unknown transform name fails parsing as E2001 rather than being
/// silently ignored. Deserialization is hand-rolled: `serde_yaml` has no
/// externally-tagged enum support, so the single-key mapping is decoded
/// as a generic YAML mapping and dispatched by its one key.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Rename a field within a table
    RenameField {
        /// Table containing the field
        table: String,
        /// Current field name
        from: String,
        /// New field name
        to: String,
    },
    /// Give a field a default value (for rows where it is absent; also
    /// satisfies a newly-`required` field)
    SetDefault {
        /// Table containing the field
        table: String,
        /// Field receiving the default
        field: String,
        /// The default value, in canonical value form
        value: Value,
    },
    /// Drop a field from every row of a table
    RemoveField {
        /// Table containing the field
        table: String,
        /// Field to remove
        field: String,
    },
    /// Widen a field's type (e.g. Int32 → Int64). Narrowing is rejected
    /// at apply time (E2003) — parse accepts any well-formed type and
    /// the direction check happens where the data is.
    WidenType {
        /// Table containing the field
        table: String,
        /// Field to retype
        field: String,
        /// The new field type (same wire format as schema `type:`)
        to: FieldType,
    },
    /// Rewrite a field's string values through an explicit mapping
    /// (enum renames, value consolidation). Values not present in the
    /// mapping pass through unchanged.
    RemapValues {
        /// Table containing the field
        table: String,
        /// Field whose values are remapped
        field: String,
        /// `old → new` mapping
        map: IndexMap<String, String>,
    },
    /// Rename a table
    RenameTable {
        /// Current table name
        from: String,
        /// New table name
        to: String,
    },
}

// Per-variant payload shapes (serde derives only — used by the manual
// `Step` deserializer below via `serde_yaml::from_value`).
#[derive(serde::Deserialize)]
struct RenameFieldBody {
    table: String,
    from: String,
    to: String,
}
#[derive(serde::Deserialize)]
struct SetDefaultBody {
    table: String,
    field: String,
    /// Bare YAML scalar / array / mapping — converted via
    /// [`yaml_to_value`] (canonical `Value` is adjacently tagged, which
    /// `serde_yaml` cannot decode directly — see [`yaml_to_value`]).
    value: serde_yaml::Value,
}

/// Convert a generic YAML value into the canonical [`Value`] model.
/// YAML cannot express canonical `Bytes`; tagged YAML values are
/// rejected.
fn yaml_to_value(yaml: serde_yaml::Value) -> Result<Value, String> {
    Ok(match yaml {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else {
                Value::Float(
                    n.as_f64()
                        .ok_or_else(|| "unrepresentable number".to_string())?,
                )
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(yaml_to_value(item)?);
            }
            Value::Array(out)
        }
        serde_yaml::Value::Mapping(map) => {
            let mut out = IndexMap::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    other => return Err(format!("unsupported mapping key: {other:?}")),
                };
                out.insert(key, yaml_to_value(v)?);
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(tag) => {
            return Err(format!(
                "YAML tags are not supported in defaults: {}",
                tag.tag
            ));
        }
    })
}
#[derive(serde::Deserialize)]
struct RemoveFieldBody {
    table: String,
    field: String,
}
#[derive(serde::Deserialize)]
struct WidenTypeBody {
    table: String,
    field: String,
    to: FieldType,
}
#[derive(serde::Deserialize)]
struct RemapValuesBody {
    table: String,
    field: String,
    map: IndexMap<String, String>,
}
#[derive(serde::Deserialize)]
struct RenameTableBody {
    from: String,
    to: String,
}

/// Decode one step payload body, keeping the step kind in the error text.
fn payload<T: serde::de::DeserializeOwned>(kind: &str, b: serde_yaml::Value) -> Result<T, String> {
    serde_yaml::from_value(b).map_err(|e| format!("bad `{kind}` payload: {e}"))
}

impl<'de> serde::Deserialize<'de> for Step {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw: IndexMap<String, serde_yaml::Value> =
            serde::Deserialize::deserialize(deserializer)?;
        if raw.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "a step must be exactly one transform mapping, got {} keys",
                raw.len()
            )));
        }
        let (kind, body) = raw.into_iter().next().unwrap();
        match kind.as_str() {
            "rename_field" => {
                let b: RenameFieldBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RenameField {
                    table: b.table,
                    from: b.from,
                    to: b.to,
                })
            }
            "set_default" => {
                let b: SetDefaultBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                let value = yaml_to_value(b.value).map_err(serde::de::Error::custom)?;
                Ok(Step::SetDefault {
                    table: b.table,
                    field: b.field,
                    value,
                })
            }
            "remove_field" => {
                let b: RemoveFieldBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RemoveField {
                    table: b.table,
                    field: b.field,
                })
            }
            "widen_type" => {
                let b: WidenTypeBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::WidenType {
                    table: b.table,
                    field: b.field,
                    to: b.to,
                })
            }
            "remap_values" => {
                let b: RemapValuesBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RemapValues {
                    table: b.table,
                    field: b.field,
                    map: b.map,
                })
            }
            "rename_table" => {
                let b: RenameTableBody = payload(&kind, body).map_err(serde::de::Error::custom)?;
                Ok(Step::RenameTable {
                    from: b.from,
                    to: b.to,
                })
            }
            other => Err(serde::de::Error::custom(format!(
                "unknown step kind `{other}` (expected rename_field / set_default / \
                 remove_field / widen_type / remap_values / rename_table)"
            ))),
        }
    }
}

impl Step {
    /// Human-readable summary used in reports and E2002 messages, e.g.
    /// `rename_field(Item.name → title)`.
    pub fn describe(&self) -> String {
        match self {
            Step::RenameField { table, from, to } => {
                format!("rename_field({table}.{from} → {to})")
            }
            Step::SetDefault { table, field, .. } => {
                format!("set_default({table}.{field})")
            }
            Step::RemoveField { table, field } => format!("remove_field({table}.{field})"),
            Step::WidenType { table, field, to } => {
                format!("widen_type({table}.{field} → {to:?})")
            }
            Step::RemapValues { table, field, map } => {
                format!(
                    "remap_values({table}.{field}, {} entr{})",
                    map.len(),
                    if map.len() == 1 { "y" } else { "ies" }
                )
            }
            Step::RenameTable { from, to } => format!("rename_table({from} → {to})"),
        }
    }
}

/// Parse one migration rule file into a [`MigrationSpec`]. Any structural
/// problem — unreadable file, invalid YAML, missing `from`/`to`/`steps`,
/// an unknown step kind, a bad field type — is E2001. Reference validity
/// (does the schema actually have these tables and fields?) is a separate
/// [`validate_spec`] pass so parsing stays schema-independent.
pub fn parse_spec(path: &Path) -> Result<MigrationSpec, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "{E2001}: cannot read migration rule {}: {e}",
            path.display()
        )
    })?;
    let spec: MigrationSpec = serde_yaml::from_str(&text).map_err(|e| {
        format!(
            "{E2001}: migration rule {} is not a valid spec: {e}",
            path.display()
        )
    })?;
    validate_structure(&spec, &path.display().to_string())?;
    Ok(spec)
}

/// Structural self-checks beyond what serde enforces: non-empty version
/// strings and at least one step (a segment that transforms nothing is a
/// pointing error, not a no-op).
fn validate_structure(spec: &MigrationSpec, origin: &str) -> Result<(), String> {
    if spec.from.trim().is_empty() {
        return Err(format!(
            "{E2001}: migration rule {origin} has an empty `from` version"
        ));
    }
    if spec.to.trim().is_empty() {
        return Err(format!(
            "{E2001}: migration rule {origin} has an empty `to` version"
        ));
    }
    if spec.from == spec.to {
        return Err(format!(
            "{E2001}: migration rule {origin} moves `{}` to itself",
            spec.from
        ));
    }
    if spec.steps.is_empty() {
        return Err(format!(
            "{E2001}: migration rule {origin} ({}) declares no steps",
            spec.from
        ));
    }
    Ok(())
}

/// Check every step's references against the from-schema (E2002): the
/// named table and field must exist, a rename must not collide with an
/// existing name. `rename_table`'s target must also be free — rules run
/// in order against one evolving document, but names are resolved against
/// the from-schema, so a target colliding with any table the document
/// could carry is refused up front rather than discovered mid-chain.
pub fn validate_spec(spec: &MigrationSpec, schema: &Schema) -> Result<(), String> {
    for step in &spec.steps {
        validate_step(step, schema)?;
    }
    Ok(())
}

fn validate_step(step: &Step, schema: &Schema) -> Result<(), String> {
    let need_table = |schema: &Schema, table: &str| -> Result<(), String> {
        if schema.tables.contains_key(table) {
            Ok(())
        } else {
            Err(format!(
                "{E2002}: {step_desc} — schema has no table `{table}`",
                step_desc = step.describe()
            ))
        }
    };
    let need_field = |schema: &Schema, table: &str, field: &str| -> Result<(), String> {
        need_table(schema, table)?;
        let fields = &schema.tables[table].fields;
        if fields.contains_key(field) {
            Ok(())
        } else {
            Err(format!(
                "{E2002}: {} — table `{table}` has no field `{field}`",
                step.describe()
            ))
        }
    };
    match step {
        Step::RenameField { table, from, to } => {
            need_field(schema, table, from)?;
            if schema.tables[table].fields.contains_key(to) {
                return Err(format!(
                    "{E2002}: {} — table `{table}` already has a field `{to}`",
                    step.describe()
                ));
            }
            Ok(())
        }
        Step::SetDefault { table, field, .. }
        | Step::RemoveField { table, field }
        | Step::WidenType { table, field, .. }
        | Step::RemapValues { table, field, .. } => need_field(schema, table, field),
        Step::RenameTable { from, to } => {
            need_table(schema, from)?;
            if schema.tables.contains_key(to) {
                return Err(format!(
                    "{E2002}: {} — schema already has a table `{to}`",
                    step.describe()
                ));
            }
            Ok(())
        }
    }
}

/// Parse a `migrations/` directory into the version step chain: every
/// `*.yaml` / `*.yml` file, ordered by file name (the chain order), each
/// parsed as one segment (E2001 on any bad file). A missing directory is
/// E2001; an empty directory yields an empty chain — projects simply have
/// nothing to migrate.
pub fn parse_migration_dir(dir: &Path) -> Result<Vec<(String, MigrationSpec)>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        format!(
            "{E2001}: cannot read migrations directory {}: {e}",
            dir.display()
        )
    })?;
    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext == "yml")
        })
        .collect();
    // File-name byte order IS the step chain order — `0001-…` sorts
    // before `0002-…`, which is the whole naming convention.
    files.sort();
    let mut chain = Vec::with_capacity(files.len());
    for name in &files {
        let path = dir.join(name);
        let spec = parse_spec(&path)?;
        chain.push((name.clone(), spec));
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_with_table(name: &str, fields: &[&str]) -> Schema {
        let mut field_map = IndexMap::new();
        for field in fields {
            field_map.insert(
                (*field).to_string(),
                crate::schema::FieldSchema {
                    name: (*field).to_string(),
                    field_type: FieldType::String,
                    description: None,
                    required: false,
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
                    targets: Vec::new(),
                    rules: Vec::new(),
                    metadata: IndexMap::new(),
                },
            );
        }
        let mut tables = IndexMap::new();
        tables.insert(
            name.to_string(),
            crate::schema::TableSchema {
                name: name.to_string(),
                description: None,
                primary_key: vec![fields.first().unwrap_or(&"id").to_string()],
                fields: field_map,
                unique_constraints: Vec::new(),
                order_by: None,
                targets: Vec::new(),
            },
        );
        Schema {
            tables,
            enums: IndexMap::new(),
            metadata: None,
        }
    }

    const SPEC_YAML: &str = r#"
from: "1.0.0"
to: "1.1.0"
steps:
  - rename_field:
      table: Item
      from: name
      to: title
  - set_default:
      table: Item
      field: rarity
      value: "common"
  - remove_field:
      table: Item
      field: legacy_id
  - widen_type:
      table: Item
      field: level
      to: { kind: Int64 }
  - remap_values:
      table: Item
      field: grade
      map:
        "S": "legendary"
        "A": "epic"
  - rename_table:
      from: Mob
      to: Monster
"#;

    #[test]
    fn parse_spec_reads_all_six_step_kinds_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("0001-rename.yaml");
        std::fs::write(&file, SPEC_YAML).unwrap();
        let spec = parse_spec(&file).unwrap();
        assert_eq!(spec.from, "1.0.0");
        assert_eq!(spec.to, "1.1.0");
        assert_eq!(spec.steps.len(), 6);
        assert_eq!(
            spec.steps[0],
            Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "title".into()
            }
        );
        assert_eq!(
            spec.steps[1],
            Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: Value::String("common".into())
            }
        );
        assert_eq!(
            spec.steps[2],
            Step::RemoveField {
                table: "Item".into(),
                field: "legacy_id".into()
            }
        );
        assert_eq!(
            spec.steps[3],
            Step::WidenType {
                table: "Item".into(),
                field: "level".into(),
                to: FieldType::Int64
            }
        );
        assert_eq!(
            spec.steps[4],
            Step::RemapValues {
                table: "Item".into(),
                field: "grade".into(),
                map: IndexMap::from([
                    ("S".to_string(), "legendary".to_string()),
                    ("A".to_string(), "epic".to_string()),
                ])
            }
        );
        assert_eq!(
            spec.steps[5],
            Step::RenameTable {
                from: "Mob".into(),
                to: "Monster".into()
            }
        );
    }

    #[test]
    fn parse_migration_dir_orders_segments_by_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let later = r#"
from: "1.1.0"
to: "2.0.0"
steps:
  - remove_field:
      table: Item
      field: legacy_id
"#;
        std::fs::write(dir.path().join("0002-second.yaml"), later).unwrap();
        std::fs::write(dir.path().join("0001-first.yaml"), SPEC_YAML).unwrap();
        std::fs::write(dir.path().join("README.md"), "not a migration").unwrap();
        let chain = parse_migration_dir(dir.path()).unwrap();
        let names: Vec<&str> = chain.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["0001-first.yaml", "0002-second.yaml"]);
        assert_eq!(chain[0].1.from, "1.0.0");
        assert_eq!(chain[1].1.from, "1.1.0");
    }

    #[test]
    fn parse_spec_reports_structural_problems_as_e2001() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let out = parse_spec(&root.join("missing.yaml")).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let bad_yaml = root.join("bad.yaml");
        std::fs::write(&bad_yaml, "from: [unclosed").unwrap();
        let out = parse_spec(&bad_yaml).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let no_steps = root.join("no-steps.yaml");
        std::fs::write(&no_steps, "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps: []\n").unwrap();
        let out = parse_spec(&no_steps).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let unknown = root.join("unknown-step.yaml");
        std::fs::write(
            &unknown,
            "from: \"1.0.0\"\nto: \"1.1.0\"\nsteps:\n  - drop_database: { table: Item }\n",
        )
        .unwrap();
        let out = parse_spec(&unknown).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let no_to = root.join("no-to.yaml");
        std::fs::write(
            &no_to,
            "from: \"1.0.0\"\nsteps:\n  - remove_field: { table: Item, field: x }\n",
        )
        .unwrap();
        let out = parse_spec(&no_to).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");

        let self_loop = root.join("self-loop.yaml");
        std::fs::write(
            &self_loop,
            "from: \"1.0.0\"\nto: \"1.0.0\"\nsteps:\n  - remove_field: { table: Item, field: x }\n",
        )
        .unwrap();
        let out = parse_spec(&self_loop).unwrap_err();
        assert!(out.starts_with("E2001"), "{out}");
    }

    #[test]
    fn validate_spec_checks_table_and_field_references_as_e2002() {
        let schema = schema_with_table("Item", &["id", "name"]);

        // Table does not exist.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RemoveField {
                table: "Weapon".into(),
                field: "id".into(),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");
        assert!(out.contains("Weapon"), "{out}");

        // Field does not exist.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::SetDefault {
                table: "Item".into(),
                field: "rarity".into(),
                value: Value::String("common".into()),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");
        assert!(out.contains("rarity"), "{out}");

        // Rename collides with an existing field.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameField {
                table: "Item".into(),
                from: "name".into(),
                to: "id".into(),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");

        // rename_table to a free name is fine; to an existing one is not.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameTable {
                from: "Item".into(),
                to: "Weapon".into(),
            }],
        };
        assert!(validate_spec(&spec, &schema).is_ok());
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![Step::RenameTable {
                from: "Item".into(),
                to: "Item".into(),
            }],
        };
        let out = validate_spec(&spec, &schema).unwrap_err();
        assert!(out.starts_with("E2002"), "{out}");

        // A clean spec passes.
        let spec = MigrationSpec {
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            steps: vec![
                Step::RenameField {
                    table: "Item".into(),
                    from: "name".into(),
                    to: "title".into(),
                },
                Step::RenameTable {
                    from: "Item".into(),
                    to: "Artifact".into(),
                },
            ],
        };
        assert!(validate_spec(&spec, &schema).is_ok());
    }
}
