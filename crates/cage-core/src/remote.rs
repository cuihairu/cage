//! Shared Remote Source infrastructure (design §45): one cache-key
//! scheme, one bounded-retry policy, one HTTP GET. Every remote consumer
//! — the R3 registry fetcher and the source adapters (HTTP API now,
//! Sheets over HTTP later) — derives from these helpers instead of
//! carrying their own copies.
//!
//! Caches are project-local (`.cage-cache/`): registry entries under
//! `registry/`, source payloads under `source/`. Keys are the first 12
//! hex chars of the source URL's blake3 — deterministic for a given URL
//! and isolated between distinct URLs.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Deterministic local cache key for a remote root or source URL: the
/// first 12 hex chars of the URL's blake3. Pure — every consumer derives
/// the same key for the same URL, so one root maps to one cache slot and
/// distinct roots never collide.
pub fn cache_key(source_url: &str) -> String {
    blake3::hash(source_url.as_bytes()).to_hex()[..12].to_string()
}

/// Project-local cache directory for a remote source payload
/// (`.cage-cache/source/<cache_key>/`), anchored at the project root so
/// the cache travels with the checkout rather than the machine.
pub fn source_cache_dir(project_root: &Path, source_url: &str) -> PathBuf {
    project_root
        .join(".cage-cache")
        .join("source")
        .join(cache_key(source_url))
}

/// One failed HTTP GET, kind-tagged so callers attach the right error
/// code: transport failures are retried and map to "fetch failed"
/// (E1901 for sources, E1802 for the registry), 401/403 map to "auth
/// rejected" (E1902), any other status is a definitive failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    /// Connection / DNS / timeout / broken response body — retryable
    Transport(String),
    /// HTTP 404 — the resource does not exist on the server
    NotFound(String),
    /// Any other HTTP status (401/403 among them)
    Status(u16),
}

impl std::fmt::Display for FetchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchFailure::Transport(e) => write!(f, "transport failure: {e}"),
            FetchFailure::NotFound(url) => write!(f, "not found: {url}"),
            FetchFailure::Status(code) => write!(f, "http status {code}"),
        }
    }
}

/// Bounded retry policy for connection-class failures: `attempts` tries
/// with a fixed `backoff` sleep between them (design §45 — connection
/// flake only; server answers are definitive and never retried).
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total tries, first attempt included
    pub attempts: u32,
    /// Sleep between tries
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            backoff: Duration::from_millis(50),
        }
    }
}

/// Run `fetch` under `policy`: transport failures are retried with the
/// backoff, success and every other failure return immediately. When the
/// budget runs out the last transport error wins.
pub fn with_retries<T>(
    policy: &RetryPolicy,
    mut fetch: impl FnMut() -> Result<T, FetchFailure>,
) -> Result<T, FetchFailure> {
    let mut last: Option<FetchFailure> = None;
    for attempt in 0..policy.attempts {
        match fetch() {
            Ok(value) => return Ok(value),
            Err(FetchFailure::Transport(e)) => {
                last = Some(FetchFailure::Transport(e));
                if attempt + 1 < policy.attempts {
                    std::thread::sleep(policy.backoff);
                }
            }
            Err(other) => return Err(other),
        }
    }
    // `attempts` is at least 1 in practice; a zero budget never calls
    // `fetch`, and there is no meaningful failure to report then.
    Err(last.unwrap_or_else(|| FetchFailure::Transport("no attempt configured".to_string())))
}

/// GET one URL, returning the response body bytes. Bounded retries on
/// transport-level flake (a busy server must not fail a build over a
/// refused/reset connection); 404s and other statuses are definitive —
/// no retry. The network layer lives here so every consumer shares one
/// policy; callers only map the failure to their own error code.
pub fn http_get(url: &str) -> Result<Vec<u8>, FetchFailure> {
    with_retries(&RetryPolicy::default(), || http_get_once(url))
}

fn http_get_once(url: &str) -> Result<Vec<u8>, FetchFailure> {
    let response = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => FetchFailure::NotFound(url.to_string()),
            ureq::Error::Status(code, _) => FetchFailure::Status(code),
            ureq::Error::Transport(t) => FetchFailure::Transport(t.to_string()),
        })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| FetchFailure::Transport(format!("reading {url}: {e}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_deterministic_isolated_and_sized() {
        assert_eq!(
            cache_key("https://r/example"),
            cache_key("https://r/example"),
            "same URL, same key — the cache slot is reproducible"
        );
        assert_ne!(cache_key("https://r/a"), cache_key("https://r/b"));
        assert_eq!(cache_key("https://r/example").len(), 12);
    }

    #[test]
    fn source_cache_dir_derives_from_url_key() {
        let root = Path::new("/proj");
        let url = "https://api.example.com/items.json";
        assert_eq!(
            source_cache_dir(root, url),
            root.join(".cage-cache").join("source").join(cache_key(url))
        );
        assert_ne!(
            source_cache_dir(root, "https://a/x.json"),
            source_cache_dir(root, "https://b/x.json"),
            "distinct URLs must not share a cache slot"
        );
    }

    #[test]
    fn with_retries_recovers_from_transport_flake() {
        let mut calls = 0u32;
        let out = with_retries(
            &RetryPolicy {
                attempts: 3,
                backoff: Duration::ZERO,
            },
            || {
                calls += 1;
                if calls < 3 {
                    Err(FetchFailure::Transport("reset".to_string()))
                } else {
                    Ok(7)
                }
            },
        );
        assert_eq!(out, Ok(7));
        assert_eq!(calls, 3, "two flakes then success = three tries");
    }

    #[test]
    fn with_retries_budget_exhausts_and_server_answers_do_not_retry() {
        let mut calls = 0u32;
        let out: Result<(), _> = with_retries(
            &RetryPolicy {
                attempts: 4,
                backoff: Duration::ZERO,
            },
            || {
                calls += 1;
                Err(FetchFailure::Transport("down".to_string()))
            },
        );
        assert_eq!(out, Err(FetchFailure::Transport("down".to_string())));
        assert_eq!(calls, 4, "the budget is exactly `attempts` tries");

        let mut calls = 0u32;
        let out: Result<(), _> = with_retries(
            &RetryPolicy {
                attempts: 4,
                backoff: Duration::ZERO,
            },
            || {
                calls += 1;
                Err(FetchFailure::Status(503))
            },
        );
        assert_eq!(out, Err(FetchFailure::Status(503)));
        assert_eq!(calls, 1, "a server answer is definitive — no retry");

        let mut calls = 0u32;
        let out: Result<(), _> = with_retries(
            &RetryPolicy {
                attempts: 4,
                backoff: Duration::ZERO,
            },
            || {
                calls += 1;
                Err(FetchFailure::NotFound("https://r/x".to_string()))
            },
        );
        assert!(out.is_err());
        assert_eq!(calls, 1, "404 is definitive — no retry");
    }
}
