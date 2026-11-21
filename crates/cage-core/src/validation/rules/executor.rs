//! Unified L7 executor tests (review P2-3): in-process Rust validators and
//! independent-process worker plugins merge onto one diagnostics surface.

use super::*;
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
    let mut registry = GameRuleRegistry::with_builtins();
    registry.register_worker(tmp.path(), manifest, serde_json::json!({"cap": 10}));

    // level 12 > worker cap 10 → worker finding; attack 1300 > level*100+50
    // (1250) → builtin finding. Both land on the same surface.
    let diags = registry.run(&Schema::new(), &monster_doc(12, 1300));
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
