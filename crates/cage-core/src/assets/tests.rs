//! Asset inventory tests (design §50, B1): class discovery, determinism,
//! remote-spec skipping, and the missing/changed/unlisted verify triad.

use super::*;
use std::fs;

fn config(yaml: &str) -> ProjectConfig {
    serde_yaml::from_str(yaml).expect("test config must parse")
}

/// Minimal project: build rule + two schema files + two source files +
/// one migration, spread over the three classes.
fn write_project(root: &Path) {
    fs::create_dir_all(root.join("schemas")).expect("schemas dir");
    fs::create_dir_all(root.join("sources")).expect("sources dir");
    fs::create_dir_all(root.join("migrations")).expect("migrations dir");
    fs::write(
        root.join("cage.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .expect("config");
    fs::write(root.join("schemas/item.yaml"), "tables:\n  Item:\n").expect("schema item");
    fs::write(root.join("schemas/monster.yaml"), "tables:\n  Monster:\n").expect("schema monster");
    fs::write(root.join("sources/Item.csv"), "id,name\n1,sword\n").expect("source csv");
    fs::create_dir_all(root.join("sources/bonus")).expect("nested dir");
    fs::write(root.join("sources/bonus/Quest.json"), "[{\"id\": 1}]").expect("nested source");
    fs::write(root.join("migrations/0001_rename.yaml"), "steps: []\n").expect("migration");
}

#[test]
fn test_inventory_classes_paths_and_hashes() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    write_project(dir.path());
    let cfg = config("project:\n  name: demo\nprofiles: {}\nsource_roots:\n  main: sources\nschema_path: schemas\n");

    let inv = inventory(dir.path(), &cfg).expect("inventory");

    assert_eq!(inv.project, "demo");
    assert_eq!(inv.cage_version, env!("CARGO_PKG_VERSION"));
    let paths: Vec<(&str, AssetClass)> = inv
        .assets
        .iter()
        .map(|a| (a.path.as_str(), a.class))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("cage.toml", AssetClass::Build),
            ("migrations/0001_rename.yaml", AssetClass::Rule),
            ("schemas/item.yaml", AssetClass::Rule),
            ("schemas/monster.yaml", AssetClass::Rule),
            ("sources/Item.csv", AssetClass::Source),
            ("sources/bonus/Quest.json", AssetClass::Source),
        ]
    );
    // Hashes are blake3 of the bytes — spot-check one.
    let item = inv
        .assets
        .iter()
        .find(|a| a.path == "schemas/item.yaml")
        .expect("item record");
    assert_eq!(
        item.hash,
        blake3::hash(b"tables:\n  Item:\n").to_hex().to_string()
    );
    assert_eq!(item.size, "tables:\n  Item:\n".len() as u64);
}

#[test]
fn test_inventory_is_deterministic() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    write_project(dir.path());
    let cfg = config("project:\n  name: demo\nprofiles: {}\nsource_roots:\n  main: sources\nschema_path: schemas\n");

    let a = inventory(dir.path(), &cfg).expect("first");
    let b = inventory(dir.path(), &cfg).expect("second");
    assert_eq!(a, b);
    // And the serde round trip is stable (the --json / baseline shape).
    let json = serde_json::to_string(&a).expect("serialize");
    let back: AssetInventory = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, a);
}

#[test]
fn test_inventory_skips_remote_specs() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    fs::write(dir.path().join("cage.toml"), "[project]\nname = \"demo\"\n").expect("config");
    let cfg = config(
        "project:\n  name: demo\nprofiles: {}\nsource_roots:\n  pin: registry:game\n  web: https://x/y/data\n  sheet: gsheet:1abc/Tab\n  local: sources\nschema_path: registry:game/schema.json\n",
    );
    // `local` points nowhere: absent local roots must fail closed…
    let err = inventory(dir.path(), &cfg).unwrap_err();
    assert!(err.contains("failed to read"), "{err}");
    // …while the remote specs are skipped, not walked.
    let cfg_local_only = config("project:\n  name: demo\nprofiles: {}\nsource_roots:\n  pin: registry:game\n  web: https://x/y/data\n  sheet: gsheet:1abc/Tab\n");
    let inv = inventory(dir.path(), &cfg_local_only).expect("inventory");
    let paths: Vec<&str> = inv.assets.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(paths, vec!["cage.toml"]);
}

#[test]
fn test_verify_reports_missing_changed_unlisted() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    write_project(dir.path());
    let cfg = config("project:\n  name: demo\nprofiles: {}\nsource_roots:\n  main: sources\nschema_path: schemas\n");
    let baseline = inventory(dir.path(), &cfg).expect("baseline");

    // Clean tree verifies clean.
    let report = verify(dir.path(), &cfg, &baseline).expect("verify");
    assert!(report.ok, "{:?}", report.problems);
    assert_eq!(report.matched, baseline.assets.len());
    assert_eq!(report.problems, [] as [String; 0]);

    // Mutate all three ways: delete, rewrite, add.
    fs::remove_file(dir.path().join("schemas/monster.yaml")).expect("delete");
    fs::write(dir.path().join("schemas/item.yaml"), "tables:\n  Item2:\n").expect("rewrite");
    fs::write(dir.path().join("sources/New.csv"), "id\n2\n").expect("add");

    let report = verify(dir.path(), &cfg, &baseline).expect("verify");
    assert!(!report.ok);
    assert_eq!(report.problems.len(), 3, "{:?}", report.problems);
    assert!(report
        .problems
        .iter()
        .any(|p| p.starts_with("missing   schemas/monster.yaml")));
    assert!(report
        .problems
        .iter()
        .any(|p| p.starts_with("changed   schemas/item.yaml")));
    assert!(report
        .problems
        .iter()
        .any(|p| p.starts_with("unlisted  sources/New.csv")));
}

#[test]
fn test_verify_rejects_foreign_baseline() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    write_project(dir.path());
    let cfg = config("project:\n  name: demo\nprofiles: {}\nsource_roots:\n  main: sources\n");
    let baseline = AssetInventory {
        project: "other".to_string(),
        cage_version: "0.1.0".to_string(),
        assets: vec![],
    };
    let err = verify(dir.path(), &cfg, &baseline).unwrap_err();
    assert!(
        err.contains("baseline is for project 'other'") && err.contains("'demo'"),
        "{err}"
    );
}

#[test]
fn test_verify_rejects_non_inventory_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    write_project(dir.path());
    let cfg = config("project:\n  name: demo\nprofiles: {}\n");
    let baseline = AssetInventory {
        project: String::new(),
        cage_version: "0.1.0".to_string(),
        assets: vec![],
    };
    let err = verify(dir.path(), &cfg, &baseline).unwrap_err();
    assert!(err.contains("no project name"), "{err}");
}
