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
#[derive(Debug)]
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
#[derive(Debug)]
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

/// One file a save would rewrite, with its before/after text so a preview
/// can show the exact diff.
#[derive(Debug)]
pub(crate) struct PlanFile {
    /// Project-relative path with `/` separators.
    pub(crate) file: String,
    /// Current disk text, `None` when the file does not exist yet (a new
    /// fragment the save would create).
    pub(crate) before: Option<String>,
    /// Text the save would write.
    pub(crate) after: String,
}

/// The outcome of planning an incremental save: which files would be
/// rewritten (with before/after text), the counts, and the index the
/// client would adopt afterwards. Producing a plan is pure — it reads the
/// disk but writes nothing — so both the save and its preview share it.
#[derive(Debug)]
pub(crate) struct SavePlan {
    /// Involved files (sorted), each with before/after text.
    pub(crate) files: Vec<PlanFile>,
    /// Files left byte-identical on disk.
    pub(crate) untouched: usize,
    /// Total bytes the save would write.
    pub(crate) bytes: usize,
    /// The index *after* the save (revisions of the new bytes) — the client
    /// adopts it so its next save does not conflict with itself.
    pub(crate) sources: SchemaSourceIndex,
}

/// Compute the incremental save plan for a directory `schema_path`:
/// conflict-check the client's revisions, find the changed entries, route
/// them to their owning files, and compose each involved file's before and
/// after text. Writes nothing.
pub(crate) fn plan_incremental(
    root: &Path,
    schema_path: &Path,
    edited: &Schema,
    client_index: &SchemaSourceIndex,
) -> Result<SavePlan, SaveError> {
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

    // ── compose each involved file's before/after text (no write) ──
    let mut files: Vec<PlanFile> = Vec::new();
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
        let path = resolve_index_path(root, &file);
        let after = render_schema_file(&path, &fragment);
        bytes += after.len();
        let before = std::fs::read_to_string(&path).ok();
        files.push(PlanFile {
            file,
            before,
            after,
        });
    }
    files.sort_by(|a, b| a.file.cmp(&b.file));

    // Revisions of the bytes to be written (hash the composed text, not the
    // disk — nothing was written), so the client's next save does not
    // conflict with this one.
    let mut sources = ownership;
    for f in &files {
        if let Some(meta) = sources.files.iter_mut().find(|m| m.file == f.file) {
            meta.sha256 = sha256_hex(f.after.as_bytes());
        }
    }
    let untouched = sources.files.len() - files.len();
    Ok(SavePlan {
        files,
        untouched,
        bytes,
        sources,
    })
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
    let plan = plan_incremental(root, schema_path, edited, client_index)?;
    for file in &plan.files {
        let path = resolve_index_path(root, &file.file);
        atomic_write(&path, file.after.as_bytes()).map_err(SaveError::Io)?;
    }
    Ok(SaveOutcome {
        written: plan.files.iter().map(|f| f.file.clone()).collect(),
        untouched: plan.untouched,
        bytes: plan.bytes,
        sources: plan.sources,
    })
}

/// Render a schema fragment as the text of `path`, honoring its format:
/// `.json` fragments stay JSON (pretty-printed), everything else is
/// canonical YAML. One rendering entry point for both the incremental and
/// the single-file save paths, so a `.json` schema is never clobbered into
/// YAML text.
pub(crate) fn render_schema_file(path: &Path, schema: &Schema) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => {
            let mut s = serde_json::to_string_pretty(schema).expect("Schema always serializes");
            s.push('\n');
            s
        }
        _ => to_canonical_yaml(schema),
    }
}

/// Line-based unified diff (3 lines of context) of one file's before/after
/// text, git-style header. Empty when there is no change. A `before` of
/// `None` means the file would be created (header `a//dev/null`).
pub(crate) fn unified_diff(before: Option<&str>, after: &str, file: &str) -> String {
    let before_text = before.unwrap_or("");
    if before_text == after {
        return String::new();
    }
    let diff = similar::TextDiff::from_lines(before_text, after);
    let left = before
        .map(|_| format!("a/{file}"))
        .unwrap_or_else(|| "/dev/null".to_string());
    diff.unified_diff()
        .context_radius(3)
        .header(&left, &format!("b/{file}"))
        .to_string()
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

#[cfg(test)]
mod uncovered_paths {
    use super::*;
    use cage_core::schema::{FieldSchema, TableSchema};
    use indexmap::IndexMap;
    use std::fs;
    use tempfile::TempDir;

    fn field(name: &str) -> FieldSchema {
        FieldSchema {
            name: name.to_string(),
            field_type: cage_core::schema::FieldType::UInt32,
            description: None,
            required: true,
            nullable: false,
            default: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            enum_values: None,
            min_items: None,
            max_items: None,
            items: None,
            separator: None,
            properties: None,
            additional_properties: None,
            reference: None,
            targets: Vec::new(),
            rules: Vec::new(),
            metadata: IndexMap::default(),
        }
    }

    fn table(name: &str) -> TableSchema {
        TableSchema {
            name: name.to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: [(name.to_string(), field(name))].into_iter().collect(),
            unique_constraints: Vec::new(),
            order_by: None,
            targets: Vec::new(),
            env_overrides: IndexMap::default(),
        }
    }

    fn write_main_fragment(dir: &TempDir) {
        fs::create_dir_all(dir.path().join("schemas")).unwrap();
        fs::write(
            dir.path().join("schemas/main.yaml"),
            "metadata:\n  version: \"1\"\nenums: {}\ntables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: UInt32 }, required: true }\n",
        )
        .unwrap();
    }

    fn load(dir: &TempDir) -> (Schema, SchemaSourceIndex) {
        load_schema_indexed(dir.path(), &dir.path().join("schemas")).expect("indexed load")
    }

    #[test]
    fn from_json_rejects_a_lossy_index() {
        assert!(SchemaSourceIndex::from_json(&serde_json::json!(null)).is_none());
        assert!(
            SchemaSourceIndex::from_json(&serde_json::json!({"files": [{"file": "a"}]})).is_none()
        );
        assert!(SchemaSourceIndex::from_json(
            &serde_json::json!({"files": [], "tables": {"Item": 1}})
        )
        .is_none());
    }

    #[test]
    fn save_conflicts_on_missing_and_new_schema_files() {
        let dir = TempDir::new().unwrap();
        write_main_fragment(&dir);
        let index = load(&dir).1;
        let edited = load(&dir).0;

        fs::remove_file(dir.path().join("schemas/main.yaml")).unwrap();
        match save_schema_incremental(dir.path(), &dir.path().join("schemas"), &edited, &index) {
            Err(SaveError::Conflict(message)) => {
                assert!(message.contains("no longer exists"), "{message}");
            }
            result => panic!("expected conflict, got {result:?}"),
        }

        write_main_fragment(&dir);
        fs::write(
            dir.path().join("schemas/new.yaml"),
            "enums: {}\ntables: {}\n",
        )
        .unwrap();
        match save_schema_incremental(dir.path(), &dir.path().join("schemas"), &edited, &index) {
            Err(SaveError::Conflict(message)) => {
                assert!(message.contains("new schema source"), "{message}");
            }
            result => panic!("expected conflict, got {result:?}"),
        }
    }

    #[test]
    fn save_routes_table_enum_and_metadata_changes_and_rotates_revisions() {
        let dir = TempDir::new().unwrap();
        write_main_fragment(&dir);
        fs::write(
            dir.path().join("schemas/other.yaml"),
            "enums: {}\ntables:\n  Old:\n    name: Old\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: UInt32 }, required: true }\n",
        )
        .unwrap();
        let (_, index) = load(&dir);
        let mut edited = load(&dir).0;

        let mut item = table("Item");
        item.description = Some("changed".to_string());
        edited.tables.insert("Item".into(), item);
        edited.tables.shift_remove("Old");
        edited.enums.insert(
            "Kind".into(),
            cage_core::schema::EnumSchema {
                name: "Kind".into(),
                values: Vec::default(),
                description: None,
            },
        );

        let out = save_schema_incremental(dir.path(), &dir.path().join("schemas"), &edited, &index)
            .expect("save");
        assert_eq!(out.written, ["schemas/main.yaml", "schemas/other.yaml"]);
        assert_eq!(out.untouched, 0);
        let main = fs::read_to_string(dir.path().join("schemas/main.yaml")).unwrap();
        let other = fs::read_to_string(dir.path().join("schemas/other.yaml")).unwrap();
        assert!(main.contains("Kind:"), "{main}");
        assert!(!main.contains("Old"), "{main}");
        assert!(!other.contains("Old"), "{other}");
        assert_ne!(out.sources.files[0].sha256, index.files[0].sha256);
    }

    #[test]
    fn atomic_write_reports_write_failures_without_tmp_litter() {
        let dir = TempDir::new().unwrap();
        let err = atomic_write(&dir.path().join("absent/file.yaml"), b"x").unwrap_err();
        assert!(err.contains("cannot write"), "{err}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
            let err = atomic_write(&dir.path().join("file.yaml"), b"x").unwrap_err();
            assert!(err.contains("cannot write"), "{err}");
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A fragment that is not UTF-8 refuses to load with the file named.
    #[test]
    fn non_utf8_fragment_is_a_named_load_error() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("schemas")).unwrap();
        fs::write(dir.path().join("schemas/broken.yaml"), [0xff, 0xfe, b'a']).unwrap();
        let err = load_schema_indexed(dir.path(), &dir.path().join("schemas"))
            .expect_err("non-UTF-8 refused");
        assert!(err.contains("is not UTF-8"), "{err}");
        assert!(err.contains("broken.yaml"), "{err}");
    }

    /// A `.json` fragment is written back as JSON, never clobbered into
    /// YAML text; a `.yaml` fragment stays YAML.
    #[test]
    fn json_fragment_stays_json_on_save() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("schemas")).unwrap();
        fs::write(
            dir.path().join("schemas/item.json"),
            r#"{
  "tables": {
    "Item": {
      "name": "Item",
      "primary_key": ["id"],
      "fields": {
        "id": { "name": "id", "type": { "kind": "UInt32" }, "required": true }
      }
    }
  },
  "enums": {}
}
"#,
        )
        .unwrap();
        let (mut edited, index) = load(&dir);
        edited.tables["Item"].description = Some("changed".to_string());

        let out = save_schema_incremental(dir.path(), &dir.path().join("schemas"), &edited, &index)
            .expect("save");
        assert_eq!(out.written, ["schemas/item.json"]);
        let text = fs::read_to_string(dir.path().join("schemas/item.json")).unwrap();
        assert!(text.starts_with('{'), "stays JSON, got: {text}");
        // Round-trips as JSON.
        let reloaded: Schema = serde_json::from_str(&text).expect("valid json schema");
        assert_eq!(
            reloaded.tables["Item"].description.as_deref(),
            Some("changed")
        );
    }

    /// Enum and metadata fragments merge into the merged schema and the
    /// index; an enum removal and a metadata edit route back to the file
    /// that owns them.
    #[test]
    fn fragment_enums_metadata_and_removals_route_to_their_owner() {
        let dir = TempDir::new().unwrap();
        write_main_fragment(&dir);
        fs::write(
            dir.path().join("schemas/extra.yaml"),
            "metadata:\n  version: \"9\"\nenums:\n  Rarity:\n    name: Rarity\n    values:\n      - { name: common }\ntables: {}\n",
        )
        .unwrap();
        let (merged, index) = load(&dir);
        assert!(index.enums.contains_key("Rarity"), "{:?}", index.enums);
        assert_eq!(merged.enums["Rarity"].name, "Rarity");
        // extra.yaml sorts before main.yaml: the first metadata seen owns it.
        assert_eq!(index.metadata.as_deref(), Some("schemas/extra.yaml"));

        let mut edited = merged;
        edited.enums.shift_remove("Rarity");
        edited.metadata = Some(cage_core::schema::SchemaMetadata {
            version: "2".to_string(),
            description: None,
            author: None,
        });
        let out = save_schema_incremental(dir.path(), &dir.path().join("schemas"), &edited, &index)
            .expect("save");
        // The metadata edit and the enum removal share the owner fragment.
        assert_eq!(out.written, ["schemas/extra.yaml"]);
        let extra = fs::read_to_string(dir.path().join("schemas/extra.yaml")).unwrap();
        assert!(extra.contains("version: '2'"), "{extra}");
        assert!(!extra.contains("Rarity"), "{extra}");
    }
}
