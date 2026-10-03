//! `cage build` with a `csharp` code target: schema-driven C# bindings are
//! written per table plus a shared enums file, stamped with the schema hash
//! the manifest records.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write_project(root: &Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();

    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "csgen"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "csharp"
output_dir = "build/cs"
file_template = "{table}.cs"
"#,
    )
    .unwrap();

    fs::write(
        root.join("schemas/item.yaml"),
        r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
      - { name: Shield, value: 2 }
"#,
    )
    .unwrap();

    fs::write(
        root.join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword", "kind": "Sword"}]}"#,
    )
    .unwrap();
}

#[test]
fn csharp_target_generates_bindings_from_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let out = Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(["build", root.to_str().unwrap(), "--profile", "client"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "build failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let item = fs::read_to_string(root.join("build/cs/Item.cs")).unwrap();
    assert!(item.contains("namespace Cage.Generated"));
    assert!(item.contains("public sealed class Item"));
    assert!(item.contains("public int id { get; init; }"));
    assert!(item.contains("public string name { get; init; } = string.Empty;"));
    assert!(item.contains("public ItemKind? kind { get; init; }"));
    // Header stamps the real schema hash (not the "(unavailable)" fallback).
    assert!(item.contains("//   schema: "));
    assert!(!item.contains("(unavailable)"));

    let enums = fs::read_to_string(root.join("build/cs/CageEnums.cs")).unwrap();
    assert!(enums.contains("public enum ItemKind"));
    assert!(enums.contains("Sword = 1,"));
    assert!(enums.contains("Shield = 2,"));

    // The manifest records the generated artifacts with the csharp format.
    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(manifest.contains("\"format\": \"csharp\""));
    assert!(manifest.contains("build/cs/Item.cs"));
    assert!(manifest.contains("build/cs/CageEnums.cs"));
}
