//! Schema 编辑器交换模型（第三阶段 Web UI / Schema Editor 的 core 侧契约）.
//!
//! The editor never parses or renders YAML itself: it exchanges the
//! canonical JSON shape (the same shape Configuration Snapshots store as
//! `schema.json`) and writes schemas back through the deterministic
//! canonical YAML renderer. Everything the frontend needs to edit — tables,
//! enums, the 19 field types, constraints, `targets` visibility, expression
//! rules — is already the serde shape of [`crate::schema::Schema`]; this
//! module turns that serde surface into an explicit, tested contract:
//!
//! - [`to_editor_json`] — canonical editor document (insertion order kept).
//! - [`from_editor_json`] — editor document → [`Schema`]; malformed input
//!   yields `E1701` diagnostics located on the JSON path.
//! - [`to_canonical_yaml`] — deterministic save-back: same [`Schema`] →
//!   same bytes (single trailing newline), round-trips through
//!   [`from_canonical_yaml`].
//! - [`from_canonical_yaml`] — canonical YAML → [`Schema`] (`E1701`).
//!
//! Determinism contract: saving an edited schema as canonical YAML and
//! running `cage build` must produce byte-identical artifacts to building
//! the same schema by hand — the generators never see the editor, only the
//! [`Schema`] value, so the invariant is: editor round-trip preserves the
//! [`Schema`] value exactly.

use crate::diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics};
use crate::error::codes::editor::E1701;
use crate::schema::Schema;

/// Canonical editor document of a schema.
///
/// Insertion order is preserved (tables / fields keep document order),
/// optional fields the `skip_serializing_if` attributes skip stay absent —
/// identical serde shape to the `schema.json` inside Configuration
/// Snapshots, so one document serves editor, snapshot and HTTP API.
pub fn to_editor_json(schema: &Schema) -> serde_json::Value {
    serde_json::to_value(schema).expect("Schema always serializes")
}

/// Parse an editor document (JSON) into a [`Schema`].
///
/// Malformed documents come back as `E1701` diagnostics, one per serde
/// error, with the JSON path (e.g. `tables.Item.fields.id.type` — `/`
/// separators cleaned to `.`) in the `field` slot, so the frontend can
/// navigate to the offending input.
pub fn from_editor_json(json: &str) -> Result<Schema, Diagnostics> {
    let mut de = serde_json::Deserializer::from_str(json);
    match serde_path_to_error::deserialize::<_, Schema>(&mut de) {
        Ok(schema) => Ok(schema),
        Err(e) => {
            // serde_path_to_error renders the root-level error as a single
            // "." — normalize it away with the "/" separator cleaning.
            let raw = e.path().to_string();
            let path = raw.strip_prefix('/').unwrap_or(&raw).replace('/', ".");
            let path = path.strip_prefix('.').unwrap_or(&path).to_string();
            let mut diags = Diagnostics::new();
            diags.add(e1701(
                format!("editor input is not a valid schema: {e}"),
                (!path.is_empty()).then_some(path.as_str()),
                "edit the document to match the canonical Schema shape (see docs/web.md)",
            ));
            Err(diags)
        }
    }
}

/// Deterministic canonical YAML save-back of a whole schema.
///
/// Same [`Schema`] value → identical bytes (insertion order preserved,
/// optional empty fields skipped). The renderer never introduces
/// wall-clock or environment-dependent content; the trailing newline is
/// part of the contract.
pub fn to_canonical_yaml(schema: &Schema) -> String {
    serde_yaml::to_string(schema).expect("Schema always serializes")
}

/// Parse canonical schema YAML (the save-back format) into a [`Schema`].
///
/// The document must be the self-contained [`Schema`] shape (`tables` /
/// `enums` / `metadata`), i.e. a whole merged schema — the `cage` CLI's
/// per-file merge additionally accepts fragments, the editor never emits
/// them. Malformed documents yield `E1701`.
pub fn from_canonical_yaml(yaml: &str) -> Result<Schema, Diagnostics> {
    match serde_yaml::from_str::<Schema>(yaml) {
        Ok(schema) => Ok(schema),
        Err(e) => {
            let mut diags = Diagnostics::new();
            diags.add(e1701(
                format!("canonical schema YAML is not a valid schema: {e}"),
                None,
                "the saved-back document must be the canonical Schema shape (see docs/web.md)",
            ));
            Err(diags)
        }
    }
}

/// One `E1701` diagnostic: interchange problem with an optional JSON path
/// in the `field` slot (the frontend highlights that input).
fn e1701(message: impl Into<String>, path: Option<&str>, hint: &str) -> Diagnostic {
    let mut diag = DiagnosticBuilder::error(E1701, message)
        .source("editor")
        .location(crate::value::SourceLocation::new("editor").with_field(path.unwrap_or("-")))
        .hint(hint);
    if let Some(p) = path {
        diag = diag.field(p);
    }
    diag.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema exercising metadata, enums, all compound field types
    /// (Array / Object / Map / Enum), constraints, `targets` visibility,
    /// references, expression rules and flattened custom metadata.
    fn fixture() -> Schema {
        from_canonical_yaml(
            r#"
metadata:
  version: "1.0"
  description: editor fixture
  author: cage
tables:
  Item:
    name: Item
    description: an item
    primary_key: [id]
    order_by: [id]
    targets: [client, server]
    unique_constraints:
      - { name: uniq_name, fields: [name] }
    fields:
      id:
        name: id
        type: { kind: UInt32 }
        required: true
        min: 1
      name:
        name: name
        type: { kind: String }
        required: true
        min_length: 1
        max_length: 64
        pattern: "^[a-z]+$"
      price:
        name: price
        type: { kind: Float64 }
        default: 0
        min: 0
        max: 10000
      tags:
        name: tags
        type: { kind: Array, value: { kind: String } }
        min_items: 1
        max_items: 8
      drops:
        name: drops
        type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }
      by_level:
        name: by_level
        type: { kind: Map, value: { key_type: int, value_type: { kind: Float32 } } }
      material:
        name: material
        type: { kind: Enum, value: Rarity }
      stats:
        name: stats
        type: { kind: Object, value: { hp: { kind: Int32 }, mp: { kind: Int32 } } }
      flag:
        name: flag
        type: { kind: Bool }
      note:
        name: note
        type: { kind: Any }
      raw_blob:
        name: raw_blob
        type: { kind: Bytes }
      slot:
        name: slot
        type: { kind: UInt8 }
      level:
        name: level
        type: { kind: Int16 }
      uid:
        name: uid
        type: { kind: Int64 }
      weight:
        name: weight
        type: { kind: Float32 }
        targets: [server]
      drop_ref:
        name: drop_ref
        type: { kind: UInt32 }
        reference:
          table: Item
          field: id
          cardinality: many
          compatible_with: [client]
        rules:
          - name: r_positive
            assert: "`drop_ref` >= 0"
            message: must be non-negative
            warning_only: true
        custom_note: keep
enums:
  Rarity:
    name: Rarity
    description: rarity tiers
    values:
      - { name: Common, value: 1, description: default }
      - { name: Rare }
"#,
        )
        .expect("fixture parses")
    }

    #[test]
    fn editor_json_round_trip_is_deterministic() {
        let schema = fixture();
        let a = to_editor_json(&schema);
        let b = to_editor_json(&schema);
        assert_eq!(a, b, "same schema → same editor document");

        // JSON file interchange: serialize → parse back → same document.
        let doc = serde_json::to_string_pretty(&a).expect("document serializes");
        let back = from_editor_json(&doc).expect("document parses");
        assert_eq!(to_editor_json(&back), a, "round trip preserves the schema");

        // Shape spot-checks: IndexMap order preserved, Map keeps its
        // key_type, empty optional fields stay absent (skip_serializing_if).
        assert_eq!(
            a["tables"]["Item"]["fields"]["drops"]["type"]["kind"],
            "Map"
        );
        assert_eq!(
            a["tables"]["Item"]["fields"]["drops"]["type"]["value"]["key_type"],
            "string"
        );
        assert_eq!(
            a["tables"]["Item"]["fields"]["drops"]["type"]["value"]["value_type"]["kind"],
            "Array"
        );
        // `default` has no skip_serializing_if: absent defaults are explicit
        // nulls in the interchange shape (frontend edits them either way).
        assert_eq!(
            a["tables"]["Item"]["fields"]["id"].get("default"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(a["enums"]["Rarity"]["values"][0]["value"], 1);
    }

    #[test]
    fn canonical_yaml_round_trip_is_idempotent() {
        let schema = fixture();
        let yaml = to_canonical_yaml(&schema);
        let again = to_canonical_yaml(&schema);
        assert_eq!(yaml, again, "same schema → same canonical YAML bytes");
        assert!(
            yaml.ends_with('\n'),
            "trailing newline is part of the contract"
        );

        // The saved-back document parses back to the exact same schema.
        let reparsed = from_canonical_yaml(&yaml).expect("canonical YAML reparses");
        assert_eq!(to_editor_json(&reparsed), to_editor_json(&schema));

        // re-rendering the reparse is byte-identical (fixed point)
        assert_eq!(to_canonical_yaml(&reparsed), yaml);

        // canonical markers: required stays explicit, default skipped
        // optional empties stay absent (e.g. no `required: false` lines)
        assert!(yaml.contains("required: true"), "{yaml}");
        assert!(!yaml.contains("required: false"), "{yaml}");
        assert!(yaml.contains("key_type: string"), "{yaml}");
        assert!(yaml.contains("name: Common"), "{yaml}");
    }

    #[test]
    fn e1701_reports_bad_editor_input_with_json_path() {
        // Unknown enum-variant kind: serde points at the `type` value.
        let bad = r#"{"tables":{"T":{"name":"T","primary_key":[],"fields":{"f":{"name":"f","type":{"kind":"Quadruple"}}}}}}"#;
        let err = from_editor_json(bad).expect_err("bad kind must fail");
        let (code, msg) = (
            err.errors()[0].code.as_str(),
            err.errors()[0].message.as_str(),
        );
        assert_eq!(code, "E1701");
        assert!(msg.contains("Quadruple"), "{msg}");
        assert_eq!(
            err.errors()[0].field.as_deref(),
            Some("tables.T.fields.f.type.kind")
        );

        // Wrong JSON root shape: root-level "." is normalized away → no field.
        let err = from_editor_json("[]").expect_err("array root must fail");
        assert_eq!(err.errors()[0].code, "E1701");
        assert!(err.errors()[0].field.is_none());
        assert_eq!(err.errors()[0].source, "editor");

        // Missing required member of the Schema shape → E1701 at the table path.
        let err = from_editor_json(r#"{"tables":{"T":{"name":"T"}}}"#)
            .expect_err("missing fields must fail");
        assert_eq!(err.errors()[0].code, "E1701");
        assert!(
            err.errors()[0].message.contains("primary_key"),
            "{:?}",
            err.errors()[0]
        );
        assert_eq!(err.errors()[0].field.as_deref(), Some("tables.T"));

        // Canonical YAML with a syntax error surfaces the same code.
        let err = from_canonical_yaml("tables: [").expect_err("bad yaml must fail");
        assert_eq!(err.errors()[0].code, "E1701");
        assert!(err.errors()[0].message.contains("canonical schema YAML"));
    }

    #[test]
    fn editor_json_matches_the_snapshot_schema_shape() {
        // The snapshot schema.json and the editor document share the shape:
        // whatever `cage snapshot` stored loads back through the editor.
        let schema = fixture();
        let snapshot_json = serde_json::to_string_pretty(&schema).expect("snapshot json");
        let as_snapshot: serde_json::Value =
            serde_json::from_str(&snapshot_json).expect("snapshot json parses");
        assert_eq!(as_snapshot, to_editor_json(&schema));
        let back = from_editor_json(&snapshot_json).expect("snapshot doc parses");
        assert_eq!(to_editor_json(&back), to_editor_json(&schema));
    }
}
