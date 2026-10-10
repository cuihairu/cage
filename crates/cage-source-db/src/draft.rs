//! Schema draft from table introspection (design §45 deferred item):
//! a live database's columns become a reviewable schema draft. The
//! structural skeleton is mechanical — tables, primary keys, NOT NULL →
//! `required` — while the semantics (ranges, patterns, enum domains,
//! cross-table references) stay with the author, surfaced as inline
//! comments where the server's own type carries a decision (DECIMAL
//! precision, JSON shape, temporal formats). Deterministic by
//! construction: the rendered bytes depend only on the introspected
//! shape, never on wall time — no timestamp in the header, same
//! discipline as every other cage artifact.

use cage_core::error::codes::remote::E1901;

/// The canonical column family a backend classifier lands on. The cage
/// type follows from it; the dialect's own spelling (`display` on
/// [`ColumnInfo`]) rides along only as comment material, never parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    /// DECIMAL / NUMERIC — cage-side String (the adapter hands it over
    /// as exact text); the draft comments on the precision decision.
    Decimal,
    /// char / varchar / text and everything textual the adapter
    /// delivers as opaque text (uuid, enum, set, custom scalars)
    Text,
    /// date / time / timestamp families — ISO text in the cached JSON
    Temporal,
    /// json / jsonb — opaque text in the cached JSON
    Json,
    /// BLOB / BYTEA / binary families — base64 in the cached JSON
    Bytes,
    /// anything the classifier cannot place — String, flagged for review
    Unknown,
}

/// One introspected column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub kind: ColumnKind,
    /// The dialect's own spelling (`int unsigned`, `numeric(10,2)`,
    /// `timestamp without time zone`) — comment material, never parsed.
    pub display: String,
    pub nullable: bool,
}

/// One introspected table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
}

/// The cage type kind a column family drafts as. DECIMAL / temporal /
/// JSON columns draft as String because that is exactly what the adapter
/// puts in the cached JSON — the draft must validate against its own
/// pipeline, not against the server's world view.
pub fn cage_kind(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Bool => "Bool",
        ColumnKind::Int8 => "Int8",
        ColumnKind::Int16 => "Int16",
        ColumnKind::Int32 => "Int32",
        ColumnKind::Int64 => "Int64",
        ColumnKind::UInt8 => "UInt8",
        ColumnKind::UInt16 => "UInt16",
        ColumnKind::UInt32 => "UInt32",
        ColumnKind::UInt64 => "UInt64",
        ColumnKind::Float32 => "Float32",
        ColumnKind::Float64 => "Float64",
        ColumnKind::Decimal | ColumnKind::Text | ColumnKind::Temporal | ColumnKind::Json => {
            "String"
        }
        ColumnKind::Bytes => "Bytes",
        ColumnKind::Unknown => "String",
    }
}

/// The inline review comment a column family earns, or `None` when the
/// cage type is the whole story.
fn comment(kind: ColumnKind, display: &str) -> Option<String> {
    match kind {
        ColumnKind::Decimal => Some(format!(
            "{display} — exact text; Float64 if precision loss is acceptable"
        )),
        ColumnKind::Temporal => Some(format!(
            "{display} — ISO text; range constraints are the Schema's call"
        )),
        ColumnKind::Json => Some(format!(
            "{display} — opaque text; nested shape checks live in the Schema"
        )),
        ColumnKind::Unknown => Some(format!("unrecognized column type '{display}' — review")),
        _ => None,
    }
}

/// A bare YAML token when the name is safe, single-quoted (with `''`
/// doubling) otherwise — column and table names come from the server
/// and are data, not code.
fn yaml_token(name: &str) -> String {
    let safe = {
        let mut chars = name.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    if safe {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Render the draft YAML: a header naming the source specs, one table
/// block per introspected table in the order the specs were given,
/// inline comments where the server's type carries a decision the
/// author must own. Byte-deterministic — same introspection, same
/// bytes, forever.
pub fn render_draft(specs: &[String], tables: &[TableInfo]) -> String {
    let mut out = String::new();
    out.push_str("# schema draft — generated from table introspection (");
    out.push_str(&specs.join(", "));
    out.push_str(")\n");
    out.push_str(
        "# review constraints (min / max / pattern / enum_values / references) before adopting\n",
    );
    if tables.is_empty() {
        // Unreachable through the CLI (specs are required), but the
        // renderer stays total.
        out.push_str("tables: {}\nenums: {}\n");
        return out;
    }
    out.push_str("tables:\n");
    for table in tables {
        let name = yaml_token(&table.name);
        out.push_str(&format!("  {name}:\n"));
        out.push_str(&format!("    name: {name}\n"));
        if table.primary_key.is_empty() {
            // `primary_key` is a required DSL field; an empty list is the
            // legal "no primary key" shape, and the comment says the
            // author must own that decision.
            out.push_str(
                "    # no primary key found on the server — declare primary_key before adopting\n",
            );
            out.push_str("    primary_key: []\n");
        } else {
            let keys: Vec<String> = table.primary_key.iter().map(|k| yaml_token(k)).collect();
            out.push_str(&format!("    primary_key: [{}]\n", keys.join(", ")));
        }
        out.push_str("    fields:\n");
        for column in &table.columns {
            let cname = yaml_token(&column.name);
            let required = if column.nullable {
                ""
            } else {
                ", required: true"
            };
            let line = format!(
                "      {cname}: {{ name: {cname}, type: {{ kind: {} }}{required} }}",
                cage_kind(column.kind)
            );
            match comment(column.kind, &column.display) {
                Some(note) => out.push_str(&format!("{line} # {note}\n")),
                None => out.push_str(&format!("{line}\n")),
            }
        }
    }
    out.push_str("enums: {}\n");
    out
}

/// Column count sanity shared by the backends: an introspection that
/// returns no columns means the table does not exist (or is not
/// visible) — a lookup failure, not empty data.
pub(crate) fn not_found(scheme: &str, table: &str) -> String {
    format!("{E1901} table '{table}' not found on the {scheme} server")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::schema::Schema;

    #[test]
    fn cage_kind_strings_cover_the_whole_family() {
        assert_eq!(cage_kind(ColumnKind::Bool), "Bool");
        assert_eq!(cage_kind(ColumnKind::UInt8), "UInt8");
        assert_eq!(cage_kind(ColumnKind::Int64), "Int64");
        assert_eq!(cage_kind(ColumnKind::Float64), "Float64");
        // Textual families all draft as String — that is what the cached
        // JSON actually holds.
        for textual in [
            ColumnKind::Decimal,
            ColumnKind::Text,
            ColumnKind::Temporal,
            ColumnKind::Json,
            ColumnKind::Unknown,
        ] {
            assert_eq!(cage_kind(textual), "String", "{textual:?}");
        }
        assert_eq!(cage_kind(ColumnKind::Bytes), "Bytes");
    }

    #[test]
    fn comments_fire_only_where_a_decision_is_needed() {
        assert_eq!(
            comment(ColumnKind::Decimal, "decimal(10,2)").as_deref(),
            Some("decimal(10,2) — exact text; Float64 if precision loss is acceptable")
        );
        assert_eq!(
            comment(ColumnKind::Temporal, "timestamp").as_deref(),
            Some("timestamp — ISO text; range constraints are the Schema's call")
        );
        assert_eq!(
            comment(ColumnKind::Json, "json").as_deref(),
            Some("json — opaque text; nested shape checks live in the Schema")
        );
        assert_eq!(
            comment(ColumnKind::Unknown, "geometry").as_deref(),
            Some("unrecognized column type 'geometry' — review")
        );
        assert_eq!(comment(ColumnKind::Int32, "int"), None);
        assert_eq!(comment(ColumnKind::Text, "varchar(64)"), None);
        assert_eq!(comment(ColumnKind::Bytes, "blob"), None);
        assert_eq!(comment(ColumnKind::Bool, "bool"), None);
    }

    #[test]
    fn yaml_token_quotes_unsafe_names_and_doubles_quotes() {
        assert_eq!(yaml_token("plain_id"), "plain_id");
        assert_eq!(yaml_token("A1"), "A1");
        assert_eq!(yaml_token("user name"), "'user name'");
        assert_eq!(yaml_token("9lives"), "'9lives'", "leading digit");
        assert_eq!(yaml_token("it's"), "'it''s'");
        assert_eq!(yaml_token("a-b"), "'a-b'", "dash is not bare-safe");
    }

    fn column(name: &str, kind: ColumnKind, display: &str, nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            kind,
            display: display.to_string(),
            nullable,
        }
    }

    fn items_table() -> TableInfo {
        TableInfo {
            name: "Items".to_string(),
            columns: vec![
                column("id", ColumnKind::Int32, "int", false),
                column("title", ColumnKind::Text, "varchar(64)", true),
                column("price", ColumnKind::Decimal, "decimal(10,2)", true),
            ],
            primary_key: vec!["id".to_string()],
        }
    }

    #[test]
    fn render_draft_matches_the_documented_shape_byte_for_byte() {
        let specs = vec!["mysql:db.Items".to_string()];
        let text = render_draft(&specs, &[items_table()]);
        let expected = "\
# schema draft — generated from table introspection (mysql:db.Items)
# review constraints (min / max / pattern / enum_values / references) before adopting
tables:
  Items:
    name: Items
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      title: { name: title, type: { kind: String } }
      price: { name: price, type: { kind: String } } # decimal(10,2) — exact text; Float64 if precision loss is acceptable
enums: {}
";
        assert_eq!(text, expected, "byte-for-byte shape");
        // Render twice — pure function, identical bytes.
        assert_eq!(render_draft(&specs, &[items_table()]), text);
    }

    /// The draft is a *schema*: the rendered YAML must parse back into
    /// the core Schema type with the expected shapes — this is the gate
    /// that keeps the renderer and the DSL honest about each other.
    #[test]
    fn rendered_draft_parses_back_into_a_schema() {
        let no_pk = TableInfo {
            name: "Logs".to_string(),
            columns: vec![
                column("at", ColumnKind::Temporal, "timestamp", false),
                column("payload", ColumnKind::Json, "json", true),
            ],
            primary_key: Vec::new(),
        };
        let text = render_draft(
            &["mysql:Items".to_string(), "pg:Logs".to_string()],
            &[items_table(), no_pk],
        );
        let schema: Schema = serde_yaml::from_str(&text).expect("draft must parse as a Schema");
        assert_eq!(schema.tables.len(), 2);
        let items = schema.tables.get("Items").unwrap();
        assert_eq!(items.primary_key, vec!["id"]);
        assert_eq!(items.fields.len(), 3);
        let id = items.fields.get("id").unwrap();
        assert!(id.required, "NOT NULL drafts as required");
        let title = items.fields.get("title").unwrap();
        assert!(!title.required, "nullable drafts without required");
        let logs = schema.tables.get("Logs").unwrap();
        assert!(logs.primary_key.is_empty());
        assert_eq!(logs.fields.get("at").unwrap().name, "at");
        assert_eq!(schema.enums.len(), 0);
    }

    /// A quoted table key is data: the renderer quotes it, and the YAML
    /// still parses back with the server's spelling.
    #[test]
    fn unusual_names_survive_the_round_trip() {
        let weird = TableInfo {
            name: "order by".to_string(),
            columns: vec![column("9col", ColumnKind::Int64, "bigint", false)],
            primary_key: vec!["9col".to_string()],
        };
        let text = render_draft(&["pg:odd".to_string()], &[weird]);
        let schema: Schema = serde_yaml::from_str(&text).unwrap();
        assert!(schema.tables.contains_key("order by"));
        let table = schema.tables.get("order by").unwrap();
        assert_eq!(table.primary_key, vec!["9col"]);
    }

    #[test]
    fn not_found_is_an_e1901_lookup_failure() {
        let e = not_found("mysql", "Ghost");
        assert!(e.contains("E1901"), "{e}");
        assert!(e.contains("Ghost"), "{e}");
    }
}
