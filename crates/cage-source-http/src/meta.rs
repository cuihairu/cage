//! The validators sidecar cache (`<key>.meta.json`) that lets an
//! unchanged remote answer 304 Not Modified (split out of lib.rs, LAY-021).

use cage_core::error::codes::internal::E9902;
use cage_core::remote::{self, Validators};
use std::path::{Path, PathBuf};

/// The validators sidecar for one cache slot: `<key>.meta.json` beside
/// the payload bytes. `None` when absent — revalidation is best-effort,
/// a missing meta just means a plain fetch.
pub(crate) fn read_meta(cache_dir: &Path, url: &str) -> Result<Option<Validators>, String> {
    let meta_file = meta_path(cache_dir, url);
    let raw = match std::fs::read(&meta_file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "{E9902} cannot read source cache meta {}: {e}",
                meta_file.display()
            ))
        }
    };
    let meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| format!("{E9902} cannot parse {}: {e}", meta_file.display()))?;
    let etag = meta
        .get("etag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let last_modified = meta
        .get("last_modified")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Ok(Some(Validators {
        etag,
        last_modified,
    }))
}

/// Store the validators a 200 response handed out — written even when
/// the server sent neither header (an empty meta keeps the sidecar
/// honest: the next fetch sees "no validators" instead of a stale pair).
pub(crate) fn write_meta(
    cache_dir: &Path,
    url: &str,
    validators: &Validators,
) -> Result<(), String> {
    let meta_file = meta_path(cache_dir, url);
    let raw = serde_json::to_vec(&serde_json::json!({
        "etag": validators.etag,
        "last_modified": validators.last_modified,
    }))
    .expect("serializing the validators meta cannot fail");
    std::fs::write(&meta_file, raw).map_err(|e| {
        format!(
            "{E9902} cannot write source cache meta {}: {e}",
            meta_file.display()
        )
    })
}

pub(crate) fn meta_path(cache_dir: &Path, url: &str) -> PathBuf {
    cache_dir.join(format!("{}.meta.json", remote::cache_key(url)))
}
