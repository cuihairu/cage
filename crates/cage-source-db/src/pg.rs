//! PostgreSQL backend glue (S2, design §45): connect from the env-carried
//! DSN, pin the session read-only (the second insurance — the first is
//! the static whitelist in `crate::validate_select`), then run the
//! statement over the text protocol. The column types come from one
//! `prepare` (the simple protocol carries no type info), the values from
//! `simple_query` as text — typed interpretation happens per the
//! declared type. NUMERIC stays verbatim text (no float round-trip),
//! bytea hex-decodes into bytes. The schema-draft helper (design §45
//! deferred item) rides the same discipline: `information_schema`
//! SELECTs with bound parameters over a read-only session.

use crate::draft::{not_found, ColumnInfo, ColumnKind, TableInfo};
use crate::{split_qualified, DbValue, RowSet};
use cage_core::error::codes::remote::E1901;
use postgres::types::Type;

/// Fetch over the process-wide pool for `dsn` (`crate::pool`): one
/// connection per server instead of one per statement, the read-only
/// pin re-applied on every acquired session.
pub(crate) fn fetcher(dsn: &str) -> impl FnMut(&str) -> Result<RowSet, String> {
    let dsn = dsn.to_string();
    move |sql| fetch(&dsn, sql)
}

/// Column metadata for the draft, all over the simple protocol (every
/// value arrives as text): `data_type` classifies the family, the
/// length/precision columns reassemble the dialect spelling for the
/// review comment.
const DRAFT_COLUMNS_SQL: &str = "SELECT column_name, data_type, character_maximum_length, \
numeric_precision, numeric_scale, is_nullable FROM information_schema.columns \
WHERE table_schema = COALESCE($1, current_schema()) AND table_name = $2 ORDER BY ordinal_position";

const DRAFT_PRIMARY_KEY_SQL: &str =
    "SELECT kcu.column_name FROM information_schema.table_constraints tc \
JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name \
AND tc.table_schema = kcu.table_schema \
WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = COALESCE($1, current_schema()) \
AND tc.table_name = $2 ORDER BY kcu.ordinal_position";

/// Introspect one table's columns and primary key over a read-only
/// pooled session. The table name never splices into SQL — it binds as
/// a parameter; an unqualified name resolves against the session's
/// current schema (`current_schema()`), a qualified one against the
/// named schema.
pub(crate) fn introspect(dsn: &str, table: &str) -> Result<TableInfo, String> {
    let pool = crate::pool::pg_pool(dsn)?;
    pool.with_conn(|client| {
        client
            .simple_query("SET default_transaction_read_only = on")
            .map_err(|e| format!("{E1901} cannot pin postgresql session read-only: {e}"))?;
        let (schema, bare) = split_qualified(table);
        let params: [&(dyn postgres::types::ToSql + Sync); 2] = [&schema, &bare];
        // The typed API (prepare + execute) is the parameterized route —
        // the simple protocol carries no parameter support. `information_
        // schema` columns decode as their declared text/integer types.
        let rows = client
            .query(DRAFT_COLUMNS_SQL, &params)
            .map_err(|e| format!("{E1901} postgresql introspection failed: {e}"))?;
        let mut columns = Vec::new();
        for row in &rows {
            let name: String = row.get(0);
            let data_type: String = row.get(1);
            let max_len: Option<i32> = row.get(2);
            let precision: Option<i32> = row.get(3);
            let scale: Option<i32> = row.get(4);
            let nullable: String = row.get(5);
            let (kind, display) = classify(
                &data_type,
                max_len.map(|v| v.max(0) as u32),
                precision.map(|v| v.max(0) as u32),
                scale.map(|v| v.max(0) as u32),
            );
            columns.push(ColumnInfo {
                name,
                kind,
                display,
                nullable: nullable.eq_ignore_ascii_case("YES"),
            });
        }
        if columns.is_empty() {
            return Err(not_found("postgresql", table));
        }
        let pk_rows = client
            .query(DRAFT_PRIMARY_KEY_SQL, &params)
            .map_err(|e| format!("{E1901} postgresql introspection failed: {e}"))?;
        let primary_key = pk_rows
            .iter()
            .map(|row| {
                let name: String = row.get(0);
                name
            })
            .collect();
        Ok(TableInfo {
            name: table.to_string(),
            columns,
            primary_key,
        })
    })
}

/// `information_schema.data_type` spelling (plus length / precision
/// columns) → canonical family and dialect display. Multi-word types
/// (`double precision`, `timestamp without time zone`) match on
/// prefixes; arrays and user-defined types land in `Unknown` — their
/// element shapes are a review decision, not an introspection detail.
pub(crate) fn classify(
    data_type: &str,
    max_len: Option<u32>,
    precision: Option<u32>,
    scale: Option<u32>,
) -> (ColumnKind, String) {
    let dt = data_type.trim().to_ascii_lowercase();
    let sized = |name: &str| match max_len {
        Some(n) => format!("{name}({n})"),
        None => name.to_string(),
    };
    let (kind, display) = match dt.as_str() {
        "smallint" => (ColumnKind::Int16, "smallint".to_string()),
        "integer" => (ColumnKind::Int32, "integer".to_string()),
        "bigint" => (ColumnKind::Int64, "bigint".to_string()),
        "real" => (ColumnKind::Float32, "real".to_string()),
        "double precision" => (ColumnKind::Float64, "double precision".to_string()),
        "numeric" | "decimal" => match (precision, scale) {
            (Some(p), Some(s)) => (ColumnKind::Decimal, format!("numeric({p},{s})")),
            _ => (ColumnKind::Decimal, "numeric".to_string()),
        },
        "boolean" => (ColumnKind::Bool, "boolean".to_string()),
        "bytea" => (ColumnKind::Bytes, "bytea".to_string()),
        "json" => (ColumnKind::Json, "json".to_string()),
        "jsonb" => (ColumnKind::Json, "jsonb".to_string()),
        "character varying" => (ColumnKind::Text, sized("varchar")),
        "character" | "char" => (ColumnKind::Text, sized("char")),
        "text" => (ColumnKind::Text, "text".to_string()),
        "uuid" => (ColumnKind::Text, "uuid".to_string()),
        "date" => (ColumnKind::Temporal, "date".to_string()),
        t if t.starts_with("timestamp") || t.starts_with("time") => {
            (ColumnKind::Temporal, t.to_string())
        }
        _ => (ColumnKind::Unknown, data_type.trim().to_string()),
    };
    (kind, display)
}

fn fetch(dsn: &str, sql: &str) -> Result<RowSet, String> {
    let pool = crate::pool::pg_pool(dsn)?;
    pool.with_conn(|client| {
        client
            .simple_query("SET default_transaction_read_only = on")
            .map_err(|e| format!("{E1901} cannot pin postgresql session read-only: {e}"))?;
        // The simple protocol's RowDescription carries names only — take the
        // declared types from a prepare of the same statement.
        let statement = client
            .prepare(sql)
            .map_err(|e| format!("{E1901} postgresql prepare failed: {e}"))?;
        let columns: Vec<(String, Type)> = statement
            .columns()
            .iter()
            .map(|c| (c.name().to_string(), c.type_().clone()))
            .collect();
        let messages = client
            .simple_query(sql)
            .map_err(|e| format!("{E1901} postgresql query failed: {e}"))?;
        let rows: Vec<postgres::SimpleQueryRow> = messages
            .into_iter()
            .filter_map(|m| match m {
                postgres::SimpleQueryMessage::Row(row) => Some(row),
                _ => None,
            })
            .collect();
        Ok(to_row_set(&columns, &rows))
    })
}

fn to_row_set(columns: &[(String, Type)], rows: &[postgres::SimpleQueryRow]) -> RowSet {
    let names: Vec<String> = columns.iter().map(|(name, _)| name.clone()).collect();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut cells = Vec::with_capacity(row.len());
        for (idx, (_, ty)) in columns.iter().enumerate() {
            cells.push(match row.get(idx) {
                None => DbValue::Null,
                Some(text) => map_text(ty, text),
            });
        }
        out.push(cells);
    }
    RowSet {
        columns: names,
        rows: out,
    }
}

/// Text-protocol values, typed by the column's declared type. Ints and
/// floats parse (malformed text falls back to Text — the Schema will
/// have the last word anyway); NUMERIC never parses. Custom types
/// (enums, arrays, JSON) land as their text representation — typed
/// interpretation is the Schema's job, not the adapter's.
fn map_text(ty: &Type, text: &str) -> DbValue {
    if *ty == Type::BOOL {
        DbValue::Bool(text == "t" || text == "true")
    } else if *ty == Type::INT2 || *ty == Type::INT4 || *ty == Type::INT8 {
        text.parse::<i64>()
            .map(DbValue::Int)
            .unwrap_or_else(|_| DbValue::Text(text.to_string()))
    } else if *ty == Type::OID {
        text.parse::<u64>()
            .map(DbValue::UInt)
            .unwrap_or_else(|_| DbValue::Text(text.to_string()))
    } else if *ty == Type::FLOAT4 || *ty == Type::FLOAT8 {
        text.parse::<f64>()
            .map(DbValue::Float)
            .unwrap_or_else(|_| DbValue::Text(text.to_string()))
    } else if *ty == Type::NUMERIC {
        DbValue::Decimal(text.to_string())
    } else if *ty == Type::BYTEA {
        hex_to_bytes(text)
            .map(DbValue::Bytes)
            .unwrap_or_else(|_| DbValue::Text(text.to_string()))
    } else {
        DbValue::Text(text.to_string())
    }
}

/// bytea renders as `\x<hex>` over the text protocol (hex format is the
/// default since PG 9.0); anything else passes through untouched.
fn hex_to_bytes(text: &str) -> Result<Vec<u8>, ()> {
    let hex = text.strip_prefix("\\x").ok_or(())?;
    if hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16).ok_or(())?;
        let lo = (pair[1] as char).to_digit(16).ok_or(())?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_the_data_type_families() {
        use ColumnKind::*;
        let cases = [
            ("smallint", Int16, "smallint"),
            ("integer", Int32, "integer"),
            ("bigint", Int64, "bigint"),
            ("real", Float32, "real"),
            ("double precision", Float64, "double precision"),
            ("boolean", Bool, "boolean"),
            ("bytea", Bytes, "bytea"),
            ("json", Json, "json"),
            ("jsonb", Json, "jsonb"),
            ("text", Text, "text"),
            ("uuid", Text, "uuid"),
            ("date", Temporal, "date"),
            (
                "timestamp without time zone",
                Temporal,
                "timestamp without time zone",
            ),
            ("time with time zone", Temporal, "time with time zone"),
            ("ARRAY", Unknown, "ARRAY"),
            ("USER-DEFINED", Unknown, "USER-DEFINED"),
        ];
        for (dt, want, display) in cases {
            let (kind, got_display) = classify(dt, None, None, None);
            assert_eq!(kind, want, "{dt}");
            assert_eq!(got_display, display, "{dt}");
        }
    }

    #[test]
    fn classify_reassembles_sized_spelling() {
        let (kind, display) = classify("character varying", Some(64), None, None);
        assert_eq!(kind, ColumnKind::Text);
        assert_eq!(display, "varchar(64)");
        let (kind, display) = classify("character", Some(8), None, None);
        assert_eq!(kind, ColumnKind::Text);
        assert_eq!(display, "char(8)");
        let (kind, display) = classify("numeric", None, Some(10), Some(2));
        assert_eq!(kind, ColumnKind::Decimal);
        assert_eq!(display, "numeric(10,2)");
        let (kind, display) = classify("numeric", None, None, None);
        assert_eq!(kind, ColumnKind::Decimal);
        assert_eq!(display, "numeric");
    }

    /// The draft's own introspection SQL must survive the same static
    /// whitelist user queries pass through — one discipline for
    /// everything that runs on the wire.
    #[test]
    fn draft_sql_passes_the_readonly_whitelist() {
        crate::validate_select(DRAFT_COLUMNS_SQL).unwrap();
        crate::validate_select(DRAFT_PRIMARY_KEY_SQL).unwrap();
        for sql in [DRAFT_COLUMNS_SQL, DRAFT_PRIMARY_KEY_SQL] {
            let upper = sql.to_ascii_uppercase();
            assert!(
                upper.contains("INFORMATION_SCHEMA"),
                "draft SQL must only read information_schema: {sql}"
            );
            assert!(!sql.contains(';'), "single statement only: {sql}");
        }
    }

    #[test]
    fn introspect_sql_binds_the_table_name() {
        // The user-supplied name appears only as a bound parameter:
        // both statements are `const` (no interpolation is even
        // possible) and carry the placeholder for the name.
        assert!(DRAFT_COLUMNS_SQL.matches("$1").count() == 1);
        assert!(DRAFT_COLUMNS_SQL.matches("$2").count() == 1);
    }
}

/// Live-server tests (design §45 落地 2026-10-13): they run only when
/// `CAGE_DB_TEST_PG_DSN` names a reachable database whose user may
/// create tables — otherwise they skip, keeping the gate server-free.
/// Self-contained: the schema is created and dropped per run, all on
/// unpinned pooled sessions (DDL needs write access; fetch sessions
/// pin themselves read-only).
#[cfg(test)]
mod live {
    use crate::pool::pg_pool;
    use crate::DbSourceAdapter;
    use cage_core::manifest::{ProjectConfig, RemoteSourceConfig};
    use cage_core::value::Value;
    use std::time::Instant;

    fn live_dsn() -> Option<String> {
        std::env::var("CAGE_DB_TEST_PG_DSN")
            .ok()
            .filter(|dsn| !dsn.trim().is_empty())
    }

    fn remote_config(queries: &[(&str, &str)]) -> RemoteSourceConfig {
        RemoteSourceConfig {
            dsn_env: Some("CAGE_DB_TEST_PG_DSN".to_string()),
            credential_env: None,
            queries: queries
                .iter()
                .map(|(k, v)| (String::from(*k), String::from(*v)))
                .collect(),
        }
    }

    #[test]
    fn live_pg_pooled_fetch_round_trips_pins_and_reuses() {
        let Some(dsn) = live_dsn() else {
            eprintln!("skip: CAGE_DB_TEST_PG_DSN is not set");
            return;
        };
        // Setup on unpinned direct sessions — DDL needs write access.
        {
            let mut client = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
            client
                .simple_query("DROP TABLE IF EXISTS cage_live_items")
                .unwrap();
            client
                .simple_query(
                    "CREATE TABLE cage_live_items (\
                     id INTEGER PRIMARY KEY, price NUMERIC(10,2), note VARCHAR(64), \
                     raw BYTEA, flag BOOLEAN, at TIMESTAMP, spare INTEGER)",
                )
                .unwrap();
            client
                .simple_query(
                    "INSERT INTO cage_live_items VALUES \
                     (1, 9.99, 'sword', '\\xdeadbeef'::bytea, TRUE, '2026-10-13 12:00:00', NULL), \
                     (2, 3.50, 'potion', NULL, FALSE, '2026-01-02 03:04:05', 7)",
                )
                .unwrap();
        }

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config.remote.insert(
            "pg".to_string(),
            remote_config(&[("live_items", "SELECT * FROM cage_live_items ORDER BY id")]),
        );
        let sql = "SELECT * FROM cage_live_items ORDER BY id";

        let doc = DbSourceAdapter::load(tmp.path(), &config, "pg:live_items", false).unwrap();
        let table = doc.tables.get("live_items").expect("live_items table");
        assert_eq!(table.rows.len(), 2);
        // The type discipline holds against a live server: NUMERIC
        // stays textual, BOOLEAN arrives as a real bool, NULL stays
        // null, bytea lands base64.
        let row = &table.rows[0];
        assert_eq!(row.fields["price"].value, Value::String("9.99".to_string()));
        assert_eq!(row.fields["note"].value, Value::String("sword".to_string()));
        assert_eq!(row.fields["flag"].value, Value::Bool(true));
        assert_eq!(row.fields["spare"].value, Value::Null);
        assert_eq!(
            row.fields["raw"].value,
            Value::String("3q2+7w==".to_string())
        );

        // Determinism: a second load lands identical cache bytes.
        let key = format!("pg\n{dsn}\n{sql}");
        let cache = cage_core::remote::source_cache_dir(tmp.path(), &key)
            .join(format!("{}.json", cage_core::remote::cache_key(&key)));
        let bytes1 = std::fs::read(&cache).unwrap();
        DbSourceAdapter::load(tmp.path(), &config, "pg:live_items", false).unwrap();
        assert_eq!(bytes1, std::fs::read(&cache).unwrap(), "byte anchor stable");

        // Introspection rides the same pooled path.
        let info = crate::introspect("pg", &dsn, "cage_live_items").unwrap();
        assert_eq!(info.primary_key, vec!["id".to_string()]);
        assert_eq!(info.columns[0].display, "integer");

        // Pooling: four more loads must not open new backends.
        let backends = |dsn: &str| -> i64 {
            let mut client = postgres::Client::connect(dsn, postgres::NoTls).unwrap();
            let row = client
                .query_one(
                    "SELECT numbackends FROM pg_stat_database WHERE datname = current_database()",
                    &[],
                )
                .unwrap();
            row.get::<_, i32>(0) as i64
        };
        let before = backends(&dsn);
        for _ in 0..4 {
            DbSourceAdapter::load(tmp.path(), &config, "pg:live_items", false).unwrap();
        }
        let opened = backends(&dsn) - before;
        assert!(opened <= 1, "4 loads must reuse the pool: {opened} opened");

        // The second insurance is real on this server: the exact pin
        // the fetch path applies blocks writes on the session.
        let mut client = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
        client
            .simple_query("SET default_transaction_read_only = on")
            .unwrap();
        let state = client
            .simple_query("SHOW transaction_read_only")
            .unwrap()
            .into_iter()
            .find_map(|m| match m {
                postgres::SimpleQueryMessage::Row(r) => Some(r.get(0).unwrap().to_string()),
                _ => None,
            })
            .unwrap();
        assert_eq!(state, "on", "the fetch-path pin holds on the session");
        let err = client
            .simple_query(
                "INSERT INTO cage_live_items VALUES (9, 1.00, 'x', NULL, FALSE, NOW(), NULL)",
            )
            .unwrap_err();
        let message = err
            .as_db_error()
            .map(|e| e.message().to_string())
            .unwrap_or_else(|| err.to_string());
        assert!(
            message.to_ascii_lowercase().contains("read-only"),
            "pinned session must refuse writes: {message}"
        );

        let mut client = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
        let _ = client.simple_query("DROP TABLE IF EXISTS cage_live_items");
    }

    #[test]
    fn live_pg_bulk_fetch_round_trips() {
        let Some(dsn) = live_dsn() else {
            eprintln!("skip: CAGE_DB_TEST_PG_DSN is not set");
            return;
        };
        let rows: i64 = std::env::var("CAGE_DB_TEST_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);
        {
            let mut client = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
            client
                .simple_query("DROP TABLE IF EXISTS cage_live_bulk")
                .unwrap();
            client
                .simple_query(
                    "CREATE TABLE cage_live_bulk (\
                     id INTEGER PRIMARY KEY, name VARCHAR(32), price NUMERIC(12,2))",
                )
                .unwrap();
            // One statement: the server materializes the series itself.
            client
                .simple_query(&format!(
                    "INSERT INTO cage_live_bulk \
                     SELECT i, 'item' || i::text, i::numeric + 0.25 \
                     FROM generate_series(0, {rows} - 1) AS t(i)"
                ))
                .unwrap();
        }

        // One-shot (the fresh-connection cut): connect + auth + pin +
        // query — the handshake every fetch used to pay.
        let start = Instant::now();
        let mut client = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
        client
            .simple_query("SET default_transaction_read_only = on")
            .unwrap();
        let one_shot_rows = client.query("SELECT * FROM cage_live_bulk", &[]).unwrap();
        let one_shot = start.elapsed();
        assert_eq!(one_shot_rows.len() as i64, rows);

        // Warm bare fetch over the pool (no pipeline): same fetch as
        // the one-shot above, minus the handshake the pool eliminated.
        let pool = pg_pool(&dsn).unwrap();
        let start = Instant::now();
        let bare_rows = pool
            .with_conn(|client| {
                client
                    .simple_query("SET default_transaction_read_only = on")
                    .map_err(|e| e.to_string())?;
                let rows = client
                    .query("SELECT * FROM cage_live_bulk", &[])
                    .map_err(|e| e.to_string())?;
                Ok(rows)
            })
            .unwrap();
        let bare = start.elapsed();
        assert_eq!(bare_rows.len() as i64, rows);

        // Pooled: the load path as the build runs it.
        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config.remote.insert(
            "pg".to_string(),
            remote_config(&[("live_bulk", "SELECT * FROM cage_live_bulk ORDER BY id")]),
        );
        let start = Instant::now();
        let doc = DbSourceAdapter::load(tmp.path(), &config, "pg:live_bulk", false).unwrap();
        let pooled = start.elapsed();
        let table = doc.tables.get("live_bulk").expect("live_bulk table");
        assert_eq!(table.rows.len() as i64, rows);
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("item0".to_string())
        );
        assert_eq!(
            table.rows[(rows - 1) as usize].fields["name"].value,
            Value::String(format!("item{}", rows - 1))
        );

        eprintln!(
            "[pg bulk {rows} rows] one-shot connect+fetch: {one_shot:?}; warm pooled bare fetch: {bare:?}; pooled full load: {pooled:?}"
        );

        let mut client = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
        let _ = client.simple_query("DROP TABLE IF EXISTS cage_live_bulk");
    }
}
