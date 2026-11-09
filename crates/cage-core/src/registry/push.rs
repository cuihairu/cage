//! Registry push routing (R3): local push, presigned remote push, and the
//! route decision between them. Split out of the registry module (LAY-002).

use super::{
    entry_record, fs, resolve_entry, version_key, BTreeMap, IndexEntry, RegistryIndex, E1801,
    E2101, E2102, E2104, E2105,
};
use std::path::Path;

/// Result of `push_entry`: what was (or would be) uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    /// Package pushed
    pub package: String,
    /// Entry version pushed (explicitly requested, or dotted-numeric latest
    /// when the version was omitted)
    pub version: String,
    /// Entry files uploaded (the package index PUT is counted separately —
    /// it always lands last)
    pub files: usize,
    /// `true` when the remote already held the exact bytes (idempotent
    /// no-op — zero PUTs), `false` when files were uploaded
    pub already_identical: bool,
    /// `true` when the push ran as a report-only dry run (zero PUTs)
    pub dry_run: bool,
}

/// Push a published entry from a local registry to a remote http(s)
/// registry root (A3, design §47): one PUT per entry file
/// (`<root>/<package>/<version>/<file>`), the package `index.json` written
/// last — merged over the remote's existing entries, so a push never
/// rewrites remote history. Credentials: `auth_env` names the environment
/// variable carrying the bearer token; it must resolve before any network
/// contact (E2105) and the value travels only in the `Authorization`
/// header — never logged, never persisted. `auth_env = None` pushes
/// anonymously (unauthenticated deployments). Failure mapping: the local
/// entry unreadable → E2101; the remote index unreadable or a PUT failing
/// transport after retries → E2101; HTTP 401/403 → E2102; any other
/// refusing status (405/501/…) → E2104 — the server has no write channel.
/// The remote already holding the same version with different bytes is an
/// E1801 conflict (remote history is never rewritten); identical bytes are
/// an idempotent no-op with zero PUTs. With `dry_run` the local read,
/// credential resolution and remote-state check all run, but nothing is
/// uploaded.
pub fn push_entry(
    source_root: &Path,
    remote_root: &str,
    package: &str,
    version: Option<&str>,
    auth_env: Option<&str>,
    dry_run: bool,
) -> Result<PushReport, String> {
    let base = remote_root.trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(format!(
            "{E2101} push targets a remote http(s) registry root, got '{remote_root}'"
        ));
    }

    // Credential resolution precedes any network contact (E2105); only the
    // env var NAME may appear in errors — never the token itself.
    let token = match auth_env {
        Some(name) => {
            let value = std::env::var(name).map_err(|_| {
                format!(
                    "{E2105} push credential missing: env '{name}' is not set (declared via \
                     --auth-env or [registry].auth_env)"
                )
            })?;
            if value.is_empty() {
                return Err(format!(
                    "{E2105} push credential missing: env '{name}' is empty"
                ));
            }
            Some(value)
        }
        None => None,
    };
    let route = PushRoute::Direct {
        base,
        token: token.as_deref(),
    };
    push_entry_inner(source_root, package, version, &route, dry_run)
}

/// One presigned upload target set (A3 presigned route, design §47):
/// every URL is issued by whoever owns the object store — cage never
/// derives one URL from another. `uploads` maps an entry-relative path
/// (`cfg/1.0.0/schema.json`) to its presigned PUT URL; the package
/// index rides its own GET/PUT pair, so the client-side merge and the
/// E1801 conflict guard survive unchanged (a presigned GET is still a
/// GET). The URL issuer owns immutability enforcement — S3 conditional
/// writes, versioned buckets, one-shot key conventions — because a
/// presigned PUT carries no cage-side state probe of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresignMap {
    /// entry-relative path (leading `/` tolerated on input) → PUT URL
    pub uploads: BTreeMap<String, String>,
    /// Presigned GET for the package index (absent = no remote index yet)
    pub index_get: Option<String>,
    /// Presigned PUT for the merged package index
    pub index_put: String,
}

impl PresignMap {
    /// The presigned PUT for an entry-relative path (`/pkg/1.0.0/f` or
    /// `pkg/1.0.0/f` both resolve, whether the map came from
    /// [`parse_presign_map`] or was built directly).
    fn upload_url(&self, rel_path: &str) -> Option<&str> {
        self.uploads
            .get(rel_path.trim_start_matches('/'))
            .or_else(|| self.uploads.get(rel_path))
            .map(String::as_str)
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct PresignMapFile {
    #[serde(default)]
    uploads: BTreeMap<String, String>,
    index: Option<PresignIndexUrls>,
}

#[derive(serde::Deserialize)]
pub(crate) struct PresignIndexUrls {
    #[serde(default)]
    get: Option<String>,
    put: Option<String>,
}

/// Parse a presign map (`--presign-map <file>`): a JSON object with an
/// `uploads` path → URL map and an `index` object carrying `get`
/// (optional) and `put`. Every URL must be absolute http(s) — a
/// relative path here would silently PUT somewhere unintended — and
/// keys normalize by dropping a leading `/`. Malformed input is E2101
/// (a local input problem, same family as an unreadable entry).
pub fn parse_presign_map(bytes: &[u8]) -> Result<PresignMap, String> {
    let file: PresignMapFile = serde_json::from_slice(bytes)
        .map_err(|e| format!("{E2101} unreadable presign map: {e}"))?;
    let check_url = |what: &str, url: &str| -> Result<(), String> {
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(())
        } else {
            Err(format!(
                "{E2101} presign map {what} is not an absolute http(s) URL: {url}"
            ))
        }
    };
    let mut uploads = BTreeMap::new();
    for (path, url) in &file.uploads {
        check_url(&format!("upload '{path}'"), url)?;
        uploads.insert(path.trim_start_matches('/').to_string(), url.clone());
    }
    let index = file
        .index
        .ok_or_else(|| format!("{E2101} presign map has no 'index' object"))?;
    if let Some(get) = &index.get {
        check_url("index.get", get)?;
    }
    let index_put = index
        .put
        .ok_or_else(|| format!("{E2101} presign map has no 'index.put' URL"))?;
    check_url("index.put", &index_put)?;
    Ok(PresignMap {
        uploads,
        index_get: index.get,
        index_put,
    })
}

/// Push a published entry through a presign map (A3 presigned route,
/// design §47): the same discipline as [`push_entry`] — local entry
/// first, remote index read for merge/conflict, every entry file
/// uploaded, the merged package index last — with the URLs, and the
/// only authorization, coming from the map. No bearer token is read or
/// sent: the URLs are the grant. Coverage is checked before the first
/// PUT — a path the map does not cover fails the push outright rather
/// than uploading a partial entry whose index would point at nothing.
pub fn push_entry_presigned(
    source_root: &Path,
    map: &PresignMap,
    package: &str,
    version: Option<&str>,
    dry_run: bool,
) -> Result<PushReport, String> {
    push_entry_inner(
        source_root,
        package,
        version,
        &PushRoute::Presigned(map),
        dry_run,
    )
}

/// How a push reaches its destinations — the shared body reads the
/// local entry, probes the remote index and merges; the route supplies
/// the index probe and the PUTs.
pub(crate) enum PushRoute<'a> {
    /// Plain PUTs against a registry root, optional bearer token.
    Direct {
        base: &'a str,
        token: Option<&'a str>,
    },
    /// Per-object presigned URLs; the index has its own GET/PUT pair.
    Presigned(&'a PresignMap),
}

impl PushRoute<'_> {
    /// Read the remote package index: `Ok(Some(bytes))` when it exists,
    /// `Ok(None)` when the remote has no such package yet. Errors carry
    /// their E-code and the URL they failed on.
    fn probe_index(&self, package: &str) -> Result<Option<Vec<u8>>, String> {
        let (url, presigned) = match self {
            PushRoute::Direct { base, .. } => (format!("{base}/{package}/index.json"), false),
            PushRoute::Presigned(map) => match &map.index_get {
                Some(url) => (url.clone(), true),
                None => return Ok(None),
            },
        };
        let what = if presigned {
            "presigned index GET"
        } else {
            "remote index"
        };
        match crate::remote::http_get(&url) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(crate::remote::FetchFailure::NotFound(_)) => Ok(None),
            // An auth rejection on the state probe is an auth rejection,
            // full stop — mapped before the generic unreachable case.
            Err(crate::remote::FetchFailure::Status(code)) if code == 401 || code == 403 => Err(
                format!("{E2102} push rejected at {url}: http status {code}"),
            ),
            Err(e) => Err(format!("{E2101} cannot read {what} {url}: {e}")),
        }
    }

    /// Upload one object. `rel_path` is the registry-relative path
    /// (`/pkg/1.0.0/file`) — the message key and, on the direct route,
    /// the URL suffix.
    fn put(&self, rel_path: &str, body: &[u8]) -> Result<(), String> {
        let (url, token) = match self {
            PushRoute::Direct { base, token } => (format!("{base}{rel_path}"), *token),
            PushRoute::Presigned(map) => match map.upload_url(rel_path) {
                Some(url) => (url.to_string(), None),
                // Unreachable: coverage is checked before the first PUT.
                None => {
                    return Err(format!(
                        "{E2101} presign map does not cover {rel_path} — nothing uploaded"
                    ))
                }
            },
        };
        Self::send_put(rel_path, &url, token, body)
    }

    /// Upload the merged package index. The direct route uses the same
    /// root path as every file; the presigned route has its dedicated
    /// `index_put` URL — the map's `uploads` never carries the index.
    fn put_index(&self, package: &str, body: &[u8]) -> Result<(), String> {
        let rel_path = format!("/{package}/index.json");
        let (url, token) = match self {
            PushRoute::Direct { base, token } => (format!("{base}{rel_path}"), *token),
            PushRoute::Presigned(map) => (map.index_put.clone(), None),
        };
        Self::send_put(&rel_path, &url, token, body)
    }

    /// One PUT with the shared failure mapping (E2101 transport /
    /// E2102 auth / E2104 no-write-channel), keyed by `rel_path`.
    fn send_put(rel_path: &str, url: &str, token: Option<&str>, body: &[u8]) -> Result<(), String> {
        crate::remote::http_put(url, token, body).map_err(|e| match e {
            crate::remote::PutFailure::Transport(detail) => {
                format!("{E2101} push transport failed for {rel_path}: {detail}")
            }
            crate::remote::PutFailure::AuthRejected(code) => {
                format!("{E2102} push rejected for {rel_path}: http status {code}")
            }
            crate::remote::PutFailure::WriteRefused(code) => {
                format!(
                    "{E2104} push refused for {rel_path}: http status {code} (the server has \
                     no write channel — publish locally and host statically)"
                )
            }
            crate::remote::PutFailure::TooLarge(n) => {
                format!(
                    "{E2101} push transport failed for {rel_path}: response body exceeds the {n} byte cap"
                )
            }
        })
    }
}

/// The shared push body (direct and presigned routes): local entry read
/// → remote index probe (merge / no-op / E1801 conflict) → coverage
/// check (presigned) → optional dry-run stop → file PUTs → merged index
/// PUT last.
pub(crate) fn push_entry_inner(
    source_root: &Path,
    package: &str,
    version: Option<&str>,
    route: &PushRoute<'_>,
    dry_run: bool,
) -> Result<PushReport, String> {
    // Local side first: the entry must read cleanly before anything remote
    // is contacted.
    let (version, entry_dir, rels) = resolve_entry(source_root, package, version)?;
    let record = entry_record(&entry_dir, &version, rels.len())?;

    // Remote state: the package index decides merge vs no-op vs conflict.
    let remote_entries: Vec<IndexEntry> = match route.probe_index(package)? {
        Some(bytes) => {
            let remote: RegistryIndex = serde_json::from_slice(&bytes)
                .map_err(|e| format!("{E2101} cannot parse remote index for '{package}': {e}"))?;
            if let Some(existing) = remote.entries.iter().find(|e| e.version == version) {
                if existing.content_hash == record.content_hash {
                    return Ok(PushReport {
                        package: package.to_string(),
                        version,
                        files: rels.len(),
                        already_identical: true,
                        dry_run,
                    });
                }
                return Err(format!(
                    "{E1801} registry version conflict: {package}/{version} already on the remote \
                     with content_hash {} — push refused (remote history is never rewritten)",
                    existing.content_hash
                ));
            }
            remote.entries
        }
        None => Vec::new(),
    };

    // Coverage before any PUT (presigned): a partial upload would leave
    // the remote index pointing at files that never landed.
    if let PushRoute::Presigned(map) = route {
        let missing: Vec<String> = rels
            .iter()
            .map(|rel| format!("/{package}/{version}/{}", rel.to_string_lossy()))
            .filter(|rel| map.upload_url(rel).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "{E2101} presign map covers {} of {} entry file(s) — missing: {} (nothing uploaded)",
                rels.len() - missing.len(),
                rels.len(),
                missing.join(", ")
            ));
        }
    }

    if dry_run {
        return Ok(PushReport {
            package: package.to_string(),
            version,
            files: rels.len(),
            already_identical: false,
            dry_run: true,
        });
    }

    for rel in &rels {
        let source = entry_dir.join(rel);
        let body = fs::read(&source)
            .map_err(|e| format!("{E2101} cannot read entry file {}: {e}", source.display()))?;
        route.put(
            &format!("/{package}/{version}/{}", rel.to_string_lossy()),
            &body,
        )?;
    }

    // The package index lands last (design §47): only after every entry
    // file is in place, merged over the remote's existing entries.
    let mut merged = remote_entries;
    merged.retain(|e| e.version != version);
    merged.push(record);
    merged.sort_by(|a, b| {
        version_key(&a.version)
            .cmp(&version_key(&b.version))
            .then_with(|| a.version.cmp(&b.version))
    });
    let merged_index = RegistryIndex {
        package: package.to_string(),
        entries: merged,
    };
    let mut index_bytes = serde_json::to_vec_pretty(&merged_index)
        .map_err(|e| format!("{E2101} cannot serialize merged index: {e}"))?;
    index_bytes.push(b'\n');
    route.put_index(package, &index_bytes)?;

    Ok(PushReport {
        package: package.to_string(),
        version,
        files: rels.len(),
        already_identical: false,
        dry_run: false,
    })
}
