//! T5.3 golden test: the same project built twice must produce
//! byte-identical artifacts and an identical manifest content hash.

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
name = "golden"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client"
file_template = "{table}.json"

[[profiles.client.targets]]
format = "csharp"
output_dir = "build/cs"

[[profiles.client.targets]]
format = "python"
output_dir = "build/python"

[[profiles.client.targets]]
format = "lua"
output_dir = "build/lua"

[[profiles.client.targets]]
format = "typescript"
output_dir = "build/typescript"

[[profiles.client.targets]]
format = "javascript"
output_dir = "build/javascript"

[[profiles.client.targets]]
format = "cpp"
output_dir = "build/cpp"

[[profiles.client.targets]]
format = "go"
output_dir = "build/go"

[[profiles.client.targets]]
format = "java"
output_dir = "build/java"
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
      price: { name: price, type: { kind: Int32 }, min: 0 }
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
        r#"{"Item": [{"id": 1, "name": "Sword", "price": 100, "kind": "Sword"}, {"id": 2, "name": "Shield", "price": 50, "kind": "Shield"}]}"#,
    )
    .unwrap();
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("build")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root.join("build"))
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn build_is_byte_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let bin = env!("CARGO_BIN_EXE_cage");
    for _ in 0..2 {
        let status = Command::new(bin)
            .args(["build", root.to_str().unwrap(), "--profile", "client"])
            .status()
            .unwrap();
        assert!(status.success());
    }

    // Build a second time into the same tree must not change any bytes.
    let first = snapshot(root);
    let status = Command::new(bin)
        .args(["build", root.to_str().unwrap(), "--profile", "client"])
        .status()
        .unwrap();
    assert!(status.success());
    let second = snapshot(root);

    // Data + code targets (json/cs/python/lua/ts/js/cpp/go/java) all
    // participate in the lock.
    assert!(
        first.len() >= 16,
        "expected code+data artifacts, got {}",
        first.len()
    );
    assert_eq!(first.len(), second.len());
    for ((name_a, bytes_a), (name_b, bytes_b)) in first.iter().zip(second.iter()) {
        assert_eq!(name_a, name_b);
        assert_eq!(bytes_a, bytes_b, "artifact {name_a} changed between builds");
    }
}
