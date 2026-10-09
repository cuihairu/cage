//! `MessagePack` Source Adapter - parses `MessagePack` files into Cage's Canonical Model
//!
//! The file shape is the `MessagePack` TARGET's wire format (design §23):
//! a bare array of row maps — `[{"id": 1, "name": "Sword"}, …]`. The binary
//! form carries no table name, so the file stem names the table (the CSV /
//! Excel convention). This is the re-consumption side of the registry loop:
//! a published entry packs `data/**.msgpack`, resolution reads it back with
//! floats bit-exact, `bin` payloads as `Bytes`, and integers beyond
//! `i64::MAX` as `UInt` — all things a JSON round-trip cannot represent.
//! Non-negative integers normalize to `Int` (source convention, like the
//! JSON adapter): the wire's positive markers cannot carry signedness.

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
// Stage: crate-prefixed type names (MsgPackSourceAdapter, ...) are idiomatic
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

/// `MessagePack` Source Adapter
pub struct MsgPackSourceAdapter;

impl MsgPackSourceAdapter {
    /// Parse a `MessagePack` file into a Document
    pub fn parse_file(path: &Path) -> Result<Document, Diagnostics> {
        let data = std::fs::read(path).map_err(|e| {
            let mut diags = Diagnostics::new();
            diags.add(
                Diagnostic::error(internal::E9902, format!("Failed to read file: {e}"))
                    .with_source(path.display().to_string()),
            );
            diags
        })?;
        Self::parse_bytes(&data, path)
    }

    /// Parse `MessagePack` bytes into a Document.
    ///
    /// Root must be an array of row maps; the file stem names the table.
    /// Floats come back bit-exact, `bin` payloads as `Bytes`; in-range
    /// integers normalize to `Int` (see `cage_value` below). An empty
    /// array yields an empty table, not a skipped one.
    pub fn parse_bytes(data: &[u8], path: &Path) -> Result<Document, Diagnostics> {
        let file_path = path.display().to_string();
        let mut diags = Diagnostics::new();

        // Decode exactly one MessagePack value; trailing bytes mean a
        // truncated or corrupt write, not another value.
        let mut cursor = std::io::Cursor::new(data);
        let decoded = match rmpv::decode::read_value(&mut cursor) {
            Ok(v) => v,
            Err(e) => {
                diags.add(
                    Diagnostic::error(parse::E0001, format!("MessagePack syntax error: {e}"))
                        .with_source(&file_path)
                        .with_hint("Check the file is complete MessagePack written by the cage msgpack target"),
                );
                return Err(diags);
            }
        };
        if cursor.position() as usize != data.len() {
            diags.add(
                Diagnostic::error(
                    parse::E0001,
                    "MessagePack syntax error: trailing bytes after the root value",
                )
                .with_source(&file_path),
            );
            return Err(diags);
        }

        let rmpv::Value::Array(rows) = decoded else {
            diags.add(
                Diagnostic::error(
                    parse::E0004,
                    "MessagePack root must be an array of row objects \
                     (the cage msgpack target's wire format)",
                )
                .with_source(&file_path),
            );
            return Err(diags);
        };

        let table_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Data")
            .to_string();

        let mut table = Table {
            name: table_name.clone(),
            primary_key_fields: Vec::new(),
            rows: Vec::new(),
            source_file: file_path.clone(),
            sheet: None,
        };

        for (row_idx, row_value) in rows.into_iter().enumerate() {
            let rmpv::Value::Map(entries) = row_value else {
                diags.add(
                    Diagnostic::error(parse::E0004, format!("Row {row_idx} is not an object"))
                        .with_source(&file_path)
                        .with_table(&table_name)
                        .with_row((row_idx + 1).to_string()),
                );
                return Err(diags);
            };
            table
                .rows
                .push(Self::map_to_row(entries, &file_path, row_idx));
        }

        let mut doc = Document::new();
        doc.metadata.format = "msgpack".to_string();
        // add_table records source_file into source_files itself.
        doc.add_table(table);
        Ok(doc)
    }

    /// One `MessagePack` map → one canonical row. Field order is sorted by
    /// name (the same canonical ordering the JSON source applies); the
    /// primary-key heuristic matches the JSON adapter (first `id` field,
    /// else the first field).
    fn map_to_row(
        entries: Vec<(rmpv::Value, rmpv::Value)>,
        file_path: &str,
        row_idx: usize,
    ) -> Row {
        let mut named: Vec<(String, rmpv::Value)> = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            match key {
                rmpv::Value::String(k) => named.push((
                    k.as_str().map_or_else(|| format!("{k:?}"), str::to_owned),
                    value,
                )),
                other => {
                    // Cannot happen for target-written files; a hand-made
                    // map with non-string keys has no field name to carry.
                    named.push((format!("{other:?}"), value));
                }
            }
        }
        named.sort_by(|a, b| a.0.cmp(&b.0));

        let mut fields = IndexMap::new();
        let mut primary_key = Vec::new();
        for (key, value) in named {
            let cage_value = Self::cage_value(value, file_path);
            let loc = SourceLocation::new(file_path)
                .with_row(row_idx + 1)
                .with_column(key.clone())
                .with_field(key.clone());
            fields.insert(key.clone(), TypedValue::new(cage_value, loc));

            if primary_key.is_empty() && key.eq_ignore_ascii_case("id") {
                primary_key.push(fields[&key].value.clone());
            }
        }

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

    /// Convert one `rmpv` value to a canonical `Value`.
    ///
    /// Floats come back bit-exact, `bin` payloads come back as `Bytes`, and
    /// integers beyond `i64::MAX` keep the `UInt` family — all things a JSON
    /// round-trip cannot represent. Non-negative integers normalize to `Int`
    /// (the source convention, matching the JSON adapter): the wire has a
    /// single positive-integer family — `write_sint(7)` and `write_uint(7)`
    /// emit the same markers — so signedness of non-negatives is
    /// unrecoverable, and `Int` keeps re-consumed data valid against the
    /// Int-typed columns every text source feeds.
    fn cage_value(value: rmpv::Value, file_path: &str) -> Value {
        match value {
            rmpv::Value::Nil => Value::Null,
            rmpv::Value::Boolean(b) => Value::Bool(b),
            rmpv::Value::Integer(i) => match i.as_i64() {
                Some(n) => Value::Int(n),
                None => Value::UInt(i.as_u64().unwrap_or_default()),
            },
            rmpv::Value::F32(f) => Value::Float(f64::from(f)),
            rmpv::Value::F64(f) => Value::Float(f),
            rmpv::Value::String(s) => match s.as_str() {
                Some(text) => Value::String(text.to_owned()),
                // Legacy raw-typed bytes with invalid UTF-8: carry as Bytes
                // rather than lossy text.
                None => Value::Bytes(s.into_bytes()),
            },
            rmpv::Value::Binary(b) => Value::Bytes(b),
            rmpv::Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|v| Self::cage_value(v, file_path))
                    .collect(),
            ),
            rmpv::Value::Map(entries) => {
                let mut map = IndexMap::new();
                for (key, value) in entries {
                    let key = match key {
                        rmpv::Value::String(k) => {
                            k.as_str().map_or_else(|| format!("{k:?}"), str::to_owned)
                        }
                        other => format!("{other:?}"),
                    };
                    map.insert(key, Self::cage_value(value, file_path));
                }
                Value::Object(map)
            }
            rmpv::Value::Ext(_, _) => {
                // MessagePack ext has no canonical mapping (the target never
                // emits it); carried as a string note rather than dropped.
                Value::String(format!("unsupported ext value in {file_path}"))
            }
        }
    }
}

/// Trait for source adapters (for future plugin system) — binary variant:
/// `parse_bytes` replaces the text-oriented `parse_str` of the textual
/// adapters, since `MessagePack` has no string form.
pub trait SourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics>;
    fn parse_bytes(&self, data: &[u8], path: &Path) -> Result<Document, Diagnostics>;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn format_name(&self) -> &'static str;
}

impl SourceAdapter for MsgPackSourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        Self::parse_file(path)
    }

    fn parse_bytes(&self, data: &[u8], path: &Path) -> Result<Document, Diagnostics> {
        Self::parse_bytes(data, path)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["msgpack"]
    }

    fn format_name(&self) -> &'static str {
        "MessagePack"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::manifest::TargetConfig;
    use cage_core::value::Value;
    use cage_core::value::{Document as Doc, Table as T};
    use cage_target_msgpack::MsgPackTargetGenerator;
    use std::io::Write;

    fn config() -> TargetConfig {
        TargetConfig {
            format: "msgpack".to_string(),
            output_dir: "build/m".to_string(),
            file_template: None,
            options: None,
        }
    }

    fn table(name: &str, rows: Vec<Row>) -> Doc {
        let mut doc = Doc::new();
        doc.add_table(T {
            name: name.to_string(),
            primary_key_fields: vec![],
            rows,
            source_file: "test".to_string(),
            sheet: None,
        });
        doc
    }

    fn row(index: usize, fields: &[(&str, Value)]) -> Row {
        let mut map = IndexMap::new();
        for (name, value) in fields {
            map.insert(
                (*name).to_string(),
                TypedValue::new(value.clone(), SourceLocation::new("t").with_field(*name)),
            );
        }
        Row {
            primary_key: fields
                .first()
                .map(|(_, v)| (*v).clone())
                .into_iter()
                .collect(),
            fields: map,
            location: SourceLocation::new("t").with_row(index + 1),
            index,
        }
    }

    /// Target emits → source reads back → identical canonical values
    /// (the registry re-consumption loop, end to end in-process). Positive
    /// integers come back as `Int` (wire markers cannot carry signedness);
    /// every other family is exact.
    #[test]
    fn target_to_source_round_trip_is_lossless() {
        let gen = MsgPackTargetGenerator::from_config(&config());
        let doc = table(
            "Item",
            vec![row(
                0,
                &[
                    ("id", Value::UInt(1)),
                    ("count", Value::Int(-5)),
                    ("price", Value::Float(9.5)),
                    ("name", Value::String("Sword".to_string())),
                    ("icon", Value::Bytes(vec![0xde, 0xad])),
                    ("hidden", Value::Null),
                    ("active", Value::Bool(true)),
                    (
                        "tags",
                        Value::Array(vec![Value::String("pvp".to_string()), Value::UInt(2)]),
                    ),
                    (
                        "meta",
                        Value::Object({
                            let mut m = IndexMap::new();
                            m.insert("z".to_string(), Value::Bool(false));
                            m.insert("a".to_string(), Value::Int(7));
                            m
                        }),
                    ),
                ],
            )],
        );
        let (path, bytes) = gen.generate_table(&doc.tables["Item"]).unwrap();
        assert!(path.ends_with("Item.msgpack"), "unexpected path {path}");

        let parsed = MsgPackSourceAdapter::parse_bytes(&bytes, Path::new("data/Item.msgpack"))
            .expect("round trip parses");
        assert_eq!(parsed.metadata.format, "msgpack");
        assert_eq!(parsed.source_files, vec!["data/Item.msgpack".to_string()]);
        let t = &parsed.tables["Item"];
        assert_eq!(t.name, "Item");
        assert_eq!(t.rows.len(), 1);
        let r = &t.rows[0];

        // Sorted field order (canonical), exact families, exact values.
        let keys: Vec<_> = r.fields.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["active", "count", "hidden", "icon", "id", "meta", "name", "price", "tags"]
        );
        assert_eq!(r.fields["id"].value, Value::Int(1));
        assert_eq!(r.fields["count"].value, Value::Int(-5));
        assert_eq!(r.fields["price"].value, Value::Float(9.5));
        assert_eq!(r.fields["name"].value, Value::String("Sword".to_string()));
        assert_eq!(r.fields["icon"].value, Value::Bytes(vec![0xde, 0xad]));
        assert_eq!(r.fields["hidden"].value, Value::Null);
        assert_eq!(r.fields["active"].value, Value::Bool(true));
        assert_eq!(
            r.fields["tags"].value,
            Value::Array(vec![Value::String("pvp".to_string()), Value::Int(2)])
        );
        let meta = match &r.fields["meta"].value {
            Value::Object(m) => m,
            other => panic!("expected object, got {other:?}"),
        };
        assert_eq!(meta["z"], Value::Bool(false));
        assert_eq!(meta["a"], Value::Int(7));
    }

    /// Numeric boundaries survive the round trip: negative ints exact,
    /// `i64::MAX` exact, values beyond `i64::MAX` keep the `UInt` family —
    /// a JSON round-trip would misread all of these.
    #[test]
    fn numeric_families_reconstruct_exactly() {
        let gen = MsgPackTargetGenerator::from_config(&config());
        let doc = table(
            "Num",
            vec![row(
                0,
                &[
                    ("i_min", Value::Int(i64::MIN)),
                    ("i_max", Value::Int(i64::MAX)),
                    ("u_min", Value::UInt(0)),
                    ("u_max", Value::UInt(u64::MAX)),
                    ("f64", Value::Float(std::f64::consts::PI)),
                ],
            )],
        );
        let (_, bytes) = gen.generate_table(&doc.tables["Num"]).unwrap();
        let parsed = MsgPackSourceAdapter::parse_bytes(&bytes, Path::new("Num.msgpack")).unwrap();
        let r = &parsed.tables["Num"].rows[0];
        assert_eq!(r.fields["i_min"].value, Value::Int(i64::MIN));
        assert_eq!(r.fields["i_max"].value, Value::Int(i64::MAX));
        assert_eq!(r.fields["u_min"].value, Value::Int(0));
        assert_eq!(r.fields["u_max"].value, Value::UInt(u64::MAX));
        let f = match r.fields["f64"].value {
            Value::Float(f) => f,
            ref other => panic!("expected float, got {other:?}"),
        };
        assert_eq!(f.to_bits(), std::f64::consts::PI.to_bits());
    }

    /// Non-finite floats encode nil on the target side; the source reads
    /// them back as Null — documented round-trip semantics, same as JSON.
    #[test]
    fn non_finite_float_reads_back_null() {
        let gen = MsgPackTargetGenerator::from_config(&config());
        let doc = table("Edge", vec![row(0, &[("ratio", Value::Float(f64::NAN))])]);
        let (_, bytes) = gen.generate_table(&doc.tables["Edge"]).unwrap();
        let parsed = MsgPackSourceAdapter::parse_bytes(&bytes, Path::new("Edge.msgpack")).unwrap();
        assert_eq!(
            parsed.tables["Edge"].rows[0].fields["ratio"].value,
            Value::Null
        );
    }

    /// Empty array → an empty table (not skipped): the target emits a
    /// one-byte `0x90` for an empty table, the source keeps the table.
    #[test]
    fn empty_array_yields_empty_table() {
        let parsed =
            MsgPackSourceAdapter::parse_bytes(&[0x90], Path::new("Empty.msgpack")).unwrap();
        assert_eq!(parsed.tables["Empty"].rows.len(), 0);
    }

    #[test]
    fn shape_violations_error_with_parse_codes() {
        // Non-array root.
        let diags = MsgPackSourceAdapter::parse_bytes(&[0xc0], Path::new("X.msgpack")).unwrap_err();
        assert!(diags.render(false).contains("root must be an array"));

        // Array of non-maps.
        let diags =
            MsgPackSourceAdapter::parse_bytes(&[0x91, 0x01], Path::new("X.msgpack")).unwrap_err();
        assert!(diags.render(false).contains("Row 0 is not an object"));

        // Trailing garbage after the root value.
        let diags =
            MsgPackSourceAdapter::parse_bytes(&[0x90, 0x01], Path::new("X.msgpack")).unwrap_err();
        assert!(diags.render(false).contains("trailing bytes"));

        // Truncated frame.
        let diags = MsgPackSourceAdapter::parse_bytes(&[0x91], Path::new("X.msgpack")).unwrap_err();
        assert!(diags.render(false).contains("MessagePack syntax error"));
    }

    /// File stem names the table (CSV convention); the primary-key
    /// heuristic matches the JSON adapter (first `id` field).
    #[test]
    fn stem_names_table_and_pk_heuristic_matches_json() {
        // Encode a two-row file by hand: fixarray(2), then two fixmaps with
        // an `id` and a `name` field each.
        let mut buf = Vec::new();
        buf.write_all(&[0x92]).unwrap();
        for id in 1..=2_u8 {
            buf.write_all(&[0x82]).unwrap();
            buf.write_all(&[0xa2, b'i', b'd', 0xcc, id]).unwrap();
            buf.write_all(&[0xa4, b'n', b'a', b'm', b'e', 0xa1, b'x'])
                .unwrap();
        }
        let parsed =
            MsgPackSourceAdapter::parse_bytes(&buf, Path::new("cfg/Stage.msgpack")).unwrap();
        let t = &parsed.tables["Stage"];
        assert_eq!(t.name, "Stage");
        assert_eq!(t.rows[0].primary_key, vec![Value::Int(1)]);
        assert_eq!(t.rows[1].primary_key, vec![Value::Int(2)]);
        assert_eq!(t.rows[0].location.row, Some(1));
        assert_eq!(t.rows[1].location.row, Some(2));
    }

    #[test]
    fn plugin_trait_surface() {
        let adapter = MsgPackSourceAdapter;
        assert_eq!(adapter.supported_extensions(), &["msgpack"]);
        assert_eq!(adapter.format_name(), "MessagePack");
        let doc = SourceAdapter::parse_bytes(&adapter, &[0x90], Path::new("T.msgpack")).unwrap();
        assert!(doc.tables.contains_key("T"));
        let doc = SourceAdapter::parse_file(&adapter, Path::new("T.msgpack"));
        assert!(doc.is_err(), "missing file reports E9902");
    }
}
