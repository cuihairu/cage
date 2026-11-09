use super::*;

/// E9006 (Profile 语义化): a projection that would silently strip a
/// structurally required / key / unique / reference-critical field is a
/// conflict reported as E9006; optional server-only fields stay a
/// legitimate client view. Validation covers the full corpus regardless of
/// profile (profiles gate runtime views, not data quality).
#[test]
fn e9006_visibility_conflict_blocks_client_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "visibility"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client"
file_template = "{table}.json"

[profiles.server]
name = "server"

[[profiles.server.targets]]
format = "json"
output_dir = "build/server"
file_template = "{table}.json"
"#,
    )
    .unwrap();
    fs::write(
        root.join("schemas/account.yaml"),
        r#"tables:
  Account:
    name: Account
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      email: { name: email, type: { kind: String }, required: true }
      secret: { name: secret, type: { kind: String }, required: true,
                targets: [server] }
enums: {}
"#,
    )
    .unwrap();
    fs::write(
        root.join("config/account.json"),
        r#"{"Account": [{"id": 1, "email": "a@b.c", "secret": "s3cr3t"}]}"#,
    )
    .unwrap();

    // client projection would lose the required server-only field → E9006.
    let out = run_cage(&["check", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 1, "check client with hidden required field");
    let stdout = stdout(&out);
    assert!(stdout.contains("E9006"), "expected E9006, got:\n{stdout}");
    assert!(
        stdout.contains("Required field hidden by profile") && stdout.contains("secret"),
        "expected field-level conflict, got:\n{stdout}"
    );

    // build shares the conflict.
    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 1, "build client with hidden required field");

    // server profile sees the full schema → clean.
    let out = run_cage(&["check", root.to_str().unwrap(), "--profile", "server"]);
    assert_code(&out, 0, "check server profile");

    // Optional server-only fields remain a legitimate client view: relax the
    // field and the client build passes, with the field stripped from output.
    let schema_text = fs::read_to_string(root.join("schemas/account.yaml")).unwrap();
    let relaxed = schema_text.replace(
        "secret: { name: secret, type: { kind: String }, required: true,\n                targets: [server] }",
        "secret: { name: secret, type: { kind: String }, targets: [server] }",
    );
    fs::write(root.join("schemas/account.yaml"), relaxed).unwrap();
    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 0, "client build with optional server-only field");
    let artifact = fs::read_to_string(root.join("build/client/Account.json")).unwrap();
    assert!(!artifact.contains("secret"), "leak: {artifact}");
}

/// `cage snapshot`: packages a self-verifying Configuration Snapshot whose
/// directory name derives from profile + build_id (deterministic, not a
/// date), verifies on rebuild, and detects tampering on `--verify`.
#[test]
fn snapshot_builds_verifies_and_detects_tampering() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "snapshot"
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
format = "msgpack"
output_dir = "build/client"
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
enums: {}
"#,
    )
    .unwrap();
    fs::write(
        root.join("config/Item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
    )
    .unwrap();

    let out = run_cage(&["snapshot", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 0, "snapshot build");
    assert!(
        stdout(&out).contains("verified"),
        "snapshot must self-verify: {}",
        stdout(&out)
    );
    let dirs: Vec<_> = fs::read_dir(root.join("build/snapshot"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(dirs.len(), 1, "one deterministic snapshot dir");
    let snap_dir = dirs[0].clone();
    let snap_before: Vec<_> = {
        let mut files: Vec<_> = fs::read_dir(&snap_dir)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                p.is_file().then(|| (p.clone(), fs::read(&p).unwrap()))
            })
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, bytes) in &files {
            fs::write(path, bytes.clone()).unwrap();
        }
        // Ledger covers manifest + schema + artifacts; HASHES.json is the
        // trust root and is never self-hashed.
        let ledger: serde_json::Value =
            serde_json::from_slice(&fs::read(snap_dir.join("HASHES.json")).unwrap()).unwrap();
        assert_eq!(ledger["files"].as_object().unwrap().len(), 4);
        assert!(snap_dir.join("data/client/Item.json").is_file());
        // msgpack is a data target: packed under data/, not generated/.
        assert!(snap_dir.join("data/client/Item.msgpack").is_file());
        assert!(snap_dir.join("schema.json").is_file());
        files
    };

    // Identical inputs → identical snapshot directory and bytes.
    let out = run_cage(&["snapshot", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 0, "snapshot rebuild");
    let dirs: Vec<_> = fs::read_dir(root.join("build/snapshot"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(dirs.len(), 1, "rebuild must reuse the deterministic dir");
    let snap_after = &dirs[0];
    for (path, bytes) in &snap_before {
        let rel = path.strip_prefix(&snap_dir).unwrap();
        let after = fs::read(snap_after.join(rel)).unwrap();
        assert_eq!(&after, bytes, "deterministic byte match for {rel:?}");
    }

    // Clean verify exits 0; tampering is flagged with exit 1.
    let out = run_cage(&["snapshot", snap_after.to_str().unwrap(), "--verify"]);
    assert_code(&out, 0, "clean verify");
    fs::write(snap_after.join("data/client/Item.json"), br#"{"id":9}"#).unwrap();
    let out = run_cage(&["snapshot", snap_after.to_str().unwrap(), "--verify"]);
    assert_code(&out, 1, "tampered verify");
    assert!(
        stdout(&out).contains("hash mismatch"),
        "expected mismatch report: {}",
        stdout(&out)
    );
}

/// Local .msgpack sources load like any other source: the file stem names
/// the table (CSV convention) and the binary rows flow through validation
/// and targets like JSON rows.
#[test]
fn build_msgpack_source_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    fs::remove_file(root.join("config/item.json")).unwrap();

    // Hand-encoded `[{"id": 1, "name": "Sword"}]` (fixarray, fixmap,
    // fixstr keys, positive fixint id, 5-char fixstr name).
    let item: &[u8] = &[
        0x91, 0x82, //
        0xA2, b'i', b'd', 0x01, //
        0xA4, b'n', b'a', b'm', b'e', 0xA5, b'S', b'w', b'o', b'r', b'd',
    ];
    // File stem names the table (CSV convention) — capital I matches the
    // schema's `Item` table.
    fs::write(root.join("config/Item.msgpack"), item).unwrap();

    let root_s = root.to_str().unwrap().to_string();
    let out = run_cage(&["build", &root_s]);
    assert_code(&out, 0, "msgpack source build");
    let built = fs::read_to_string(root.join("build/json/Item.json")).unwrap();
    assert!(built.contains("Sword"), "built from msgpack: {built}");
}
