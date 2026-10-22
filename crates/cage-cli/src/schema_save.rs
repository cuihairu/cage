//! Source-file index and incremental atomic save-back for schemas.
//!
//! `schema_path` may name one YAML file or a directory of fragments merged
//! in sorted file order (`load_schema`). The editor edits the merged
//! document, so a save-back has to know which source file each table/enum
//! came from — that mapping plus the file bytes' sha256 is the
//! [`SchemaSourceIndex`]:
//!
//! - **Load builds the index** ([`load_schema_indexed`]): every schema file
//!   gets `{file, sha256}` and every table/enum its owning file. The index
//!   rides along in `GET /api/schema` as `sources`.
//! - **Save checks conflicts** ([`save_schema_incremental`]): the client
//!   echoes the index back in the save envelope; every listed file's
//!   current sha256 must still match (and no new schema file may have
//!   appeared), else the save is refused with 409 — a stale editor must
//!   reload, never clobber.
//! - **Only involved files are rewritten**: per-table/enum comparison of
//!   the edited merged schema against the on-disk one picks the owner
//!   files of exactly the changed/added/removed entries (plus the metadata
//!   owner when the metadata changed). Untouched files are never opened
//!   for writing — their bytes stay identical.
//! - **Writes are atomic**: each rewritten file is composed as its
//!   canonical fragment, written to a sibling temp file and renamed into
//!   place, so a crash mid-save leaves the old file intact, never a
//!   truncated one.
//!
//! Safety note: client-supplied index strings are only ever *compared*
//! against file names discovered under `schema_path` on disk — no client
//! path is ever opened for writing, so the envelope cannot steer a write
//! outside the project's schema directory.
//!
//! Single-file `schema_path` keeps the pre-existing contract: the whole
//! document is saved back as canonical YAML (atomic now); the envelope and
//! index are optional there (a one-file index enables the same conflict
//! check).

use cage_core::edit::to_canonical_yaml;
use cage_core::schema::Schema;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// One schema source file and the revision it was loaded at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceFileMeta {
    /// Project-relative path with `/` separators (stable across OSes).
    pub(crate) file: String,
    /// sha256 hex of the file bytes at load time.
    pub(crate) sha256: String,
}

/// Where every table/enum/metadata of the merged schema came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SchemaSourceIndex {
    /// All schema files, sorted by `file`.
    pub(crate) files: Vec<SourceFileMeta>,
    /// Table name → owning file (project-relative).
    pub(crate) tables: BTreeMap<String, String>,
    /// Enum name → owning file (project-relative).
    pub(crate) enums: BTreeMap<String, String>,
    /// Owning file of the merged schema's metadata (the first fragment
    /// that carried one), when any.
    pub(crate) metadata: Option<String>,
}

impl SchemaSourceIndex {
    /// The wire shape of `GET /api/schema`'s `sources` field.
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "files": self.files.iter().map(|f| serde_json::json!({
                "file": f.file,
                "sha256": f.sha256,
            })).collect::<Vec<_>>(),
            "tables": self.tables,
            "enums": self.enums,
            "metadata": self.metadata,
        })
    }

    /// Parse the `sources` field echoed back by the save envelope.
    /// Unknown/missing members degrade to `None` — a parsed index is only
    /// ever *compared* against the server's own disk state, so a refused
    /// save (409) is the worst outcome of a lossy round-trip, never a
    /// wrong write.
    pub(crate) fn from_json(v: &serde_json::Value) -> Option<Self> {
        let obj = v.as_object()?;
        let files = obj
            .get("files")?
            .as_array()?
            .iter()
            .map(|f| {
                Some(SourceFileMeta {
                    file: f.get("file")?.as_str()?.to_string(),
                    sha256: f.get("sha256")?.as_str()?.to_string(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let map = |key: &str| -> Option<BTreeMap<String, String>> {
            let m = obj.get(key)?.as_object()?;
            m.iter()
                .map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        };
        Some(Self {
            files,
            tables: map("tables")?,
            enums: map("enums")?,
            metadata: obj
                .get("metadata")
                .and_then(|m| m.as_str())
                .map(String::from),
        })
    }
}

/// sha256 of bytes as lowercase hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// Project-relative path with `/` separators. When the file sits outside
/// the project root (`../shared/schemas`) the absolute path is kept, and
/// [`resolve_index_path`]'s `root.join` round-trips it (joining an
/// absolute path replaces the base).
fn rel_of(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    rel.to_string_lossy().replace('\\', "/")
}

/// Resolve an index file name back to its path (mirror of [`rel_of`]).
fn resolve_index_path(root: &Path, file: &str) -> std::path::PathBuf {
    root.join(file.replace('\\', "/"))
}

/// Load the schema at `schema_path` (file or directory, merged in sorted
/// file order) together with its [`SchemaSourceIndex`].
pub(crate) fn load_schema_indexed(
    root: &Path,
    schema_path: &Path,
) -> Result<(Schema, SchemaSourceIndex), String> {
    let mut files = crate::collect_files(schema_path, &["yaml", "yml", "json"])?;
    files.sort();
    let mut merged = Schema::new();
    let mut index = SchemaSourceIndex::default();
    for file in files {
        let rel = rel_of(root, &file);
        let bytes =
            std::fs::read(&file).map_err(|e| format!("failed to read {}: {e}", file.display()))?;
        let sha256 = sha256_hex(&bytes);
        let content = String::from_utf8(bytes)
            .map_err(|e| format!("schema {} is not UTF-8: {e}", file.display()))?;
        let fragment: Schema = match file.extension().and_then(|e| e.to_str()) {
            Some("json") => serde_json::from_str(&content)
                .map_err(|e| format!("invalid schema {}: {e}", file.display()))?,
            _ => serde_yaml::from_str(&content)
                .map_err(|e| format!("invalid schema {}: {e}", file.display()))?,
        };
        for (name, table) in fragment.tables {
            index.tables.insert(name.clone(), rel.clone());
            merged.tables.insert(name, table);
        }
        for (name, enum_schema) in fragment.enums {
            index.enums.insert(name.clone(), rel.clone());
            merged.enums.insert(name, enum_schema);
        }
        if merged.metadata.is_none() {
            let had_metadata = fragment.metadata.is_some();
            merged.metadata = fragment.metadata;
            if had_metadata {
                index.metadata = Some(rel.clone());
            }
        }
        index.files.push(SourceFileMeta { file: rel, sha256 });
    }
    Ok((merged, index))
}

/// Outcome of an incremental save.
pub(crate) struct SaveOutcome {
    /// Files rewritten (project-relative), sorted.
    pub(crate) written: Vec<String>,
    /// Files left byte-identical on disk.
    pub(crate) untouched: usize,
    /// Total canonical bytes written.
    pub(crate) bytes: usize,
    /// The index *after* the save (revisions of the new bytes) — the client
    /// adopts it so its next save does not conflict with itself.
    pub(crate) sources: SchemaSourceIndex,
}

/// Save failure: `Conflict` renders as 409, `Io` as 500.
pub(crate) enum SaveError {
    /// A source file changed (or appeared) since the client loaded it.
    Conflict(String),
    /// Disk/parse failure while checking or writing.
    Io(String),
}

/// The serialized form of a schema member (equality basis for the diff).
fn serialized<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).expect("schema members serialize")
}

/// Save the edited merged schema back to a directory `schema_path`,
/// rewriting only the files that own a changed entry.
///
/// `client_index` is the index the editor received at load time; it is
/// required (the caller refuses directory saves without one) and is
/// conflict-checked against the files on disk before anything is written.
pub(crate) fn save_schema_incremental(
    root: &Path,
    schema_path: &Path,
    edited: &Schema,
    client_index: &SchemaSourceIndex,
) -> Result<SaveOutcome, SaveError> {
    let (current, disk_index) = load_schema_indexed(root, schema_path).map_err(SaveError::Io)?;

    // ── conflict check: the loaded revisions must still be current ──
    let disk_by_file: BTreeMap<&str, &str> = disk_index
        .files
        .iter()
        .map(|f| (f.file.as_str(), f.sha256.as_str()))
        .collect();
    let client_files: std::collections::BTreeSet<&str> =
        client_index.files.iter().map(|f| f.file.as_str()).collect();
    for f in &client_index.files {
        match disk_by_file.get(f.file.as_str()) {
            Some(sha) if *sha == f.sha256 => {}
            Some(_) => {
                return Err(SaveError::Conflict(format!(
                    "conflict: schema source '{}' changed on disk since it was loaded — \
                     reload the schema and re-apply the edit",
                    f.file
                )));
            }
            None => {
                return Err(SaveError::Conflict(format!(
                    "conflict: schema source '{}' no longer exists on disk — \
                     reload the schema and re-apply the edit",
                    f.file
                )));
            }
        }
    }
    for f in &disk_index.files {
        if !client_files.contains(f.file.as_str()) {
            return Err(SaveError::Conflict(format!(
                "conflict: new schema source '{}' appeared since the schema was loaded — \
                 reload the schema and re-apply the edit",
                f.file
            )));
        }
    }

    // ── which entries changed? (presence or serialized value differs) ──
    let mut changed_tables: Vec<String> = Vec::new();
    for (name, table) in &edited.tables {
        match current.tables.get(name) {
            Some(cur) if serialized(cur) == serialized(table) => {}
            _ => changed_tables.push(name.clone()),
        }
    }
    for name in current.tables.keys() {
        if !edited.tables.contains_key(name) {
            changed_tables.push(name.clone());
        }
    }
    let mut changed_enums: Vec<String> = Vec::new();
    for (name, enum_schema) in &edited.enums {
        match current.enums.get(name) {
            Some(cur) if serialized(cur) == serialized(enum_schema) => {}
            _ => changed_enums.push(name.clone()),
        }
    }
    for name in current.enums.keys() {
        if !edited.enums.contains_key(name) {
            changed_enums.push(name.clone());
        }
    }
    let metadata_changed = serialized(&current.metadata) != serialized(&edited.metadata);

    // Ownership starts from the client's (conflict-checked) index; entries
    // the index does not know are new and go to the metadata owner (the
    // "main" fragment), falling back to the first file.
    let mut ownership = SchemaSourceIndex {
        files: disk_index.files.clone(),
        tables: client_index.tables.clone(),
        enums: client_index.enums.clone(),
        metadata: client_index.metadata.clone(),
    };
    let default_owner = client_index
        .metadata
        .clone()
        .or_else(|| client_index.files.first().map(|f| f.file.clone()))
        .unwrap_or_else(|| "schema.yaml".to_string());
    for name in &changed_tables {
        ownership
            .tables
            .entry(name.clone())
            .or_insert_with(|| default_owner.clone());
    }
    for name in &changed_enums {
        ownership
            .enums
            .entry(name.clone())
            .or_insert_with(|| default_owner.clone());
    }

    // ── involved files = owners of changed entries (+ metadata owner) ──
    let mut involved: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mark = |file: Option<String>, involved: &mut std::collections::BTreeSet<String>| {
        if let Some(f) = file {
            involved.insert(f);
        }
    };
    for name in &changed_tables {
        mark(ownership.tables.get(name).cloned(), &mut involved);
    }
    for name in &changed_enums {
        mark(ownership.enums.get(name).cloned(), &mut involved);
    }
    if metadata_changed {
        // A present-but-changed metadata is written to its owner (or the
        // default owner for a newly added one); a removed metadata is
        // dropped from the owner the client recorded.
        if let Some(owner) = ownership
            .metadata
            .clone()
            .or_else(|| edited.metadata.is_some().then(|| default_owner.clone()))
        {
            if edited.metadata.is_some() {
                ownership.metadata = Some(owner.clone());
            }
            involved.insert(owner);
        }
    }

    // ── compose and atomically replace each involved file ──
    let mut written = Vec::new();
    let mut bytes = 0usize;
    for file in involved {
        let mut fragment = Schema::new();
        for (name, table) in &edited.tables {
            if ownership.tables.get(name).map(String::as_str) == Some(file.as_str()) {
                fragment.tables.insert(name.clone(), table.clone());
            }
        }
        for (name, enum_schema) in &edited.enums {
            if ownership.enums.get(name).map(String::as_str) == Some(file.as_str()) {
                fragment.enums.insert(name.clone(), enum_schema.clone());
            }
        }
        if ownership.metadata.as_deref() == Some(file.as_str()) {
            fragment.metadata.clone_from(&edited.metadata);
        }
        let yaml = to_canonical_yaml(&fragment);
        bytes += yaml.len();
        let path = resolve_index_path(root, &file);
        atomic_write(&path, yaml.as_bytes()).map_err(SaveError::Io)?;
        written.push(file);
    }
    written.sort();

    // Revisions of the bytes just written — read back from disk, so the
    // client's next save does not conflict with this one.
    let mut sources = ownership;
    for file in &written {
        let path = resolve_index_path(root, file);
        let content = std::fs::read(&path)
            .map_err(|e| SaveError::Io(format!("cannot read {}: {e}", path.display())))?;
        if let Some(f) = sources.files.iter_mut().find(|f| f.file == *file) {
            f.sha256 = sha256_hex(&content);
        }
    }
    let untouched = sources.files.len() - written.len();
    Ok(SaveOutcome {
        written,
        untouched,
        bytes,
        sources,
    })
}

/// Write `bytes` to `path` atomically: sibling temp file + rename, so a
/// crash leaves either the old bytes or the new bytes, never a truncated
/// file.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_file_name(format!(
        ".{}.tmp.{}",
        path.file_name()
            .map_or_else(|| "schema".to_string(), |n| n.to_string_lossy().to_string()),
        std::process::id()
    ));
    let result = std::fs::write(&tmp, bytes)
        .map_err(|e| format!("cannot write {}: {e}", tmp.display()))
        .and_then(|()| {
            std::fs::rename(&tmp, path)
                .map_err(|e| format!("cannot replace {}: {e}", path.display()))
        });
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
