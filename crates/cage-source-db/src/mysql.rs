//! MySQL backend glue (S2, design §45): connect from the env-carried
//! DSN, pin the session read-only (the second insurance — the first is
//! the static whitelist in `crate::validate_select`), run the statement
//! over the text protocol and map the typed values into the
//! backend-neutral row set.

use crate::{DbValue, RowSet};
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
