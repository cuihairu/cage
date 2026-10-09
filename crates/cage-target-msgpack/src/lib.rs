//! `MessagePack` Target Generator - generates `MessagePack` artifacts from the validated Cage
//! model.
//!
//! The encoding target is `MessagePack` (smallest-form, byte-deterministic).

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
// Stage: crate-prefixed type names (MsgPackTargetGenerator, ...) are idiomatic
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
    error::codes::build,
    manifest::TargetConfig,
    normalize::normalize_value,
    value::{Document, Row, Table, Value},
};
use rmp::encode::{
    write_array_len, write_bin, write_bool, write_f64, write_map_len, write_nil, write_sint,
    write_str, write_uint,
};
use std::path::PathBuf;

/// `MessagePack` Target Generator
///
/// Determinism contract: rmp always emits the smallest `MessagePack`
/// representation for a value, the row field iteration order is either
/// sorted-by-name (`sort_keys = true`, the default) or the source order, and
/// non-finite floats encode as `nil` (NaN bit patterns are not stable across
/// platforms — same rule as the JSON target's non-finite → null). Same input
/// document therefore always yields byte-identical output.
pub struct MsgPackTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.msgpack")
    pub file_template: String,
    /// Sort map keys by name for deterministic output
    pub sort_keys: bool,
}

impl Default for MsgPackTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/msgpack"),
            file_template: "{table}.msgpack".to_string(),
            sort_keys: true,
        }
    }
}

impl MsgPackTargetGenerator {
    /// Create from target config
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            file_template: config
                .file_template
                .clone()
                .unwrap_or_else(|| "{table}.msgpack".to_string()),
            sort_keys: true,
        };

        if let Some(opts) = &config.options {
            if let Some(v) = opts.get("sort_keys") {
                gen.sort_keys = v.as_bool().unwrap_or(true);
            }
        }

        gen
    }

    /// Generate `MessagePack` artifacts for all tables in the document.
    ///
    /// One `MessagePack` file per table, honoring `file_template`.
    ///
    /// (rustdoc: see crate docs for the `MessagePack` determinism contract.)
    pub fn generate(
        &self,
        document: &Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics> {
        let mut diags = Diagnostics::new();
        let mut artifacts = Vec::new();

        // Filter tables by profile targets if specified
        let tables_to_generate: Vec<_> = if profile_targets.is_empty() {
            document.tables.values().collect()
        } else {
            document
                .tables
                .values()
                .filter(|t| {
                    profile_targets.contains(&t.name) || profile_targets.contains(&"*".to_string())
                })
                .collect()
        };

        for table in tables_to_generate {
            match self.generate_table(table) {
                Ok((path, content)) => artifacts.push((path, content)),
                Err(e) => {
                    diags.add(e);
                }
            }
        }

        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(artifacts)
        }
    }

    /// Generate the `MessagePack` artifact for a single table: an array of
    /// row maps.
    ///
    /// Each row is a `MessagePack` map keyed by field name.
    pub fn generate_table(&self, table: &Table) -> Result<(String, Vec<u8>), Diagnostic> {
        let mut buf = Vec::new();
        write_array_len(&mut buf, table.rows.len() as u32).map_err(|e| {
            Diagnostic::error(build::E9002, format!("MessagePack encoding failed: {e}"))
                .with_source("msgpack-target")
                .with_table(&table.name)
        })?;
        for row in &table.rows {
            self.write_row(&mut buf, row);
        }

        let file_name = self.file_template.replace("{table}", &table.name);
        let path = self
            .output_dir
            .join(&file_name)
            .to_string_lossy()
            .to_string();

        Ok((path, buf))
    }

    /// Encode one row as a `MessagePack` map. Field order follows
    /// `sort_keys`: sorted by name (canonical) or source order.
    fn write_row(&self, buf: &mut Vec<u8>, row: &Row) {
        let mut entries: Vec<(&str, &cage_core::value::TypedValue)> =
            row.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
        if self.sort_keys {
            entries.sort_by(|a, b| a.0.cmp(b.0));
        }

        let _ = write_map_len(buf, entries.len() as u32);
        for (name, typed) in entries {
            let normalized = normalize_value(&typed.value);
            let _ = write_str(buf, name);
            Self::write_value(buf, &normalized);
        }
    }

    /// Encode one canonical `Value` at the current buffer position.
    fn write_value(buf: &mut Vec<u8>, value: &Value) {
        // Every call below targets a Vec<u8>; in-memory writes cannot fail,
        // so the Results are discarded on purpose.
        match value {
            // Finite floats keep full f64 precision; Null and non-finite
            // floats both encode as nil — NaN payloads differ across
            // platforms, so binary encoding would break the determinism
            // contract (JSON target: same rule, non-finite → null).
            Value::Float(f) if f.is_finite() => {
                let _ = write_f64(buf, *f);
            }
            Value::Null | Value::Float(_) => {
                let _ = write_nil(buf);
            }
            Value::Bool(b) => {
                let _ = write_bool(buf, *b);
            }
            Value::Int(i) => {
                let _ = write_sint(buf, *i);
            }
            Value::UInt(u) => {
                let _ = write_uint(buf, *u);
            }
            Value::String(s) => {
                let _ = write_str(buf, s);
            }
            // Native bin format — no base64 detour like JSON.
            Value::Bytes(b) => {
                let _ = write_bin(buf, b);
            }
            Value::Array(arr) => {
                let _ = write_array_len(buf, arr.len() as u32);
                for item in arr {
                    Self::write_value(buf, item);
                }
            }
            Value::Object(obj) => {
                // normalize_value already emitted object keys in sorted
                // order (BTreeMap), so iteration here is deterministic.
                let _ = write_map_len(buf, obj.len() as u32);
                for (k, v) in obj {
                    let _ = write_str(buf, k);
                    Self::write_value(buf, v);
                }
            }
        }
    }
}

/// Trait for target generators
pub trait TargetGenerator {
    fn generate(
        &self,
        document: &cage_core::value::Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics>;
    fn format_name(&self) -> &'static str;
    fn file_extension(&self) -> &'static str;
}

impl TargetGenerator for MsgPackTargetGenerator {
    fn generate(
        &self,
        document: &Document,
        profile_targets: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, Diagnostics> {
        self.generate(document, profile_targets)
    }

    fn format_name(&self) -> &'static str {
        "MessagePack"
    }

    fn file_extension(&self) -> &'static str {
        "msgpack"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::value::{SourceLocation, TypedValue};
    use indexmap::IndexMap;

    fn make_test_doc() -> Document {
        let mut doc = Document::new();
        let mut table = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "items.json".to_string(),
            sheet: None,
        };
        table.rows.push(Row {
            primary_key: vec![Value::UInt(1)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(1), SourceLocation::new("items.json")),
                );
                f.insert(
                    "name".to_string(),
                    TypedValue::new(
                        Value::String("Sword".to_string()),
                        SourceLocation::new("items.json"),
                    ),
                );
                f.insert(
                    "price".to_string(),
                    TypedValue::new(Value::Int(100), SourceLocation::new("items.json")),
                );
                f
            },
            location: SourceLocation::new("items.json"),
            index: 0,
        });
        table.rows.push(Row {
            primary_key: vec![Value::UInt(2)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(2), SourceLocation::new("items.json")),
                );
                f.insert(
                    "name".to_string(),
                    TypedValue::new(
                        Value::String("Shield".to_string()),
                        SourceLocation::new("items.json"),
                    ),
                );
                f.insert(
                    "price".to_string(),
                    TypedValue::new(Value::Int(200), SourceLocation::new("items.json")),
                );
                f
            },
            location: SourceLocation::new("items.json"),
            index: 1,
        });
        doc.add_table(table);
        doc
    }

    #[test]
    fn test_generate_golden_bytes() {
        // Hand-computed smallest-form MessagePack for the two-row fixture,
        // row fields in name order (id, name, price):
        //   fixarray(2) = 0x92
        //   fixmap(3) = 0x83; "id" fixstr+1; "name" fixstr+"Sword";
        //   "price" fixstr + 100 = positive fixint 0x64 (row 1)
        //   and 200 = uint8 0xcc 0xc8 (row 2).
        let doc = make_test_doc();
        let gen = MsgPackTargetGenerator {
            output_dir: PathBuf::from("build/mp"),
            ..Default::default()
        };
        let artifacts = gen.generate(&doc, &[]).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].0.ends_with("Item.msgpack"));

        let expected: Vec<u8> = [
            0x92, // fixarray(2)
            0x83, 0xa2, b'i', b'd', 0x01, // fixmap(3) "id": 1
            0xa4, b'n', b'a', b'm', b'e', 0xa5, b'S', b'w', b'o', b'r',
            b'd', // "name": "Sword"
            0xa5, b'p', b'r', b'i', b'c', b'e', 0x64, // "price": 100
            0x83, 0xa2, b'i', b'd', 0x02, // row 2: "id": 2
            0xa4, b'n', b'a', b'm', b'e', 0xa6, b'S', b'h', b'i', b'e', b'l',
            b'd', // "Shield"
            0xa5, b'p', b'r', b'i', b'c', b'e', 0xcc, 0xc8, // "price": 200 (uint8)
        ]
        .to_vec();
        assert_eq!(artifacts[0].1, expected);
    }

    #[test]
    fn test_deterministic_output() {
        let doc = make_test_doc();
        let gen = MsgPackTargetGenerator {
            output_dir: PathBuf::from("build/mp"),
            ..Default::default()
        };
        let artifacts1 = gen.generate(&doc, &[]).unwrap();
        let artifacts2 = gen.generate(&doc, &[]).unwrap();
        assert_eq!(artifacts1[0].1, artifacts2[0].1);
    }

    #[test]
    fn test_generate_row_with_all_value_kinds() {
        // Every Value variant through the real generate() pipeline:
        // bytes → bin, null → nil, non-finite floats → nil, -0.0
        // canonicalized by normalize_value, nested composites, unicode.
        let mut doc = Document::new();
        let mut table = Table {
            name: "Rich".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "rich.json".to_string(),
            sheet: None,
        };
        let mut fields = IndexMap::new();
        let mut add = |name: &str, value: Value| {
            fields.insert(
                name.to_string(),
                TypedValue::new(value, SourceLocation::new("rich.json")),
            );
        };
        add("id", Value::UInt(u64::MAX));
        add("nil", Value::Null);
        add("flag", Value::Bool(true));
        add("delta", Value::Int(i64::MIN));
        add("ratio", Value::Float(-0.0)); // normalize → 0.0
        add("huge", Value::Float(1.5e308));
        add("text", Value::String("  列 ⚔  ".to_string())); // trim + unicode
        add("blob", Value::Bytes(b"Hello".to_vec()));
        add("list", Value::Array(vec![Value::Int(1), Value::Null]));
        add("map", {
            let mut m = IndexMap::new();
            m.insert("k".to_string(), Value::Bool(true));
            m.insert("deep".to_string(), Value::Array(vec![Value::UInt(2)]));
            Value::Object(m)
        });
        add("nan", Value::Float(f64::NAN));
        add("inf", Value::Float(f64::INFINITY));
        table.rows.push(Row {
            primary_key: vec![Value::UInt(u64::MAX)],
            fields,
            location: SourceLocation::new("rich.json"),
            index: 0,
        });
        doc.add_table(table);

        let gen = MsgPackTargetGenerator {
            output_dir: PathBuf::from("build/mp"),
            ..Default::default()
        };
        let artifacts = gen.generate(&doc, &[]).unwrap();

        // Spot-check the interesting encodings inside the row map: sorted
        // field order is [blob, delta, flag, huge, id, inf, list, map, nan,
        // nil, ratio, text].
        let bytes = &artifacts[0].1;
        let find = |needle: &[u8]| {
            bytes
                .windows(needle.len())
                .position(|w| w == needle)
                .unwrap_or_else(|| panic!("pattern {needle:?} not found"))
        };
        // "blob" key + bin8 "Hello" (bin marker 0xc4, length 5).
        assert_eq!(
            &bytes[find(b"blob")..find(b"blob") + 11],
            &[b'b', b'l', b'o', b'b', 0xc4, 0x05, b'H', b'e', b'l', b'l', b'o']
        );
        // "delta" + int64 min: marker 0xd3 then the two's-complement bytes.
        let d = find(b"delta");
        assert_eq!(&bytes[d + 5..d + 7], &[0xd3, 0x80]);
        // "nan"/"inf" encode as nil (0xc0).
        let n = find(b"nan");
        assert_eq!(&bytes[n + 3..n + 4], &[0xc0]);
        let i = find(b"inf");
        assert_eq!(&bytes[i + 3..i + 4], &[0xc0]);
        // "ratio" is -0.0 normalized to +0.0 → f64 marker 0xcb + zero bytes.
        let r = find(b"ratio");
        assert_eq!(&bytes[r + 5..r + 7], &[0xcb, 0x00]);
        // "id" holds u64::MAX: marker 0xcf + eight 0xff bytes.
        let id = find(b"id");
        assert_eq!(&bytes[id + 2..id + 3], &[0xcf]);
        assert!(bytes[id + 3..id + 11].iter().all(|&b| b == 0xff));
        // Byte-for-byte deterministic.
        let again = gen.generate(&doc, &[]).unwrap();
        assert_eq!(artifacts[0].1, again[0].1);
    }

    #[test]
    fn test_sort_keys_false_keeps_source_order() {
        // Fields deliberately inserted in non-name order: sort_keys = false
        // keeps source order in the row map (nested objects still come out
        // sorted via normalize_value's BTreeMap — same as the JSON target).
        let mut doc = Document::new();
        let mut table = Table {
            name: "Order".to_string(),
            primary_key_fields: vec!["zeta".to_string()],
            rows: vec![],
            source_file: "order.json".to_string(),
            sheet: None,
        };
        let mut fields = IndexMap::new();
        for name in ["zeta", "alpha"] {
            fields.insert(
                name.to_string(),
                TypedValue::new(
                    Value::String(name.to_string()),
                    SourceLocation::new("order.json"),
                ),
            );
        }
        table.rows.push(Row {
            primary_key: vec![Value::String("zeta".to_string())],
            fields,
            location: SourceLocation::new("order.json"),
            index: 0,
        });
        doc.add_table(table);

        let gen = MsgPackTargetGenerator {
            output_dir: PathBuf::from("build/mp"),
            sort_keys: false,
            ..Default::default()
        };
        let artifacts = gen.generate(&doc, &[]).unwrap();
        // fixarray(1), fixmap(2), "zeta":"zeta", "alpha":"alpha"
        let expected: Vec<u8> = [
            0x91, 0x82, 0xa4, b'z', b'e', b't', b'a', 0xa4, b'z', b'e', b't', b'a', 0xa5, b'a',
            b'l', b'p', b'h', b'a', 0xa5, b'a', b'l', b'p', b'h', b'a',
        ]
        .to_vec();
        assert_eq!(artifacts[0].1, expected);

        // The sorted variant reorders the very same source data.
        let sorted = MsgPackTargetGenerator {
            sort_keys: true,
            ..gen
        };
        let artifacts = sorted.generate(&doc, &[]).unwrap();
        let expected: Vec<u8> = [
            0x91, 0x82, 0xa5, b'a', b'l', b'p', b'h', b'a', 0xa5, b'a', b'l', b'p', b'h', b'a',
            0xa4, b'z', b'e', b't', b'a', 0xa4, b'z', b'e', b't', b'a',
        ]
        .to_vec();
        assert_eq!(artifacts[0].1, expected);
    }

    #[test]
    fn test_profile_target_filtering() {
        let doc = make_test_doc();
        let gen = MsgPackTargetGenerator::default();
        let matched = gen.generate(&doc, &["Item".to_string()]).unwrap();
        assert_eq!(matched.len(), 1);
        assert!(matched[0].0.ends_with("Item.msgpack"));
        let all = gen.generate(&doc, &["*".to_string()]).unwrap();
        assert_eq!(all.len(), 1);
        let none = gen.generate(&doc, &["Ghost".to_string()]).unwrap();
        assert_eq!(none.len(), 0);
    }

    /// Build a `TargetConfig` directly — this crate has no `serde_yaml`
    /// dev-dependency; the struct is plain data.
    fn config(
        output_dir: &str,
        file_template: Option<&str>,
        options: Option<indexmap::IndexMap<String, serde_json::Value>>,
    ) -> TargetConfig {
        TargetConfig {
            format: "msgpack".to_string(),
            output_dir: output_dir.to_string(),
            file_template: file_template.map(str::to_string),
            options,
        }
    }

    fn opts(pairs: &[(&str, serde_json::Value)]) -> indexmap::IndexMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_from_config_defaults() {
        let gen = MsgPackTargetGenerator::from_config(&config("build/m", None, None));
        assert_eq!(gen.output_dir, PathBuf::from("build/m"));
        assert_eq!(gen.file_template, "{table}.msgpack");
        assert!(gen.sort_keys);
    }

    #[test]
    fn test_from_config_option_overrides() {
        let gen = MsgPackTargetGenerator::from_config(&config(
            "build/m",
            Some("{table}_gen.mp"),
            Some(opts(&[("sort_keys", serde_json::json!(false))])),
        ));
        assert_eq!(gen.file_template, "{table}_gen.mp");
        assert!(!gen.sort_keys);

        // Non-bool option values fall back to the default.
        let gen = MsgPackTargetGenerator::from_config(&config(
            "build/m",
            None,
            Some(opts(&[("sort_keys", serde_json::json!(1))])),
        ));
        assert!(gen.sort_keys);

        // Unrelated keys keep the defaults.
        let gen = MsgPackTargetGenerator::from_config(&config(
            "build/m",
            None,
            Some(opts(&[("unknown", serde_json::json!(true))])),
        ));
        assert!(gen.sort_keys);
    }

    #[test]
    fn test_target_generator_trait() {
        let doc = make_test_doc();
        let gen = MsgPackTargetGenerator::default();
        assert_eq!(gen.format_name(), "MessagePack");
        assert_eq!(gen.file_extension(), "msgpack");
        let artifacts = TargetGenerator::generate(&gen, &doc, &[]).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].0.ends_with("Item.msgpack"));
    }
}
