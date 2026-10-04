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
//! fetch. Publish and list stay local-only: the registry never rewrites
//! history, and the protocol has no package enumeration.
//!
//! The HTTP GET itself comes from `cage_core::remote` (§45): one fetch,
//! one retry policy for every remote consumer.

use cage_core::error::codes::registry::{E1802, E1803};
use cage_core::registry::{RegistryIndex, VersionReq};
use cage_core::remote::{http_get, FetchFailure};
use std::path::{Path, PathBuf};

/// Whether a `[registry].path` is a remote root (R3): an http(s) URL.
pub(crate) fn is_remote_root(spec: &str) -> bool {
    spec.starts_with("http://") || spec.starts_with("https://")
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
