//! `cage gen` and code-target builds: schema-driven bindings for C#,
//! Python, Lua, TypeScript, JavaScript, C++, Go and Java. Gen skips data
//! targets, writes the same manifest 口径 as build, and is byte-deterministic
//! across runs (golden lock).

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

[[profiles.codeonly.targets]]
format = "ts"
output_dir = "build/typescript"

[[profiles.codeonly.targets]]
format = "js"
output_dir = "build/javascript"

[[profiles.codeonly.targets]]
format = "cpp"
output_dir = "build/cpp"

[[profiles.codeonly.targets]]
format = "go"
output_dir = "build/go"

[[profiles.codeonly.targets]]
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
      kind: { name: kind, type: { kind: Enum, value: ItemKind } }
      tags: { name: tags, type: { kind: Array, value: { kind: String } }, default: [new] }
      drops: { name: drops, type: { kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } } }
      weights: { name: weights, type: { kind: Map, value: { key_type: int, value_type: { kind: Float32 } } } }
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
        r#"{"Item": [{"id": 1, "name": "Sword", "kind": "Sword", "tags": ["a"], "drops": {"common": [1, 2]}, "weights": {"1": 0.5, "2": 0.25}}]}"#,
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
    assert!(stdout.contains("cage gen: OK (profile 'client', 18 artifacts"));

    // All eight code targets materialize with schema-hash-stamped headers.
    let cs = fs::read_to_string(root.join("build/cs/Item.cs")).unwrap();
    assert!(cs.contains("//   schema: "));
    assert!(!cs.contains("(unavailable)"));
    // Map fields: Dictionary<K, V> with the crate's IReadOnly* array idiom.
    assert!(cs.contains("Dictionary<string, IReadOnlyList<int>>"));
    assert!(cs.contains("Dictionary<long, float>"));
    let py = fs::read_to_string(root.join("build/python/Item.py")).unwrap();
    assert!(py.contains("#   schema: "));
    assert!(py.contains("@dataclass(frozen=True)"));
    assert!(py.contains("class Item:"));
    // Map fields: dict annotations.
    assert!(py.contains("dict[str, list[int]]"));
    assert!(py.contains("dict[int, float]"));
    let lua = fs::read_to_string(root.join("build/lua/Item.lua")).unwrap();
    assert!(lua.contains("--   schema: "));
    assert!(lua.contains("M.name = \"Item\""));
    assert!(lua.contains("function M.new(t)"));
    // Map fields: labels distinguish the hash-part table from Array tables.
    assert!(lua.contains("map<string, array<integer>>"));
    assert!(lua.contains("map<int, number>"));
    assert!(lua.contains("键值表（hash part）"));
    let ts = fs::read_to_string(root.join("build/typescript/Item.ts")).unwrap();
    assert!(ts.contains("//   schema: "));
    assert!(ts.contains("export interface Item"));
    assert!(ts.contains("export const ItemDefaults"));
    // Map fields: Map types in the interface.
    assert!(ts.contains("Map<string, number[]>"));
    assert!(ts.contains("Map<number, number>"));
    let js = fs::read_to_string(root.join("build/javascript/Item.js")).unwrap();
    assert!(js.contains("//   schema: "));
    assert!(js.contains("@typedef {Object} Item"));
    assert!(js.contains("Map<string, number[]>"));
    // JS mode pairs every module with a declaration file by default.
    assert!(fs::read(root.join("build/javascript/Item.d.ts")).is_ok());
    let cpp = fs::read_to_string(root.join("build/cpp/Item.h")).unwrap();
    assert!(cpp.contains("//   schema: "));
    assert!(cpp.contains("namespace cage::generated"));
    assert!(cpp.contains("struct Item"));
    // Map fields: unordered_map with sorted include injection.
    assert!(cpp.contains("std::unordered_map<std::string, std::vector<std::int32_t>>"));
    assert!(cpp.contains("std::unordered_map<std::int64_t, float>"));
    assert!(cpp.contains("#include <unordered_map>"));
    let go = fs::read_to_string(root.join("build/go/Item.go")).unwrap();
    assert!(go.contains("// Code generated by Cage — DO NOT EDIT."));
    assert!(go.contains("type Item struct"));
    assert!(go.contains("func NewItem() Item"));
    // Map fields: typed maps, nilable without a pointer.
    assert!(go.contains("map[string][]int32"));
    assert!(go.contains("map[int64]float32"));
    let java = fs::read_to_string(root.join("build/java/Item.java")).unwrap();
    assert!(java.contains("//   schema: "));
    assert!(java.contains("package cage.generated;"));
    assert!(java.contains("public final class Item"));
    // Map fields: HashMap with boxed value types.
    assert!(java.contains("HashMap<String, List<Integer>>"));
    assert!(java.contains("HashMap<Long, Float>"));
    assert!(java.contains("import java.util.HashMap;"));
    // Shared enums units per language.
    assert!(fs::read(root.join("build/cs/CageEnums.cs")).is_ok());
    assert!(fs::read(root.join("build/python/cage_enums.py")).is_ok());
    assert!(fs::read(root.join("build/lua/cage_enums.lua")).is_ok());
    assert!(fs::read(root.join("build/typescript/cage_enums.ts")).is_ok());
    assert!(fs::read(root.join("build/javascript/cage_enums.js")).is_ok());
    assert!(fs::read(root.join("build/javascript/cage_enums.d.ts")).is_ok());
    assert!(fs::read(root.join("build/cpp/cage_enums.h")).is_ok());
    assert!(fs::read(root.join("build/go/cage_enums.go")).is_ok());
    assert!(fs::read(root.join("build/java/CageEnums.java")).is_ok());

    // Data targets are skipped by gen.
    assert!(!root.join("build/json").exists());

    // The manifest records the code artifacts with their raw format strings
    // and shares the build 口径 (project/profile/hashes).
    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(manifest.contains("\"format\": \"csharp\""));
    assert!(manifest.contains("\"format\": \"python\""));
    assert!(manifest.contains("\"format\": \"lua\""));
    assert!(manifest.contains("\"format\": \"typescript\""));
    assert!(manifest.contains("\"format\": \"javascript\""));
    assert!(manifest.contains("\"format\": \"cpp\""));
    assert!(manifest.contains("\"format\": \"go\""));
    assert!(manifest.contains("\"format\": \"java\""));
    assert!(manifest.contains("\"profile\": \"client\""));
    assert!(manifest.contains("build/python/cage_enums.py"));
    assert!(manifest.contains("build/lua/cage_enums.lua"));
    assert!(manifest.contains("build/typescript/cage_enums.ts"));
    assert!(manifest.contains("build/java/CageEnums.java"));
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
    let ts_1 = artifact_paths(root, "build/typescript");
    let js_1 = artifact_paths(root, "build/javascript");
    let cpp_1 = artifact_paths(root, "build/cpp");
    let go_1 = artifact_paths(root, "build/go");
    let java_1 = artifact_paths(root, "build/java");

    assert_success(
        &run_cage(&["gen", root.to_str().unwrap(), "--profile", "codeonly"]),
        "gen #2",
    );
    let cs_2 = artifact_paths(root, "build/cs");
    let py_2 = artifact_paths(root, "build/python");
    let lua_2 = artifact_paths(root, "build/lua");
    let ts_2 = artifact_paths(root, "build/typescript");
    let js_2 = artifact_paths(root, "build/javascript");
    let cpp_2 = artifact_paths(root, "build/cpp");
    let go_2 = artifact_paths(root, "build/go");
    let java_2 = artifact_paths(root, "build/java");

    // Same input → byte-identical output (T4 Canonical 口径).
    assert_eq!(cs_1, cs_2);
    assert_eq!(py_1, py_2);
    assert_eq!(lua_1, lua_2);
    assert_eq!(ts_1, ts_2);
    assert_eq!(js_1, js_2);
    assert_eq!(cpp_1, cpp_2);
    assert_eq!(go_1, go_2);
    assert_eq!(java_1, java_2);

    // Short aliases (cs/py/ts/js/cpp) resolve to the same generators as the
    // long names: the codeonly profile produced the very files client did.
    assert_eq!(cs_2.len(), 2); // Item.cs + CageEnums.cs
    assert_eq!(py_2.len(), 2); // Item.py + cage_enums.py
    assert_eq!(lua_2.len(), 2);
    assert_eq!(ts_2.len(), 2); // Item.ts + cage_enums.ts
    assert_eq!(js_2.len(), 4); // Item.js/.d.ts + cage_enums.js/.d.ts (emit_dts default)
    assert_eq!(cpp_2.len(), 2); // Item.h + cage_enums.h
    assert_eq!(go_2.len(), 2); // Item.go + cage_enums.go
    assert_eq!(java_2.len(), 2); // Item.java + CageEnums.java
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
    assert!(fs::read(root.join("build/typescript/Item.ts")).is_ok());
    assert!(fs::read(root.join("build/java/Item.java")).is_ok());
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
