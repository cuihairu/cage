//! Remote Configuration Registry (R3, read-only): resolve a
//! `registry:<package>[@<version>]` reference against an HTTP(S) registry
//! root — `[registry] path = "https://host/registry"`.
//!
//! The protocol is deliberately minimal (design §29): anonymous GETs only,
//! one resource per file, registry entries immutable once published.
//! Documents:
//!
//! ```text
//! GET <root>/<package>/index.json              → the package index
//! GET <root>/<package>/<version>/HASHES.json   → the entry ledger
//! GET <root>/<package>/<version>/<file>        → an entry file
//! ```
//!
//! Resolution materializes the verified entry into a project-local cache
//! (`.cage-cache/registry/<url-blake3[..12]>/<package>/<version>/`). Every
//! downloaded byte is checked against the ledger's blake3 as it lands, then
//! the cache directory passes the same `verify_snapshot` trust gate as a
//! local entry («未经校验不载入»). A cached entry that re-verifies clean
//! is served without a single request — builds work offline after the first
//! fetch. Publish stays local-only (the registry never rewrites history);
//! `registry list` joins the read side (§49 L1) — one GET per named
//! package, since the protocol still has no package-name enumeration.
//!
//! The HTTP GET itself comes from `cage_core::remote` (§45): one fetch,
//! one retry policy for every remote consumer.

use cage_core::error::codes::distribution::{E2101, E2102, E2105};
use cage_core::error::codes::registry::{E1802, E1803};
use cage_core::registry::{RegistryIndex, VersionReq};
use cage_core::remote::{http_get, http_get_bearer, FetchFailure};
use std::path::{Path, PathBuf};

/// Whether a `[registry].path` is a remote root (R3): an http(s) URL.
pub(crate) fn is_remote_root(spec: &str) -> bool {
    spec.starts_with("http://") || spec.starts_with("https://")
}

/// `cage registry list --registry <remote>` (§49 L1): fetch one package's
/// index from a remote root. Anonymous by default; when `auth_env` names a
/// variable, the push-protocol bearer rides the GET — E2102 on 401/403,
/// E2105 resolved before any network contact, the exact push semantics.
/// `project_root` anchors the same `.cage-cache/registry/<url-key>/` tree
/// the R3 resolver uses: fresh bytes overwrite the cache, a transport
/// failure falls back to the cached copy (S4 offline semantics, reported
/// as `Ok((index, true))`). A 404 is the definitive "package absent" the
/// push probe also treats as normal pre-first-publish state — an empty
/// index under the package's own name, not an error.
pub(crate) fn fetch_remote_index(
    project_root: Option<&Path>,
    root_url: &str,
    package: &str,
    auth_env: Option<&str>,
) -> Result<(RegistryIndex, bool), String> {
    // Credential resolution precedes any network contact (E2105); only the
    // env var NAME may appear in errors — never the token itself.
    let token = match auth_env {
        Some(name) => {
            let value = std::env::var(name)
                .map_err(|_| format!("{E2105} list credential missing: env '{name}' is not set"))?;
            if value.is_empty() {
                return Err(format!(
                    "{E2105} list credential missing: env '{name}' is empty"
                ));
            }
            Some(value)
        }
        None => None,
    };
    let base = root_url.trim_end_matches('/');
    let index_url = format!("{base}/{package}/index.json");
    let cache_index = project_root.map(|root| {
        root.join(".cage-cache")
            .join("registry")
            .join(cage_core::registry::cache_key(root_url))
            .join(package)
            .join("index.json")
    });
    let mut from_cache = false;
    let bytes = match http_get_bearer(&index_url, token.as_deref()) {
        Ok(bytes) => {
            if let Some(path) = &cache_index {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, &bytes);
            }
            bytes
        }
        Err(FetchFailure::Transport(e)) => {
            let cached = cache_index
                .as_deref()
                .and_then(|path| std::fs::read(path).ok())
                .ok_or_else(|| {
                    format!(
                        "{E2101} cannot read remote index {index_url}: transport failure: {e}; \
                         no cached copy — pass the project root as the path argument to cache \
                         listings, or retry online"
                    )
                })?;
            from_cache = true;
            cached
        }
        Err(FetchFailure::NotFound(_)) => serde_json::to_vec(&RegistryIndex {
            package: package.to_string(),
            entries: Vec::new(),
        })
        .expect("empty index serializes"),
        Err(FetchFailure::Status(code)) if code == 401 || code == 403 => {
            return Err(format!(
                "{E2102} list rejected at {index_url}: http status {code}"
            ));
        }
        Err(FetchFailure::RateLimited(wait)) => {
            return Err(format!(
                "{E2101} cannot read remote index {index_url}: rate limited, retry after {}s",
                wait.as_secs()
            ));
        }
        Err(FetchFailure::Status(code)) => {
            return Err(format!(
                "{E2101} cannot read remote index {index_url}: http status {code}"
            ));
        }
    };
    let index: RegistryIndex = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{E1802} corrupt remote registry index {index_url}: {e}"))?;
    Ok((index, from_cache))
}

/// Reject a ledger-relative path that could escape the cache directory.
fn safe_rel(rel: &str) -> Result<&str, String> {
    let escape = rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.split('/').any(|seg| seg.is_empty() || seg == "..");
    if escape {
        return Err(format!(
            "{E1803} remote registry ledger lists unsafe path '{rel}'"
        ));
    }
    Ok(rel)
}

/// Resolve a `registry:<package>[@<version>]` reference against the remote
/// root `root_url`, returning the verified cache directory for the chosen
/// entry (E1802 on unreachable root / unknown package / unsatisfiable
/// requirement, E1803 on ledger or byte mismatches). `project_root`
/// anchors the `.cage-cache` tree; the cache key derives from the URL, so
/// distinct roots never collide.
pub(crate) fn resolve_remote(
    project_root: &Path,
    root_url: &str,
    package: &str,
    version: Option<&str>,
    requirement: Option<&VersionReq>,
) -> Result<PathBuf, String> {
    let base = root_url.trim_end_matches('/');
    let cache_dir = project_root
        .join(".cage-cache")
        .join("registry")
        .join(cage_core::registry::cache_key(root_url));

    // 1. Package index — the version list is all the protocol enumerates.
    // The fetched copy is cached (`.cage-cache/.../<package>/index.json`)
    // so offline builds keep resolving once the package was seen online;
    // a transport failure falls back to it, a 404 stays definitive.
    let index_url = format!("{base}/{package}/index.json");
    let cache_index = cache_dir.join(package).join("index.json");
    let index_bytes = match http_get(&index_url) {
        Ok(bytes) => {
            if let Some(parent) = cache_index.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{E1802} cannot create {}: {e}", parent.display()))?;
            }
            let _ = std::fs::write(&cache_index, &bytes);
            bytes
        }
        Err(FetchFailure::Transport(e)) => {
            let cached = std::fs::read(&cache_index).map_err(|_| {
                format!(
                    "{E1802} cannot reach registry: {e} ({index_url}); no cached index for \
                     {package} — run once online to populate .cage-cache"
                )
            })?;
            cached
        }
        Err(other) => return Err(format!("{E1802} {other} ({index_url})")),
    };
    let index: RegistryIndex = serde_json::from_slice(&index_bytes)
        .map_err(|e| format!("{E1802} corrupt remote registry index {index_url}: {e}"))?;
    let version = cage_core::registry::select_version(&index, version, requirement, root_url)?;

    let cache_entry = cache_dir.join(package).join(&version);
    if let Ok(report) = cage_core::snapshot::verify_snapshot(&cache_entry) {
        if report.ok {
            return Ok(cache_entry);
        }
    }

    // 2. Fetch the ledger, then every file it lists, hashing each download
    //    against the recorded blake3 before it touches the cache.
    let entry_base = format!("{base}/{package}/{version}");
    let ledger_raw = http_get(&format!("{entry_base}/HASHES.json"))
        .map_err(|e| format!("{E1802} {e} ({entry_base}/HASHES.json)"))?;
    let ledger: serde_json::Value = serde_json::from_slice(&ledger_raw)
        .map_err(|e| format!("{E1803} corrupt remote ledger for {package}/{version}: {e}"))?;
    let files = ledger
        .get("files")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            format!("{E1803} remote ledger for {package}/{version} has no 'files' object")
        })?;

    // A stale or partial cache gives way to a fresh download.
    if cache_entry.exists() {
        std::fs::remove_dir_all(&cache_entry).map_err(|e| {
            format!(
                "{E1802} cannot refresh cache {}: {e}",
                cache_entry.display()
            )
        })?;
    }
    for (rel, hash) in files {
        let rel = safe_rel(rel)?;
        let expected = hash.as_str().ok_or_else(|| {
            format!("{E1803} remote ledger for {package}/{version}: non-string hash for '{rel}'")
        })?;
        if rel == "HASHES.json" {
            // The ledger never self-hashes; our copy is written verbatim
            // below so `verify_snapshot` can run on the cache.
            continue;
        }
        let bytes = http_get(&format!("{entry_base}/{rel}"))
            .map_err(|e| format!("{E1803} {e} ({entry_base}/{rel})"))?;
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != expected {
            return Err(format!(
                "{E1803} checksum mismatch downloading {package}/{version}/{rel} \
                 (ledger {expected}, got {actual})"
            ));
        }
        let abs = cache_entry.join(rel);
        std::fs::create_dir_all(abs.parent().expect("file has a parent")).map_err(|e| {
            format!(
                "{E1802} cannot create {}: {e}",
                abs.parent().unwrap().display()
            )
        })?;
        std::fs::write(&abs, &bytes)
            .map_err(|e| format!("{E1802} cannot write cache {}: {e}", abs.display()))?;
    }
    std::fs::write(cache_entry.join("HASHES.json"), &ledger_raw)
        .map_err(|e| format!("{E1802} cannot write cache ledger: {e}"))?;

    // 3. The trust gate — identical to local resolution: the entry verifies
    //    clean or it is not handed out.
    let report =
        cage_core::snapshot::verify_snapshot(&cache_entry).map_err(|e| format!("{E1803} {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E1803} remote entry verification failed: {package}/{version} ({} problem(s)): {}",
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    Ok(cache_entry)
}
