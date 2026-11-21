//! Schema draft inference (P1-6): the reverse of the build pipeline — a
//! parsed data [`Document`] (Excel/CSV tables) proposes a [`Schema`].
//!
//! Pure over a [`Document`], so the web import endpoint, any CLI face and
//! the unit tests all share one inference. The output is deliberately a
//! *draft*: it recommends a per-column type, required/nullable, a numeric
//! min/max and an inline enum for low-cardinality text columns. It never
//! guesses at business rules, cross-table references or key semantics beyond
//! a unique first column — the planner refines those in the editor.

use crate::schema::{FieldSchema, FieldType, Schema, SchemaMetadata, TableSchema};
use crate::value::{Document, Value};
use indexmap::IndexMap;

/// Knobs for [`infer_schema`].
#[derive(Debug, Clone)]
pub struct InferOptions {
    /// A string column with at most this many distinct non-blank values
    /// (and at least two) is promoted to an inline enum; above it the
    /// column stays `String`.
    pub enum_threshold: usize,
}

impl Default for InferOptions {
    fn default() -> Self {
        Self { enum_threshold: 16 }
    }
}

/// The inferred draft plus the advisory notes a planner should see: which
/// columns were promoted to enums, which fell back to `String` because the
/// values mixed kinds, and so on. Purely informational — the draft stands
/// on its own.
#[derive(Debug, Clone)]
pub struct InferredSchema {
    /// The draft schema.
    pub schema: Schema,
    /// Human-readable advisory notes (one per column decision worth a peek).
    pub warnings: Vec<String>,
}

/// Infer a draft [`Schema`] from a parsed data [`Document`]. Each source
/// table becomes one table schema: field names come from the rows
/// (first-seen order), types from the observed values, and the primary key
/// from the first column that is present and unique on every row. An empty
/// document infers an empty schema (a valid draft, not an error).
pub fn infer_schema(doc: &Document, opts: &InferOptions) -> Result<InferredSchema, String> {
    let mut tables = IndexMap::new();
    let mut warnings = Vec::new();
    for (name, table) in &doc.tables {
        let (tschema, mut table_warnings) = infer_table(name, table, opts);
        warnings.append(&mut table_warnings);
        tables.insert(name.clone(), tschema);
    }
    Ok(InferredSchema {
        schema: Schema {
            tables,
            enums: IndexMap::new(),
            metadata: Some(SchemaMetadata {
                version: "0.1.0".to_string(),
                description: Some("Inferred draft from a data file import".to_string()),
                author: None,
            }),
        },
        warnings,
    })
}

/// One column's observed values, collected in first-seen order (deduped).
#[derive(Default)]
struct Column {
    /// Non-blank distinct values, in source order.
    values: Vec<Value>,
    /// Number of blank cells (Null / empty string / empty array or object).
    blank: usize,
}

/// How a column's values are summarized into a draft [`FieldType`].
enum Kind {
    /// No non-blank value anywhere — defer to `Null`.
    Blank,
    Bool,
    /// All integral; the chosen signed/unsigned width and observed bounds.
    Int {
        field_type: FieldType,
        min: f64,
        max: f64,
    },
    /// Any float present (or a wide unsigned that needs the float lane).
    Float {
        min: f64,
        max: f64,
    },
    /// Low-cardinality strings: inline enum.
    Enum(Vec<String>),
    /// High-cardinality or structurally mixed strings.
    String,
    /// Mixed kinds or structured cells (array/object/bytes): the planner
    /// must look — the draft says `String` and warns.
    NeedsLook,
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// Classify a [`Column`] into its [`Kind`] under the given [`InferOptions`].
fn classify(col: &Column, opts: &InferOptions) -> Kind {
    if col.values.is_empty() {
        return Kind::Blank;
    }
    // All-Bool fast path.
    if col.values.iter().all(|v| matches!(v, Value::Bool(_))) {
        return Kind::Bool;
    }
    // Numeric lane: every value integral or floating, nothing structured.
    let all_numeric = col
        .values
        .iter()
        .all(|v| matches!(v, Value::Int(_) | Value::UInt(_) | Value::Float(_)));
    if all_numeric {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut any_float = false;
        // Signed extent over every integral value (UInt widened): a
        // negative Int next to a large UInt must not shrink the range.
        let mut signed_min = i128::MAX;
        let mut signed_max = i128::MIN;
        for v in &col.values {
            match v {
                Value::Int(i) => {
                    min = min.min(*i as f64);
                    max = max.max(*i as f64);
                    signed_min = signed_min.min(i128::from(*i));
                    signed_max = signed_max.max(i128::from(*i));
                }
                Value::UInt(u) => {
                    min = min.min(*u as f64);
                    max = max.max(*u as f64);
                    signed_min = signed_min.min(i128::from(*u));
                    signed_max = signed_max.max(i128::from(*u));
                }
                Value::Float(fl) => {
                    any_float = true;
                    min = min.min(*fl);
                    max = max.max(*fl);
                }
                _ => {}
            }
        }
        if any_float {
            return Kind::Float { min, max };
        }
        // All integral. Non-negative stays unsigned when it fits u32 (a
        // draft doesn't over-fit the observed range to 8/16-bit widths).
        if signed_min >= 0 {
            let ft = if signed_max <= i128::from(u32::MAX) {
                FieldType::UInt32
            } else {
                FieldType::UInt64
            };
            return Kind::Int {
                field_type: ft,
                min,
                max,
            };
        }
        if signed_min >= i128::from(i32::MIN) && signed_max <= i128::from(i32::MAX) {
            return Kind::Int {
                field_type: FieldType::Int32,
                min,
                max,
            };
        }
        return Kind::Int {
            field_type: FieldType::Int64,
            min,
            max,
        };
    }
    // String lane: every non-blank value is a String.
    let all_string = col.values.iter().all(|v| matches!(v, Value::String(_)));
    if all_string {
        let distinct: Vec<String> = col
            .values
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        if distinct.len() >= 2 && distinct.len() <= opts.enum_threshold {
            return Kind::Enum(distinct);
        }
        return Kind::String;
    }
    Kind::NeedsLook
}

/// Field name synthesis: an empty Excel header becomes `col_<n>`, and a
/// collision with an already-used name gets a numeric suffix. Returns the
/// (possibly synthesized) name plus the used set it registered into.
fn field_name(raw: &str, index: usize, used: &mut std::collections::HashSet<String>) -> String {
    let base = if raw.trim().is_empty() {
        format!("col_{}", index + 1)
    } else {
        raw.to_string()
    };
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}_{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Infer one table schema from one loaded [`crate::value::Table`].
fn infer_table(
    name: &str,
    table: &crate::value::Table,
    opts: &InferOptions,
) -> (TableSchema, Vec<String>) {
    let mut warnings = Vec::new();

    // Field order: the union of every row's keys, in first-seen order.
    let mut order: Vec<String> = Vec::new();
    for row in &table.rows {
        for key in row.fields.keys() {
            if !order.contains(key) {
                order.push(key.clone());
            }
        }
    }

    // Column stats, keyed by order index (so empty/duplicate headers still
    // land in distinct columns).
    let mut cols: Vec<Column> = Vec::with_capacity(order.len());
    for _ in 0..order.len() {
        cols.push(Column::default());
    }
    let row_count = table.rows.len();
    for row in &table.rows {
        for (i, field) in order.iter().enumerate() {
            match row.fields.get(field) {
                Some(tv) if !is_blank(&tv.value) => {
                    let col = &mut cols[i];
                    if !col.values.contains(&tv.value) {
                        col.values.push(tv.value.clone());
                    }
                }
                _ => cols[i].blank += 1,
            }
        }
    }

    let mut used = std::collections::HashSet::new();
    let mut synth_names = Vec::with_capacity(order.len());
    let mut fields = IndexMap::new();
    for (i, field) in order.iter().enumerate() {
        let fname = field_name(field, i, &mut used);
        synth_names.push(fname.clone());
        let kind = classify(&cols[i], opts);
        let mut field_schema = FieldSchema {
            name: fname.clone(),
            field_type: FieldType::Null,
            description: None,
            required: false,
            nullable: true,
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
            separator: None,
            properties: None,
            additional_properties: None,
            reference: None,
            targets: Vec::new(),
            rules: Vec::new(),
            metadata: IndexMap::new(),
        };
        match kind {
            Kind::Blank => {
                // No information at all: Null type, nullable — the planner
                // picks a real type.
            }
            Kind::Bool => {
                field_schema.field_type = FieldType::Bool;
            }
            Kind::Int {
                field_type,
                min,
                max,
            } => {
                field_schema.field_type = field_type;
                field_schema.min = Some(min);
                field_schema.max = Some(max);
            }
            Kind::Float { min, max } => {
                field_schema.field_type = FieldType::Float64;
                field_schema.min = Some(min);
                field_schema.max = Some(max);
            }
            Kind::Enum(values) => {
                field_schema.field_type = FieldType::String;
                field_schema.enum_values = Some(values);
                warnings.push(format!(
                    "表 {name}.{fname}: {} 个不同取值，建议为枚举（enum_values）",
                    field_schema.enum_values.as_ref().map_or(0, Vec::len)
                ));
            }
            Kind::String => {
                field_schema.field_type = FieldType::String;
            }
            Kind::NeedsLook => {
                field_schema.field_type = FieldType::String;
                warnings.push(format!(
                    "表 {name}.{fname}: 混型/结构化取值，草稿暂定为 String，请人工核对"
                ));
            }
        }
        let blank = cols[i].blank;
        field_schema.required = blank == 0 && row_count > 0;
        field_schema.nullable = blank > 0;
        if field_schema.required && !field_schema.nullable {
            field_schema.nullable = false;
        }
        fields.insert(fname, field_schema);
    }

    // Primary key: the first column present and unique on every row; else
    // the first column; else none. Uses the same synthesized names the
    // `fields` map got, so the key always references a real column.
    let primary_key = if order.is_empty() {
        Vec::new()
    } else {
        let mut pk_index = 0;
        for (i, col) in cols.iter().enumerate() {
            if col.blank == 0 && col.values.len() == row_count && row_count > 0 {
                pk_index = i;
                break;
            }
        }
        vec![synth_names[pk_index].clone()]
    };

    let tschema = TableSchema {
        name: name.to_string(),
        description: Some("Inferred from data import".to_string()),
        primary_key,
        fields,
        unique_constraints: Vec::new(),
        order_by: None,
        targets: Vec::new(),
        env_overrides: IndexMap::new(),
    };
    (tschema, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Row, SourceLocation, Table, TypedValue};

    fn typed(v: Value) -> TypedValue {
        TypedValue::new(v, SourceLocation::new("<test>"))
    }

    fn table(name: &str, rows: Vec<Vec<(String, Value)>>) -> Table {
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(idx, fields)| Row {
                primary_key: Vec::new(),
                fields: fields.into_iter().map(|(k, v)| (k, typed(v))).collect(),
                location: SourceLocation::new("<test>"),
                index: idx,
            })
            .collect();
        Table {
            name: name.to_string(),
            primary_key_fields: Vec::new(),
            rows,
            source_file: "t.xlsx".to_string(),
            sheet: Some(name.to_string()),
        }
    }

    fn doc(tables: Vec<Table>) -> Document {
        let mut d = Document::new();
        for t in tables {
            d.tables.insert(t.name.clone(), t);
        }
        d
    }

    #[test]
    fn integer_columns_infer_int_and_bounds() {
        let d = doc(vec![table(
            "Monster",
            vec![
                vec![
                    ("id".into(), Value::Int(1)),
                    ("level".into(), Value::Int(3)),
                ],
                vec![
                    ("id".into(), Value::Int(2)),
                    ("level".into(), Value::Int(7)),
                ],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let t = &inferred.schema.tables["Monster"];
        assert_eq!(t.fields["id"].field_type, FieldType::UInt32);
        assert!(t.fields["id"].required);
        assert_eq!(t.fields["id"].min, Some(1.0));
        assert_eq!(t.fields["id"].max, Some(2.0));
        assert_eq!(t.fields["level"].field_type, FieldType::UInt32);
        assert_eq!(t.primary_key, vec!["id"]);
    }

    #[test]
    fn negative_integers_infer_signed_and_nullable_tracks_blank_cells() {
        let d = doc(vec![table(
            "Delta",
            vec![
                vec![("a".into(), Value::Int(-5)), ("b".into(), Value::Int(1))],
                vec![("a".into(), Value::Int(0)), ("b".into(), Value::Null)],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let t = &inferred.schema.tables["Delta"];
        assert_eq!(t.fields["a"].field_type, FieldType::Int32);
        assert_eq!(t.fields["a"].min, Some(-5.0));
        assert!(t.fields["a"].required);
        // b has one blank cell: not required, nullable.
        assert!(!t.fields["b"].required);
        assert!(t.fields["b"].nullable);
    }

    #[test]
    fn floats_and_mixed_numerics_infer_float64() {
        let d = doc(vec![table(
            "Rate",
            vec![
                vec![("x".into(), Value::Float(1.5))],
                vec![("x".into(), Value::Float(2.25))],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let f = &inferred.schema.tables["Rate"].fields["x"];
        assert_eq!(f.field_type, FieldType::Float64);
        assert_eq!(f.min, Some(1.5));
        assert_eq!(f.max, Some(2.25));
    }

    #[test]
    fn low_cardinality_strings_become_inline_enums() {
        let d = doc(vec![table(
            "Item",
            vec![
                vec![("kind".into(), Value::String("weapon".into()))],
                vec![("kind".into(), Value::String("armor".into()))],
                vec![("kind".into(), Value::String("weapon".into()))],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let f = &inferred.schema.tables["Item"].fields["kind"];
        assert_eq!(f.field_type, FieldType::String);
        assert_eq!(
            f.enum_values.as_deref(),
            Some(&["weapon".to_string(), "armor".to_string()][..])
        );
        assert!(inferred.warnings.iter().any(|w| w.contains("kind")));
    }

    #[test]
    fn high_cardinality_strings_stay_string_without_enum() {
        let d = doc(vec![table(
            "Log",
            vec![
                vec![("tag".into(), Value::String("a".into()))],
                vec![("tag".into(), Value::String("b".into()))],
                vec![("tag".into(), Value::String("c".into()))],
                vec![("tag".into(), Value::String("d".into()))],
                vec![("tag".into(), Value::String("e".into()))],
            ],
        )]);
        // Threshold 3: four distinct strings stay a plain String.
        let inferred = infer_schema(&d, &InferOptions { enum_threshold: 3 }).unwrap();
        let f = &inferred.schema.tables["Log"].fields["tag"];
        assert_eq!(f.field_type, FieldType::String);
        assert_eq!(f.enum_values, None);
    }

    #[test]
    fn bools_infer_bool() {
        let d = doc(vec![table(
            "Flag",
            vec![
                vec![("on".into(), Value::Bool(true))],
                vec![("on".into(), Value::Bool(false))],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        assert_eq!(
            inferred.schema.tables["Flag"].fields["on"].field_type,
            FieldType::Bool
        );
    }

    #[test]
    fn mixed_kinds_fall_back_to_string_with_a_warning() {
        let d = doc(vec![table(
            "Odd",
            vec![
                vec![("v".into(), Value::Int(1))],
                vec![("v".into(), Value::String("x".into()))],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let f = &inferred.schema.tables["Odd"].fields["v"];
        assert_eq!(f.field_type, FieldType::String);
        assert!(inferred.warnings.iter().any(|w| w.contains("混型")));
    }

    #[test]
    fn all_blank_column_is_null_and_nullable() {
        let d = doc(vec![table(
            "Ghost",
            vec![
                vec![("e".into(), Value::Null)],
                vec![("e".into(), Value::Null)],
            ],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let f = &inferred.schema.tables["Ghost"].fields["e"];
        assert_eq!(f.field_type, FieldType::Null);
        assert!(!f.required);
        assert!(f.nullable);
    }

    #[test]
    fn empty_and_duplicate_headers_get_synthesized_names() {
        // A blank header gets a synthesized name (a Row's fields map already
        // collapses duplicate column names, so the dedupe suffix in
        // field_name is defensive rather than reachable through real data).
        let d = doc(vec![table(
            "Weird",
            vec![vec![
                ("id".into(), Value::Int(1)),
                ("   ".into(), Value::Int(2)),
            ]],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let names: Vec<&String> = inferred.schema.tables["Weird"].fields.keys().collect();
        assert_eq!(names, vec!["id", "col_2"]);
    }

    #[test]
    fn empty_document_is_a_valid_draft() {
        let inferred = infer_schema(&doc(vec![]), &InferOptions::default()).unwrap();
        assert!(inferred.schema.tables.is_empty());
        assert!(inferred.schema.enums.is_empty());
    }

    #[test]
    fn json_round_trip_of_the_draft_schema() {
        let d = doc(vec![table(
            "Monster",
            vec![vec![
                ("id".into(), Value::Int(1)),
                ("hp".into(), Value::Int(100)),
            ]],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let v = serde_json::to_value(&inferred.schema).unwrap();
        assert_eq!(
            v["tables"]["Monster"]["fields"]["id"]["type"]["kind"],
            "UInt32"
        );
        let back: Schema = serde_json::from_value(v).unwrap();
        assert_eq!(
            back.tables["Monster"].fields["id"].field_type,
            FieldType::UInt32
        );
    }

    #[test]
    fn the_draft_serializes_without_editor_contract_breakage() {
        // The inferred schema must pass through the editor JSON bridge the
        // same way any saved schema does (P1-1: one schema shape).
        let d = doc(vec![table(
            "Monster",
            vec![vec![("id".into(), Value::Int(1))]],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        let editor = crate::edit::to_editor_json(&inferred.schema);
        let round: Schema = crate::edit::from_editor_json(&serde_json::to_string(&editor).unwrap())
            .expect("draft must parse back into a Schema");
        assert_eq!(
            round.tables["Monster"].fields["id"].field_type,
            FieldType::UInt32
        );
    }

    #[test]
    fn uint64_columns_stay_unsigned() {
        let big = u64::MAX;
        let d = doc(vec![table(
            "Big",
            vec![vec![("u".into(), Value::UInt(big))]],
        )]);
        let inferred = infer_schema(&d, &InferOptions::default()).unwrap();
        assert_eq!(
            inferred.schema.tables["Big"].fields["u"].field_type,
            FieldType::UInt64
        );
    }
}
