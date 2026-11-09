//! Consumer-side resolution tests: `registry:<pkg>[@<ver>]` error cases,
//! `[dependencies]` pins with `schema_path: registry:<pkg>`, and the
//! tamper/ledger-drift/orphan refusal gates.

use super::*;
use std::fs;

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
