//! Process-wide connection pools (design §45 留待实现期 → 落地
//! 2026-10-13): one pool per DSN, shared by every fetch / introspect in
//! the process, so a build with N sources against one server opens one
//! connection instead of N. mysql rides the driver's own `Pool`;
//! postgres (0.19 ships no sync pool) rides the small idle-stack pool
//! below. Semantics are unchanged from the fresh-connection cut: the
//! caller pins every acquired session read-only before any statement
//! runs, and a connection whose use failed is discarded instead of
//! returned — never hand back a session whose protocol state may be
//! mid-flight. Pool creation is lazy per DSN and connection failures
//! are never registered, so a bad DSN keeps erroring until it works.

use cage_core::error::codes::remote::E1901;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Idle-stack pool over a reconnect factory. `with_conn` lends one
/// connection to the closure: idle reuse first, else a fresh connect.
/// `Ok` returns the connection to the idle stack (capped, oldest
/// evicted); `Err` drops it. Nestable — the lock only guards the idle
/// stack, never a connection while it is lent out.
pub(crate) struct IdlePool<C> {
    connect: Box<dyn Fn() -> Result<C, String> + Send + Sync>,
    max_idle: usize,
    idle: Mutex<Vec<C>>,
}

impl<C> IdlePool<C> {
    pub(crate) fn new(
        connect: Box<dyn Fn() -> Result<C, String> + Send + Sync>,
        max_idle: usize,
    ) -> Self {
        Self {
            connect,
            max_idle,
            idle: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_conn<T>(
        &self,
        use_conn: impl FnOnce(&mut C) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut conn = self.acquire()?;
        match use_conn(&mut conn) {
            Ok(value) => {
                self.release(conn);
                Ok(value)
            }
            Err(e) => Err(e), // the failed session drops here, unreturned
        }
    }

    fn acquire(&self) -> Result<C, String> {
        if let Some(conn) = self.idle.lock().unwrap().pop() {
            return Ok(conn);
        }
        (self.connect)()
    }

    fn release(&self, conn: C) {
        let mut idle = self.idle.lock().unwrap();
        idle.push(conn);
        while idle.len() > self.max_idle {
            idle.remove(0);
        }
    }

    /// Idle-stack depth — test observability only.
    #[cfg(test)]
    pub(crate) fn idle_len(&self) -> usize {
        self.idle.lock().unwrap().len()
    }
}

static MYSQL_POOLS: OnceLock<Mutex<HashMap<String, mysql::Pool>>> = OnceLock::new();
static PG_POOLS: OnceLock<Mutex<HashMap<String, Arc<IdlePool<postgres::Client>>>>> =
    OnceLock::new();

/// The process-wide mysql pool for `dsn` — one per DSN, built lazily
/// (the driver connects on the first `get_conn`). A DSN that does not
/// parse is an error every time: failures are never registered.
pub(crate) fn mysql_pool(dsn: &str) -> Result<mysql::Pool, String> {
    let map = MYSQL_POOLS.get_or_init(Default::default);
    if let Some(pool) = map.lock().unwrap().get(dsn) {
        return Ok(pool.clone());
    }
    let opts = mysql::Opts::from_url(dsn).map_err(|e| format!("{E1901} invalid mysql DSN: {e}"))?;
    let opts = mysql::OptsBuilder::from_opts(opts).pool_opts(
        mysql::PoolOpts::default()
            // One fetch at a time per DSN today — a handful of idle
            // sessions is the ceiling the config-source shape needs.
            .with_constraints(mysql::PoolConstraints::new(0, 8).expect("0 <= 8")),
    );
    let pool = mysql::Pool::new(opts).map_err(|e| format!("{E1901} invalid mysql DSN: {e}"))?;
    map.lock().unwrap().insert(dsn.to_string(), pool.clone());
    Ok(pool)
}

/// The process-wide postgres pool for `dsn` — same registry shape over
/// the hand-rolled idle stack; `connect` runs only inside `with_conn`.
pub(crate) fn pg_pool(dsn: &str) -> Result<Arc<IdlePool<postgres::Client>>, String> {
    let map = PG_POOLS.get_or_init(Default::default);
    if let Some(pool) = map.lock().unwrap().get(dsn) {
        return Ok(pool.clone());
    }
    let key = dsn.to_string();
    let dsn = dsn.to_string();
    let pool = Arc::new(IdlePool::new(
        Box::new(move || {
            postgres::Client::connect(&dsn, postgres::NoTls)
                .map_err(|e| format!("{E1901} cannot connect to postgresql: {e}"))
        }),
        4,
    ));
    map.lock().unwrap().insert(key, pool.clone());
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake connection: the id doubles as "how many connects happened".
    struct FakeConn(u32);

    fn counting_factory(
        connects: Arc<AtomicUsize>,
    ) -> Box<dyn Fn() -> Result<FakeConn, String> + Send + Sync> {
        Box::new(move || {
            let id = connects.fetch_add(1, Ordering::SeqCst) as u32 + 1;
            Ok(FakeConn(id))
        })
    }

    #[test]
    fn pool_reuses_the_idle_connection_across_uses() {
        let connects = Arc::new(AtomicUsize::new(0));
        let pool = IdlePool::new(counting_factory(connects.clone()), 4);
        let mut seen = Vec::new();
        for _ in 0..2 {
            pool.with_conn(|conn: &mut FakeConn| {
                seen.push(conn.0);
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(connects.load(Ordering::SeqCst), 1, "second use must reuse");
        assert_eq!(seen[0], seen[1], "the same session is lent twice");
        assert_eq!(pool.idle_len(), 1);
    }

    #[test]
    fn a_failed_use_discards_the_connection() {
        let connects = Arc::new(AtomicUsize::new(0));
        let pool = IdlePool::new(counting_factory(connects.clone()), 4);
        let err = pool
            .with_conn(|_: &mut FakeConn| Err::<(), _>("query blew up".to_string()))
            .unwrap_err();
        assert!(err.contains("query blew up"), "{err}");
        let mut ids = Vec::new();
        pool.with_conn(|conn: &mut FakeConn| {
            ids.push(conn.0);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            connects.load(Ordering::SeqCst),
            2,
            "the failed session never re-joined the idle stack"
        );
        assert_eq!(ids[0], 2, "a fresh session was made for the next use");
    }

    #[test]
    fn a_failed_connect_propagates_and_retries_next_time() {
        let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = failing.clone();
        let pool = IdlePool::new(
            Box::new(move || {
                if flag.load(Ordering::SeqCst) {
                    Err("connect refused".to_string())
                } else {
                    Ok(FakeConn(7))
                }
            }),
            4,
        );
        let err = pool.with_conn(|_: &mut FakeConn| Ok(())).unwrap_err();
        assert!(err.contains("connect refused"), "{err}");
        failing.store(false, Ordering::SeqCst);
        pool.with_conn(|conn: &mut FakeConn| {
            assert_eq!(conn.0, 7);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_idle_stack_is_capped() {
        let connects = Arc::new(AtomicUsize::new(0));
        let pool = IdlePool::new(counting_factory(connects.clone()), 1);
        // Nested use: the outer session is held while the inner one is
        // acquired, so two connects happen — and at most one may idle.
        pool.with_conn(|_outer: &mut FakeConn| pool.with_conn(|_inner: &mut FakeConn| Ok(())))
            .unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 2);
        assert_eq!(pool.idle_len(), 1, "cap holds after both are released");
    }

    #[test]
    fn pg_registry_hands_out_one_pool_per_dsn() {
        let a = pg_pool("postgresql://cage-unit-a@127.0.0.1:1/x").unwrap();
        let a2 = pg_pool("postgresql://cage-unit-a@127.0.0.1:1/x").unwrap();
        let b = pg_pool("postgresql://cage-unit-b@127.0.0.1:1/x").unwrap();
        assert!(Arc::ptr_eq(&a, &a2), "same DSN → same pool");
        assert!(!Arc::ptr_eq(&a, &b), "different DSN → different pool");
        // Pool creation is lazy: port 1 never saw a packet.
    }

    #[test]
    fn mysql_registry_rejects_an_unparsable_dsn_every_time() {
        for _ in 0..2 {
            let err = mysql_pool("not a url").unwrap_err();
            assert!(err.contains("invalid mysql DSN"), "{err}");
        }
    }

    #[test]
    fn mysql_registry_builds_a_lazy_pool_per_dsn() {
        // Pool::new never connects — port 1 stays untouched. Both calls
        // succeed: the first builds and registers, the second reuses.
        let pool = mysql_pool("mysql://cage-unit@127.0.0.1:1/cage_unit").unwrap();
        let again = mysql_pool("mysql://cage-unit@127.0.0.1:1/cage_unit").unwrap();
        drop((pool, again));
    }
}
