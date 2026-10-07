//! MySQL / PostgreSQL source adapter (S2, design §45): `mysql:<name>` /
//! `pg:<name>` resolves a named read-only query from
//! `[remote.<scheme>].queries` first, else expands the name as a table
//! (`SELECT * FROM <name>`). The statement passes a static read-only
//! whitelist at load time (E1905) and the session is pinned read-only
//! before anything runs — double insurance. The DSN comes from the env
//! var named in `[remote.<scheme>].dsn_env` (E1904 when missing; it
//! never lives in cage.toml).
//!
//! The row set serializes to canonical JSON and lands in the
//! project-local cache — the same byte anchor and the same JSON parsing
//! (`{表名: 行数组}` shape) as every other source, L0-L7 with no bypass.
//! Type discipline: NULL → Null, DECIMAL / NUMERIC → string (no Float
//! detour, precision is the Schema's call), binary columns → base64.
//! Rows without ORDER BY are sorted by their serialized form, so the
//! build never depends on the server's return order.

mod mysql;
mod pg;

use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::{E1904, E1905};
use cage_core::manifest::{ProjectConfig, RemoteSourceConfig};
use cage_core::remote;
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
use std::path::Path;

/// Backend-neutral cell value: one column of one row, before it becomes
/// JSON. Precision-bearing types stay textual by design.
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    /// SQL NULL
    Null,
    /// BOOLEAN / BOOL
    Bool(bool),
    /// Signed integer columns
    Int(i64),
    /// Unsigned integer columns
    UInt(u64),
    /// FLOAT / DOUBLE / REAL
    Float(f64),
    /// DECIMAL / NUMERIC — kept verbatim as text, no float round-trip
    Decimal(String),
    /// Textual columns (and everything textual the server sends back:
    /// dates, JSON, enums, arrays — typed interpretation is the
    /// Schema's job, not the adapter's)
    Text(String),
    /// BYTEA / BLOB — base64 in the cached JSON
    Bytes(Vec<u8>),
}

impl DbValue {
    /// Canonical JSON form: precision stays textual, binary is base64.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            DbValue::Null => serde_json::Value::Null,
            DbValue::Bool(b) => serde_json::Value::Bool(*b),
            DbValue::Int(i) => serde_json::Value::Number((*i).into()),
            DbValue::UInt(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
            DbValue::Float(f) => match serde_json::Number::from_f64(*f) {
                Some(n) => serde_json::Value::Number(n),
                None => serde_json::Value::Null,
            },
            DbValue::Decimal(s) | DbValue::Text(s) => serde_json::Value::String(s.clone()),
            DbValue::Bytes(b) => {
                use base64::Engine as _;
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b))
            }
        }
    }
}

/// Backend-neutral row set: columns in result order, one cell vector per
/// row. The shared mapping (sort → serialize → cache → parse) consumes
/// this, so both backends stay thin protocol glue.
#[derive(Debug, Clone)]
pub struct RowSet {
    /// Column names in result order
    pub columns: Vec<String>,
    /// One cell vector per row, aligned with `columns`
    pub rows: Vec<Vec<DbValue>>,
}

/// Split a `[source_roots]` value into (scheme, name): `mysql:Monsters`
/// or `pg:top_drops`. Anything else is not a DB source.
pub fn parse_spec(spec: &str) -> Option<(&str, &str)> {
    if let Some(rest) = spec.strip_prefix("mysql:") {
        return (!rest.is_empty()).then_some(("mysql", rest));
    }
    let rest = spec.strip_prefix("pg:")?;
    (!rest.is_empty()).then_some(("pg", rest))
}

/// Static read-only whitelist (the first of the two insurances): exactly
/// one statement, starting with SELECT — semicolons, comments, row-lock
/// clauses and `INTO` targets are rejected outright (E1905). The second
/// insurance is the session-wide read-only pin at connect time.
pub fn validate_select(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    let reject = |why: &str| {
        let preview: String = trimmed.chars().take(60).collect();
        format!("{E1905} remote query rejected (read-only SELECT only), {why}: {preview}")
    };
    if trimmed.is_empty() {
        return Err(reject("empty statement"));
    }
    if trimmed.contains(';') {
        return Err(reject("semicolon — single statement only"));
    }
    if trimmed.contains("--") || trimmed.contains("/*") || trimmed.contains("*/") {
        return Err(reject("comment"));
    }
    let upper = trimmed.to_ascii_uppercase();
    if !upper.starts_with("SELECT") {
        return Err(reject("statement must start with SELECT"));
    }
    if upper.contains("FOR UPDATE") || upper.contains("FOR SHARE") {
        return Err(reject("row-lock clause"));
    }
    if upper.contains(" INTO ") {
        return Err(reject("INTO clause"));
    }
    Ok(())
}

/// Whether `name` is safe to splice into `SELECT * FROM <name>`: a plain
/// identifier (schema-qualified with `.` allowed), nothing spiky.
fn valid_table_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$')
}

/// Resolve `name` to (sql, result-table-name): a named query from
/// `[remote.<scheme>].queries` wins, else the name must be a safe table
/// name and expands to `SELECT * FROM <name>`.
pub fn resolve_query(
    scheme: &str,
    name: &str,
    config: Option<&RemoteSourceConfig>,
) -> Result<(String, String), String> {
    if let Some(sql) = config.and_then(|c| c.queries.get(name)) {
        return Ok((sql.clone(), name.to_string()));
    }
    if !valid_table_name(name) {
        return Err(format!(
            "{E1905} '{name}' is neither a named query in [remote.{scheme}].queries nor a \
             safe table name"
        ));
    }
    Ok((format!("SELECT * FROM {name}"), name.to_string()))
}

/// Resolve the DSN from the env var named in
/// `[remote.<scheme>].dsn_env` — unset, empty, or undeclared is E1904;
/// the DSN itself is never read from the config.
pub fn resolve_dsn(scheme: &str, config: Option<&RemoteSourceConfig>) -> Result<String, String> {
    let env_name = config.and_then(|c| c.dsn_env.as_deref()).ok_or_else(|| {
        format!(
            "{E1904} [remote.{scheme}] has no dsn_env — declare the env var NAME in cage.toml; \
             the DSN itself comes from the environment"
        )
    })?;
    match std::env::var(env_name) {
        Ok(dsn) if !dsn.trim().is_empty() => Ok(dsn),
        _ => Err(format!(
            "{E1904} env {env_name} is not set ([remote.{scheme}].dsn_env)"
        )),
    }
}

/// Whether the statement carries its own ordering — if so the returned
/// row order is honored (the author promised determinism); otherwise the
/// shared mapping sorts by serialized form.
fn has_order_by(sql: &str) -> bool {
    sql.to_ascii_uppercase().contains("ORDER BY")
}

/// Serialize the row set canonically: an object per row with keys in
/// column order (the whole-cage order discipline — CSV / Excel / local
/// JSON all keep field order), the row list sorted by its serialized
/// form unless the statement orders explicitly. The output bytes depend
/// only on (column order, row content) — never on the server's return
/// order.
pub fn canonical_json(table_name: &str, row_set: &RowSet, preserve_row_order: bool) -> Vec<u8> {
    let mut rows: Vec<serde_json::Value> = row_set
        .rows
        .iter()
        .map(|cells| {
            let mut obj = serde_json::Map::new();
            for (col, val) in row_set.columns.iter().zip(cells) {
                obj.insert(col.clone(), val.to_json());
            }
            serde_json::Value::Object(obj)
        })
        .collect();
    if !preserve_row_order {
        // Our own serialization never fails, but sort needs a total key:
        // the default (empty string) is unreachable in practice.
        rows.sort_by_key(|row| serde_json::to_string(row).unwrap_or_default());
    }
    let mut doc = serde_json::Map::new();
    doc.insert(table_name.to_string(), serde_json::Value::Array(rows));
    serde_json::to_vec_pretty(&serde_json::Value::Object(doc))
        .expect("serialized map is valid JSON")
}

/// The shared pipeline once the query and DSN are settled: fetch →
/// canonical JSON → cache file → standard JSON parse. `key_input`
/// fingerprints the byte source (scheme, DSN, SQL) into the cache path,
/// so distinct servers/queries never share a slot. `fetch` is the thin
/// backend glue — everything around it is shared and tested. A fetch
/// failure is transport-class by construction (connect/query against a
/// live server), so with a previous copy in the cache slot it falls
/// back to that copy (E1906 WARNING) unless `strict` (`--no-cache`);
/// credential (E1904) and query-shape (E1905) problems never reach
/// here — they fire in `load` before any connection, where stale bytes
/// must not mask a configuration error.
pub fn materialize(
    project_root: &Path,
    table_name: &str,
    sql: &str,
    key_input: &str,
    mut fetch: impl FnMut(&str) -> Result<RowSet, String>,
    strict: bool,
) -> Result<Document, String> {
    let row_set = match fetch(sql) {
        Ok(row_set) => row_set,
        Err(e) => {
            // The warning carries the table name only — `key_input`
            // embeds the one-way-hashed DSN's source string, which may
            // include credentials.
            let cache_dir = remote::source_cache_dir(project_root, key_input);
            let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(key_input)));
            if let Some(path) = remote::source_cache_fallback(&cache_file, table_name, strict) {
                return JsonSourceAdapter::parse_file(&path).map_err(|diags| {
                    eprintln!("{}", diags.render(false));
                    format!("failed to parse row set for {table_name}")
                });
            }
            return Err(e);
        }
    };
    let bytes = canonical_json(table_name, &row_set, has_order_by(sql));

    let cache_dir = remote::source_cache_dir(project_root, key_input);
    let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(key_input)));
    std::fs::create_dir_all(&cache_dir).map_err(|e| {
        format!(
            "{E9902} cannot create source cache {}: {e}",
            cache_dir.display()
        )
    })?;
    std::fs::write(&cache_file, &bytes).map_err(|e| {
        format!(
            "{E9902} cannot write source cache {}: {e}",
            cache_file.display()
        )
    })?;
    JsonSourceAdapter::parse_file(&cache_file).map_err(|diags| {
        eprintln!("{}", diags.render(false));
        format!("failed to parse row set for {table_name}")
    })
}

/// The boxed backend fetch the dispatcher hands to `materialize`: SQL
/// in, row set out. The two drivers return distinct opaque closures, so
/// they sit behind one trait object.
type RowSetFetcher = Box<dyn FnMut(&str) -> Result<RowSet, String>>;

/// MySQL / PostgreSQL source adapter.
pub struct DbSourceAdapter;

impl DbSourceAdapter {
    /// Load `mysql:<name>` / `pg:<name>` against `config`'s
    /// `[remote.<scheme>]` settings: resolve query (E1905) → resolve DSN
    /// (E1904) → connect + read-only pin + fetch (E1901, with the S4
    /// offline fallback to the previous cache copy unless `strict`) →
    /// materialize through the shared cache-and-parse path.
    pub fn load(
        project_root: &Path,
        config: &ProjectConfig,
        spec: &str,
        strict: bool,
    ) -> Result<Document, String> {
        let (scheme, name) = parse_spec(spec)
            .ok_or_else(|| format!("{E1905} not a mysql:/pg: source spec: {spec}"))?;
        let remote_cfg = config.remote.get(scheme);
        let (sql, table) = resolve_query(scheme, name, remote_cfg)?;
        validate_select(&sql)?;
        let dsn = resolve_dsn(scheme, remote_cfg)?;

        // The cache key covers everything that identifies the bytes:
        // scheme, server (via DSN — one-way hashed) and statement.
        let key_input = format!("{scheme}\n{dsn}\n{sql}");
        let fetcher: RowSetFetcher = match scheme {
            "mysql" => Box::new(mysql::fetcher(&dsn)),
            _ => Box::new(pg::fetcher(&dsn)),
        };
        materialize(project_root, &table, &sql, &key_input, fetcher, strict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cage_core::manifest::RemoteSourceConfig;

    fn config_with(queries: &[(&str, &str)], dsn_env: Option<&str>) -> RemoteSourceConfig {
        RemoteSourceConfig {
            dsn_env: dsn_env.map(String::from),
            credential_env: None,
            queries: queries
                .iter()
                .map(|(k, v)| (String::from(*k), String::from(*v)))
                .collect(),
        }
    }

    #[test]
    fn parse_spec_splits_scheme_and_name() {
        assert_eq!(parse_spec("mysql:Monsters"), Some(("mysql", "Monsters")));
        assert_eq!(parse_spec("pg:top_drops"), Some(("pg", "top_drops")));
        assert_eq!(parse_spec("gsheet:abc/Tab"), None, "S3 scheme, not ours");
        assert_eq!(parse_spec("https://api/x.json"), None);
        assert_eq!(parse_spec("mysql:"), None, "empty name is not a spec");
    }

    #[test]
    fn validate_select_allows_reads_and_rejects_everything_else() {
        for ok in [
            "SELECT * FROM Monsters",
            "  select 1  ",
            "SELECT id, name FROM db.items WHERE id > 10 ORDER BY id",
        ] {
            assert!(validate_select(ok).is_ok(), "must allow: {ok}");
        }
        let bad = [
            "",
            "UPDATE t SET x = 1",
            "INSERT INTO t VALUES (1)",
            "DELETE FROM t",
            "DROP TABLE t",
            "SELECT 1; DROP TABLE t",
            "SELECT 1 -- sneaky",
            "SELECT /* sneaky */ 1",
            "SELECT 1 FOR UPDATE",
            "SELECT 1 FOR SHARE",
            "SELECT 1 INTO OUTFILE '/tmp/x'",
            "WITH cte AS (SELECT 1) SELECT * FROM cte",
            "EXEC something",
        ];
        for sql in bad {
            let err = validate_select(sql).unwrap_err();
            assert!(err.contains("E1905"), "{sql:?} → {err}");
        }
    }

    #[test]
    fn resolve_query_prefers_named_queries_and_expands_tables() {
        let cfg = config_with(&[("top", "SELECT id FROM items ORDER BY id DESC")], None);
        let (sql, table) = resolve_query("mysql", "top", Some(&cfg)).unwrap();
        assert_eq!(sql, "SELECT id FROM items ORDER BY id DESC");
        assert_eq!(table, "top", "the query name names the result table");

        let (sql, table) = resolve_query("pg", "drop_tables", None).unwrap();
        assert_eq!(sql, "SELECT * FROM drop_tables");
        assert_eq!(table, "drop_tables");

        let err = resolve_query("mysql", "Monsters; DROP TABLE x", None).unwrap_err();
        assert!(err.contains("E1905"), "{err}");
        let err = resolve_query("mysql", "has space", None).unwrap_err();
        assert!(err.contains("E1905"), "{err}");
    }

    #[test]
    fn resolve_dsn_needs_declared_and_nonempty_env() {
        let err = resolve_dsn("mysql", None).unwrap_err();
        assert!(err.contains("E1904"), "{err}");

        let cfg = config_with(&[], None);
        let err = resolve_dsn("mysql", Some(&cfg)).unwrap_err();
        assert!(err.contains("E1904"), "{err}");

        let cfg = config_with(&[], Some("CAGE_TEST_DB_DSN_UNSET"));
        let err = resolve_dsn("mysql", Some(&cfg)).unwrap_err();
        assert!(err.contains("CAGE_TEST_DB_DSN_UNSET"), "{err}");
        assert!(err.contains("E1904"), "{err}");
    }

    #[test]
    fn dbvalue_to_json_maps_types_with_precision_intact() {
        use base64::Engine as _;
        assert_eq!(DbValue::Null.to_json(), serde_json::json!(null));
        assert_eq!(DbValue::Bool(true).to_json(), serde_json::json!(true));
        assert_eq!(DbValue::Int(-7).to_json(), serde_json::json!(-7));
        assert_eq!(
            DbValue::UInt(u64::MAX).to_json(),
            serde_json::json!(u64::MAX)
        );
        assert_eq!(DbValue::Float(1.5).to_json(), serde_json::json!(1.5));
        // DECIMAL keeps its textual precision — no float round-trip
        assert_eq!(
            DbValue::Decimal("12.340".to_string()).to_json(),
            serde_json::json!("12.340")
        );
        assert_eq!(
            DbValue::Text("hi".into()).to_json(),
            serde_json::json!("hi")
        );
        assert_eq!(
            DbValue::Bytes(vec![0xde, 0xad]).to_json(),
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode([0xde, 0xad]))
        );
    }

    fn row_set(rows: Vec<Vec<DbValue>>) -> RowSet {
        RowSet {
            columns: vec!["id".to_string(), "price".to_string(), "note".to_string()],
            rows,
        }
    }

    #[test]
    fn canonical_json_sorts_rows_without_order_by_and_keeps_it_with() {
        let a = row_set(vec![vec![
            DbValue::Int(1),
            DbValue::Decimal("9.99".into()),
            DbValue::Null,
        ]]);
        let b = row_set(vec![vec![
            DbValue::Int(2),
            DbValue::Decimal("3.50".into()),
            DbValue::Text("drop".into()),
        ]]);

        // Same content, opposite return order → identical bytes.
        let bytes1 = canonical_json(
            "Items",
            &RowSet {
                columns: a.columns.clone(),
                rows: vec![a.rows[0].clone(), b.rows[0].clone()],
            },
            false,
        );
        let bytes2 = canonical_json(
            "Items",
            &RowSet {
                columns: b.columns.clone(),
                rows: vec![b.rows[0].clone(), a.rows[0].clone()],
            },
            false,
        );
        assert_eq!(
            bytes1, bytes2,
            "return order must not leak into the byte anchor"
        );

        // Explicit ORDER BY → the author's order is kept verbatim.
        let ordered1 = canonical_json(
            "Items",
            &RowSet {
                columns: a.columns.clone(),
                rows: vec![a.rows[0].clone(), b.rows[0].clone()],
            },
            true,
        );
        let ordered2 = canonical_json(
            "Items",
            &RowSet {
                columns: a.columns.clone(),
                rows: vec![b.rows[0].clone(), a.rows[0].clone()],
            },
            true,
        );
        assert_ne!(ordered1, ordered2);
        assert_eq!(
            std::str::from_utf8(&ordered1).unwrap(),
            concat!(
                "{\n  \"Items\": [\n    {\n      \"id\": 1,\n      \"price\": \"9.99\",\n",
                "      \"note\": null\n    },\n    {\n      \"id\": 2,\n",
                "      \"price\": \"3.50\",\n      \"note\": \"drop\"\n    }\n  ]\n}"
            ),
            "keys keep column order, DECIMAL stays a string, NULL stays null"
        );
    }

    #[test]
    fn materialize_round_trips_through_the_cache_file() {
        let tmp = tempfile::tempdir().unwrap();
        let row_set = row_set(vec![vec![
            DbValue::Int(1),
            DbValue::Decimal("9.99".into()),
            DbValue::Text("sword".into()),
        ]]);
        let doc = materialize(
            tmp.path(),
            "Items",
            "SELECT * FROM Items",
            "mysql\ndsn\nsql",
            |_| Ok(row_set.clone()),
            false,
        )
        .unwrap();
        let table = doc.tables.get("Items").expect("Items table");
        assert_eq!(table.rows.len(), 1);

        // The cache file is the byte anchor: same key → same slot.
        let cache_dir = remote::source_cache_dir(tmp.path(), "mysql\ndsn\nsql");
        assert!(cache_dir
            .join(format!("{}.json", remote::cache_key("mysql\ndsn\nsql")))
            .is_file());
    }

    #[test]
    fn load_rejects_in_order_spec_then_query_then_dsn() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = ProjectConfig::default();

        // Not a DB spec at all
        let err = DbSourceAdapter::load(tmp.path(), &config, "sqlite:x", false).unwrap_err();
        assert!(err.contains("E1905"), "{err}");

        // Named query failing the whitelist fires before any connection
        config.remote.insert(
            "mysql".to_string(),
            config_with(&[("bad", "UPDATE items SET price = 0")], None),
        );
        let err = DbSourceAdapter::load(tmp.path(), &config, "mysql:bad", false).unwrap_err();
        assert!(err.contains("E1905"), "{err}");

        // Whitelist-clean query, but no DSN env declared
        config
            .remote
            .insert("mysql".to_string(), config_with(&[], None));
        let err = DbSourceAdapter::load(tmp.path(), &config, "mysql:items", false).unwrap_err();
        assert!(err.contains("E1904"), "{err}");
    }

    /// S4 offline semantics: a failed fetch (transport-class by the time
    /// it reaches `materialize`) with a previous copy in the cache slot
    /// serves that copy; `strict` refuses and keeps the original error.
    #[test]
    fn failed_fetch_falls_back_to_the_cache_strict_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let row_set = row_set(vec![vec![DbValue::Int(7), DbValue::Text("potion".into())]]);

        // A successful run leaves the canonical bytes in the slot.
        materialize(
            tmp.path(),
            "Items",
            "SELECT * FROM Items",
            "mysql\ndsn\nsql",
            |_| Ok(row_set.clone()),
            false,
        )
        .unwrap();

        let offline = materialize(
            tmp.path(),
            "Items",
            "SELECT * FROM Items",
            "mysql\ndsn\nsql",
            |_| Err("connect refused".to_string()),
            false,
        )
        .expect("offline fallback must serve the cached copy");
        assert_eq!(offline.tables.get("Items").unwrap().rows.len(), 1);

        let err = materialize(
            tmp.path(),
            "Items",
            "SELECT * FROM Items",
            "mysql\ndsn\nsql",
            |_| Err("connect refused".to_string()),
            true,
        )
        .unwrap_err();
        assert!(err.contains("connect refused"), "{err}");
    }

    /// With no previous copy in the slot the fetch error stands as-is —
    /// the fallback never invents an empty document.
    #[test]
    fn no_cache_copy_means_the_transport_error_stands() {
        let tmp = tempfile::tempdir().unwrap();
        let err = materialize(
            tmp.path(),
            "Items",
            "SELECT * FROM Items",
            "mysql\ndsn\nsql",
            |_| Err("connect refused".to_string()),
            false,
        )
        .unwrap_err();
        assert!(err.contains("connect refused"), "{err}");
    }
}
