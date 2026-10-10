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

/// A fresh connection per fetch: no pooling in the first cut (design
/// §45 留待实现期), one statement per source per build.
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
/// session. The table name never splices into SQL — it binds as a
/// parameter; an unqualified name resolves against the DSN's current
/// database (`DATABASE()`), a qualified one against the named schema.
pub(crate) fn introspect(dsn: &str, table: &str) -> Result<TableInfo, String> {
    let opts = mysql::Opts::from_url(dsn).map_err(|e| format!("{E1901} invalid mysql DSN: {e}"))?;
    let mut conn =
        mysql::Conn::new(opts).map_err(|e| format!("{E1901} cannot connect to mysql: {e}"))?;
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
    let opts = mysql::Opts::from_url(dsn).map_err(|e| format!("{E1901} invalid mysql DSN: {e}"))?;
    let mut conn =
        mysql::Conn::new(opts).map_err(|e| format!("{E1901} cannot connect to mysql: {e}"))?;
    conn.query_drop("SET SESSION TRANSACTION READ ONLY")
        .map_err(|e| format!("{E1901} cannot pin mysql session read-only: {e}"))?;
    let rows = conn
        .query(sql)
        .map_err(|e| format!("{E1901} mysql query failed: {e}"))?;
    Ok(to_row_set(&rows))
}

fn to_row_set(rows: &[mysql::Row]) -> RowSet {
    let columns: Vec<String> = rows
        .first()
        .map(|r| {
            r.columns_ref()
                .iter()
                .map(|c| c.name_str().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut cells = Vec::with_capacity(row.len());
        for idx in 0..row.len() {
            let column = &row.columns_ref()[idx];
            let value = row.as_ref(idx).unwrap_or(&Value::NULL);
            cells.push(map_cell(column, value));
        }
        out.push(cells);
    }
    RowSet { columns, rows: out }
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
    // raw bytes, not text.
    let binary = column.character_set() == 63
        || matches!(
            ty,
            ColumnType::MYSQL_TYPE_BIT
                | ColumnType::MYSQL_TYPE_GEOMETRY
                | ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
        );
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
