//! Process-level coverage of the local Configuration Registry (R1 + R2):
//! `cage registry publish` (fresh build → self-verifying snapshot → entry,
//! idempotent identical re-publish, E1801 version conflict), `cage registry
//! list`, consumer-side source resolution — `registry:<pkg>[@<ver>]`
//! source roots resolve to the entry's data artifacts (manifest-driven
//! table identity), refusing tampered entries (E1803) and unresolved
//! references (E1802) — and the R2 layer: `[dependencies]` pins (version
//! ranges pick the max satisfying version; an explicit `@<ver>` outside
//! its pin is rejected) and `schema_path: registry:<pkg>` loading the
//! entry's published schema.

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
    write_consumer_full(root, name, source_root, None, "schema.yaml");
}

/// Full-shape consumer: `source_root` lands in `[source_roots].main`,
/// `deps` (a version-requirement string) becomes a `[dependencies]` pin on
/// the `common` package, and `schema_path` picks between a local
/// `schema.yaml` and the entry schema (`registry:common`, R2 — the
/// consumer then ships no schema file of its own).
fn write_consumer_full(
    root: &Path,
    name: &str,
    source_root: &str,
    deps: Option<&str>,
    schema_path: &str,
) {
    let deps_block = deps
        .map(|d| format!("\n[dependencies]\ncommon = \"{d}\"\n"))
        .unwrap_or_default();
    let body = r#"output_dir = "build"
schema_path = "SCHEMA_PATH"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "registry:PLACEHOLDER"

[registry]
path = "../reg"
DEPS_LINE
[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#
    .replace("registry:PLACEHOLDER", source_root)
    .replace("SCHEMA_PATH", schema_path)
    .replace("DEPS_LINE\n", &deps_block);
    write(&root.join(format!("{name}/cage.toml")), &body);
    if !schema_path.starts_with("registry:") {
        write(&root.join(format!("{name}/schema.yaml")), SCHEMA);
    }
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

#[test]
fn registry_dependencies_pin_and_schema_from_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    // Three versions with distinct data — latest is 2.0.0.
    for (v, marker) in [("1.0.0", "Sword"), ("1.9.0", "Shield"), ("2.0.0", "Lance")] {
        write(&root.join("pub/config/item.json"), &item_rows(marker));
        let out = run_cage(&[
            "registry",
            "publish",
            &pub_root,
            "--registry",
            &reg,
            "--version",
            v,
        ]);
        assert_code(&out, 0, v);
    }

    // A pin narrows the resolution: bare `registry:common` with
    // `common = ">=1.0, <2.0"` must land on 1.9.0, not the 2.0.0 latest —
    // and the schema comes from the entry too (`schema_path` is also a
    // registry: spec; the consumer ships no schema file of its own).
    write_consumer_full(
        root,
        "depcon",
        "registry:common",
        Some(">=1.0, <2.0"),
        "registry:common",
    );
    let depcon = root.join("depcon").to_str().unwrap().to_string();
    assert!(!root.join("depcon/schema.yaml").exists());
    let out = run_cage(&["check", &depcon]);
    assert_code(&out, 0, "pinned check with entry schema");
    assert!(stdout(&out).contains("OK (1 tables"), "{}", stdout(&out));
    let out = run_cage(&["build", &depcon, "--profile", "client"]);
    assert_code(&out, 0, "pinned build with entry schema");
    let artifact = fs::read_to_string(root.join("depcon/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Shield"),
        "pin must resolve 1.9.0, not the 2.0.0 latest: {artifact}"
    );

    // Caret pins pick the same max-satisfying version (local schema file
    // this time — registry source + filesystem schema is the mixed mode).
    write_consumer_full(
        root,
        "caretcon",
        "registry:common",
        Some("^1.0.0"),
        "schema.yaml",
    );
    let caretcon = root.join("caretcon").to_str().unwrap().to_string();
    let out = run_cage(&["build", &caretcon, "--profile", "client"]);
    assert_code(&out, 0, "caret pin build");
    let artifact = fs::read_to_string(root.join("caretcon/build/client/json/Item.json")).unwrap();
    assert!(
        artifact.contains("Shield"),
        "caret ^1.0.0 must hit 1.9.0: {artifact}"
    );

    // An explicit @version outside its pin is rejected (E1802) — the pin
    // wins over the reference.
    write_consumer_full(
        root,
        "conflict",
        "registry:common@2.0.0",
        Some("<2.0"),
        "schema.yaml",
    );
    let conflict = root.join("conflict").to_str().unwrap().to_string();
    let out = run_cage(&["check", &conflict]);
    assert_code(&out, 2, "version vs pin conflict");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("does not satisfy"),
        "{}",
        stderr(&out)
    );

    // A requirement nothing satisfies → E1802 with the published list.
    write_consumer_full(
        root,
        "impossible",
        "registry:common",
        Some(">=3.0"),
        "schema.yaml",
    );
    let impossible = root.join("impossible").to_str().unwrap().to_string();
    let out = run_cage(&["check", &impossible]);
    assert_code(&out, 2, "unsatisfiable requirement");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("satisfies requirement"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("2.0.0"), "{}", stderr(&out));

    // A malformed dependency pin → E1802 naming the dependency.
    write_consumer_full(
        root,
        "badpin",
        "registry:common",
        Some("abc"),
        "schema.yaml",
    );
    let badpin = root.join("badpin").to_str().unwrap().to_string();
    let out = run_cage(&["check", &badpin]);
    assert_code(&out, 2, "malformed pin");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("dependency 'common'"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn registry_verify_detects_tamper_ledger_drift_and_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    for (v, marker) in [("1.0.0", "Sword"), ("1.9.0", "Shield"), ("2.0.0", "Lance")] {
        write(&root.join("pub/config/item.json"), &item_rows(marker));
        let out = run_cage(&[
            "registry",
            "publish",
            &pub_root,
            "--registry",
            &reg,
            "--version",
            v,
        ]);
        assert_code(&out, 0, v);
    }

    // A fresh registry audits clean: every recorded entry re-hashed.
    let out = run_cage(&["registry", "verify", "--registry", &reg]);
    assert_code(&out, 0, "clean verify");
    assert!(stdout(&out).contains("OK"), "{}", stdout(&out));
    assert!(stdout(&out).contains("3 entry"), "{}", stdout(&out));

    // A hand-edit inside an entry breaks its byte hashes → E1803 finding.
    fs::write(
        root.join("reg/common/1.0.0/data/client/json/Item.json"),
        r#"[{"id": 1, "name": "Forged"}]"#,
    )
    .unwrap();
    let out = run_cage(&["registry", "verify", "--registry", &reg]);
    assert_code(&out, 2, "tampered entry");
    assert!(
        stdout(&out).contains("E1803") && stdout(&out).contains("common/1.0.0"),
        "{}",
        stdout(&out)
    );

    // Restore the bytes, then drift the ledger away from the index record:
    // verify_snapshot alone would pass (files match the ledger), the
    // registry audit cross-checks the index record against it.
    write(&root.join("pub/config/item.json"), &item_rows("Sword"));
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "1.0.0",
    ]);
    assert_code(&out, 0, "restore 1.0.0");
    let ledger_path = root.join("reg/common/1.9.0/HASHES.json");
    let mut ledger: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&ledger_path).unwrap()).unwrap();
    ledger["build_id"] = serde_json::Value::String("forged000000000000000".to_string());
    fs::write(&ledger_path, serde_json::to_string_pretty(&ledger).unwrap()).unwrap();
    let out = run_cage(&["registry", "verify", "--registry", &reg]);
    assert_code(&out, 2, "ledger/index drift");
    assert!(
        stdout(&out).contains("does not match ledger") && stdout(&out).contains("common/1.9.0"),
        "{}",
        stdout(&out)
    );

    // An entry directory with no index record (interrupted remove, hand
    // edit) is reported — and gc will sweep it.
    fs::create_dir_all(root.join("reg/common/0.5.0")).unwrap();
    let out = run_cage(&["registry", "verify", "--registry", &reg]);
    assert_code(&out, 2, "orphan directory");
    assert!(
        stdout(&out).contains("without index record") && stdout(&out).contains("0.5.0"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn registry_gc_keeps_window_and_remove_republish() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    for (v, marker) in [
        ("1.0.0", "Sword"),
        ("1.1.0", "Shield"),
        ("1.2.0", "Bow"),
        ("2.0.0", "Lance"),
    ] {
        write(&root.join("pub/config/item.json"), &item_rows(marker));
        let out = run_cage(&[
            "registry",
            "publish",
            &pub_root,
            "--registry",
            &reg,
            "--version",
            v,
        ]);
        assert_code(&out, 0, v);
    }

    // A consumer pinned to the oldest version resolves fine before gc —
    // this is exactly the rollback surface the keep window protects.
    write_consumer_full(root, "oldpin", "registry:common@1.0.0", None, "schema.yaml");
    let oldpin = root.join("oldpin").to_str().unwrap().to_string();
    let out = run_cage(&["build", &oldpin, "--profile", "client"]);
    assert_code(&out, 0, "old pin pre-gc");

    // Dry run: the removal plan is reported, the registry is untouched.
    let before = tree_fingerprint(&root.join("reg"));
    let out = run_cage(&["registry", "gc", "--registry", &reg, "--dry-run"]);
    assert_code(&out, 0, "gc dry run");
    assert!(stdout(&out).contains("dry run"), "{}", stdout(&out));
    assert!(stdout(&out).contains("common/1.0.0"), "{}", stdout(&out));
    assert_eq!(
        tree_fingerprint(&root.join("reg")),
        before,
        "dry run must not touch the registry"
    );

    // Real gc (default keep 3): only 1.0.0 falls out of the window.
    let out = run_cage(&["registry", "gc", "--registry", &reg]);
    assert_code(&out, 0, "gc default window");
    assert!(stdout(&out).contains("common/1.0.0"), "{}", stdout(&out));
    assert!(!root.join("reg/common/1.0.0").exists());
    assert!(root.join("reg/common/1.1.0").is_dir());
    // The list reflects the rewritten index.
    let out = run_cage(&["registry", "list", "--registry", &reg]);
    assert_code(&out, 0, "list after gc");
    assert!(
        !stdout(&out).contains("1.0.0") && stdout(&out).contains("1.1.0"),
        "{}",
        stdout(&out)
    );
    // The window survives a consumer rebuild (rollback surface intact).
    write_consumer_full(
        root,
        "shiftpin",
        "registry:common@1.1.0",
        None,
        "schema.yaml",
    );
    let shiftpin = root.join("shiftpin").to_str().unwrap().to_string();
    let out = run_cage(&["build", &shiftpin, "--profile", "client"]);
    assert_code(&out, 0, "kept version builds");

    // keep is a floor of one: --keep 1 leaves the newest only, and the
    // windowed-out version stops resolving for consumers (E1802).
    let out = run_cage(&["registry", "gc", "--registry", &reg, "--keep", "1"]);
    assert_code(&out, 0, "gc keep 1");
    assert!(root.join("reg/common/2.0.0").is_dir());
    assert!(!root.join("reg/common/1.1.0").exists());
    let out = run_cage(&["check", &shiftpin]);
    assert_code(&out, 2, "windowed-out version gone");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));

    // Explicit remove: dry run first, then the real removal; the empty
    // package index stays behind and the registry still audits clean.
    let out = run_cage(&[
        "registry",
        "remove",
        "common",
        "2.0.0",
        "--registry",
        &reg,
        "--dry-run",
    ]);
    assert_code(&out, 0, "remove dry run");
    assert!(stdout(&out).contains("would remove"), "{}", stdout(&out));
    assert!(root.join("reg/common/2.0.0").is_dir());
    let out = run_cage(&["registry", "remove", "common", "2.0.0", "--registry", &reg]);
    assert_code(&out, 0, "remove 2.0.0");
    assert!(!root.join("reg/common/2.0.0").exists());
    let out = run_cage(&["registry", "verify", "--registry", &reg]);
    assert_code(&out, 0, "verify after remove");
    let out = run_cage(&["registry", "remove", "common", "9.9.9", "--registry", &reg]);
    assert_code(&out, 2, "remove missing");
    assert!(stderr(&out).contains("E1802"), "{}", stderr(&out));

    // Removal is explicit history editing: the exact same snapshot
    // re-publishes cleanly into the emptied version slot.
    write(&root.join("pub/config/item.json"), &item_rows("Lance"));
    let out = run_cage(&[
        "registry",
        "publish",
        &pub_root,
        "--registry",
        &reg,
        "--version",
        "2.0.0",
    ]);
    assert_code(&out, 0, "republish after remove");
    assert!(!stderr(&out).contains("E1801"), "{}", stderr(&out));
    write_consumer_full(root, "newpin", "registry:common@2.0.0", None, "schema.yaml");
    let newpin = root.join("newpin").to_str().unwrap().to_string();
    let out = run_cage(&["build", &newpin, "--profile", "client"]);
    assert_code(&out, 0, "republished version builds");
}

#[test]
fn registry_export_is_byte_deterministic_tar() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    let out = run_cage(&["registry", "publish", &pub_root, "--registry", &reg]);
    assert_code(&out, 0, "publish for export");

    // Two exports of the same entry land byte-identical (A1 determinism
    // contract: member-name order, zeroed mtime/uid/gid).
    let b1 = root.join("b1.tar").to_str().unwrap().to_string();
    let b2 = root.join("b2.tar").to_str().unwrap().to_string();
    let out = run_cage(&[
        "registry",
        "export",
        "common@0.1.0",
        "-o",
        &b1,
        "--registry",
        &reg,
    ]);
    assert_code(&out, 0, "export pinned");
    assert!(stdout(&out).contains("common/0.1.0"), "{}", stdout(&out));
    let out = run_cage(&[
        "registry",
        "export",
        "common",
        "-o",
        &b2,
        "--registry",
        &reg,
    ]);
    assert_code(&out, 0, "export latest");
    assert_eq!(
        fs::read(&b1).unwrap(),
        fs::read(&b2).unwrap(),
        "same entry must export to identical bytes"
    );

    // The bundle reads back as a plain tar with the entry files + ledger +
    // index excerpt, headers carrying the zeroed metadata.
    let mut archive = tar::Archive::new(fs::File::open(&b1).unwrap());
    let mut names = Vec::new();
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        names.push(entry.path().unwrap().to_string_lossy().into_owned());
        assert_eq!(entry.header().mtime().unwrap(), 0, "mtime zeroed");
        assert_eq!(entry.header().uid().unwrap(), 0, "uid zeroed");
        assert_eq!(entry.header().gid().unwrap(), 0, "gid zeroed");
    }
    names.sort();
    assert!(names.contains(&"index.json".to_string()), "{names:?}");
    assert!(
        names.contains(&"common/0.1.0/HASHES.json".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"common/0.1.0/data/client/json/Item.json".to_string()),
        "{names:?}"
    );

    // Missing package and missing version are E2101 failures.
    let bx = root.join("bx.tar").to_str().unwrap().to_string();
    let out = run_cage(&[
        "registry",
        "export",
        "ghost@0.1.0",
        "-o",
        &bx,
        "--registry",
        &reg,
    ]);
    assert_code(&out, 1, "export missing package");
    assert!(stderr(&out).contains("E2101"), "{}", stderr(&out));
    let out = run_cage(&[
        "registry",
        "export",
        "common@9.9.9",
        "-o",
        &bx,
        "--registry",
        &reg,
    ]);
    assert_code(&out, 1, "export missing version");
    assert!(stderr(&out).contains("E2101"), "{}", stderr(&out));
}

#[test]
fn registry_export_compress_zstd_roundtrip_and_determinism() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    let out = run_cage(&["registry", "publish", &pub_root, "--registry", &reg]);
    assert_code(&out, 0, "publish for export");

    // Two zstd exports of the same entry land byte-identical (fixed
    // compression level — the container form of the A1 determinism
    // contract), and the file head carries the zstd frame magic.
    let z1 = root.join("z1.tar.zst").to_str().unwrap().to_string();
    let z2 = root.join("z2.tar.zst").to_str().unwrap().to_string();
    let out = run_cage(&[
        "registry",
        "export",
        "common",
        "-o",
        &z1,
        "--compress",
        "zstd",
        "--registry",
        &reg,
    ]);
    assert_code(&out, 0, "zstd export");
    assert!(stdout(&out).contains("zstd container"), "{}", stdout(&out));
    let out = run_cage(&[
        "registry",
        "export",
        "common@0.1.0",
        "-o",
        &z2,
        "--compress",
        "zstd",
        "--registry",
        &reg,
    ]);
    assert_code(&out, 0, "second zstd export");
    let wrapped = fs::read(&z1).unwrap();
    assert_eq!(
        wrapped,
        fs::read(&z2).unwrap(),
        "fixed level must export byte-identical containers"
    );
    assert_eq!(
        &wrapped[..4],
        &[0x28, 0xB5, 0x2F, 0xFD],
        "zstd frame magic at the head"
    );

    // The compressed bundle imports like its plain form: dry run first
    // (nothing written), then a real import whose entry verifies clean
    // and carries the source bytes (the ledger hashes uncompressed
    // content, so the container is invisible to the trust gate).
    let reg_b = root.join("regB");
    fs::create_dir_all(&reg_b).unwrap();
    let reg_b_s = reg_b.to_str().unwrap().to_string();
    let out = run_cage(&[
        "registry",
        "import",
        &z1,
        "--dry-run",
        "--registry",
        &reg_b_s,
    ]);
    assert_code(&out, 0, "dry-run import of zstd bundle");
    assert!(stdout(&out).contains("would import"), "{}", stdout(&out));
    assert!(!reg_b.join("common").exists(), "dry run must not write");
    let out = run_cage(&["registry", "import", &z1, "--registry", &reg_b_s]);
    assert_code(&out, 0, "import of zstd bundle");
    assert!(
        stdout(&out).contains("imported common/0.1.0"),
        "{}",
        stdout(&out)
    );
    let out = run_cage(&["registry", "verify", "--registry", &reg_b_s]);
    assert_code(&out, 0, "imported entry verifies");
    assert_eq!(
        fs::read(reg_b.join("common/0.1.0/data/client/json/Item.json")).unwrap(),
        fs::read(root.join("reg/common/0.1.0/data/client/json/Item.json")).unwrap(),
        "decompressed entry must match the source bytes"
    );

    // Re-import is the usual idempotent no-op.
    let out = run_cage(&["registry", "import", &z1, "--registry", &reg_b_s]);
    assert_code(&out, 0, "re-import zstd bundle");
    assert!(
        stdout(&out).contains("identical, no-op"),
        "{}",
        stdout(&out)
    );

    // Extension-blind sniff: the compressed bundle imports the same under a
    // plain `.tar` name — the file head, not the suffix, decides.
    let lying = root.join("actually-zstd.tar").to_str().unwrap().to_string();
    fs::copy(&z1, &lying).unwrap();
    let reg_c = root.join("regC");
    fs::create_dir_all(&reg_c).unwrap();
    let reg_c_s = reg_c.to_str().unwrap().to_string();
    let out = run_cage(&["registry", "import", &lying, "--registry", &reg_c_s]);
    assert_code(&out, 0, "zstd bundle under a .tar name");
    assert!(
        stdout(&out).contains("imported common/0.1.0"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn registry_export_compress_rejects_unknown_format() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let bx = root.join("bx.tar").to_str().unwrap().to_string();

    // Unknown container format is a usage error (exit 2) — only the literal
    // `zstd` is accepted, and nothing is written.
    let out = run_cage(&[
        "registry",
        "export",
        "common",
        "-o",
        &bx,
        "--compress",
        "gzip",
        "--registry",
        &reg,
    ]);
    assert_code(&out, 2, "unknown --compress value");
    assert!(
        stderr(&out).contains("zstd"),
        "the error names the accepted value: {}",
        stderr(&out)
    );
    assert!(
        !root.join("bx.tar").exists(),
        "failed export writes nothing"
    );
}

/// Rebuild a bundle with one member's bytes replaced (tamper fixture).
fn repack_bundle(src: &Path, dst: &Path, member_suffix: &str, new_bytes: &[u8]) {
    use std::io::Read;
    let data = fs::read(src).unwrap();
    let mut ar = tar::Archive::new(&data[..]);
    let out = fs::File::create(dst).unwrap();
    let mut builder = tar::Builder::new(out);
    for e in ar.entries().unwrap() {
        let mut e = e.unwrap();
        let name = e.path().unwrap().to_string_lossy().into_owned();
        let mut content = Vec::new();
        e.read_to_end(&mut content).unwrap();
        let bytes = if name.ends_with(member_suffix) {
            new_bytes.to_vec()
        } else {
            content
        };
        let mut h = tar::Header::new_ustar();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_uid(0);
        h.set_gid(0);
        builder.append_data(&mut h, &name, &bytes[..]).unwrap();
    }
    builder.into_inner().unwrap();
}

#[test]
fn registry_import_roundtrips_and_gates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg_a = root.join("regA").to_str().unwrap().to_string();
    let reg_b = root.join("regB");
    fs::create_dir_all(&reg_b).unwrap();
    let reg_b_s = reg_b.to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();
    let bundle = root.join("b.tar").to_str().unwrap().to_string();

    let out = run_cage(&["registry", "publish", &pub_root, "--registry", &reg_a]);
    assert_code(&out, 0, "publish into regA");
    let out = run_cage(&[
        "registry",
        "export",
        "common@0.1.0",
        "-o",
        &bundle,
        "--registry",
        &reg_a,
    ]);
    assert_code(&out, 0, "export from regA");

    // Dry run: full report, nothing written.
    let out = run_cage(&[
        "registry",
        "import",
        &bundle,
        "--dry-run",
        "--registry",
        &reg_b_s,
    ]);
    assert_code(&out, 0, "dry-run import");
    assert!(stdout(&out).contains("would import"), "{}", stdout(&out));
    assert!(!reg_b.join("common").exists(), "dry run must not write");

    // Clean import → the entry is a first-class citizen of regB: a consumer
    // resolves and builds against it, entry schema included.
    let out = run_cage(&["registry", "import", &bundle, "--registry", &reg_b_s]);
    assert_code(&out, 0, "import");
    assert!(
        stdout(&out).contains("imported common/0.1.0"),
        "{}",
        stdout(&out)
    );
    write(
        &root.join("imp/cage.toml"),
        r#"output_dir = "build"
schema_path = "registry:common"

[project]
name = "imp"
version = "0.1.0"

[source_roots]
main = "registry:common@0.1.0"

[registry]
path = "../regB"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#,
    );
    let importer = root.join("imp").to_str().unwrap().to_string();
    let out = run_cage(&["build", &importer, "--profile", "client"]);
    assert_code(&out, 0, "imported entry builds a consumer");

    // Re-importing the same bundle is an idempotent no-op.
    let out = run_cage(&["registry", "import", &bundle, "--registry", &reg_b_s]);
    assert_code(&out, 0, "re-import identical");
    assert!(
        stdout(&out).contains("identical, no-op"),
        "{}",
        stdout(&out)
    );

    // Tampered bundle bytes never enter a registry (E2103).
    let tampered = root.join("tampered.tar").to_str().unwrap().to_string();
    repack_bundle(
        Path::new(&bundle),
        Path::new(&tampered),
        "Item.json",
        b"hacked",
    );
    let reg_c = root.join("regC");
    fs::create_dir_all(&reg_c).unwrap();
    let reg_c_s = reg_c.to_str().unwrap().to_string();
    let out = run_cage(&["registry", "import", &tampered, "--registry", &reg_c_s]);
    assert_code(&out, 1, "tampered bundle");
    assert!(stderr(&out).contains("E2103"), "{}", stderr(&out));
    assert!(
        !reg_c.join("common").exists(),
        "refused bytes must not land"
    );

    // Same version, different bytes already in the target → E1801 conflict.
    let out = run_cage(&[
        "registry",
        "remove",
        "common",
        "0.1.0",
        "--registry",
        &reg_b_s,
    ]);
    assert_code(&out, 0, "remove for conflict setup");
    write(&root.join("pub/config/item.json"), &item_rows("Lance"));
    let out = run_cage(&["registry", "publish", &pub_root, "--registry", &reg_b_s]);
    assert_code(&out, 0, "publish conflicting bytes");
    let out = run_cage(&["registry", "import", &bundle, "--registry", &reg_b_s]);
    assert_code(&out, 1, "conflicting import");
    assert!(stderr(&out).contains("E1801"), "{}", stderr(&out));
}

/// The msgpack re-consumption loop (R1 end to end): the publisher carries a
/// msgpack-only data target, so the entry's `data/` has nothing but
/// `msgpack/Item.msgpack` — consumer resolution must read the binary
/// format back (floats bit-exact, e.g. 0.25) and build against the entry
/// schema. Before the msgpack source adapter this failed with E1802 "no
/// data artifacts".
#[test]
fn registry_entry_with_msgpack_data_target_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let schema = r#"tables:
  Item:
    name: Item
    description: An inventory item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String }, required: true }
      ratio: { name: ratio, type: { kind: Float64 } }
enums: {}
"#;
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
format = "msgpack"
output_dir = "build/client/msgpack"
file_template = "{table}.msgpack"
"#,
    );
    write(&root.join("pub/schema.yaml"), schema);
    write(
        &root.join("pub/config/item.json"),
        r#"{
  "Item": [
    { "id": 1, "name": "Sword", "ratio": 0.25 }
  ]
}
"#,
    );

    // Consumer: registry source root, entry schema, plain json target.
    write(
        &root.join("consumer/cage.toml"),
        r#"output_dir = "build"
schema_path = "registry:common"

[project]
name = "consumer"
version = "0.1.0"

[source_roots]
main = "registry:common"

[registry]
path = "../reg"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
"#,
    );

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
    assert_code(&out, 0, "publish msgpack-only entry");

    // The entry packed the msgpack artifact under data/ (profile view
    // layout: data/<profile>/<format>/<table>.msgpack).
    let packed = root.join("reg/common/1.0.0/data/client/msgpack/Item.msgpack");
    assert!(packed.is_file(), "packed: {:?}", packed.display());

    // Consumer build: resolution re-consumes the packed msgpack rows.
    let consumer = root.join("consumer").to_str().unwrap().to_string();
    let out = run_cage(&["build", &consumer]);
    assert_code(&out, 0, "consumer build from msgpack entry");
    let built = fs::read_to_string(root.join("consumer/build/client/json/Item.json")).unwrap();
    assert!(built.contains("0.25"), "float survived: {built}");
    assert!(built.contains("Sword"), "row survived: {built}");
}
