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
//!
//! The [`diff`] submodule drafts a rule file from two schema versions
//! (design §46 deferred item): mechanically safe transforms become steps,
//! renames and enum remaps stay `# TODO` comments for the author.

pub mod diff;

use crate::schema::FieldType;
use crate::value::Value;
use indexmap::IndexMap;
use serde::Deserialize;

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
    /// [`yaml_to_value`] (mappings are decoded canonically first, so
    /// adjacently-tagged values work; see [`yaml_to_value`]).
    value: serde_yaml::Value,
}

/// Convert a generic YAML value into the canonical [`Value`] model.
/// Mappings are decoded canonically first: the adjacently-tagged form
/// (`{type: UInt, value: 5}`) is how the rule-draft renderer writes
/// defaults whose family a bare scalar cannot carry (plain `5` reads
/// back as `Int`). A mapping that does not decode as a tagged value —
/// or anything that is not a mapping — falls through to the plain
/// reading. YAML tags are rejected; plain YAML cannot express `Bytes`.
fn yaml_to_value(yaml: serde_yaml::Value) -> Result<Value, String> {
    if let serde_yaml::Value::Mapping(_) = yaml {
        if let Ok(tagged) = serde_yaml::from_value::<Value>(yaml.clone()) {
            return Ok(tagged);
        }
    }
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

pub use apply::{apply, reverify, MigrateReport, StepReport};
pub use parse::{parse_migration_dir, parse_spec, validate_spec};

mod apply;
mod parse;
#[cfg(test)]
mod tests;
