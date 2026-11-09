//! `cage registry publish` lifecycle tests: end-to-end
//! publish/list/resolve/build, the E1801 version conflict, and the GC
//! window / remove-and-republish rounds.

use super::*;
use std::fs;

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
