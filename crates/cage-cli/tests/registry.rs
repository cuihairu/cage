//! Process-level coverage of the local Configuration Registry (R1):
//! `cage registry publish` (fresh build → self-verifying snapshot → entry,
//! idempotent identical re-publish, E1801 version conflict), `cage registry
//! list`, and consumer-side source resolution — `registry:<pkg>[@<ver>]`
//! source roots resolve to the entry's data artifacts (manifest-driven
//! table identity), refusing tampered entries (E1803) and unresolved
//! references (E1802).

use std::fs;
use std::path::Path;
use std::process::Command;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_code(out: &std::process::Output, want: i32, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(want),
        "{what}: expected exit {want}\nstdout:\n{}\nstderr:\n{}",
        stdout(out),
        stderr(out)
    );
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// One Item row per version, so a consumer's built artifact proves which
/// entry version it resolved.
fn item_rows(marker: &str) -> String {
    format!(
        r#"{{
  "Item": [
    {{ "id": 1, "name": "{marker}" }}
  ]
}}
"#
    )
}

const SCHEMA: &str = r#"tables:
  Item:
    name: Item
    description: An inventory item
    primary_key: [id]
    fields:
      id:
        name: id
        type: { kind: Int32 }
        required: true
      name:
        name: name
        type: { kind: String }
        required: true
enums: {}
"#;

fn write_publisher(root: &Path) {
    write(
        &root.join("pub/cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "common"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"

[[profiles.client.targets]]
format = "csv"
output_dir = "build/client/csv"
file_template = "{table}.csv"
"#,
    );
    write(&root.join("pub/schema.yaml"), SCHEMA);
    write(&root.join("pub/config/item.json"), &item_rows("Sword"));
}

fn write_consumer(root: &Path, name: &str, source_root: &str) {
    write(
        &root.join(format!("{name}/cage.toml")),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "registry:PLACEHOLDER"

[registry]
path = "../reg"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
        .replace("registry:PLACEHOLDER", source_root)
        .as_str(),
    );
    write(&root.join(format!("{name}/schema.yaml")), SCHEMA);
}

/// sha256-free determinism probe: sorted (path, size) digest of a tree.
fn tree_fingerprint(root: &Path) -> String {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let meta = fs::metadata(&p).unwrap();
                entries.push(format!(
                    "{}:{}",
                    p.strip_prefix(root).unwrap().display(),
                    meta.len()
                ));
            }
        }
    }
    entries.sort();
    entries.join("\n")
}

#[test]
fn registry_publish_list_resolve_build_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    write_consumer(root, "consumer", "registry:common");
    write_consumer(root, "pinned", "registry:common@1.0.0");
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    // Publish 1.0.0 → entry + index.
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "publish 1.0.0");
    assert!(
        stdout(&out).contains("published common/1.0.0"),
        "{}",
        stdout(&out)
    );
    assert!(root.join("reg/common/1.0.0/HASHES.json").is_file());
    assert!(root
        .join("reg/common/1.0.0/data/client/json/Item.json")
        .is_file());
    let index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("reg/common/index.json")).unwrap())
            .unwrap();
    assert_eq!(index["package"], "common");
    assert_eq!(index["entries"][0]["version"], "1.0.0");

    // Determinism: re-publishing identical bytes rewrites nothing.
    let before = tree_fingerprint(&root.join("reg"));
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "re-publish identical");
    assert!(
        stdout(&out).contains("identical, no-op"),
        "{}",
        stdout(&out)
    );
    assert_eq!(
        tree_fingerprint(&root.join("reg")),
        before,
        "registry bytes must not move"
    );

    // 1.9.0 / 1.10.0 with different data — dotted-numeric latest is 1.10.0.
    write(&root.join("pub/config/item.json"), &item_rows("Shield"));
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.9.0",
    ]);
    assert_code(&out, 0, "publish 1.9.0");
    write(&root.join("pub/config/item.json"), &item_rows("Bow"));
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.10.0",
    ]);
    assert_code(&out, 0, "publish 1.10.0");

    let out = run_cage(&["registry", "list", "--registry", &reg]);
    assert_code(&out, 0, "list");
    let list = stdout(&out);
    for line in ["common", "1.0.0", "1.9.0", "1.10.0"] {
        assert!(list.contains(line), "list must contain {line}:\n{list}");
    }

    // Consumer resolves `registry:common` → latest (1.10.0, "Bow") and
    // builds real artifacts from it.
    let consumer = root.join("consumer").to_str().unwrap().to_string();
    let out = run_cage(&["check", &consumer]);
    assert_code(&out, 0, "consumer check");
    assert!(stdout(&out).contains("OK (1 tables"), "{}", stdout(&out));
    let out = run_cage(&["build", &consumer, "--profile", "client"]);
    assert_code(&out, 0, "consumer build");
    let artifact = fs::read_to_string(root.join("consumer/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Bow"),
        "latest must resolve to 1.10.0: {artifact}"
    );

    // Pinned consumer stays on 1.0.0 ("Sword").
    let pinned = root.join("pinned").to_str().unwrap().to_string();
    let out = run_cage(&["check", &pinned]);
    assert_code(&out, 0, "pinned check");
    let out = run_cage(&["build", &pinned, "--profile", "client"]);
    assert_code(&out, 0, "pinned build");
    let artifact = fs::read_to_string(root.join("pinned/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Sword"),
        "pin must stay on 1.0.0: {artifact}"
    );
}

#[test]
fn registry_publish_conflict_and_missing_version() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "initial publish");

    // Same version, different bytes → E1801, exit 1, entry untouched.
    write(&root.join("pub/config/item.json"), &item_rows("Shield"));
    let before = tree_fingerprint(&root.join("reg"));
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 1, "version conflict");
    assert!(stderr(&out).contains("E1801"), "{}", stderr(&out));
    assert_eq!(
        tree_fingerprint(&root.join("reg")),
        before,
        "conflict must not write"
    );

    // No --version and no project.version → usage error (exit 2).
    write(
        &root.join("noversion/cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "noversion"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#,
    );
    write(&root.join("noversion/schema.yaml"), SCHEMA);
    write(
        &root.join("noversion/config/item.json"),
        &item_rows("Sword"),
    );
    let nov = root.join("noversion").to_str().unwrap().to_string();
    let out = run_cage(&["registry", "publish", &nov, "--registry", &reg]);
    assert_code(&out, 2, "missing version");
    assert!(stderr(&out).contains("no version"), "{}", stderr(&out));
}

#[test]
fn registry_resolve_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    write_consumer(root, "consumer", "registry:common");
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    let consumer = root.join("consumer").to_str().unwrap().to_string();

    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "initial publish");

    // Unknown package → E1802 (the valid consumer is exercised later).
    write_consumer(root, "ghost", "registry:ghost");
    let ghost = root.join("ghost").to_str().unwrap().to_string();
    let out = run_cage(&["check", &ghost]);
    assert_code(&out, 2, "unknown package");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));

    // Unknown version → E1802 with the published list.
    write_consumer(root, "badver", "registry:common@9.9.9");
    let badver = root.join("badver").to_str().unwrap().to_string();
    let out = run_cage(&["check", &badver]);
    assert_code(&out, 2, "unknown version");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(stderr(&out).contains("1.0.0"), "{}", stderr(&out));

    // Path-escape spec → E1802 before anything touches the filesystem.
    write_consumer(root, "evil", "registry:../evil");
    let evil = root.join("evil").to_str().unwrap().to_string();
    let out = run_cage(&["check", &evil]);
    assert_code(&out, 2, "path escape");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));

    // Missing [registry].path → E1802 telling the author to wire it.
    write(
        &root.join("unwired/cage.toml"),
        r#"output_dir = "build"
schema_path = "schema.yaml"

[project]
name = "unwired"
version = "0.1.0"

[source_roots]
main = "registry:common"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#,
    );
    write(&root.join("unwired/schema.yaml"), SCHEMA);
    let unwired = root.join("unwired").to_str().unwrap().to_string();
    let out = run_cage(&["check", &unwired]);
    assert_code(&out, 2, "unwired registry");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(stderr(&out).contains("[registry]"), "{}", stderr(&out));

    // Tampered entry → E1803, the ledger catches the edit.
    let data = root.join("reg/common/1.0.0/data/client/json/Item.json");
    fs::write(&data, r#"[{"id": 1, "name": "Forged"}]"#).unwrap();
    let out = run_cage(&["check", &consumer]);
    assert_code(&out, 2, "tampered entry");
    assert!(stderr(&out).contains("E1803"), "{}", stderr(&out));
    assert!(stderr(&out).contains("hash mismatch"), "{}", stderr(&out));
}
