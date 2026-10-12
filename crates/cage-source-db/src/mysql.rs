//! MySQL backend glue (S2, design §45): connect from the env-carried
//! DSN, pin the session read-only (the second insurance — the first is
//! the static whitelist in `crate::validate_select`), run the statement
//! over the text protocol and map the typed values into the
//! backend-neutral row set. The schema-draft helper (design §45
//! deferred item) rides the same discipline: `information_schema`
//! SELECTs with bound parameters over a read-only session.

use crate::draft::{not_found, ColumnInfo, ColumnKind, TableInfo};
use crate::{split_qualified, DbValue, RowSet};
use cage_core::error::codes::remote::E1901;
use mysql::consts::ColumnType;
use mysql::prelude::Queryable;
use mysql::Value;

/// Fetch over the process-wide pool for `dsn` (`crate::pool`): one
/// connection per server instead of one per statement, the read-only
/// pin re-applied on every acquired session.
pub(crate) fn fetcher(dsn: &str) -> impl FnMut(&str) -> Result<RowSet, String> {
    let dsn = dsn.to_string();
    move |sql| fetch(&dsn, sql)
}

/// Column metadata for the draft: `COLUMN_TYPE` carries the full
/// spelling (`int unsigned`, `tinyint(1)`, `decimal(10,2)`) — the one
/// string that classifies the family and feeds the review comment.
const DRAFT_COLUMNS_SQL: &str =
    "SELECT column_name, column_type, is_nullable FROM information_schema.columns \
WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? ORDER BY ordinal_position";

const DRAFT_PRIMARY_KEY_SQL: &str = "SELECT column_name FROM information_schema.key_column_usage \
WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND constraint_name = 'PRIMARY' \
ORDER BY ordinal_position";

/// Introspect one table's columns and primary key over a read-only
/// pooled session. The table name never splices into SQL — it binds as
/// a parameter; an unqualified name resolves against the DSN's current
/// database (`DATABASE()`), a qualified one against the named schema.
pub(crate) fn introspect(dsn: &str, table: &str) -> Result<TableInfo, String> {
    let pool = crate::pool::mysql_pool(dsn)?;
    let mut conn = pool
        .get_conn()
        .map_err(|e| format!("{E1901} cannot connect to mysql: {e}"))?;
    conn.query_drop("SET SESSION TRANSACTION READ ONLY")
        .map_err(|e| format!("{E1901} cannot pin mysql session read-only: {e}"))?;
    let (schema, bare) = split_qualified(table);
    let rows: Vec<(String, String, String)> = conn
        .exec(DRAFT_COLUMNS_SQL, (schema.as_deref(), bare))
        .map_err(|e| format!("{E1901} mysql introspection failed: {e}"))?;
    if rows.is_empty() {
        return Err(not_found("mysql", table));
    }
    let columns = rows
        .into_iter()
        .map(|(name, column_type, nullable)| {
            let kind = classify(&column_type);
            ColumnInfo {
                display: column_type,
                kind,
                nullable: nullable.eq_ignore_ascii_case("YES"),
                name,
            }
        })
        .collect();
    let primary_key: Vec<String> = conn
        .exec(DRAFT_PRIMARY_KEY_SQL, (schema.as_deref(), bare))
        .map_err(|e| format!("{E1901} mysql introspection failed: {e}"))?
        .into_iter()
        .map(|(name,)| name)
        .collect();
    Ok(TableInfo {
        name: table.to_string(),
        columns,
        primary_key,
    })
}

/// `COLUMN_TYPE` spelling → canonical family. `tinyint(1)` is the MySQL
/// bool convention; the `unsigned` suffix lifts the width family;
/// display widths (`int(11)`) are ignored — the base type decides.
pub(crate) fn classify(column_type: &str) -> ColumnKind {
    let lower = column_type.trim().to_ascii_lowercase();
    let unsigned = lower.ends_with(" unsigned");
    let core = lower.strip_suffix(" unsigned").unwrap_or(&lower);
    let base = core.split('(').next().unwrap_or(core).trim();
    match base {
        "tinyint" if core.starts_with("tinyint(1)") => ColumnKind::Bool,
        "tinyint" if unsigned => ColumnKind::UInt8,
        "tinyint" => ColumnKind::Int8,
        "smallint" if unsigned => ColumnKind::UInt16,
        "smallint" => ColumnKind::Int16,
        "mediumint" | "int" | "integer" if unsigned => ColumnKind::UInt32,
        "mediumint" | "int" | "integer" => ColumnKind::Int32,
        "bigint" if unsigned => ColumnKind::UInt64,
        "bigint" => ColumnKind::Int64,
        "bool" | "boolean" => ColumnKind::Bool,
        "float" => ColumnKind::Float32,
        "double" | "real" => ColumnKind::Float64,
        "decimal" | "dec" | "fixed" | "numeric" => ColumnKind::Decimal,
        "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" => ColumnKind::Text,
        "date" | "datetime" | "timestamp" | "time" | "year" | "newdate" => ColumnKind::Temporal,
        "json" => ColumnKind::Json,
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" | "bit"
        | "geometry" => ColumnKind::Bytes,
        _ => ColumnKind::Unknown,
    }
}

fn fetch(dsn: &str, sql: &str) -> Result<RowSet, String> {
    let pool = crate::pool::mysql_pool(dsn)?;
    let mut conn = pool
        .get_conn()
        .map_err(|e| format!("{E1901} cannot connect to mysql: {e}"))?;
    conn.query_drop("SET SESSION TRANSACTION READ ONLY")
        .map_err(|e| format!("{E1901} cannot pin mysql session read-only: {e}"))?;
    // `query_iter` streams rows off the wire one at a time — a large
    // table never materializes twice (driver buffer + collected Vec)
    // the way `query` would.
    let result = conn
        .query_iter(sql)
        .map_err(|e| format!("{E1901} mysql query failed: {e}"))?;
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<DbValue>> = Vec::new();
    for row in result {
        let row = row.map_err(|e| format!("{E1901} mysql query failed: {e}"))?;
        if columns.is_empty() {
            columns = row
                .columns_ref()
                .iter()
                .map(|c| c.name_str().into_owned())
                .collect();
        }
        let mut cells = Vec::with_capacity(row.len());
        for idx in 0..row.len() {
            let column = &row.columns_ref()[idx];
            let value = row.as_ref(idx).unwrap_or(&Value::NULL);
            cells.push(map_cell(column, value));
        }
        rows.push(cells);
    }
    Ok(RowSet { columns, rows })
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn map_cell(column: &mysql::Column, value: &Value) -> DbValue {
    if matches!(value, Value::NULL) {
        return DbValue::Null;
    }
    let ty = column.column_type();
    // charset 63 is the binary collation — those "string" columns carry
    // raw bytes, not text. Temporal types are excluded: the wire marks
    // them 63 too, but they arrive as their textual spelling (the text
    // protocol sends them as bytes) and the discipline keeps dates as
    // text — pinned by the live round-trip test.
    let binary = !matches!(
        ty,
        ColumnType::MYSQL_TYPE_DATE
            | ColumnType::MYSQL_TYPE_DATETIME
            | ColumnType::MYSQL_TYPE_TIMESTAMP
            | ColumnType::MYSQL_TYPE_TIME
            | ColumnType::MYSQL_TYPE_NEWDATE
            | ColumnType::MYSQL_TYPE_YEAR
    ) && (column.character_set() == 63
        || matches!(
            ty,
            ColumnType::MYSQL_TYPE_BIT
                | ColumnType::MYSQL_TYPE_GEOMETRY
                | ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
        ));
    match (ty, value) {
        (_, Value::Int(i)) => DbValue::Int(*i),
        (_, Value::UInt(u)) => DbValue::UInt(*u),
        (_, Value::Float(f)) => DbValue::Float(f64::from(*f)),
        (_, Value::Double(f)) => DbValue::Float(*f),
        (ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL, Value::Bytes(b)) => {
            DbValue::Decimal(utf8(b))
        }
        (ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_YEAR, Value::Date(y, m, d, ..)) => {
            DbValue::Text(format!("{y:04}-{m:02}-{d:02}"))
        }
        (
            ColumnType::MYSQL_TYPE_DATETIME
            | ColumnType::MYSQL_TYPE_TIMESTAMP
            | ColumnType::MYSQL_TYPE_NEWDATE,
            Value::Date(y, mo, d, h, mi, s, us),
        ) => {
            let base = format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
            if *us > 0 {
                DbValue::Text(format!("{base}.{us:06}"))
            } else {
                DbValue::Text(base)
            }
        }
        (ColumnType::MYSQL_TYPE_TIME, Value::Time(neg, days, h, mi, s, us)) => {
            let hours = u64::from(*days) * 24 + u64::from(*h);
            let sign = if *neg { "-" } else { "" };
            let base = format!("{sign}{hours:02}:{mi:02}:{s:02}");
            if *us > 0 {
                DbValue::Text(format!("{base}.{us:06}"))
            } else {
                DbValue::Text(base)
            }
        }
        (_, Value::Bytes(b)) => {
            if binary {
                DbValue::Bytes(b.clone())
            } else {
                DbValue::Text(utf8(b))
            }
        }
        _ => DbValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_the_column_type_families() {
        use ColumnKind::*;
        let cases = [
            ("int", Int32),
            ("int(11)", Int32),
            ("int unsigned", UInt32),
            ("tinyint(1)", Bool),
            ("tinyint", Int8),
            ("tinyint unsigned", UInt8),
            ("smallint", Int16),
            ("smallint unsigned", UInt16),
            ("mediumint", Int32),
            ("mediumint unsigned", UInt32),
            ("bigint", Int64),
            ("bigint unsigned", UInt64),
            ("boolean", Bool),
            ("float", Float32),
            ("double", Float64),
            ("real", Float64),
            ("decimal(10,2)", Decimal),
            ("numeric", Decimal),
            ("varchar(64)", Text),
            ("text", Text),
            ("longtext", Text),
            ("date", Temporal),
            ("datetime", Temporal),
            ("timestamp", Temporal),
            ("year", Temporal),
            ("json", Json),
            ("blob", Bytes),
            ("varbinary(16)", Bytes),
            ("geometry", Bytes),
            ("point", Unknown),
        ];
        for (hint, want) in cases {
            assert_eq!(classify(hint), want, "{hint}");
        }
    }

    #[test]
    fn classify_is_insensitive_to_case_and_padding() {
        assert_eq!(classify("  INT UNSIGNED "), ColumnKind::UInt32);
        assert_eq!(classify("BigInt"), ColumnKind::Int64);
        assert_eq!(classify("DATETIME"), ColumnKind::Temporal);
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
        assert!(DRAFT_COLUMNS_SQL.matches('?').count() == 2);
        assert!(DRAFT_PRIMARY_KEY_SQL.matches('?').count() == 2);
    }
}

/// Live-server tests (design §45 落地 2026-10-13): they run only when
/// `CAGE_DB_TEST_MYSQL_DSN` names a reachable database whose user may
/// create tables — otherwise they skip, keeping the gate server-free.
/// Self-contained: the schema is created and dropped per run, all on
/// unpinned pooled sessions (DDL needs write access; fetch sessions
/// pin themselves read-only).
#[cfg(test)]
mod live {
    use super::*;
    use crate::{pool::mysql_pool, DbSourceAdapter};
    use cage_core::manifest::{ProjectConfig, RemoteSourceConfig};
    use cage_core::value::Value;
    use std::time::Instant;

    fn live_dsn() -> Option<String> {
        std::env::var("CAGE_DB_TEST_MYSQL_DSN")
            .ok()
            .filter(|dsn| !dsn.trim().is_empty())
    }

    fn remote_config(queries: &[(&str, &str)]) -> RemoteSourceConfig {
        RemoteSourceConfig {
            dsn_env: Some("CAGE_DB_TEST_MYSQL_DSN".to_string()),
            credential_env: None,
            queries: queries
                .iter()
                .map(|(k, v)| (String::from(*k), String::from(*v)))
                .collect(),
        }
    }

    #[test]
    fn live_mysql_pooled_fetch_round_trips_pins_and_reuses() {
        let Some(dsn) = live_dsn() else {
            eprintln!("skip: CAGE_DB_TEST_MYSQL_DSN is not set");
            return;
        };
        let pool = mysql_pool(&dsn).unwrap();

        // Setup on unpinned pooled sessions — DDL needs write access.
        {
            let mut conn = pool.get_conn().unwrap();
            conn.query_drop("DROP TABLE IF EXISTS cage_live_items")
                .unwrap();
            conn.query_drop(
                "CREATE TABLE cage_live_items (\
                 id INT UNSIGNED PRIMARY KEY, price DECIMAL(10,2), note VARCHAR(64), \
                 raw VARBINARY(8), flag BOOLEAN, at DATETIME, spare INT NULL)",
            )
            .unwrap();
            conn.query_drop(
                "INSERT INTO cage_live_items VALUES \
                 (1, 9.99, 'sword', X'DEADBEEF', TRUE, '2026-10-13 12:00:00', NULL), \
                 (2, 3.50, 'potion', NULL, FALSE, '2026-01-02 03:04:05', 7)",
            )
            .unwrap();
        }

        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config.remote.insert(
            "mysql".to_string(),
            remote_config(&[("live_items", "SELECT * FROM cage_live_items ORDER BY id")]),
        );
        let sql = "SELECT * FROM cage_live_items ORDER BY id";

        let doc = DbSourceAdapter::load(tmp.path(), &config, "mysql:live_items", false).unwrap();
        let table = doc.tables.get("live_items").expect("live_items table");
        assert_eq!(table.rows.len(), 2);
        // The type discipline holds against a live server: DECIMAL
        // stays textual, NULL stays null, temporal arrives as text.
        let row = &table.rows[0];
        assert_eq!(row.fields["price"].value, Value::String("9.99".to_string()));
        assert_eq!(row.fields["note"].value, Value::String("sword".to_string()));
        assert_eq!(row.fields["spare"].value, Value::Null);
        assert_eq!(
            row.fields["at"].value,
            Value::String("2026-10-13 12:00:00".to_string())
        );

        // Determinism: a second load lands identical cache bytes.
        let key = format!("mysql\n{dsn}\n{sql}");
        let cache = cage_core::remote::source_cache_dir(tmp.path(), &key)
            .join(format!("{}.json", cage_core::remote::cache_key(&key)));
        let bytes1 = std::fs::read(&cache).unwrap();
        DbSourceAdapter::load(tmp.path(), &config, "mysql:live_items", false).unwrap();
        assert_eq!(bytes1, std::fs::read(&cache).unwrap(), "byte anchor stable");

        // Introspection rides the same pooled path. (The display is the
        // server's verbatim COLUMN_TYPE — MySQL spells `int unsigned`,
        // MariaDB `int(10) unsigned`; the family is what's stable.)
        let info = crate::introspect("mysql", &dsn, "cage_live_items").unwrap();
        assert_eq!(info.primary_key, vec!["id".to_string()]);
        assert_eq!(info.columns[0].kind, ColumnKind::UInt32);
        assert!(info.columns[0].display.contains("unsigned"));

        // Pooling: four more loads must not open new connections.
        let connections = |pool: &mysql::Pool| -> u64 {
            let mut conn = pool.get_conn().unwrap();
            let row: (String, String) = conn
                .query_first("SHOW GLOBAL STATUS LIKE 'Connections'")
                .unwrap()
                .unwrap();
            row.1.parse().unwrap()
        };
        let before = connections(&pool);
        for _ in 0..4 {
            DbSourceAdapter::load(tmp.path(), &config, "mysql:live_items", false).unwrap();
        }
        let opened = connections(&pool) - before;
        assert!(opened <= 1, "4 loads must reuse the pool: {opened} opened");

        // The second insurance is real on this server: the exact pin
        // the fetch path applies blocks writes on the session.
        let mut conn = pool.get_conn().unwrap();
        conn.query_drop("SET SESSION TRANSACTION READ ONLY")
            .unwrap();
        let ro: Option<u8> = conn
            .query_first("SELECT @@SESSION.transaction_read_only")
            .unwrap();
        assert_eq!(ro, Some(1), "the fetch-path pin holds on the session");
        let err = conn
            .query_drop("INSERT INTO cage_live_items VALUES (9, 1.00, 'x', NULL, 0, NOW(), NULL)")
            .unwrap_err();
        assert!(
            err.to_string().to_ascii_lowercase().contains("read only"),
            "pinned session must refuse writes: {err}"
        );

        let mut conn = pool.get_conn().unwrap();
        let _ = conn.query_drop("DROP TABLE IF EXISTS cage_live_items");
    }

    #[test]
    fn live_mysql_bulk_fetch_streams_and_round_trips() {
        let Some(dsn) = live_dsn() else {
            eprintln!("skip: CAGE_DB_TEST_MYSQL_DSN is not set");
            return;
        };
        let rows: u64 = std::env::var("CAGE_DB_TEST_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000);
        let pool = mysql_pool(&dsn).unwrap();
        {
            use std::fmt::Write as _;
            let mut conn = pool.get_conn().unwrap();
            conn.query_drop("DROP TABLE IF EXISTS cage_live_bulk")
                .unwrap();
            conn.query_drop(
                "CREATE TABLE cage_live_bulk (\
                 id INT UNSIGNED PRIMARY KEY, name VARCHAR(32), price DECIMAL(12,2))",
            )
            .unwrap();
            for batch in (0..rows).step_by(1000) {
                let mut sql = String::from("INSERT INTO cage_live_bulk VALUES ");
                for i in batch..(batch + 1000).min(rows) {
                    if i > batch {
                        sql.push(',');
                    }
                    let _ = write!(sql, "({i}, 'item{i}', {i}.25)");
                }
                conn.query_drop(&sql).unwrap();
            }
        }

        // One-shot (the fresh-connection cut): connect + auth + pin +
        // query — the handshake every fetch used to pay.
        let start = Instant::now();
        let opts = mysql::Opts::from_url(&dsn).unwrap();
        let mut conn = mysql::Conn::new(opts).unwrap();
        conn.query_drop("SET SESSION TRANSACTION READ ONLY")
            .unwrap();
        let one_shot_rows: Vec<mysql::Row> = conn.query("SELECT * FROM cage_live_bulk").unwrap();
        let one_shot = start.elapsed();
        assert_eq!(one_shot_rows.len() as u64, rows);

        // Warm bare fetch over the pool (no pipeline): same fetch as
        // the one-shot above, minus the handshake the pool eliminated.
        let start = Instant::now();
        let mut conn = pool.get_conn().unwrap();
        conn.query_drop("SET SESSION TRANSACTION READ ONLY")
            .unwrap();
        let bare_rows: Vec<mysql::Row> = conn.query("SELECT * FROM cage_live_bulk").unwrap();
        let bare = start.elapsed();
        assert_eq!(bare_rows.len() as u64, rows);

        // Pooled + streamed: the load path as the build runs it.
        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();
        config.remote.insert(
            "mysql".to_string(),
            remote_config(&[("live_bulk", "SELECT * FROM cage_live_bulk ORDER BY id")]),
        );
        let start = Instant::now();
        let doc = DbSourceAdapter::load(tmp.path(), &config, "mysql:live_bulk", false).unwrap();
        let pooled = start.elapsed();
        let table = doc.tables.get("live_bulk").expect("live_bulk table");
        assert_eq!(table.rows.len() as u64, rows);
        assert_eq!(
            table.rows[0].fields["name"].value,
            Value::String("item0".to_string())
        );
        assert_eq!(
            table.rows[(rows - 1) as usize].fields["name"].value,
            Value::String(format!("item{}", rows - 1))
        );

        eprintln!(
            "[mysql bulk {rows} rows] one-shot connect+fetch: {one_shot:?}; warm pooled bare fetch: {bare:?}; pooled full load: {pooled:?}"
        );

        let mut conn = pool.get_conn().unwrap();
        let _ = conn.query_drop("DROP TABLE IF EXISTS cage_live_bulk");
    }
}
