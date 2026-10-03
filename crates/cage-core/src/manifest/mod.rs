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
    /// Cage version that produced the build
    pub cage_version: String,
    /// Blake3 hash of the schema document
    pub schema_hash: String,
    /// Blake3 hash of all source content
    pub source_hash: String,
    /// Blake3 hash of all artifact content
    pub content_hash: String,
    /// Artifacts by output path
    pub artifacts: IndexMap<String, ArtifactInfo>,
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
}

impl ManifestGenerator {
    /// Create a generator for a project / profile / tool version
    pub fn new(project_name: String, profile_name: String, cage_version: String) -> Self {
        Self {
            project_name,
            profile_name,
            cage_version,
        }
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
        let content_hash = Self::hash_content(artifacts);

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
            cage_version: self.cage_version.clone(),
            schema_hash,
            source_hash,
            content_hash,
            artifacts: artifact_infos,
        }
    }

    /// Hash the build inputs (schema + source document) exactly as
    /// [`Self::generate`] does — the pair an incremental build compares
    /// against the previous manifest to decide whether a rebuild can be
    /// skipped.
    pub fn input_hashes(schema: &crate::schema::Schema, document: &Document) -> (String, String) {
        (Self::hash_schema(schema), Self::hash_source(document))
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

/// Profile configuration for build
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildProfile {
    /// Profile name (e.g., client / server)
    pub name: String,
    /// Target generators to run
    pub targets: Vec<TargetConfig>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Template variables available to targets
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub variables: IndexMap<String, String>,
}

/// Target configuration within a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetConfig {
    /// Output format (json, csv, csharp, etc.)
    pub format: String,
    /// Output directory
    pub output_dir: String,
    /// File name template, e.g., "{table}.json"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_template: Option<String>,
    /// Format-specific options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<IndexMap<String, serde_json::Value>>,
}

/// Project configuration (cage.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project identity
    pub project: ProjectInfo,
    /// Build profiles by name
    pub profiles: IndexMap<String, BuildProfile>,
    /// Logical source name -> path mapping
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub source_roots: IndexMap<String, String>,
    /// Directory containing schema files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_path: Option<String>,
    /// Whether warnings are escalated to errors
    #[serde(default, skip_serializing_if = "is_false")]
    pub warnings_as_errors: bool,
    /// Default output directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
}

/// Project identity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Project name
    pub name: String,
    /// Project version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for ProjectConfig {
    fn default() -> Self {
        let mut profiles = IndexMap::new();
        profiles.insert(
            "client".to_string(),
            BuildProfile {
                name: "client".to_string(),
                targets: vec![TargetConfig {
                    format: "json".to_string(),
                    output_dir: "build/client".to_string(),
                    file_template: Some("{table}.json".to_string()),
                    options: None,
                }],
                description: Some("Client build profile".to_string()),
                variables: IndexMap::new(),
            },
        );
        profiles.insert(
            "server".to_string(),
            BuildProfile {
                name: "server".to_string(),
                targets: vec![TargetConfig {
                    format: "json".to_string(),
                    output_dir: "build/server".to_string(),
                    file_template: Some("{table}.json".to_string()),
                    options: None,
                }],
                description: Some("Server build profile".to_string()),
                variables: IndexMap::new(),
            },
        );

        Self {
            project: ProjectInfo {
                name: "game".to_string(),
                version: Some("0.1.0".to_string()),
                description: None,
            },
            profiles,
            source_roots: IndexMap::new(),
            schema_path: Some("schemas".to_string()),
            warnings_as_errors: false,
            output_dir: Some("build".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldSchema, FieldType, Schema, TableSchema};
    use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
    use indexmap::IndexMap;

    fn make_test_schema() -> Schema {
        let mut schema = Schema::new();
        let mut table = TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
        };
        table.fields.insert(
            "id".to_string(),
            FieldSchema {
                name: "id".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
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
                properties: None,
                additional_properties: None,
                reference: None,
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        schema.add_table(table);
        schema
    }

    fn make_test_doc() -> Document {
        let mut doc = Document::new();
        let mut table = Table {
            name: "Item".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows: vec![],
            source_file: "items.json".to_string(),
            sheet: None,
        };
        table.rows.push(Row {
            primary_key: vec![Value::UInt(1)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    TypedValue::new(Value::UInt(1), SourceLocation::new("items.json")),
                );
                f
            },
            location: SourceLocation::new("items.json"),
            index: 0,
        });
        doc.add_table(table);
        doc
    }

    /// Build a one-row document with the given fields, so `hash_source`
    /// exercises `value_to_bytes` on every field value.
    fn doc_with_row(fields: Vec<(&str, Value)>, pk: Value) -> Document {
        let mut doc = Document::new();
        let mut table = Table {
            name: "KitchenSink".to_string(),
            primary_key_fields: vec!["pk".to_string()],
            rows: vec![],
            source_file: "all_values.json".to_string(),
            sheet: None,
        };
        let mut f = IndexMap::new();
        for (k, v) in fields {
            f.insert(
                k.to_string(),
                TypedValue::new(v, SourceLocation::new("all_values.json")),
            );
        }
        table.rows.push(Row {
            primary_key: vec![pk],
            fields: f,
            location: SourceLocation::new("all_values.json"),
            index: 0,
        });
        doc.add_table(table);
        doc
    }

    #[test]
    fn test_hash_source_covers_all_value_variants_and_is_stable() {
        let mut nested_obj = IndexMap::new();
        nested_obj.insert("b".to_string(), Value::Bool(false));
        nested_obj.insert("a".to_string(), Value::String("x".to_string()));
        let mut obj = IndexMap::new();
        obj.insert("o".to_string(), Value::Object(nested_obj));
        obj.insert("n".to_string(), Value::Null);

        let doc = doc_with_row(
            vec![
                ("nul", Value::Null),
                ("bt", Value::Bool(true)),
                ("bf", Value::Bool(false)),
                ("i", Value::Int(-42)),
                ("u", Value::UInt(42)),
                ("f", Value::Float(1.25)),
                ("s", Value::String("text".to_string())),
                ("b", Value::Bytes(vec![1, 2, 3])),
                (
                    "arr",
                    Value::Array(vec![Value::UInt(1), Value::Float(-0.5), Value::Bool(true)]),
                ),
                ("obj", Value::Object(obj)),
            ],
            Value::UInt(9),
        );

        let schema = Schema::new();
        let (h1, s1) = ManifestGenerator::input_hashes(&schema, &doc);
        let (h2, s2) = ManifestGenerator::input_hashes(&schema, &doc);
        // Same input -> same hash (stability)
        assert_eq!(h1, h2);
        assert_eq!(s1, s2);

        // Empty document hashes deterministically too
        let empty = Document::new();
        let (e1, es1) = ManifestGenerator::input_hashes(&schema, &empty);
        let (e2, es2) = ManifestGenerator::input_hashes(&schema, &empty);
        assert_eq!(e1, e2);
        assert_eq!(es1, es2);
        assert_ne!(s1, es1);

        // Changed field value -> different source hash
        let changed = doc_with_row(vec![("f", Value::Float(2.5))], Value::UInt(9));
        let (_, s_changed) = ManifestGenerator::input_hashes(&schema, &changed);
        assert_ne!(s1, s_changed);

        // Changed primary key -> different source hash
        let changed_pk = doc_with_row(vec![("f", Value::Float(1.25))], Value::UInt(10));
        let (_, s_pk) = ManifestGenerator::input_hashes(&schema, &changed_pk);
        assert_ne!(s1, s_pk);

        // Float and its string twin hash differently (floats use fixed
        // scientific notation), so cross-type confusion is detectable
        let float_row = doc_with_row(vec![("x", Value::Float(1.25))], Value::UInt(9));
        let string_row = doc_with_row(
            vec![("x", Value::String("1.25".to_string()))],
            Value::UInt(9),
        );
        let (_, s_float) = ManifestGenerator::input_hashes(&schema, &float_row);
        let (_, s_string) = ManifestGenerator::input_hashes(&schema, &string_row);
        assert_ne!(s_float, s_string);
    }

    #[test]
    fn test_generate_records_artifact_metadata_and_is_order_insensitive() {
        let schema = make_test_schema();
        let doc = make_test_doc();
        let generator = ManifestGenerator::new(
            "proj".to_string(),
            "server".to_string(),
            "9.9.9".to_string(),
        );

        let m1 = generator.generate(
            &schema,
            &doc,
            &[
                (
                    "b.json".to_string(),
                    b"B".to_vec(),
                    "json".to_string(),
                    None,
                ),
                (
                    "a.json".to_string(),
                    b"A".to_vec(),
                    "csv".to_string(),
                    Some("Item".to_string()),
                ),
            ],
        );
        let m2 = generator.generate(
            &schema,
            &doc,
            &[
                (
                    "a.json".to_string(),
                    b"A".to_vec(),
                    "csv".to_string(),
                    Some("Item".to_string()),
                ),
                (
                    "b.json".to_string(),
                    b"B".to_vec(),
                    "json".to_string(),
                    None,
                ),
            ],
        );
        // Artifact input order must not change the content hash
        assert_eq!(m1.content_hash, m2.content_hash);

        assert_eq!(m1.cage_version, "9.9.9");
        assert_eq!(m1.artifacts.len(), 2);
        let a = &m1.artifacts["a.json"];
        assert_eq!(a.path, "a.json");
        assert_eq!(a.hash, blake3::hash(b"A").to_hex().to_string());
        assert_eq!(a.size, 1);
        assert_eq!(a.format, "csv");
        assert_eq!(a.table.as_deref(), Some("Item"));
        assert_eq!(a.encoding, "utf-8");
        let b = &m1.artifacts["b.json"];
        assert_eq!(b.size, 1);
        assert!(b.table.is_none());

        // Empty artifact list is a valid build
        let empty = generator.generate(&schema, &doc, &[]);
        assert!(empty.artifacts.is_empty());
        assert_ne!(empty.content_hash, "");
    }

    fn manifest_with_out_json(content: &[u8], size: usize) -> BuildManifest {
        BuildManifest {
            project: "test".to_string(),
            profile: "client".to_string(),
            cage_version: "0.1.0".to_string(),
            schema_hash: "abc".to_string(),
            source_hash: "def".to_string(),
            content_hash: "ghi".to_string(),
            artifacts: {
                let mut m = IndexMap::new();
                m.insert(
                    "out.json".to_string(),
                    ArtifactInfo {
                        path: "out.json".to_string(),
                        hash: blake3::hash(content).to_hex().to_string(),
                        size,
                        format: "json".to_string(),
                        table: None,
                        encoding: "utf-8".to_string(),
                    },
                );
                m
            },
        }
    }

    #[test]
    fn test_verify_manifest_hash_and_size_mismatch_both_reported() {
        // Content differs from the hashed bytes AND has a different length,
        // so both the hash-mismatch and size-mismatch diagnostics fire.
        let manifest = manifest_with_out_json(b"{}", 2);
        let diags = verify_manifest(&manifest, &[("out.json".to_string(), b"hello".to_vec())]);
        let errors = diags.errors();
        assert_eq!(errors.len(), 2);
        assert!(errors
            .iter()
            .any(|d| d.message.contains("Artifact hash mismatch: out.json")));
        assert!(errors
            .iter()
            .any(|d| d.message.contains("Artifact size mismatch: out.json")));
        // Same content, same length -> clean
        let diags = verify_manifest(&manifest, &[("out.json".to_string(), b"{}".to_vec())]);
        assert!(!diags.has_errors());
        assert!(!diags.has_warnings());
    }

    #[test]
    fn test_verify_manifest_missing_artifact_is_error() {
        let manifest = manifest_with_out_json(b"{}", 2);
        let diags = verify_manifest(&manifest, &[]);
        let errors = diags.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Missing artifact: out.json");
        assert_eq!(errors[0].source, "manifest");
    }

    #[test]
    fn test_verify_manifest_extra_artifact_is_warning() {
        let manifest = manifest_with_out_json(b"{}", 2);
        let diags = verify_manifest(
            &manifest,
            &[
                ("out.json".to_string(), b"{}".to_vec()),
                ("extra.json".to_string(), b"{}".to_vec()),
            ],
        );
        assert!(!diags.has_errors());
        let warnings = diags.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].message,
            "Extra artifact not in manifest: extra.json"
        );
    }

    #[test]
    fn test_build_manifest_serialization_roundtrip() {
        // Second artifact without a table association
        let mut manifest = manifest_with_out_json(b"{}", 2);
        manifest.artifacts.insert(
            "b.csv".to_string(),
            ArtifactInfo {
                path: "b.csv".to_string(),
                hash: "deadbeef".to_string(),
                size: 7,
                format: "csv".to_string(),
                table: None,
                encoding: "utf-8".to_string(),
            },
        );

        let json = serde_json::to_string(&manifest).expect("serialize");
        let back: BuildManifest = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.project, manifest.project);
        assert_eq!(back.profile, manifest.profile);
        assert_eq!(back.cage_version, manifest.cage_version);
        assert_eq!(back.schema_hash, manifest.schema_hash);
        assert_eq!(back.source_hash, manifest.source_hash);
        assert_eq!(back.content_hash, manifest.content_hash);
        // IndexMap preserves artifact order across the roundtrip
        assert_eq!(
            back.artifacts.keys().collect::<Vec<_>>(),
            manifest.artifacts.keys().collect::<Vec<_>>()
        );
        let a = &back.artifacts["out.json"];
        assert_eq!(a.hash, blake3::hash(b"{}").to_hex().to_string());
        assert_eq!(a.size, 2);
        assert_eq!(a.format, "json");
        assert_eq!(a.table, None);
        assert_eq!(a.encoding, "utf-8");
        let b = &back.artifacts["b.csv"];
        assert_eq!(b.hash, "deadbeef");
        assert_eq!(b.table, None);
        // Serialization is deterministic
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn test_project_config_roundtrip_full() {
        let mut variables = IndexMap::new();
        variables.insert("lang".to_string(), "zh".to_string());
        let mut options = IndexMap::new();
        options.insert("delimiter".to_string(), serde_json::json!(";"));

        let mut profiles = IndexMap::new();
        profiles.insert(
            "client".to_string(),
            BuildProfile {
                name: "client".to_string(),
                targets: vec![TargetConfig {
                    format: "csv".to_string(),
                    output_dir: "build/csv".to_string(),
                    file_template: Some("{table}.csv".to_string()),
                    options: Some(options),
                }],
                description: Some("CSV export".to_string()),
                variables,
            },
        );

        let cfg = ProjectConfig {
            project: ProjectInfo {
                name: "demo".to_string(),
                version: Some("1.2.3".to_string()),
                description: Some("a demo".to_string()),
            },
            profiles,
            source_roots: {
                let mut m = IndexMap::new();
                m.insert("main".to_string(), "data/main".to_string());
                m
            },
            schema_path: Some("schema".to_string()),
            warnings_as_errors: true,
            output_dir: Some("build".to_string()),
        };

        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(json.contains("\"warnings_as_errors\":true"));
        let back: ProjectConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.project.name, "demo");
        assert_eq!(back.project.version.as_deref(), Some("1.2.3"));
        assert_eq!(back.project.description.as_deref(), Some("a demo"));
        assert_eq!(
            back.source_roots.get("main").map(String::as_str),
            Some("data/main")
        );
        assert_eq!(back.schema_path.as_deref(), Some("schema"));
        assert!(back.warnings_as_errors);
        assert_eq!(back.output_dir.as_deref(), Some("build"));

        let profile = &back.profiles["client"];
        assert_eq!(profile.description.as_deref(), Some("CSV export"));
        assert_eq!(
            profile.variables.get("lang").map(String::as_str),
            Some("zh")
        );
        let target = &profile.targets[0];
        assert_eq!(target.format, "csv");
        assert_eq!(target.output_dir, "build/csv");
        assert_eq!(target.file_template.as_deref(), Some("{table}.csv"));
        let opts = target.options.as_ref().expect("options preserved");
        assert_eq!(opts.get("delimiter"), Some(&serde_json::json!(";")));
    }

    #[test]
    fn test_project_config_minimal_json_omits_optionals() {
        let cfg = ProjectConfig {
            project: ProjectInfo {
                name: "bare".to_string(),
                version: None,
                description: None,
            },
            profiles: IndexMap::new(),
            source_roots: IndexMap::new(),
            schema_path: None,
            warnings_as_errors: false,
            output_dir: None,
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Every skip_serializing_if path stays silent when the value is absent
        assert!(v.get("source_roots").is_none());
        assert!(v.get("schema_path").is_none());
        assert!(v.get("warnings_as_errors").is_none());
        assert!(v.get("output_dir").is_none());
        assert!(v["project"].get("version").is_none());
        assert!(v["project"].get("description").is_none());

        // Profile / target optionals are omitted the same way
        let profile = BuildProfile {
            name: "p".to_string(),
            targets: vec![TargetConfig {
                format: "json".to_string(),
                output_dir: "o".to_string(),
                file_template: None,
                options: None,
            }],
            description: None,
            variables: IndexMap::new(),
        };
        let pv: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&profile).unwrap()).unwrap();
        assert!(pv.get("description").is_none());
        assert!(pv.get("variables").is_none());
        assert!(pv["targets"][0].get("file_template").is_none());
        assert!(pv["targets"][0].get("options").is_none());
        let back: BuildProfile = serde_json::from_value(pv).expect("deserialize");
        assert!(back.description.is_none());
        assert!(back.variables.is_empty());
        assert!(back.targets[0].file_template.is_none());
        assert!(back.targets[0].options.is_none());

        // Parsing minimal TOML-ish JSON applies serde defaults
        let back: ProjectConfig = serde_json::from_str(
            r#"{"project":{"name":"bare"},"profiles":{"p":{"name":"p","targets":[{"format":"json","output_dir":"o"}]}}}"#,
        )
        .expect("deserialize");
        assert_eq!(back.project.name, "bare");
        assert!(back.source_roots.is_empty());
        assert!(!back.warnings_as_errors);
        assert!(back.profiles["p"].variables.is_empty());
        assert!(back.profiles["p"].targets[0].options.is_none());
    }

    #[test]
    fn test_project_config_default_shape_and_roundtrip() {
        let cfg = ProjectConfig::default();
        assert_eq!(cfg.project.name, "game");
        assert_eq!(cfg.project.version.as_deref(), Some("0.1.0"));
        assert!(cfg.project.description.is_none());
        assert_eq!(cfg.profiles.len(), 2);

        let client = &cfg.profiles["client"];
        assert_eq!(client.name, "client");
        assert_eq!(client.description.as_deref(), Some("Client build profile"));
        assert!(client.variables.is_empty());
        assert_eq!(client.targets.len(), 1);
        assert_eq!(client.targets[0].format, "json");
        assert_eq!(client.targets[0].output_dir, "build/client");
        assert_eq!(
            client.targets[0].file_template.as_deref(),
            Some("{table}.json")
        );
        assert!(client.targets[0].options.is_none());

        let server = &cfg.profiles["server"];
        assert_eq!(server.name, "server");
        assert_eq!(server.targets[0].output_dir, "build/server");

        assert_eq!(cfg.schema_path.as_deref(), Some("schemas"));
        assert_eq!(cfg.output_dir.as_deref(), Some("build"));
        assert!(!cfg.warnings_as_errors);
        assert!(cfg.source_roots.is_empty());

        // Default serializes without warnings_as_errors (is_false) and
        // survives a full roundtrip
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(v.get("warnings_as_errors").is_none());
        let back: ProjectConfig = serde_json::from_value(v).expect("deserialize");
        assert!(!back.warnings_as_errors);
        assert_eq!(back.profiles.len(), 2);
        assert_eq!(back.output_dir.as_deref(), Some("build"));
    }

    #[test]
    fn test_manifest_generation() {
        let schema = make_test_schema();
        let doc = make_test_doc();
        let generator = ManifestGenerator::new(
            "test-game".to_string(),
            "client".to_string(),
            "0.1.0".to_string(),
        );

        let artifacts = vec![(
            "build/client/item.json".to_string(),
            br#"{"id":1}"#.to_vec(),
            "json".to_string(),
            Some("Item".to_string()),
        )];

        let manifest = generator.generate(&schema, &doc, &artifacts);

        assert_eq!(manifest.project, "test-game");
        assert_eq!(manifest.profile, "client");
        assert_ne!(manifest.schema_hash, "");
        assert_ne!(manifest.source_hash, "");
        assert_ne!(manifest.content_hash, "");
        assert_eq!(manifest.artifacts.len(), 1);
    }

    #[test]
    fn test_deterministic_hashes() {
        let schema = make_test_schema();
        let doc = make_test_doc();
        let generator = ManifestGenerator::new(
            "test".to_string(),
            "client".to_string(),
            "0.1.0".to_string(),
        );

        let artifacts = vec![
            (
                "a.json".to_string(),
                b"{}".to_vec(),
                "json".to_string(),
                None,
            ),
            (
                "b.json".to_string(),
                b"{}".to_vec(),
                "json".to_string(),
                None,
            ),
        ];

        let m1 = generator.generate(&schema, &doc, &artifacts);
        let m2 = generator.generate(&schema, &doc, &artifacts);

        assert_eq!(m1.schema_hash, m2.schema_hash);
        assert_eq!(m1.source_hash, m2.source_hash);
        assert_eq!(m1.content_hash, m2.content_hash);
    }

    #[test]
    fn test_input_hashes_match_generate() {
        let schema = make_test_schema();
        let doc = make_test_doc();
        let generator = ManifestGenerator::new(
            "test".to_string(),
            "client".to_string(),
            "0.1.0".to_string(),
        );
        let manifest = generator.generate(&schema, &doc, &[]);
        let (schema_hash, source_hash) = ManifestGenerator::input_hashes(&schema, &doc);
        assert_eq!(schema_hash, manifest.schema_hash);
        assert_eq!(source_hash, manifest.source_hash);
    }

    #[test]
    fn test_verify_manifest() {
        let manifest = BuildManifest {
            project: "test".to_string(),
            profile: "client".to_string(),
            cage_version: "0.1.0".to_string(),
            schema_hash: "abc".to_string(),
            source_hash: "def".to_string(),
            content_hash: "ghi".to_string(),
            artifacts: {
                let mut m = IndexMap::new();
                m.insert(
                    "out.json".to_string(),
                    ArtifactInfo {
                        path: "out.json".to_string(),
                        hash: blake3::hash(b"{}").to_hex().to_string(),
                        size: 2,
                        format: "json".to_string(),
                        table: None,
                        encoding: "utf-8".to_string(),
                    },
                );
                m
            },
        };

        // Matching artifact
        let diags = verify_manifest(&manifest, &[("out.json".to_string(), b"{}".to_vec())]);
        assert!(!diags.has_errors());

        // Mismatched content
        let diags = verify_manifest(&manifest, &[("out.json".to_string(), b"[]".to_vec())]);
        assert!(diags.has_errors());
    }
}
