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
        env_overrides: IndexMap::new(),
    };
    table.fields.insert(
        "id".to_string(),
        FieldSchema {
            name: "id".to_string(),
            field_type: FieldType::UInt32,
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
    let (h1, s1) = ManifestGenerator::input_hashes(&schema, &doc, None);
    let (h2, s2) = ManifestGenerator::input_hashes(&schema, &doc, None);
    // Same input -> same hash (stability)
    assert_eq!(h1, h2);
    assert_eq!(s1, s2);

    // Empty document hashes deterministically too
    let empty = Document::new();
    let (e1, es1) = ManifestGenerator::input_hashes(&schema, &empty, None);
    let (e2, es2) = ManifestGenerator::input_hashes(&schema, &empty, None);
    assert_eq!(e1, e2);
    assert_eq!(es1, es2);
    assert_ne!(s1, es1);

    // Changed field value -> different source hash
    let changed = doc_with_row(vec![("f", Value::Float(2.5))], Value::UInt(9));
    let (_, s_changed) = ManifestGenerator::input_hashes(&schema, &changed, None);
    assert_ne!(s1, s_changed);

    // Changed primary key -> different source hash
    let changed_pk = doc_with_row(vec![("f", Value::Float(1.25))], Value::UInt(10));
    let (_, s_pk) = ManifestGenerator::input_hashes(&schema, &changed_pk, None);
    assert_ne!(s1, s_pk);

    // Float and its string twin hash differently (floats use fixed
    // scientific notation), so cross-type confusion is detectable
    let float_row = doc_with_row(vec![("x", Value::Float(1.25))], Value::UInt(9));
    let string_row = doc_with_row(
        vec![("x", Value::String("1.25".to_string()))],
        Value::UInt(9),
    );
    let (_, s_float) = ManifestGenerator::input_hashes(&schema, &float_row, None);
    let (_, s_string) = ManifestGenerator::input_hashes(&schema, &string_row, None);
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
        environment: None,
        cage_version: "0.1.0".to_string(),
        generator_version: ManifestGenerator::GENERATOR_VERSION.to_string(),
        build_id: "test-build-id".to_string(),
        schema_hash: "abc".to_string(),
        source_hash: "def".to_string(),
        content_hash: "ghi".to_string(),
        dependencies: IndexMap::new(),
        table_hashes: IndexMap::new(),
        targets: Vec::new(),
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
    assert_eq!(back.generator_version, manifest.generator_version);
    assert_eq!(back.build_id, manifest.build_id);
    assert_eq!(back.schema_hash, manifest.schema_hash);
    assert_eq!(back.source_hash, manifest.source_hash);
    assert_eq!(back.content_hash, manifest.content_hash);

    // Old manifests (pre-build_id / pre-generator_version) must still parse:
    // serde(default) keeps them readable with empty fields.
    let mut json_without: serde_json::Value =
        serde_json::from_str(&json).expect("reparse as value");
    if let serde_json::Value::Object(map) = &mut json_without {
        map.remove("build_id");
        map.remove("generator_version");
        map.remove("dependencies");
        map.remove("table_hashes");
    }
    let legacy: BuildManifest =
        serde_json::from_value(json_without).expect("legacy manifest must deserialize");
    assert_eq!(legacy.build_id, "");
    assert_eq!(legacy.generator_version, "");
    assert!(legacy.dependencies.is_empty());
    assert!(legacy.table_hashes.is_empty());
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
        registry: None,
        dependencies: IndexMap::new(),
        remote: IndexMap::new(),
        source: None,
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
        registry: None,
        dependencies: IndexMap::new(),
        remote: IndexMap::new(),
        source: None,
    };
    let json = serde_json::to_string(&cfg).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Every skip_serializing_if path stays silent when the value is absent
    assert!(v.get("source_roots").is_none());
    assert!(v.get("remote").is_none());
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
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
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
    // build_id must be deterministic too: identical inputs → identical id
    assert_eq!(m1.build_id, m2.build_id);
    assert_eq!(m1.generator_version, ManifestGenerator::GENERATOR_VERSION);
    assert_eq!(m1.build_id.len(), 24);
    // New ledger fields are deterministic as well
    assert_eq!(m1.table_hashes, m2.table_hashes);
    assert_eq!(m1.dependencies, m2.dependencies);
    assert!(!m1.table_hashes.is_empty());
    // A single changed row must rotate the per-table hash
    let mut doc2 = make_test_doc();
    if let Some(table) = doc2.tables.values_mut().next() {
        table.rows.push(crate::value::Row {
            primary_key: vec![Value::UInt(2)],
            fields: {
                let mut f = IndexMap::new();
                f.insert(
                    "id".to_string(),
                    crate::value::TypedValue::new(
                        Value::UInt(2),
                        SourceLocation::new("items.json"),
                    ),
                );
                f
            },
            location: SourceLocation::new("items.json"),
            index: 1,
        });
    }
    let m3 = generator.generate(&schema, &doc2, &artifacts);
    assert_ne!(
        m1.table_hashes, m3.table_hashes,
        "row change must rotate the per-table hash"
    );
}

#[test]
fn test_dependency_ledger_records_references() {
    use crate::schema::{FieldSchema, FieldType, ReferenceSchema, TableSchema};

    let mut schema = Schema::new();
    let mut item = TableSchema {
        name: "Item".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    item.fields.insert(
        "id".to_string(),
        FieldSchema {
            name: "id".to_string(),
            field_type: FieldType::UInt32,
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
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        },
    );
    let mut monster = TableSchema {
        name: "Monster".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields: IndexMap::new(),
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: IndexMap::new(),
    };
    monster.fields.insert(
        "drop_item".to_string(),
        FieldSchema {
            name: "drop_item".to_string(),
            field_type: FieldType::UInt32,
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
            reference: Some(ReferenceSchema {
                table: "Item".to_string(),
                field: "id".to_string(),
                predicate: None,
                cardinality: "one".to_string(),
                compatible_with: None,
            }),
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        },
    );
    schema.add_table(item);
    schema.add_table(monster);

    let doc = Document::new();
    let generator = ManifestGenerator::new(
        "test".to_string(),
        "client".to_string(),
        "0.1.0".to_string(),
    );
    let manifest = generator.generate(&schema, &doc, &[]);

    assert_eq!(
        manifest.dependencies.get("Monster").map(Vec::as_slice),
        Some(&["Item".to_string()][..]),
        "Monster references Item"
    );
    assert!(
        manifest.dependencies.get("Item").is_none_or(Vec::is_empty),
        "Item references nothing"
    );
}

#[test]
fn test_build_id_rotates_with_semantic_changes() {
    let schema = make_test_schema();
    let doc = make_test_doc();
    let generator = ManifestGenerator::new(
        "test".to_string(),
        "client".to_string(),
        "0.1.0".to_string(),
    );

    let base = generator.generate(&schema, &doc, &[]);
    let mut other_doc = make_test_doc();
    if let Some(table) = other_doc.tables.values_mut().next() {
        if let Some(row) = table.rows.first_mut() {
            row.fields.clear();
        }
    }
    let other = generator.generate(&schema, &other_doc, &[]);
    assert_ne!(
        base.build_id, other.build_id,
        "data change must rotate build_id"
    );

    let other_profile = ManifestGenerator::new(
        "test".to_string(),
        "server".to_string(),
        "0.1.0".to_string(),
    );
    let other_profile_m = other_profile.generate(&schema, &doc, &[]);
    assert_ne!(
        base.build_id, other_profile_m.build_id,
        "profile change must rotate build_id"
    );
}

#[test]
fn target_records_sorted_and_hash_sensitive() {
    let mk = |format: &str, dir: &str, template: Option<&str>, sort_keys: bool| TargetConfig {
        format: format.to_string(),
        output_dir: dir.to_string(),
        file_template: template.map(str::to_string),
        options: Some(IndexMap::from([(
            "sort_keys".to_string(),
            serde_json::Value::Bool(sort_keys),
        )])),
    };

    // Records sort by (format, output_dir): list order is irrelevant.
    let a = mk("json", "build/a", None, true);
    let b = mk("msgpack", "build/b", Some("{table}.m"), false);
    let forward = ManifestGenerator::target_records(&[a.clone(), b.clone()]);
    let backward = ManifestGenerator::target_records(&[b.clone(), a.clone()]);
    assert_eq!(forward, backward);
    assert_eq!(forward.len(), 2);
    assert_eq!(forward[0].format, "json");
    assert_eq!(forward[1].format, "msgpack");

    // Every bytes-affecting field rotates the hash.
    let option_change = ManifestGenerator::target_records(&[mk("json", "build/a", None, false)]);
    assert_ne!(forward[0].hash, option_change[0].hash);
    let template_change =
        ManifestGenerator::target_records(&[mk("json", "build/a", Some("{t}.json"), true)]);
    assert_ne!(forward[0].hash, template_change[0].hash);
    let dir_change = ManifestGenerator::target_records(&[mk("json", "build/other", None, true)]);
    assert_ne!(forward[0].hash, dir_change[0].hash);
    // Same config → same fingerprint (the determinism contract).
    let same = ManifestGenerator::target_records(&[mk("json", "build/a", None, true)]);
    assert_eq!(forward[0], same[0]);
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
    let (schema_hash, source_hash) = ManifestGenerator::input_hashes(&schema, &doc, None);
    assert_eq!(schema_hash, manifest.schema_hash);
    assert_eq!(source_hash, manifest.source_hash);
}

/// The script-adapter fingerprint mixes into `source_hash` / `build_id`:
/// a different script or config rotates the id even when the parsed
/// document is byte-identical (design §51 Q2).
#[test]
fn source_extra_rotates_build_id() {
    let schema = make_test_schema();
    let doc = make_test_doc();
    let mk = || {
        ManifestGenerator::new(
            "test".to_string(),
            "client".to_string(),
            "0.1.0".to_string(),
        )
    };
    let m_base = mk().generate(&schema, &doc, &[]);
    let m_script = mk()
        .with_source_extra(Some("script-a".to_string()))
        .generate(&schema, &doc, &[]);
    let m_other = mk()
        .with_source_extra(Some("script-b".to_string()))
        .generate(&schema, &doc, &[]);
    assert_ne!(m_base.source_hash, m_script.source_hash, "extra rotates");
    assert_ne!(m_script.source_hash, m_other.source_hash);
    assert_ne!(m_script.build_id, m_other.build_id);
    // input_hashes agrees with generate under the same extra.
    let (_, s) = ManifestGenerator::input_hashes(&schema, &doc, Some("script-a"));
    assert_eq!(s, m_script.source_hash);
}

#[test]
fn test_verify_manifest() {
    let manifest = BuildManifest {
        project: "test".to_string(),
        profile: "client".to_string(),
        environment: None,
        cage_version: "0.1.0".to_string(),
        generator_version: ManifestGenerator::GENERATOR_VERSION.to_string(),
        build_id: "test-build-id".to_string(),
        schema_hash: "abc".to_string(),
        source_hash: "def".to_string(),
        content_hash: "ghi".to_string(),
        dependencies: IndexMap::new(),
        table_hashes: IndexMap::new(),
        targets: Vec::new(),
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
