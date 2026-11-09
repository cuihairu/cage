//! `cage registry import` tests: bundle roundtrip with tamper/format
//! gates (via the repack fixture) and the msgpack data-target roundtrip.

use super::*;
use std::fs;
use std::path::Path;

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
