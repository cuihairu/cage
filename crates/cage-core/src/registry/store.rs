//! Registry store: index read/write, publish, package listing, entry
//! resolution (with ledger verification), and bundle export/import.
//! Split out of the registry module (LAY-002).

use super::{
    latest_version, select_version, valid_component, version_key, BundleCompression, ExportReport,
    ImportReport, IndexEntry, PublishReport, RegistryIndex, VersionReq, BUNDLE_ZSTD_LEVEL, E1801,
    E1802, E1803, E2101, E2103, INDEX_FILE, LEDGER_FILE, ZSTD_MAGIC,
};
use crate::snapshot;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

/// Read (or initialize) a package's index. A missing index is an empty one;
/// a corrupt index is an error (never silently rewritten by resolve).
pub(crate) fn read_index(root: &Path, package: &str) -> Result<RegistryIndex, String> {
    let index_path = root.join(package).join(INDEX_FILE);
    match fs::read(&index_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RegistryIndex {
            package: package.to_string(),
            entries: Vec::new(),
        }),
        Err(e) => Err(format!(
            "{E1802} cannot read registry index {}: {e}",
            index_path.display()
        )),
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "{E1802} corrupt registry index {}: {e}",
                index_path.display()
            )
        }),
    }
}

pub(crate) fn write_index(root: &Path, index: &RegistryIndex) -> Result<(), String> {
    let index_path = root.join(&index.package).join(INDEX_FILE);
    let mut bytes = serde_json::to_vec_pretty(index)
        .map_err(|e| format!("{E1801} cannot serialize registry index: {e}"))?;
    bytes.push(b'\n');
    fs::create_dir_all(index_path.parent().expect("index parent")).map_err(|e| {
        format!(
            "{E1801} cannot create {}: {e}",
            index_path.parent().unwrap().display()
        )
    })?;
    fs::write(&index_path, bytes).map_err(|e| {
        format!(
            "{E1801} cannot write registry index {}: {e}",
            index_path.display()
        )
    })
}

/// Recursively collect the paths of every file under `dir`, relative to
/// `dir` (so a plain `target.join(rel)` yields the entry copy path) and
/// sorted for determinism.
pub(crate) fn files_under(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let entries = fs::read_dir(&cur)
            .map_err(|e| format!("{E1801} cannot read dir {}: {e}", cur.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{E1801} dir entry error: {e}"))?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                // Registry-relative paths are portable `/`-separated keys
                // (push URLs, tar member names) — normalize the platform
                // separator at the walk so every consumer stringifies the
                // same dialect. On unix this is an identity.
                let rel = p
                    .strip_prefix(dir)
                    .expect("under dir")
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                files.push(std::path::PathBuf::from(rel));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Publish a verified snapshot directory into the registry as
/// `<package>/<version>`. The snapshot must verify clean before anything
/// is written (E1803 otherwise). Re-publishing the identical pack is an
/// idempotent no-op; the same version with a different pack is an E1801
/// version conflict. Pack identity is the whole pack — `build_id` and
/// `content_hash` — not artifact bytes alone: a schema-only change (e.g.
/// an environment-ized pack of the same data) rotates `build_id` and must
/// conflict, never silently no-op onto the old pack. The package index is
/// rewritten deterministically.
pub fn publish(
    root: &Path,
    package: &str,
    version: &str,
    snapshot_dir: &Path,
) -> Result<PublishReport, String> {
    if !valid_component(package) {
        return Err(format!(
            "{E1801} invalid registry package name '{package}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }
    if !valid_component(version) {
        return Err(format!(
            "{E1801} invalid registry version '{version}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }

    // The trust gate: only a self-verifying snapshot enters the registry.
    // verify_snapshot's own errors (unreadable/corrupt ledger) are tagged
    // with E1803 here — the gate, not the snapshot's internal reason, is
    // what the consumer reports.
    let report = snapshot::verify_snapshot(snapshot_dir).map_err(|e| format!("{E1803} {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E1803} refusing to publish unverifiable snapshot {} ({} problem(s)): {}",
            snapshot_dir.display(),
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(snapshot_dir.join(LEDGER_FILE))
            .map_err(|e| format!("{E1801} cannot read ledger: {e}"))?,
    )
    .map_err(|e| format!("{E1801} ledger parse: {e}"))?;
    let build_id = ledger
        .get("build_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "{E1801} ledger has no build_id".to_string())?
        .to_string();
    let content_hash = ledger
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "{E1801} ledger has no content_hash".to_string())?;

    let mut index = read_index(root, package)?;
    let existing = index.entries.iter().find(|e| e.version == version);
    if let Some(existing) = existing {
        // Pack identity spans the whole entry: same artifacts AND same
        // schema/build. Artifact bytes alone would let an environment-ized
        // repack of identical data silently no-op onto the old pack.
        if existing.content_hash == content_hash && existing.build_id == build_id {
            return Ok(PublishReport {
                entry_path: format!("{package}/{version}"),
                already_identical: true,
            });
        }
        return Err(format!(
            "{E1801} registry version conflict: {package}/{version} already published with \
             build_id {} / content_hash {} — republish under a new version",
            existing.build_id, existing.content_hash
        ));
    }

    // Copy every snapshot file into the entry (deterministic byte copy —
    // the snapshot is already a fixed point, so the entry matches a fresh
    // snapshot of the same inputs byte for byte).
    let target = root.join(package).join(version);
    let source_files = files_under(snapshot_dir)?;
    let file_count = source_files.len();
    for rel in &source_files {
        let abs = target.join(rel);
        fs::create_dir_all(abs.parent().expect("entry parent")).map_err(|e| {
            format!(
                "{E1801} cannot create {}: {e}",
                abs.parent().unwrap().display()
            )
        })?;
        fs::copy(snapshot_dir.join(rel), &abs)
            .map_err(|e| format!("{E1801} cannot copy {}: {e}", abs.display()))?;
    }

    index.entries.push(IndexEntry {
        version: version.to_string(),
        build_id,
        content_hash: content_hash.to_string(),
        files: file_count,
    });
    index.entries.sort_by(|a, b| {
        version_key(&a.version)
            .cmp(&version_key(&b.version))
            .then_with(|| a.version.cmp(&b.version))
    });
    write_index(root, &index)?;

    Ok(PublishReport {
        entry_path: format!("{package}/{version}"),
        already_identical: false,
    })
}

/// List every package in the registry (deterministic name order) with its
/// index — the `cage registry list` backing. A directory without an
/// `index.json` is not a package and is skipped.
pub fn packages(root: &Path) -> Result<Vec<RegistryIndex>, String> {
    let mut names = Vec::new();
    let entries =
        fs::read_dir(root).map_err(|e| format!("{E1802} cannot read {}: {e}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{E1802} dir entry error: {e}"))?;
        let p = entry.path();
        if p.is_dir() && p.join(INDEX_FILE).is_file() {
            if let Some(name) = p.file_name().and_then(std::ffi::OsStr::to_str) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.iter().map(|n| read_index(root, n)).collect()
}

/// Export a published entry as a deterministic tar bundle (A1, design §47):
/// every entry file — artifacts, `manifest.json`, `schema.json`, the
/// `HASHES.json` ledger — plus the package index excerpt, at member paths
/// `<package>/<version>/<file>` with the excerpt as `index.json`. Same
/// entry, same bytes: members are written in name order with zeroed
/// mtime/uid/gid and a fixed 0o644 mode, so the bundle is byte-reproducible
/// from the registry alone and travels over offline / audit / object-storage
/// channels; `import_bundle` (A2) re-verifies the riding ledger before
/// anything enters a registry. A missing entry (package, version, directory
/// or ledger) and any bundle write failure are E2101. `version = None`
/// exports the latest entry in dotted-numeric order.
/// Locate a published entry in a local registry for the distribution path
/// (A1/A3): names validated against the registry character set, the package
/// index read, the version resolved (explicit must exist, `None` = latest
/// in dotted-numeric order) and the entry directory plus its ledger
/// confirmed on disk. Every failure is E2101 — the entry cannot be read
/// for distribution. A package dir without an index reads as empty and
/// lands in the not-found paths.
pub(crate) fn resolve_entry(
    root: &Path,
    package: &str,
    version: Option<&str>,
) -> Result<(String, PathBuf, Vec<PathBuf>), String> {
    if !valid_component(package) {
        return Err(format!(
            "{E2101} invalid registry package name '{package}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }
    if let Some(v) = version {
        if !valid_component(v) {
            return Err(format!(
                "{E2101} invalid registry version '{v}' (allowed: letters, digits, '.', '-', '_')"
            ));
        }
    }
    let index = read_index(root, package)?;
    let version = match version {
        Some(v) => {
            if !index.entries.iter().any(|e| e.version == v) {
                return Err(format!(
                    "{E2101} registry entry not found: {package}/{v} (registry {})",
                    root.display()
                ));
            }
            v.to_string()
        }
        None => latest_version(
            &index
                .entries
                .iter()
                .map(|e| e.version.clone())
                .collect::<Vec<_>>(),
        )
        .ok_or_else(|| {
            format!(
                "{E2101} registry package not found: {package} (registry {})",
                root.display()
            )
        })?
        .to_string(),
    };
    let entry_dir = root.join(package).join(&version);
    if !entry_dir.is_dir() {
        return Err(format!(
            "{E2101} registry entry directory missing: {} (registry {})",
            entry_dir.display(),
            root.display()
        ));
    }
    let rels = files_under(&entry_dir)?;
    if !rels
        .iter()
        .any(|rel| rel == std::path::Path::new(LEDGER_FILE))
    {
        return Err(format!(
            "{E2101} registry entry has no {LEDGER_FILE} ledger: {}",
            entry_dir.display()
        ));
    }
    Ok((version, entry_dir, rels))
}

/// Rebuild the entry's index record from its riding ledger (the same
/// fields publish writes): the distribution path needs `build_id` /
/// `content_hash` to merge indexes and detect conflicts without trusting
/// any second copy of the record.
pub(crate) fn entry_record(
    entry_dir: &Path,
    version: &str,
    files: usize,
) -> Result<IndexEntry, String> {
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(entry_dir.join(LEDGER_FILE))
            .map_err(|e| format!("{E2101} cannot read ledger: {e}"))?,
    )
    .map_err(|e| format!("{E2101} ledger parse: {e}"))?;
    let build_id = ledger
        .get("build_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{E2101} ledger has no build_id"))?
        .to_string();
    let content_hash = ledger
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "{E2101} ledger has no content_hash".to_string())?;
    Ok(IndexEntry {
        version: version.to_string(),
        build_id,
        content_hash: content_hash.to_string(),
        files,
    })
}

/// Export a registry entry as a deterministic, self-verifying bundle (A1).
///
/// The bundle is a tar archive containing:
/// - `index.json` — an excerpt of the registry index covering only the exported entry.
/// - All artifact files under `package/version/...` as recorded in the entry's `files` manifest.
///
/// The tar is deterministic: all header fields (mtime, uid, gid, mode) are zeroed or fixed,
/// and members are sorted lexicographically by name. The resulting bytes are byte-for-byte
/// identical for the same source entry regardless of build environment.
///
/// With [`BundleCompression::Zstd`] the finished tar is wrapped in a single
/// zstd frame at the fixed level [`BUNDLE_ZSTD_LEVEL`]: same tar, same
/// level, same zstd library version → the same container bytes. The
/// container is transparent to [`import_bundle`], which sniffs the frame
/// magic instead of trusting extensions — and irrelevant to the trust gate,
/// because the `HASHES.json` ledger hashes uncompressed content, so a
/// zstd-wrapped bundle verifies identically to its plain form.
///
/// Returns an `ExportReport` with the bundle path, size, and content hash.
pub fn export_bundle(
    root: &Path,
    package: &str,
    version: Option<&str>,
    compression: BundleCompression,
    out: &Path,
) -> Result<ExportReport, String> {
    let (version, entry_dir, rels) = resolve_entry(root, package, version)?;

    // The excerpt carries just the exported entry — the receiving side
    // re-derives everything else from the ledger (A2).
    let excerpt = RegistryIndex {
        package: package.to_string(),
        entries: vec![entry_record(&entry_dir, &version, rels.len())?],
    };
    let mut excerpt_bytes = serde_json::to_vec_pretty(&excerpt)
        .map_err(|e| format!("{E2101} cannot serialize index excerpt: {e}"))?;
    excerpt_bytes.push(b'\n');

    let mut members: Vec<(String, Vec<u8>)> = Vec::with_capacity(rels.len() + 1);
    members.push(("index.json".to_string(), excerpt_bytes));
    for rel in &rels {
        let source = entry_dir.join(rel);
        let bytes = fs::read(&source)
            .map_err(|e| format!("{E2101} cannot read entry file {}: {e}", source.display()))?;
        members.push((
            format!("{package}/{version}/{}", rel.to_string_lossy()),
            bytes,
        ));
    }
    members.sort_by(|a, b| a.0.cmp(&b.0));

    // The deterministic tar is built in memory first, so whichever container
    // follows wraps exactly the bytes a plain export would carry.
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in &members {
        // Determinism contract: header fields we set ourselves — the tar
        // layer never sees a filesystem stat, so nothing environment-sourced
        // (mtime, uid/gid, mode) can leak into the bytes.
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|e| format!("{E2101} cannot pack bundle member '{name}': {e}"))?;
    }
    let tar_bytes = builder
        .into_inner()
        .map_err(|e| format!("{E2101} cannot finalize bundle {}: {e}", out.display()))?;

    match compression {
        BundleCompression::Plain => {
            fs::write(out, &tar_bytes)
                .map_err(|e| format!("{E2101} cannot write bundle {}: {e}", out.display()))?;
        }
        BundleCompression::Zstd => {
            // Bulk single-threaded encode at the pinned [`BUNDLE_ZSTD_LEVEL`]:
            // deterministic for a given zstd library version, keeping the
            // bundle byte-reproducible like its plain-tar form.
            let wrapped = zstd::stream::encode_all(&tar_bytes[..], BUNDLE_ZSTD_LEVEL)
                .map_err(|e| format!("{E2101} cannot compress bundle {}: {e}", out.display()))?;
            fs::write(out, &wrapped)
                .map_err(|e| format!("{E2101} cannot write bundle {}: {e}", out.display()))?;
        }
    }

    Ok(ExportReport {
        package: package.to_string(),
        version,
        files: rels.len(),
    })
}

/// Strip a bundle's compression container, if it carries one: a zstd frame
/// magic at the head selects the zstd form, anything else passes through as
/// the plain tar. The sniff is content-based, never extension-based —
/// bundles travel renamed across offline and object-storage channels, so a
/// `.tar`/`.tar.zst` suffix carries no format truth. A head that claims the
/// frame but fails to decode is a corrupt container; the caller reports it
/// against the bundle file. Compression is container-level only: the
/// `HASHES.json` ledger hashes uncompressed content, so the trust gate runs
/// on the decompressed entries exactly as it does for a plain bundle.
pub(crate) fn decompress_bundle(raw: Vec<u8>) -> Result<Vec<u8>, String> {
    if !raw.starts_with(&ZSTD_MAGIC) {
        return Ok(raw);
    }
    zstd::stream::decode_all(Cursor::new(&raw[..]))
        .map_err(|e| format!("zstd container decode failed: {e}"))
}

/// Import a bundle produced by `export_bundle` (A2, design §47): the riding
/// ledger must pass `verify_snapshot` and match the bundle's index excerpt
/// before anything enters the registry — unverifiable bytes never touch the
/// target (staging is a temp dir, discarded on every refusal path). A
/// byte-identical re-import is an idempotent no-op; the same version with
/// different bytes is the usual E1801 conflict (the registry never rewrites
/// history, not even through distribution). Structural problems — unsafe
/// member paths, malformed index, members outside the entry, missing
/// ledger — and any trust-gate refusal are E2103. The container form is
/// sniffed from the file head, not the name: plain tars load unchanged and
/// a `--compress zstd` frame decompresses first, so both forms import
/// identically under any extension. With `dry_run` the full gate runs and
/// the report comes back without writing anything.
pub fn import_bundle(root: &Path, file: &Path, dry_run: bool) -> Result<ImportReport, String> {
    let raw = fs::read(file)
        .map_err(|e| format!("{E2101} cannot read bundle {}: {e}", file.display()))?;
    let bytes = decompress_bundle(raw)
        .map_err(|e| format!("{E2103} malformed bundle {}: {e}", file.display()))?;
    let mut archive = tar::Archive::new(&bytes[..]);

    // Members are untrusted input: only plain relative paths survive the
    // component check (no absolute, no `..`, no prefix segments).
    let mut excerpt: Option<RegistryIndex> = None;
    let mut members: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in archive
        .entries()
        .map_err(|e| format!("{E2103} malformed bundle {}: {e}", file.display()))?
    {
        let mut entry = entry.map_err(|e| format!("{E2103} malformed bundle entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("{E2103} malformed bundle entry path: {e}"))?
            .into_owned();
        if path.is_absolute()
            || !path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "{E2103} unsafe bundle member path '{}'",
                path.display()
            ));
        }
        let name = path.to_string_lossy().into_owned();
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .map_err(|e| format!("{E2103} cannot read bundle member '{name}': {e}"))?;
        if name == INDEX_FILE {
            if excerpt.is_some() {
                return Err(format!("{E2103} duplicate {INDEX_FILE} in bundle"));
            }
            let parsed: RegistryIndex = serde_json::from_slice(&content)
                .map_err(|e| format!("{E2103} malformed {INDEX_FILE} in bundle: {e}"))?;
            excerpt = Some(parsed);
        } else {
            members.push((name, content));
        }
    }

    // A1 bundles carry exactly one entry: the excerpt names it, the members
    // realize it.
    let index = excerpt.ok_or_else(|| format!("{E2103} bundle has no {INDEX_FILE}"))?;
    if index.entries.len() != 1 {
        return Err(format!(
            "{E2103} bundle {INDEX_FILE} must carry exactly one entry, got {}",
            index.entries.len()
        ));
    }
    let excerpt_entry = &index.entries[0];
    let package = index.package;
    let version = excerpt_entry.version.clone();
    let prefix = format!("{package}/{version}/");
    let mut has_ledger = false;
    for (name, _) in &members {
        if !name.starts_with(&prefix) {
            return Err(format!(
                "{E2103} bundle member '{name}' is outside {prefix}"
            ));
        }
        if *name == format!("{prefix}{LEDGER_FILE}") {
            has_ledger = true;
        }
    }
    if !has_ledger {
        return Err(format!("{E2103} bundle has no {LEDGER_FILE} ledger"));
    }

    // Stage into a temp dir — the registry target stays untouched until the
    // trust gate has passed.
    let staging =
        tempfile::tempdir().map_err(|e| format!("{E2103} cannot create staging directory: {e}"))?;
    for (name, content) in &members {
        let abs = staging.path().join(&name[prefix.len()..]);
        fs::create_dir_all(abs.parent().expect("entry parent"))
            .map_err(|e| format!("{E2103} cannot stage {}: {e}", abs.display()))?;
        fs::write(&abs, content)
            .map_err(|e| format!("{E2103} cannot stage {}: {e}", abs.display()))?;
    }

    // The trust gate: same gate as publish, refused as a bundle problem.
    let report = snapshot::verify_snapshot(staging.path())
        .map_err(|e| format!("{E2103} bundle failed its trust gate: {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E2103} refusing to import unverifiable bundle {} ({} problem(s)): {}",
            file.display(),
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(staging.path().join(LEDGER_FILE))
            .map_err(|e| format!("{E2103} bundle ledger read: {e}"))?,
    )
    .map_err(|e| format!("{E2103} bundle ledger parse: {e}"))?;
    let build_id = ledger
        .get("build_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let content_hash = ledger
        .get("content_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if build_id != excerpt_entry.build_id || content_hash != excerpt_entry.content_hash {
        return Err(format!(
            "{E2103} bundle {INDEX_FILE} record does not match its ledger (build_id {build_id}, content_hash {content_hash})"
        ));
    }
    let staged_files = files_under(staging.path())?;
    if staged_files.len() != excerpt_entry.files {
        return Err(format!(
            "{E2103} bundle {INDEX_FILE} record claims {} files, ledger covers {}",
            excerpt_entry.files,
            staged_files.len()
        ));
    }

    // Conflict pre-check shares publish's rule (whole-pack identity:
    // build_id + content_hash) so dry runs report the exact outcome a real
    // import would produce.
    let existing_index = read_index(root, &package)?;
    let mut already_identical = false;
    if let Some(existing) = existing_index.entries.iter().find(|e| e.version == version) {
        if existing.content_hash != content_hash || existing.build_id != build_id {
            return Err(format!(
                "{E1801} registry version conflict: {package}/{version} already published with \
                 build_id {} / content_hash {} — import refused",
                existing.build_id, existing.content_hash
            ));
        }
        already_identical = true;
    }
    if dry_run {
        return Ok(ImportReport {
            package,
            version,
            files: staged_files.len(),
            already_identical,
            dry_run: true,
        });
    }

    let report = publish(root, &package, &version, staging.path())?;
    Ok(ImportReport {
        package,
        version,
        files: staged_files.len(),
        already_identical: report.already_identical,
        dry_run: false,
    })
}

/// Source cache (§45). Delegates to `crate::remote::cache_key`.
pub fn cache_key(root_url: &str) -> String {
    crate::remote::cache_key(root_url)
}

/// Resolve a registry reference to an entry directory. `version` omitted
/// means the highest published version (dotted-numeric order). The entry's
/// ledger is verified before it is handed out — resolving a tampered entry
/// is an E1803 error, missing package/version an E1802 error.
pub fn resolve(root: &Path, package: &str, version: Option<&str>) -> Result<PathBuf, String> {
    resolve_pinned(root, package, version, None)
}

/// Resolve a registry reference under a dependency requirement (R2): an
/// explicit `version` must satisfy `requirement` (E1802 otherwise); with
/// only a requirement, the highest published version satisfying it wins.
pub fn resolve_pinned(
    root: &Path,
    package: &str,
    version: Option<&str>,
    requirement: Option<&VersionReq>,
) -> Result<PathBuf, String> {
    let index = read_index(root, package)?;
    let display = root.display().to_string();
    let version = select_version(&index, version, requirement, &display)?;

    let entry = root.join(package).join(&version);
    let report = snapshot::verify_snapshot(&entry).map_err(|e| format!("{E1803} {e}"))?;
    if !report.ok {
        return Err(format!(
            "{E1803} registry entry verification failed: {package}/{version} ({} problem(s)): {}",
            report.mismatches.len(),
            report.mismatches.join("; ")
        ));
    }
    Ok(entry)
}
