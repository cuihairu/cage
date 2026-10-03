//! `cage gen` and code-target builds: schema-driven bindings for C#,
//! Python and Lua. Gen skips data targets, writes the same manifest 口径
//! as build, and is byte-deterministic across runs (golden lock).

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
name = "codegen"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"

[[profiles.client.targets]]
format = "csharp"
output_dir = "build/cs"
file_template = "{table}.cs"

[[profiles.client.targets]]
format = "python"
output_dir = "build/python"
file_template = "{table}.py"

[[profiles.client.targets]]
format = "lua"
output_dir = "build/lua"
file_template = "{table}.lua"

[profiles.codeonly]
name = "codeonly"

[[profiles.codeonly.targets]]
format = "cs"
output_dir = "build/cs"

[[profiles.codeonly.targets]]
format = "py"
output_dir = "build/python"

[[profiles.codeonly.targets]]
format = "lua"
output_dir = "build/lua"
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
      tags: { name: tags, type: { kind: Array, value: { kind: String } }, default: [new] }
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
        r#"{"Item": [{"id": 1, "name": "Sword", "kind": "Sword", "tags": ["a"]}]}"#,
    )
    .unwrap();
}

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn artifact_paths(root: &Path, dir: &str) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in fs::read_dir(root.join(dir)).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            files.push((name, fs::read(entry.path()).unwrap()));
        }
    }
    files.sort();
    files
}

#[test]
fn gen_writes_code_targets_and_skips_data_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_success(&out, "gen");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cage gen: OK (profile 'client', 6 artifacts"));

    // All three code targets materialize with schema-hash-stamped headers.
    let cs = fs::read_to_string(root.join("build/cs/Item.cs")).unwrap();
    assert!(cs.contains("//   schema: "));
    assert!(!cs.contains("(unavailable)"));
    let py = fs::read_to_string(root.join("build/python/Item.py")).unwrap();
    assert!(py.contains("#   schema: "));
    assert!(py.contains("@dataclass(frozen=True)"));
    assert!(py.contains("class Item:"));
    let lua = fs::read_to_string(root.join("build/lua/Item.lua")).unwrap();
    assert!(lua.contains("--   schema: "));
    assert!(lua.contains("M.name = \"Item\""));
    assert!(lua.contains("function M.new(t)"));
    // Shared enums modules per language.
    assert!(fs::read(root.join("build/cs/CageEnums.cs")).is_ok());
    assert!(fs::read(root.join("build/python/cage_enums.py")).is_ok());
    assert!(fs::read(root.join("build/lua/cage_enums.lua")).is_ok());

    // Data targets are skipped by gen.
    assert!(!root.join("build/json").exists());

    // The manifest records the code artifacts with their raw format strings
    // and shares the build 口径 (project/profile/hashes).
    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(manifest.contains("\"format\": \"csharp\""));
    assert!(manifest.contains("\"format\": \"python\""));
    assert!(manifest.contains("\"format\": \"lua\""));
    assert!(manifest.contains("\"profile\": \"client\""));
    assert!(manifest.contains("build/python/cage_enums.py"));
    assert!(manifest.contains("build/lua/cage_enums.lua"));
}

#[test]
fn gen_is_byte_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    assert_success(
        &run_cage(&["gen", root.to_str().unwrap(), "--profile", "codeonly"]),
        "gen #1",
    );
    let cs_1 = artifact_paths(root, "build/cs");
    let py_1 = artifact_paths(root, "build/python");
    let lua_1 = artifact_paths(root, "build/lua");

    assert_success(
        &run_cage(&["gen", root.to_str().unwrap(), "--profile", "codeonly"]),
        "gen #2",
    );
    let cs_2 = artifact_paths(root, "build/cs");
    let py_2 = artifact_paths(root, "build/python");
    let lua_2 = artifact_paths(root, "build/lua");

    // Same input → byte-identical output (T4 Canonical 口径).
    assert_eq!(cs_1, cs_2);
    assert_eq!(py_1, py_2);
    assert_eq!(lua_1, lua_2);

    // Short aliases (cs/py) resolve to the same generators as the long
    // names: the codeonly profile produced the very files client did.
    assert_eq!(cs_2.len(), 2); // Item.cs + CageEnums.cs
    assert_eq!(py_2.len(), 2); // Item.py + cage_enums.py
    assert_eq!(lua_2.len(), 2);
}

#[test]
fn build_writes_code_targets_alongside_data() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "client"]);
    assert_success(&out, "build");

    // Data + code artifacts all present and recorded.
    assert!(fs::read(root.join("build/json/Item.json")).is_ok());
    assert!(fs::read(root.join("build/python/Item.py")).is_ok());
    assert!(fs::read(root.join("build/lua/Item.lua")).is_ok());
    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(manifest.contains("build/json/Item.json"));
    assert!(manifest.contains("\"format\": \"json\""));
}

#[test]
fn gen_without_code_targets_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"

[project]
name = "dataonly"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
"#,
    )
    .unwrap();
    fs::write(
        root.join("schemas/item.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\n",
    )
    .unwrap();
    fs::write(root.join("config/item.json"), r#"{"Item": [{"id": 1}]}"#).unwrap();

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no code targets"));
}
