//! HTTP API source adapter (S1, design §45): an `http(s)` URL in
//! `[source_roots]` is fetched once (bounded retries on transport flake,
//! rate-limit negotiation on a 429 with an honorable `Retry-After`),
//! the response bytes are materialized in the project-local cache
//! (`.cage-cache/source/<cache_key>/<cache_key>.json`), and the cached
//! file is parsed by the standard JSON adapter. The remote body follows
//! the exact same shape rules as a local `.json` source —
//! `{表名: 行数组}` / single object → `Root` / bare array → `Data` —
//! and enters the same L0-L7 pipeline. The remote is a data source, not
//! a trusted one: nothing skips validation.
//!
//! Conditional revalidation (design §45 — the document-level core of
//! 增量拉取): the `ETag` / `Last-Modified` validators the server handed
//! out with the last 200 are kept beside the cached bytes
//! (`<cache_key>.meta.json`) and sent back as `If-None-Match` /
//! `If-Modified-Since`; a 304 answer reuses the cached bytes and touches
//! nothing. Sources the server cannot revalidate just fetch plainly.
//!
//! Pagination (design §45): when the response carries an RFC 8288
//! `Link: <...>; rel="next"` header the walk follows it — every page
//! must be a JSON array of rows, and pages concatenate in link order
//! into one merged document (cached bytes = merged array; a bare array
//! parses as the `Data` table). A source without a next link keeps the
//! single-GET contract byte-for-byte. Only the first GET revalidates —
//! its validators describe the collection state; follow-up pages are
//! plain GETs, and a 304 on the first page skips the walk entirely.

use cage_core::error::codes::internal::E9902;
use cage_core::error::codes::remote::{E1901, E1902};
use cage_core::remote::{self, FetchFailure};
use cage_core::value::Document;
use cage_source_json::JsonSourceAdapter;
use std::path::{Path, PathBuf};

/// Whether a `[source_roots]` value names a remote source: an http(s)
/// URL — the same predicate the registry root uses, so a URL can never
/// mean both.
fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Fetch a remote JSON source into the project-local cache and parse it
/// into a `Document`.
pub struct HttpSourceAdapter;

impl HttpSourceAdapter {
    /// GET `url` (revalidating against the cached validators first, and
    /// following `Link: rel="next"` pagination when the server
    /// advertises it) and land the merged document bytes at
    /// `.cage-cache/source/<cache_key(url)>/<cache_key(url)>.json`,
    /// returning the cached file path. A 304 revalidation answer keeps
    /// the cached bytes and meta untouched. Errors: E1901 when the fetch
    /// fails (transport after bounded retries, a rate limit past its
    /// budget, 404, any other non-auth status, or a pagination-contract
    /// violation), E1902 on 401/403, E9902 when the cache cannot be
    /// written. A transport failure with a previous copy on disk falls
    /// back to it instead (E1906 WARNING) unless `strict` (`--no-cache`)
    /// is set; everything else never falls back — a server that answered
    /// has spoken, and stale bytes must not mask that.
    pub fn fetch(project_root: &Path, url: &str, strict: bool) -> Result<PathBuf, String> {
        if !is_http_url(url) {
            return Err(format!("{E1901} not an http(s) remote source URL: {url}"));
        }
        // The cache slot is keyed by the URL alone, so it resolves
        // before any bytes move — the offline fallback needs it too.
        let cache_dir = remote::source_cache_dir(project_root, url);
        let cache_file = cache_dir.join(format!("{}.json", remote::cache_key(url)));
        // Revalidation needs the validators AND the bytes they describe:
        // a meta without its payload is just stale bookkeeping — fetch
        // plainly and rewrite both.
        let validators = if cache_file.is_file() {
            read_meta(&cache_dir, url)?
        } else {
            None
        };
        let (bytes, fresh) = match fetch_pages(url, validators.as_ref()) {
            Ok(PageFetch::NotModified) => {
                // The server confirmed the cached bytes are current.
                return Ok(cache_file);
            }
            Ok(PageFetch::Fresh { bytes, validators }) => (bytes, validators),
            Err(PageFailure::Http(FetchFailure::Transport(e))) => {
                if let Some(path) = remote::source_cache_fallback(&cache_file, url, strict) {
                    return Ok(path);
                }
                return Err(format!(
                    "{E1901} remote source fetch failed: transport failure: {e} ({url})"
                ));
            }
            Err(PageFailure::Http(FetchFailure::Status(code @ (401 | 403)))) => {
                return Err(format!(
                    "{E1902} remote source rejected access (HTTP {code}): {url}"
                ));
            }
            Err(PageFailure::Http(FetchFailure::NotFound(inner))) => {
                return Err(format!(
                    "{E1901} remote source not found (HTTP 404): {inner}"
                ));
            }
            Err(PageFailure::Http(e)) => {
                return Err(format!("{E1901} remote source fetch failed: {e} ({url})"));
            }
            Err(e) => {
                return Err(format!("{E1901} remote source pagination failed: {e}"));
            }
        };

        // The byte anchor: whatever the server said is written verbatim
        // before anything parses it — later stages always see the same
        // file a re-run would. (Paginated sources materialize the merged
        // document, itself deterministic for a given server state.)
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
        write_meta(&cache_dir, url, &fresh)?;
        Ok(cache_file)
    }

    /// `fetch` + parse the cached bytes with the standard JSON adapter —
    /// identical shapes, identical diagnostics (E0001 with line/column
    /// on a malformed body). Parse diagnostics render to stderr and the
    /// error carries the URL, mirroring how local source files report.
    pub fn load(project_root: &Path, url: &str, strict: bool) -> Result<Document, String> {
        let cache_file = Self::fetch(project_root, url, strict)?;
        JsonSourceAdapter::parse_file(&cache_file).map_err(|diags| {
            eprintln!("{}", diags.render(false));
            format!("failed to parse remote source {url}")
        })
    }
}

mod meta;
mod paging;
#[cfg(test)]
mod tests;

pub(crate) use meta::{read_meta, write_meta};
pub(crate) use paging::{fetch_pages, PageFailure, PageFetch};
