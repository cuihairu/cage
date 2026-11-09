//! Build Manifest - deterministic build artifacts with hashes for traceability

use crate::value::Document;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Build manifest generated after successful build
#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct BuildManifest {
    /// Project name
    pub project: String,
    /// Build profile name
    pub profile: String,
    /// Environment the build validated under (`--env`), when one was
    /// given; `None` = base schema, no environment overrides applied.
    /// Environments rotate `schema_hash`/`build_id` through the resolved
    /// schema — this field names the environment itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Cage version that produced the build
    pub cage_version: String,
    /// Manifest structure version (bump when the field layout evolves);
    /// independent of the cage toolchain version
    #[serde(default)]
    pub generator_version: String,
    /// Deterministic build id: blake3 fingerprint of
    /// `(profile + schema_hash + source_hash + content_hash)`. Identical
    /// inputs yield the identical id (same input → same manifest bytes, the
    /// determinism contract), any semantic change rotates it. Deliberately
    /// NOT wall-clock unique — a timestamp here would break deterministic
    /// builds.
    #[serde(default)]
    pub build_id: String,
    /// Blake3 hash of the schema document
    pub schema_hash: String,
    /// Blake3 hash of all source content
    pub source_hash: String,
    /// Blake3 hash of all artifact content
    pub content_hash: String,
    /// Reference topology: table -> sorted list of tables it references
    /// (the dependency graph's forward edges, as manifest 账本)
    #[serde(default)]
    pub dependencies: IndexMap<String, Vec<String>>,
    /// Per-table source hash (deterministic rows fingerprint): the change
    /// detector for incremental builds — a table whose hash moved is
    /// "changed", its transitive dependents are "affected".
    #[serde(default)]
    pub table_hashes: IndexMap<String, String>,
    /// Target configuration fingerprints (one per target of the built
    /// profile, sorted canonically): the target-level change detector —
    /// a target whose hash moved (or was added) regenerates all of its
    /// artifacts, a target removed from the profile has its stale
    /// artifacts deleted. Empty on legacy manifests; the incremental
    /// build takes one full build to record them.
    #[serde(default)]
    pub targets: Vec<TargetRecord>,
    /// Artifacts by output path
    pub artifacts: IndexMap<String, ArtifactInfo>,
}

/// Fingerprint of one target's configuration: identity is
/// `(format, output_dir)` and `hash` covers every bytes-affecting field
/// (`format`, `output_dir`, `file_template`, `options`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRecord {
    /// Output format (json, csv, msgpack, csharp, ...)
    pub format: String,
    /// Output directory (project-relative)
    pub output_dir: String,
    /// Blake3 over the canonical config: `format`, `output_dir`,
    /// `file_template` and `options` (option keys sorted)
    pub hash: String,
}

/// Individual artifact information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactInfo {
    /// Output path (relative to project root)
    pub path: String,
    /// Blake3 hash of content
    pub hash: String,
    /// Content size in bytes
    pub size: usize,
    /// Output format (json, csv, csharp, etc.)
    pub format: String,
    /// Which table this artifact represents
    pub table: Option<String>,
    /// Text encoding (utf-8, binary, etc.)
    pub encoding: String,
}

/// Manifest generator
pub struct ManifestGenerator {
    project_name: String,
    profile_name: String,
    cage_version: String,
    targets: Vec<TargetRecord>,
    environment: Option<String>,
    /// Extra bytes mixed into `source_hash` / `build_id` that are not part
    /// of the parsed Canonical Model — the script-adapter fingerprint
    /// (design §51 Q2): the script bytes, the resolved path, the canonical
    /// `script_config` and the contract version. `None` means no extra input
    /// (no script adapter declared).
    source_extra: Option<String>,
}

impl ManifestGenerator {
    /// Manifest structure version — bump on breaking layout changes
    pub const GENERATOR_VERSION: &str = "1.2.0";

    /// Create a generator for a project / profile / tool version
    pub fn new(project_name: String, profile_name: String, cage_version: String) -> Self {
        Self {
            project_name,
            profile_name,
            cage_version,
            targets: Vec::new(),
            environment: None,
            source_extra: None,
        }
    }

    /// Mix the script-adapter fingerprint into `source_hash` / `build_id`
    /// (design §51 Q2: editing the script or its config rotates the id).
    pub fn with_source_extra(mut self, extra: Option<String>) -> Self {
        self.source_extra = extra;
        self
    }

    /// Record the build's environment (`--env`). `None` (or `Some("")`)
    /// means base schema.
    pub fn with_environment(mut self, env: Option<&str>) -> Self {
        self.environment = env.filter(|e| !e.is_empty()).map(str::to_string);
        self
    }

    /// Record the built profile's targets in the manifest (the incremental
    /// build's target-level change detector). Without this the manifest
    /// carries no target fingerprints.
    pub fn with_targets(mut self, targets: &[TargetConfig]) -> Self {
        self.targets = Self::target_records(targets);
        self
    }

    /// Canonical target fingerprints, sorted by (`format`, `output_dir`).
    /// The hash covers every bytes-affecting field: `format`,
    /// `output_dir`, `file_template` and `options` (option keys sorted;
    /// nested option values serialize as `serde_json` emits them — same
    /// file, same bytes).
    pub fn target_records(targets: &[TargetConfig]) -> Vec<TargetRecord> {
        let mut records: Vec<TargetRecord> = targets
            .iter()
            .map(|t| {
                let mut lines = vec![
                    t.format.clone(),
                    t.output_dir.clone(),
                    t.file_template.clone().unwrap_or_default(),
                ];
                if let Some(opts) = &t.options {
                    let mut keys: Vec<&String> = opts.keys().collect();
                    keys.sort();
                    for key in keys {
                        lines.push(format!("{key}={}", opts[key]));
                    }
                }
                TargetRecord {
                    format: t.format.clone(),
                    output_dir: t.output_dir.clone(),
                    hash: blake3::hash(lines.join("\n").as_bytes())
                        .to_hex()
                        .to_string(),
                }
            })
            .collect();
        records.sort_by(|a, b| (&a.format, &a.output_dir).cmp(&(&b.format, &b.output_dir)));
        records
    }

    /// Generate manifest from build outputs
    pub fn generate(
        &self,
        schema: &crate::schema::Schema,
        document: &Document,
        artifacts: &[(String, Vec<u8>, String, Option<String>)], // (path, content, format, table)
    ) -> BuildManifest {
        let schema_hash = Self::hash_schema(schema);
        let source_hash = Self::hash_source(document);
        let source_hash = Self::mix_extra(source_hash, self.source_extra.as_deref());
        let content_hash = Self::hash_content(artifacts);
        let build_id = Self::build_id(
            &self.profile_name,
            &schema_hash,
            &source_hash,
            &content_hash,
        );
        let dependencies = Self::dependency_ledger(schema);
        let table_hashes = Self::table_hashes(document);
        let targets = self.targets.clone();

        let mut artifact_infos = IndexMap::new();
        for (path, content, format, table) in artifacts {
            let hash = blake3::hash(content).to_hex().to_string();
            artifact_infos.insert(
                path.clone(),
                ArtifactInfo {
                    path: path.clone(),
                    hash,
                    size: content.len(),
                    format: format.clone(),
                    table: table.clone(),
                    encoding: "utf-8".to_string(),
                },
            );
        }

        BuildManifest {
            project: self.project_name.clone(),
            profile: self.profile_name.clone(),
            environment: self.environment.clone(),
            cage_version: self.cage_version.clone(),
            generator_version: Self::GENERATOR_VERSION.to_string(),
            build_id,
            schema_hash,
            source_hash,
            content_hash,
            dependencies,
            table_hashes,
            targets,
            artifacts: artifact_infos,
        }
    }

    /// Reference topology for the manifest: for each table, the sorted list
    /// of tables it references (deterministic order — `HashSet` iteration is
    /// not). Empty when the schema has no reference edges.
    fn dependency_ledger(schema: &crate::schema::Schema) -> IndexMap<String, Vec<String>> {
        let graph = crate::reference::DependencyGraph::from_schema(schema);
        let mut ledger = IndexMap::new();
        // Deterministic: tables in sorted order, deps within each table sorted.
        let mut edges = graph.forward_edges();
        edges.sort_by_key(|(name, _)| *name);
        for (table, deps) in edges {
            let mut deps: Vec<String> = deps.iter().cloned().collect();
            deps.sort();
            ledger.insert(table.clone(), deps);
        }
        ledger
    }

    /// Per-table source hash: blake3 over the table's normalized rows in
    /// name order (deterministic — same input always yields the same hash).
    /// The incremental build compares these against the previous manifest
    /// to detect which tables changed.
    pub fn table_hashes(document: &Document) -> IndexMap<String, String> {
        let mut hashes = IndexMap::new();
        let mut tables: Vec<_> = document.tables.iter().collect();
        tables.sort_by_key(|(k, _)| *k);
        for (name, table) in tables {
            // Deterministic serialization: rows in source order, fields in
            // insertion order — identical normalized input → identical bytes.
            let json = serde_json::to_vec(table).expect("Table serialization failed");
            hashes.insert(name.clone(), blake3::hash(&json).to_hex().to_string());
        }
        hashes
    }

    /// Hash the build inputs (schema + source document) exactly as
    /// [`Self::generate`] does — the pair an incremental build compares
    /// against the previous manifest to decide whether a rebuild can be
    /// skipped.
    pub fn input_hashes(
        schema: &crate::schema::Schema,
        document: &Document,
        source_extra: Option<&str>,
    ) -> (String, String) {
        let source_hash = Self::hash_source(document);
        (
            Self::hash_schema(schema),
            Self::mix_extra(source_hash, source_extra),
        )
    }

    /// Mix the extra fingerprint bytes into `source_hash` (identity when
    /// `None`), so a script edit rotates the id even when the parsed model
    /// is unchanged.
    fn mix_extra(source_hash: String, extra: Option<&str>) -> String {
        match extra {
            None => source_hash,
            Some(extra) => {
                let mut ctx = blake3::Hasher::new();
                ctx.update(source_hash.as_bytes());
                ctx.update(b"|");
                ctx.update(extra.as_bytes());
                ctx.finalize().to_hex().to_string()
            }
        }
    }

    /// Deterministic build id: fingerprints the build's semantic identity
    /// (profile + all three input/output hashes). Deterministic by contract —
    /// see the field's doc comment.
    fn build_id(profile: &str, schema_hash: &str, source_hash: &str, content_hash: &str) -> String {
        let mut ctx = blake3::Hasher::new();
        ctx.update(profile.as_bytes());
        ctx.update(b"|");
        ctx.update(schema_hash.as_bytes());
        ctx.update(b"|");
        ctx.update(source_hash.as_bytes());
        ctx.update(b"|");
        ctx.update(content_hash.as_bytes());
        ctx.finalize().to_hex()[..24].to_string()
    }

    fn hash_schema(schema: &crate::schema::Schema) -> String {
        // Deterministic serialization of schema (sorted keys)
        let json = serde_json::to_vec(schema).expect("Schema serialization failed");
        blake3::hash(&json).to_hex().to_string()
    }

    fn hash_source(document: &Document) -> String {
        // Hash all source content deterministically
        let mut hasher = blake3::Hasher::new();

        // Sort tables by name for determinism
        let mut tables: Vec<_> = document.tables.iter().collect();
        tables.sort_by_key(|(k, _)| *k);

        for (table_name, table) in tables {
            hasher.update(table_name.as_bytes());
            hasher.update(b"\0");

            // Hash rows in order
            for row in &table.rows {
                // Hash primary key
                for pk in &row.primary_key {
                    hasher.update(&Self::value_to_bytes(pk));
                }
                // Hash all fields in sorted order
                let mut fields: Vec<_> = row.fields.iter().collect();
                fields.sort_by_key(|(k, _)| *k);
                for (field_name, typed_value) in fields {
                    hasher.update(field_name.as_bytes());
                    hasher.update(b"\0");
                    hasher.update(&Self::value_to_bytes(&typed_value.value));
                }
            }
        }

        hasher.finalize().to_hex().to_string()
    }

    fn hash_content(artifacts: &[(String, Vec<u8>, String, Option<String>)]) -> String {
        let mut hasher = blake3::Hasher::new();

        // Sort artifacts by path for determinism
        let mut sorted = artifacts.to_vec();
        sorted.sort_by_key(|(path, _, _, _)| path.clone());

        for (path, content, _, _) in &sorted {
            hasher.update(path.as_bytes());
            hasher.update(b"\0");
            hasher.update(content);
        }

        hasher.finalize().to_hex().to_string()
    }

    fn value_to_bytes(value: &crate::value::Value) -> Vec<u8> {
        // Deterministic binary representation
        match value {
            crate::value::Value::Null => b"null".to_vec(),
            crate::value::Value::Bool(b) => {
                if *b {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                }
            }
            crate::value::Value::Int(i) => i.to_string().into_bytes(),
            crate::value::Value::UInt(u) => u.to_string().into_bytes(),
            crate::value::Value::Float(f) => {
                // Use fixed precision for determinism
                format!("{f:.17e}").into_bytes()
            }
            crate::value::Value::String(s) => s.as_bytes().to_vec(),
            crate::value::Value::Bytes(b) => b.clone(),
            crate::value::Value::Array(arr) => {
                let mut bytes = Vec::new();
                bytes.push(b'[');
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        bytes.push(b',');
                    }
                    bytes.extend_from_slice(&Self::value_to_bytes(v));
                }
                bytes.push(b']');
                bytes
            }
            crate::value::Value::Object(obj) => {
                let mut bytes = Vec::new();
                bytes.push(b'{');
                // Sort keys
                let mut keys: Vec<_> = obj.keys().collect();
                keys.sort();
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        bytes.push(b',');
                    }
                    bytes.extend_from_slice(k.as_bytes());
                    bytes.push(b':');
                    bytes.extend_from_slice(&Self::value_to_bytes(&obj[*k]));
                }
                bytes.push(b'}');
                bytes
            }
        }
    }
}

/// Verify manifest against actual artifacts
pub fn verify_manifest(
    manifest: &BuildManifest,
    artifacts: &[(String, Vec<u8>)],
) -> crate::diagnostics::Diagnostics {
    let mut diags = crate::diagnostics::Diagnostics::new();

    // Check all artifacts in manifest exist
    for (path, expected_info) in &manifest.artifacts {
        let found = artifacts.iter().find(|(p, _)| p == path);

        match found {
            Some((_, content)) => {
                let actual_hash = blake3::hash(content).to_hex().to_string();
                if actual_hash != expected_info.hash {
                    diags.add(
                        crate::diagnostics::Diagnostic::error(
                            crate::error::codes::build::E9003,
                            format!("Artifact hash mismatch: {path}"),
                        )
                        .with_source("manifest")
                        .with_hint(format!(
                            "Expected: {}, Got: {}",
                            expected_info.hash, actual_hash
                        )),
                    );
                }
                if content.len() != expected_info.size {
                    diags.add(
                        crate::diagnostics::Diagnostic::error(
                            crate::error::codes::build::E9003,
                            format!("Artifact size mismatch: {path}"),
                        )
                        .with_source("manifest")
                        .with_hint(format!(
                            "Expected: {}, Got: {}",
                            expected_info.size,
                            content.len()
                        )),
                    );
                }
            }
            None => {
                diags.add(
                    crate::diagnostics::Diagnostic::error(
                        crate::error::codes::build::E9003,
                        format!("Missing artifact: {path}"),
                    )
                    .with_source("manifest"),
                );
            }
        }
    }

    // Check for extra artifacts not in manifest
    let manifest_paths: std::collections::HashSet<_> = manifest.artifacts.keys().collect();
    for (path, _) in artifacts {
        if !manifest_paths.contains(path) {
            diags.add(
                crate::diagnostics::Diagnostic::warning(
                    crate::error::codes::build::E9003,
                    format!("Extra artifact not in manifest: {path}"),
                )
                .with_source("manifest"),
            );
        }
    }

    diags
}

mod config;
#[cfg(test)]
mod tests;

pub use config::{
    BuildProfile, ProjectConfig, ProjectInfo, RegistryConfig, RemoteSourceConfig, SourceConfig,
    TargetConfig,
};
