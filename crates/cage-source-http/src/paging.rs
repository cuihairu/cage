//! HTTP pagination walk: bounded next-link following, conditional GET
//! revalidation and the JSON-array page shape gate (split out of lib.rs,
//! LAY-021).

use cage_core::remote::{self, FetchFailure, Validators};
use std::collections::HashSet;

/// Hard bound on the pagination walk: a server must not turn one source
/// into an unbounded request loop. 1000 pages is far past any real
/// config row set and still bounded work for a build.
const MAX_PAGES: usize = 1000;

/// One failure of the paginated fetch. `Http` wraps the shared failure
/// kinds verbatim; the rest are pagination-contract violations — the
/// server spoke, so they are definitive and never offline-fallback
/// material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageFailure {
    Http(FetchFailure),
    /// A page body is not valid JSON or not a JSON array
    NotArray(String),
    /// A next link that cannot resolve against its page URL
    BadLink(String),
    /// A resolved page URL the walk already fetched
    Cycle(String),
    /// More than the walked bound: the bound, then the page URL
    TooManyPages(usize, String),
}

impl std::fmt::Display for PageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PageFailure::Http(e) => write!(f, "{e}"),
            PageFailure::NotArray(url) => {
                write!(f, "pagination page is not a JSON array: {url}")
            }
            PageFailure::BadLink(url) => {
                write!(f, "pagination next link is not a usable URL: {url}")
            }
            PageFailure::Cycle(url) => write!(f, "pagination cycle at: {url}"),
            PageFailure::TooManyPages(bound, url) => {
                write!(f, "pagination exceeded {bound} pages at: {url}")
            }
        }
    }
}

/// One paginated fetch outcome. `NotModified` only ever comes from the
/// first GET (the revalidation round trip); `Fresh` carries the merged
/// bytes and the first response's validators for the cache meta.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PageFetch {
    NotModified,
    Fresh {
        bytes: Vec<u8>,
        validators: Validators,
    },
}

/// Fetch `first` — revalidating against `validators` when the cache has
/// them — and, when the server advertises an RFC 8288 `Link: rel="next"`,
/// walk the pages: every page is a JSON array of rows, relative next
/// targets resolve against their page URL (RFC 3986), pages concatenate
/// in link order, and the merged array is the returned document. A 304
/// on the first GET is [`PageFetch::NotModified`] — the walk is skipped
/// entirely, the cached merged document stays authoritative. Follow-up
/// pages are plain GETs: the first page's validators describe the
/// collection state, so a fresh first page already means new work. No
/// next link on the first response returns the body bytes verbatim — the
/// single-GET contract is untouched.
pub(crate) fn fetch_pages(
    first: &str,
    validators: Option<&Validators>,
) -> Result<PageFetch, PageFailure> {
    fetch_pages_bound(first, validators, MAX_PAGES)
}

pub(crate) fn fetch_pages_bound(
    first: &str,
    validators: Option<&Validators>,
    max_pages: usize,
) -> Result<PageFetch, PageFailure> {
    let first_url = url::Url::parse(first).map_err(|_| PageFailure::BadLink(first.to_string()))?;
    let response = match validators {
        Some(v) => match remote::http_get_conditional(first, v).map_err(PageFailure::Http)? {
            remote::ConditionalGet::NotModified => return Ok(PageFetch::NotModified),
            remote::ConditionalGet::Fresh(response) => response,
        },
        None => remote::http_get_full(first).map_err(PageFailure::Http)?,
    };
    let fresh_validators = Validators {
        etag: response.etag.clone(),
        last_modified: response.last_modified.clone(),
    };
    let Some(next_raw) = response.link.as_deref().and_then(next_page_link) else {
        return Ok(PageFetch::Fresh {
            bytes: response.body,
            validators: fresh_validators,
        });
    };

    let mut merged = page_array(&response.body, first)?;
    let mut visited: HashSet<String> = HashSet::from([first_url.to_string()]);
    let mut current = first_url;
    let mut next = Some(next_raw);
    while let Some(raw) = next {
        current = current
            .join(&raw)
            .map_err(|_| PageFailure::BadLink(raw.clone()))?;
        let key = current.to_string();
        if !visited.insert(key.clone()) {
            return Err(PageFailure::Cycle(key));
        }
        if visited.len() > max_pages {
            return Err(PageFailure::TooManyPages(max_pages, key));
        }
        let response = remote::http_get_full(current.as_str()).map_err(PageFailure::Http)?;
        merged.extend(page_array(&response.body, current.as_str())?);
        next = response.link.as_deref().and_then(next_page_link);
    }
    Ok(PageFetch::Fresh {
        bytes: serde_json::to_vec(&merged).expect("serializing merged row arrays cannot fail"),
        validators: fresh_validators,
    })
}
/// A page body must be a JSON array of rows — the only shape that
/// merges across pages. Anything else (object, scalar, garbage) is a
/// contract violation naming the page.
fn page_array(body: &[u8], page_url: &str) -> Result<Vec<serde_json::Value>, PageFailure> {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Array(rows)) => Ok(rows),
        _ => Err(PageFailure::NotArray(page_url.to_string())),
    }
}

/// The `rel="next"` target of one `Link` header value (RFC 8288):
/// members are `<uri-reference>; key=value` separated by top-level
/// commas (commas inside the angle brackets never split), and `rel` is
/// a case-insensitive space-separated token list. Only the target of
/// the first `next` member wins.
pub(crate) fn next_page_link(header: &str) -> Option<String> {
    for member in split_link_members(header) {
        let open = member.find('<')?;
        let close = open + member[open..].find('>')?;
        let target = member[open + 1..close].trim();
        if member[close + 1..].split(';').any(param_is_rel_next) {
            return Some(target.to_string());
        }
    }
    None
}

/// `true` when one `;`-parameter is a `rel` whose token list contains
/// `next` (case-insensitive, quotes stripped).
pub(crate) fn param_is_rel_next(param: &str) -> bool {
    let Some(rest) = param.trim().strip_prefix("rel") else {
        return false;
    };
    let Some(value) = rest.trim().strip_prefix('=') else {
        return false;
    };
    value
        .trim()
        .trim_matches('"')
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("next"))
}

/// Split a `Link` header into members at top-level commas — commas
/// inside `<...>` targets never split.
fn split_link_members(header: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, c) in header.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                members.push(&header[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    members.push(&header[start..]);
    members
}
