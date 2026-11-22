//! Unified L7 executor tests (review P2-3): in-process Rust validators and
//! independent-process worker plugins merge onto one diagnostics surface.

use super::*;
use crate::plugin::AttestationOutcome;
use crate::schema::{FieldSchema, FieldType, TableSchema};
use crate::value::{Row, SourceLocation, Table, TypedValue};
use indexmap::IndexMap;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

fn sdk_path() -> PathBuf {
    repo_root().join("sdk").join("python")
}

/// Embed the absolute SDK path into a fixture script (the host exports no SDK
/// path env var; the fixture imports `cage_sdk` by absolute path).
fn sdk_script(source: &str) -> String {
    source.replace("__SDK__", &sdk_path().to_string_lossy())
}

fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn monster_doc(level: i64, attack: i64) -> Document {
    let mut fields: IndexMap<String, TypedValue> = IndexMap::new();
    fields.insert(
        "id".to_string(),
        TypedValue::new(Value::Int(1), SourceLocation::default()),
    );
    fields.insert(
        "level".to_string(),
        TypedValue::new(Value::Int(level), SourceLocation::default()),
    );
    fields.insert(
        "attack".to_string(),
        TypedValue::new(Value::Int(attack), SourceLocation::default()),
    );
    let row = Row {
        primary_key: vec![Value::Int(1)],
        fields,
        location: SourceLocation::default(),
        index: 0,
    };
    let mut doc = Document::new();
    doc.add_table(Table {
        name: "Monster".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: vec![row],
        source_file: "config/monster.json".to_string(),
        sheet: None,
    });
    doc
}

/// A schema whose Monster table declares the `level`/`attack` fields the
/// `power_curve` package needs (review P3-2 capability declaration).
fn power_curve_schema() -> Schema {
    let mut fields = IndexMap::new();
    for name in ["id", "level", "attack"] {
        fields.insert(
            name.to_string(),
            FieldSchema {
                name: name.to_string(),
                field_type: FieldType::Int32,
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
                metadata: indexmap::IndexMap::new(),
            },
        );
    }
    let mut schema = Schema::new();
    schema.add_table(TableSchema {
        name: "Monster".to_string(),
        description: None,
        primary_key: vec!["id".to_string()],
        fields,
        unique_constraints: vec![],
        order_by: None,
        targets: vec![],
        env_overrides: indexmap::IndexMap::new(),
    });
    schema
}

fn write_plugin(dir: &Path, manifest: &str, script: &str) {
    std::fs::create_dir_all(dir).expect("plugin dir");
    std::fs::write(dir.join("plugin.toml"), manifest).expect("write manifest");
    std::fs::write(dir.join("plugin.py"), script).expect("write entry");
}

const LEVEL_CAP_PLUGIN: &str = r#"
import sys
sys.path.insert(0, "__SDK__")
from cage_sdk import Plugin, run

class LevelCap(Plugin):
    id = "level_cap"
    version = "1.0.0"
    def validate(self, ctx):
        cap = ctx.options().get("cap", 10)
        table = ctx.table("Monster")
        for row in table["rows"]:
            level = row["fields"].get("level")
            if isinstance(level, (int, float)) and level > cap:
                ctx.error("level too high", table="Monster", row=row["index"], field="level")

run(LevelCap())
"#;

#[test]
#[cfg(unix)]
fn worker_and_builtin_share_one_diagnostics_surface() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"level_cap\"\nversion = \"1.0.0\"\nentry = \"plugin.py\"\n",
        &sdk_script(LEVEL_CAP_PLUGIN),
    );
    let manifest = crate::plugin::PluginManifest::load(tmp.path()).expect("manifest");
    let mut registry = GameRuleRegistry::with_packages(&[RulePackage::SamplePowerCurve]);
    registry.register_worker(tmp.path(), manifest, serde_json::json!({"cap": 10}));

    // level 12 > worker cap 10 → worker finding; attack 1300 > level*100+50
    // (1250) → builtin finding. Both land on the same surface.
    let diags = registry.run(&power_curve_schema(), &monster_doc(12, 1300));
    let worker = diags
        .iter()
        .find(|d| d.message == "level too high")
        .expect("worker finding on the shared surface");
    let builtin = diags
        .iter()
        .find(|d| d.message.contains("power curve"))
        .expect("builtin finding on the shared surface");
    assert_eq!(worker.code, "E1601");
    assert_eq!(builtin.code, "E1601");
    assert_eq!(worker.field.as_deref(), Some("level"));
}

#[test]
fn an_unloadable_plugin_fails_validation_not_skipped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // No plugin.toml in the directory: load fails, must surface as E1602.
    let mut registry = GameRuleRegistry::new();
    registry.register_worker_dir(tmp.path(), tmp.path(), serde_json::json!({}));
    let diags = registry.run(&Schema::new(), &monster_doc(1, 1));
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, gamerule::E1602);
    assert!(diags[0].message.contains("plugin.toml not found"));
}

#[test]
fn discovery_registers_plugin_directories_in_id_order() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    // `zz` is registered but broken: discovery must not drop it.
    write_plugin(
        &plugins.join("aa"),
        "id = \"aa\"\nversion = \"1\"\nentry = \"plugin.py\"\n",
        "print('placeholder')\n",
    );
    write_plugin(
        &plugins.join("zz"),
        "id = \"zz\"\nversion = \"1\"\nentry = \"plugin.py\"\npython = \"not a version\"\n",
        "print('placeholder')\n",
    );

    let mut registry = GameRuleRegistry::new();
    registry.discover_workers(tmp.path());
    assert_eq!(registry.len(), 2);

    // The broken `zz` manifest surfaces as an E1602, not a silent drop.
    let diags = registry.run(&Schema::new(), &monster_doc(1, 1));
    assert!(
        diags.iter().any(|d| d.code == gamerule::E1602),
        "broken manifest must fail validation: {diags:?}"
    );
}

#[test]
fn a_missing_plugins_directory_is_no_plugins() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut registry = GameRuleRegistry::new();
    registry.discover_workers(tmp.path());
    assert_eq!(registry.len(), 0);
    assert!(registry.run(&Schema::new(), &monster_doc(1, 1)).is_empty());
}

// ---- P2-4: the auditable validation record ----

#[test]
#[cfg(unix)]
fn attested_run_records_plugin_identity_and_environment() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"level_cap\"\nversion = \"1.2.3\"\nentry = \"plugin.py\"\n",
        &sdk_script(LEVEL_CAP_PLUGIN),
    );
    let manifest = crate::plugin::PluginManifest::load(tmp.path()).expect("manifest");
    let mut registry = GameRuleRegistry::with_packages(&[RulePackage::SamplePowerCurve]);
    registry.register_worker(tmp.path(), manifest, serde_json::json!({"cap": 10}));

    let (diags, attestation) = registry.run_attested(&power_curve_schema(), &monster_doc(12, 1300));
    assert!(!diags.is_empty());

    assert_eq!(attestation.plugins.len(), 1);
    let plugin = &attestation.plugins[0];
    assert_eq!(plugin.id, "level_cap");
    assert_eq!(plugin.version, "1.2.3");
    // Only the worker plugin is recorded — built-ins are not plugin outputs.
    assert!(matches!(
        plugin.outcome,
        AttestationOutcome::Ran { errors: 1, .. }
    ));
    // The environment fingerprint names the interpreter that actually ran.
    assert!(attestation.environment.python.is_some());
    assert_eq!(
        attestation.environment.protocol_version,
        cage_plugin_protocol::PROTOCOL_VERSION
    );
    assert!(attestation.passed);

    // The same inputs reproduce the same digest; a changed option changes it.
    let (_, again) = registry.run_attested(&power_curve_schema(), &monster_doc(12, 1300));
    assert_eq!(attestation.digest(), again.digest());

    let manifest2 = crate::plugin::PluginManifest::load(tmp.path()).expect("manifest");
    let mut other = GameRuleRegistry::with_packages(&[RulePackage::SamplePowerCurve]);
    other.register_worker(tmp.path(), manifest2, serde_json::json!({"cap": 1000}));
    let (_, changed) = other.run_attested(&power_curve_schema(), &monster_doc(12, 1300));
    assert_ne!(attestation.digest(), changed.digest());
    assert_ne!(
        attestation.plugins[0].options_digest,
        changed.plugins[0].options_digest
    );
}

#[test]
#[cfg(unix)]
fn attested_failure_still_records_the_attempt() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"crasher\"\nversion = \"1\"\nentry = \"plugin.py\"\n",
        "import sys\nsys.exit(3)\n",
    );
    let manifest = crate::plugin::PluginManifest::load(tmp.path()).expect("manifest");
    let mut registry = GameRuleRegistry::new();
    registry.register_worker(tmp.path(), manifest, serde_json::json!({}));

    let (diags, attestation) = registry.run_attested(&Schema::new(), &monster_doc(1, 1));
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, gamerule::E1603);
    assert!(!attestation.passed);
    match &attestation.plugins[0].outcome {
        AttestationOutcome::Failed { class, .. } => assert_eq!(class, "crash"),
        AttestationOutcome::Ran { .. } => panic!("expected a failed record"),
    }
}

#[test]
fn an_unloadable_plugin_is_attested_as_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plugin_dir = tmp.path().join("plugins").join("ghost");
    std::fs::create_dir_all(&plugin_dir).expect("mkdir");
    std::fs::write(plugin_dir.join("plugin.py"), "print('x')\n").expect("write");

    let mut registry = GameRuleRegistry::new();
    registry.register_worker_dir(tmp.path(), &plugin_dir, serde_json::json!({}));

    let (diags, attestation) = registry.run_attested(&Schema::new(), &monster_doc(1, 1));
    assert_eq!(diags[0].code, gamerule::E1602);
    assert_eq!(attestation.plugins.len(), 1);
    assert_eq!(attestation.plugins[0].id, "ghost");
    assert!(matches!(
        attestation.plugins[0].outcome,
        AttestationOutcome::Failed { .. }
    ));
    assert!(!attestation.passed);
}

#[test]
fn a_run_without_workers_has_no_python_in_the_fingerprint() {
    let registry = GameRuleRegistry::with_packages(&[RulePackage::SamplePowerCurve]);
    let (_, attestation) = registry.run_attested(&power_curve_schema(), &monster_doc(1, 1));
    assert_eq!(attestation.plugins.len(), 0);
    assert!(attestation.environment.python.is_none());
    assert!(attestation.passed);
    // The data actually validated is fingerprinted.
    assert_eq!(attestation.document_digest.len(), 64);
    assert_eq!(attestation.schema_digest.len(), 64);
}

// ---- P2-5: the project policy's required-plugin enforcement ----

#[test]
fn require_plugins_fails_when_the_required_plugin_is_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut registry = GameRuleRegistry::new();
    // No plugins/ directory: discovery finds nothing, the requirement stands.
    registry.discover_workers(tmp.path());
    registry.require_plugins(tmp.path(), &["drop_rewards".to_string()]);
    assert_eq!(registry.len(), 1);

    let (diags, attestation) = registry.run_attested(&Schema::new(), &monster_doc(1, 1));
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, gamerule::E1602);
    assert!(diags[0].message.contains("drop_rewards"), "{diags:?}");
    assert!(diags[0].message.contains("required"), "{diags:?}");
    // The missing plugin is still attested — the record names what was required.
    assert_eq!(attestation.plugins.len(), 1);
    assert_eq!(attestation.plugins[0].id, "drop_rewards");
    match &attestation.plugins[0].outcome {
        AttestationOutcome::Failed { class, .. } => assert_eq!(class, "missing"),
        AttestationOutcome::Ran { .. } => panic!("an absent plugin cannot run"),
    }
    assert!(!attestation.passed);
}

#[test]
fn require_plugins_does_not_flag_a_present_plugin_as_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        &tmp.path().join("plugins").join("level_cap"),
        "id = \"level_cap\"\nversion = \"1.0.0\"\nentry = \"plugin.py\"\n",
        "print('placeholder')\n",
    );
    let mut registry = GameRuleRegistry::new();
    registry.discover_workers(tmp.path());
    assert_eq!(registry.worker_ids(), vec!["level_cap".to_string()]);
    registry.require_plugins(tmp.path(), &["level_cap".to_string()]);
    // A satisfied requirement adds no entry.
    assert_eq!(registry.len(), 1);

    // The placeholder script is not valid NDJSON (invalid_output) — the point
    // here is only that the id is recognised as present, never as missing.
    let diags = registry.run(&Schema::new(), &monster_doc(1, 1));
    assert!(
        diags.iter().all(|d| d.code != gamerule::E1602),
        "a present required plugin must not read as missing: {diags:?}"
    );
}

#[test]
fn require_plugins_does_not_duplicate_a_broken_manifest() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        &tmp.path().join("plugins").join("broken"),
        "id = \"broken\"\nversion = \"1\"\nentry = \"plugin.py\"\npython = \"not a version\"\n",
        "print('placeholder')\n",
    );
    let mut registry = GameRuleRegistry::new();
    registry.discover_workers(tmp.path());
    registry.require_plugins(tmp.path(), &["broken".to_string()]);
    // Discovery already registered it (as a load failure): no second entry.
    assert_eq!(registry.len(), 1);
    let diags = registry.run(&Schema::new(), &monster_doc(1, 1));
    let missing = diags.iter().filter(|d| d.code == gamerule::E1602).count();
    assert_eq!(
        missing, 1,
        "one diagnostic for one absent plugin: {diags:?}"
    );
}
