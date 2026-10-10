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

use crate::error::codes::remote::E1906;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Cap on honoring a server-sent `Retry-After` (HTTP 429): waits up to
/// this long are honored verbatim (rate-limit negotiation — the server
/// said when to come back), longer asks are refused immediately as a
/// definitive failure — a build must not stall for an hour on one
/// source. Absent or unparseable headers are never guessed at.
pub const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(30);

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

/// Offline fallback gate (S4, design §45): a transport-class fetch
/// failure may fall back to the previously materialized cache copy
/// instead of failing the build — an offline machine keeps building off
/// the last fetched bytes. Only transport-class failures fall back: a
/// 404 means the source was deleted remotely and 401/403 mean access
/// may have been revoked, so serving stale bytes there would be
/// silently wrong — those stay hard errors. `strict` (`--no-cache`)
/// disables the fallback entirely. Returns the cache path after
/// printing the E1906 WARNING line, or `None` to keep the original
/// transport error. `context` lands in the warning verbatim — callers
/// pass the spec/URL only, never anything carrying credentials.
pub fn source_cache_fallback(cache_file: &Path, context: &str, strict: bool) -> Option<PathBuf> {
    if strict || !cache_file.is_file() {
        return None;
    }
    eprintln!(
        "warning: {E1906} remote source unreachable, serving the previous cached copy: \
         {context} (cache: {})",
        cache_file.display()
    );
    Some(cache_file.to_path_buf())
}

/// One failed HTTP GET, kind-tagged so callers attach the right error
/// code: transport failures are retried and map to "fetch failed"
/// (E1901 for sources, E1802 for the registry), a 429 with an
/// honorable `Retry-After` is retried after that wait, 401/403 map to
/// "auth rejected" (E1902), any other status is a definitive failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    /// Connection / DNS / timeout / broken response body — retryable
    Transport(String),
    /// HTTP 429 whose `Retry-After` the policy honors (present,
    /// parseable, within [`MAX_RATE_LIMIT_WAIT`]) — retryable after
    /// exactly that wait. A 429 without one stays `Status(429)`.
    RateLimited(Duration),
    /// HTTP 404 — the resource does not exist on the server
    NotFound(String),
    /// Any other HTTP status (401/403 among them)
    Status(u16),
}

impl std::fmt::Display for FetchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchFailure::Transport(e) => write!(f, "transport failure: {e}"),
            FetchFailure::RateLimited(wait) => {
                write!(
                    f,
                    "rate limited, server asked to retry after {}s",
                    wait.as_secs()
                )
            }
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

/// Which failures of a network call are retryable under [`RetryPolicy`]
/// versus definitive server answers, and what to wait between tries.
/// `FetchFailure` (GET) and `PutFailure` (PUT) both classify their
/// `Transport` variant; only GET also carries a rate-limit cooldown.
pub trait RetryClassify {
    /// `true` for failures a later try may still satisfy — connection /
    /// DNS / timeout-class, or a rate limit whose asked wait is honored
    fn is_retryable(&self) -> bool;
    /// Sleep before the next try; `None` falls back to the policy's
    /// fixed backoff (transport flake), `Some(d)` honors a server
    /// negotiation (`Retry-After`) instead
    fn cooldown(&self) -> Option<Duration>;
}

impl RetryClassify for FetchFailure {
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            FetchFailure::Transport(_) | FetchFailure::RateLimited(_)
        )
    }
    fn cooldown(&self) -> Option<Duration> {
        match self {
            FetchFailure::RateLimited(wait) => Some(*wait),
            _ => None,
        }
    }
}

impl RetryClassify for PutFailure {
    fn is_retryable(&self) -> bool {
        matches!(self, PutFailure::Transport(_))
    }
    fn cooldown(&self) -> Option<Duration> {
        None
    }
}

/// Run `fetch` under `policy`: retryable failures are retried — with the
/// failure's negotiated cooldown when it has one, the fixed backoff
/// otherwise — success and every other failure return immediately. When
/// the budget runs out the last retryable error wins. Generic over the
/// failure type — GET and PUT share the one policy while keeping their
/// own failure kinds.
pub fn with_retries<T, E: RetryClassify>(
    policy: &RetryPolicy,
    mut fetch: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let mut last: Option<E> = None;
    for attempt in 0..policy.attempts {
        match fetch() {
            Ok(value) => return Ok(value),
            Err(e) if e.is_retryable() => {
                let wait = e.cooldown().unwrap_or(policy.backoff);
                last = Some(e);
                if attempt + 1 < policy.attempts {
                    std::thread::sleep(wait);
                }
            }
            Err(other) => return Err(other),
        }
    }
    // `attempts` is at least 1 in practice; a zero budget never calls
    // `fetch`, and there is no meaningful failure to report then.
    Err(last.expect("retry policy with zero attempts"))
}

/// GET one URL, returning the response body bytes. Bounded retries on
/// transport-level flake (a busy server must not fail a build over a
/// refused/reset connection) and on a 429 whose `Retry-After` the policy
/// honors; 404s, 401/403 and other statuses are definitive — no retry.
/// The network layer lives here so every consumer shares one policy;
/// callers only map the failure to their own error code.
pub fn http_get(url: &str) -> Result<Vec<u8>, FetchFailure> {
    http_get_full(url).map(|response| response.body)
}

/// One successful GET: the body bytes plus the response headers the
/// consumers need (`Link` per RFC 8288 for the pagination walk; `ETag` /
/// `Last-Modified` per RFC 7232 for the conditional revalidation round
/// trip). Everything else about the response stays inside this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpGet {
    /// The response body bytes
    pub body: Vec<u8>,
    /// Raw `Link` response header value when the server sent one
    pub link: Option<String>,
    /// Raw `ETag` response header value when the server sent one
    pub etag: Option<String>,
    /// Raw `Last-Modified` response header value when the server sent one
    pub last_modified: Option<String>,
}

/// The validators a server handed out with the last fetched
/// representation (RFC 7232), sent back on the next fetch as
/// `If-None-Match` / `If-Modified-Since`. Both optional — a server may
/// offer either or neither; whatever is absent simply does not revalidate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    /// `ETag` from the last 200 response, if any
    pub etag: Option<String>,
    /// `Last-Modified` from the last 200 response, if any
    pub last_modified: Option<String>,
}

/// One conditional GET outcome: `NotModified` is the server confirming
/// the cached representation is current (304 — the bytes stay cached,
/// nothing is re-downloaded); `Fresh` is a new representation plus the
/// validators to store with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalGet {
    /// 304 Not Modified — the caller's cached bytes remain authoritative
    NotModified,
    /// A 200 response: body and headers as with a plain GET
    Fresh(HttpGet),
}

/// [`http_get_full`] under RFC 7232 revalidation: the validators (from
/// the cache meta) ride along as conditional request headers, and a 304
/// comes back as [`ConditionalGet::NotModified`] instead of a failure.
/// Retries and failure classification are the one shared policy — a 304
/// is a success answer, never a retry candidate.
pub fn http_get_conditional(
    url: &str,
    validators: &Validators,
) -> Result<ConditionalGet, FetchFailure> {
    with_retries(&RetryPolicy::default(), || {
        http_get_conditional_once(url, validators)
    })
}

fn http_get_conditional_once(
    url: &str,
    validators: &Validators,
) -> Result<ConditionalGet, FetchFailure> {
    let mut request = ureq::get(url).timeout(Duration::from_secs(30));
    if let Some(etag) = &validators.etag {
        request = request.set("If-None-Match", etag);
    }
    if let Some(last_modified) = &validators.last_modified {
        request = request.set("If-Modified-Since", last_modified);
    }
    match request.call() {
        // ureq only turns statuses >= 400 into errors, so a 304 arrives as
        // an Ok response — the guard arm must come first; reading the body
        // would yield empty bytes and masquerade as fresh content.
        Ok(response) if response.status() == 304 => Ok(ConditionalGet::NotModified),
        Ok(response) => Ok(ConditionalGet::Fresh(read_response(response, url)?)),
        Err(e) => Err(map_fetch_error(e, url)),
    }
}

/// [`http_get`] with the response headers surfaced.
pub fn http_get_full(url: &str) -> Result<HttpGet, FetchFailure> {
    with_retries(&RetryPolicy::default(), || http_get_once_full(url))
}

fn http_get_once_full(url: &str) -> Result<HttpGet, FetchFailure> {
    let response = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| map_fetch_error(e, url))?;
    read_response(response, url)
}

/// The one failure classification for GETs — shared by the plain and
/// conditional paths.
fn map_fetch_error(e: ureq::Error, url: &str) -> FetchFailure {
    match e {
        ureq::Error::Status(404, _) => FetchFailure::NotFound(url.to_string()),
        ureq::Error::Status(429, response) => rate_limited(&response),
        ureq::Error::Status(code, _) => FetchFailure::Status(code),
        ureq::Error::Transport(t) => FetchFailure::Transport(t.to_string()),
    }
}

/// Drain one 2xx response: body bytes plus the `Link` / `ETag` /
/// `Last-Modified` headers the consumers need.
fn read_response(response: ureq::Response, url: &str) -> Result<HttpGet, FetchFailure> {
    let link = response.header("Link").map(str::to_string);
    let etag = response.header("ETag").map(str::to_string);
    let last_modified = response.header("Last-Modified").map(str::to_string);
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| FetchFailure::Transport(format!("reading {url}: {e}")))?;
    Ok(HttpGet {
        body: bytes,
        link,
        etag,
        last_modified,
    })
}

/// 429 → retryable only when the asked wait is one the policy honors:
/// `Retry-After` present, parseable (delta-seconds or HTTP-date) and
/// within [`MAX_RATE_LIMIT_WAIT`]. Anything else — header absent,
/// garbage, or asking more than the cap — is a definitive `Status(429)`
/// answer: no guessing, and a build must not stall for an hour.
fn rate_limited(response: &ureq::Response) -> FetchFailure {
    let asked = response
        .header("Retry-After")
        .and_then(|value| parse_retry_after(value, now_unix()));
    match asked {
        Some(wait) if wait <= MAX_RATE_LIMIT_WAIT => FetchFailure::RateLimited(wait),
        _ => FetchFailure::Status(429),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Parse a `Retry-After` header value (RFC 7231): delta-seconds
/// (`120`) or an IMF-fixdate HTTP-date (`Wed, 21 Oct 2015 07:28:00
/// GMT`). Returns the wait counted from `now`; dates in the past yield
/// `ZERO` (retry immediately), unparseable values yield `None` — the
/// wait is never guessed at.
pub fn parse_retry_after(value: &str, now: u64) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = parse_http_date(value)?;
    Some(Duration::from_secs(at.saturating_sub(now)))
}

/// Parse one IMF-fixdate (`Wed, 21 Oct 2015 07:28:00 GMT`) to Unix
/// seconds — the only date form servers send for `Retry-After` in
/// practice. RFC 7231's two obsolete formats are out of scope.
fn parse_http_date(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let _weekday = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let month = month_from_name(parts.next()?)?;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if parts.next() != Some("GMT") {
        return None;
    }
    if !(1..=31).contains(&day)
        || !(1..=12).contains(&month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second) as u64)
}

/// Month name → month number (1..=12), English abbreviations and full
/// names alike — HTTP only ever uses the three-letter form, but the
/// check costs nothing extra.
fn month_from_name(name: &str) -> Option<u32> {
    let month = match name {
        "Jan" | "January" => 1,
        "Feb" | "February" => 2,
        "Mar" | "March" => 3,
        "Apr" | "April" => 4,
        "May" => 5,
        "Jun" | "June" => 6,
        "Jul" | "July" => 7,
        "Aug" | "August" => 8,
        "Sep" | "September" => 9,
        "Oct" | "October" => 10,
        "Nov" | "November" => 11,
        "Dec" | "December" => 12,
        _ => return None,
    };
    Some(month)
}

/// Days since 1970-01-01 for a proleptic-Gregorian civil date
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One failed HTTP PUT, kind-tagged for the distribution write path (§47
/// A3): transport failures are retried like `FetchFailure::Transport`,
/// 401/403 are credential rejections (E2102), and any other server answer
/// is a definitive write refusal (E2104 — 405/501 mean the server has no
/// write channel at all).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutFailure {
    /// Connection / DNS / timeout — retryable
    Transport(String),
    /// HTTP 401 / 403 — the credentials were rejected
    AuthRejected(u16),
    /// Any other HTTP status — the server refuses (or cannot) store
    WriteRefused(u16),
}

impl std::fmt::Display for PutFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PutFailure::Transport(e) => write!(f, "transport failure: {e}"),
            PutFailure::AuthRejected(code) => write!(f, "auth rejected: http status {code}"),
            PutFailure::WriteRefused(code) => write!(f, "write refused: http status {code}"),
        }
    }
}

/// PUT `body` to one URL, optionally with an `Authorization: Bearer` header.
/// Same one-policy discipline as `http_get`: bounded retries on
/// transport-level flake only; a server answer is definitive and maps to
/// the caller's error code via `PutFailure`'s kind. The token travels only
/// in the request header — never logged, never persisted.
pub fn http_put(url: &str, token: Option<&str>, body: &[u8]) -> Result<(), PutFailure> {
    with_retries(&RetryPolicy::default(), || http_put_once(url, token, body))
}

fn http_put_once(url: &str, token: Option<&str>, body: &[u8]) -> Result<(), PutFailure> {
    let mut request = ureq::put(url).timeout(Duration::from_secs(30));
    if let Some(token) = token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = request
        // `send_bytes`, not `send`: the Read-based sender streams chunked
        // (no Content-Length), which static registry hosts are not obliged
        // to decode — a fixed-length body is the interoperable form.
        .send_bytes(body)
        .map_err(|e| match e {
            ureq::Error::Status(code, _) if code == 401 || code == 403 => {
                PutFailure::AuthRejected(code)
            }
            ureq::Error::Status(code, _) => PutFailure::WriteRefused(code),
            ureq::Error::Transport(t) => PutFailure::Transport(t.to_string()),
        })?;
    let mut sink = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut sink)
        .map_err(|e| PutFailure::Transport(format!("reading {url}: {e}")))?;
    Ok(())
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

    /// A rate limit with an honorable wait is retried — after the
    /// server's asked cooldown, not the fixed backoff — within the same
    /// attempts budget.
    #[test]
    fn with_retries_honors_rate_limit_cooldown() {
        let mut calls = 0u32;
        let out: Result<(), _> = with_retries(
            &RetryPolicy {
                attempts: 3,
                backoff: Duration::ZERO,
            },
            || {
                calls += 1;
                Err(FetchFailure::RateLimited(Duration::ZERO))
            },
        );
        assert_eq!(
            out,
            Err(FetchFailure::RateLimited(Duration::ZERO)),
            "budget exhausted → the last rate-limit answer wins"
        );
        assert_eq!(calls, 3, "the rate limit consumes the same budget");
    }

    #[test]
    fn cooldown_is_negotiated_for_rate_limits_backoff_for_transport() {
        let wait = Duration::from_secs(7);
        assert_eq!(
            FetchFailure::RateLimited(wait).cooldown(),
            Some(wait),
            "the server's wait is the cooldown"
        );
        assert_eq!(
            FetchFailure::Transport("reset".to_string()).cooldown(),
            None,
            "transport falls back to the policy backoff"
        );
        assert_eq!(FetchFailure::NotFound("u".to_string()).cooldown(), None);
        assert!(FetchFailure::RateLimited(wait).is_retryable());
        assert!(!FetchFailure::Status(429).is_retryable());
        assert!(!PutFailure::WriteRefused(405).is_retryable());
    }

    #[test]
    fn parse_retry_after_reads_delta_seconds_and_dates() {
        let instant = 1_445_412_480u64; // 2015-10-21 07:28:00 GMT
        assert_eq!(
            parse_retry_after("120", instant),
            Some(Duration::from_secs(120)),
            "delta-seconds form"
        );
        assert_eq!(
            parse_retry_after(" 0 ", instant),
            Some(Duration::ZERO),
            "zero asks to retry immediately"
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", instant),
            Some(Duration::ZERO),
            "a date at `now` yields no wait"
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:10 GMT", instant),
            Some(Duration::from_secs(10)),
            "a future date yields the difference"
        );
        assert_eq!(
            parse_retry_after("Tue, 20 Oct 2015 07:28:00 GMT", instant),
            Some(Duration::ZERO),
            "a past date never goes negative"
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 PST", instant),
            None,
            "only GMT is accepted"
        );
        for garbage in [
            "",
            "soon",
            "-5",
            "Wed, 21 Oct 2015",
            "32 Oc 2015 07:28:00 GMT",
        ] {
            assert_eq!(parse_retry_after(garbage, instant), None, "{garbage}");
        }
    }

    #[test]
    fn parse_http_date_matches_known_epochs() {
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(
            parse_http_date("Tue, 01 Jan 2030 00:00:00 GMT"),
            Some(1_893_456_000),
            "leap years between epochs are counted correctly"
        );
        assert_eq!(
            parse_http_date("Sat, 29 Feb 2020 12:00:00 GMT"),
            Some(1_582_977_600)
        );
        assert_eq!(parse_http_date("nope"), None);
        assert_eq!(parse_http_date("Wed, 32 Oct 2015 07:28:00 GMT"), None);
    }
}
