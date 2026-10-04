//! PostgreSQL backend glue (S2, design §45): connect from the env-carried
//! DSN, pin the session read-only (the second insurance — the first is
//! the static whitelist in `crate::validate_select`), then run the
//! statement over the text protocol. The column types come from one
//! `prepare` (the simple protocol carries no type info), the values from
//! `simple_query` as text — typed interpretation happens per the
//! declared type. NUMERIC stays verbatim text (no float round-trip),
//! bytea hex-decodes into bytes.

use crate::{DbValue, RowSet};
use cage_core::error::codes::remote::E1901;
use postgres::types::Type;

/// A fresh connection per fetch: no pooling in the first cut (design
/// §45 留待实现期), one statement per source per build.
pub(crate) fn fetcher(dsn: &str) -> impl FnMut(&str) -> Result<RowSet, String> {
    let dsn = dsn.to_string();
    move |sql| fetch(&dsn, sql)
}

fn fetch(dsn: &str, sql: &str) -> Result<RowSet, String> {
    let mut client = postgres::Client::connect(dsn, postgres::NoTls)
        .map_err(|e| format!("{E1901} cannot connect to postgresql: {e}"))?;
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
