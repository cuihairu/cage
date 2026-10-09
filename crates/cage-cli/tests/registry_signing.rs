//! Process-level coverage of signed bundles (A6, design §47): `cage
//! registry keygen` writes the seed to a file it names (never stdout),
//! `cage registry export --sign --key-env <VAR>` produces a detached
//! `<bundle>.sig` covering the exact bundle bytes, and `cage registry
//! import --verify-sig --key-env <VAR>` verifies it against the consumer's
//! trusted key before the ledger trust gate. Tampered bytes, a missing or
//! malformed sidecar, and a bundle from a different key are all E2107.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run_cage_with_env(args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cage"));
    cmd.args(args);
    for (name, value) in env {
        cmd.env(name, value);
    }
    cmd.output().unwrap()
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
"#,
    );
    write(&root.join("pub/schema.yaml"), SCHEMA);
    write(
        &root.join("pub/config/item.json"),
        r#"{
  "Item": [
    { "id": 1, "name": "Sword" }
  ]
}
"#,
    );
}

#[test]
fn signed_bundle_export_import_and_refusals() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_publisher(root);
    let reg = root.join("reg").to_str().unwrap().to_string();
    let pub_root = root.join("pub").to_str().unwrap().to_string();

    let out = run_cage_with_env(&["registry", "publish", &pub_root, "--registry", &reg], &[]);
    assert_code(&out, 0, "publish");

    // 1. keygen: the seed goes only to the named file (0600), stdout
    // carries the public key — the secret never appears in any output.
    let seed_file = root.join("signing-key.txt");
    let out = run_cage_with_env(
        &["registry", "keygen", "-o", seed_file.to_str().unwrap()],
        &[],
    );
    assert_code(&out, 0, "keygen");
    let keygen_out = stdout(&out);
    assert!(
        !keygen_out.contains(fs::read_to_string(&seed_file).unwrap().trim()),
        "the seed must never be printed: {keygen_out}"
    );
    assert!(keygen_out.contains("public key:"), "{keygen_out}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&seed_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "seed file is owner-only");
    }
    let seed = fs::read_to_string(&seed_file).unwrap().trim().to_string();

    // The public key in the keygen output is exactly what the seed produces.
    let public = keygen_out
        .lines()
        .find_map(|l| l.strip_prefix("cage registry keygen: public key: "))
        .expect("keygen prints the public key")
        .to_string();

    // 2. export --sign: detached signature rides beside the bundle.
    let bundle = root.join("bundle.tar");
    let out = run_cage_with_env(
        &[
            "registry",
            "export",
            "common@0.1.0",
            "-o",
            bundle.to_str().unwrap(),
            "--sign",
            "--key-env",
            "CAGE_TEST_SIGNING_KEY",
            "--registry",
            &reg,
        ],
        &[("CAGE_TEST_SIGNING_KEY", &seed)],
    );
    assert_code(&out, 0, "export --sign");
    assert!(stdout(&out).contains("signed bundle"), "{}", stdout(&out));
    let sig_path = root.join("bundle.tar.sig");
    let sidecar = fs::read_to_string(&sig_path).unwrap();
    assert!(
        sidecar.contains("\"ed25519\"") && sidecar.contains(&public),
        "sidecar names the algorithm and the signing key's public half:\n{sidecar}"
    );

    // 3. import --verify-sig: the trusted key (public half) gates entry.
    let out = run_cage_with_env(
        &[
            "registry",
            "import",
            bundle.to_str().unwrap(),
            "--verify-sig",
            "--key-env",
            "CAGE_TEST_VERIFYING_KEY",
            "--registry",
            &reg,
            "--dry-run",
        ],
        &[("CAGE_TEST_VERIFYING_KEY", &public)],
    );
    assert_code(&out, 0, "import --verify-sig (dry run)");
    assert!(
        stdout(&out).contains("signature verified"),
        "{}",
        stdout(&out)
    );

    // 4. Tampered bundle: the signature no longer verifies (E2107).
    let tampered_path = root.join("tampered.tar");
    let mut bytes = fs::read(&bundle).unwrap();
    bytes[0] ^= 0xFF;
    fs::write(&tampered_path, &bytes).unwrap();
    let out = run_cage_with_env(
        &[
            "registry",
            "import",
            tampered_path.to_str().unwrap(),
            "--verify-sig",
            "--key-env",
            "CAGE_TEST_VERIFYING_KEY",
            "--registry",
            &reg,
            "--dry-run",
        ],
        &[("CAGE_TEST_VERIFYING_KEY", &public)],
    );
    assert_code(&out, 1, "tampered bundle refuses");
    assert!(stderr(&out).contains("E2107"), "{}", stderr(&out));

    // 5. Missing sidecar: E2107 (the bundle cannot be attributed).
    let bare = root.join("bare.tar");
    fs::write(&bare, fs::read(&bundle).unwrap()).unwrap();
    let out = run_cage_with_env(
        &[
            "registry",
            "import",
            bare.to_str().unwrap(),
            "--verify-sig",
            "--key-env",
            "CAGE_TEST_VERIFYING_KEY",
            "--registry",
            &reg,
            "--dry-run",
        ],
        &[("CAGE_TEST_VERIFYING_KEY", &public)],
    );
    assert_code(&out, 1, "missing sidecar refuses");
    assert!(stderr(&out).contains("E2107"), "{}", stderr(&out));

    // 6. A bundle signed by a different key is refused under the trusted key.
    let out = run_cage_with_env(
        &[
            "registry",
            "keygen",
            "-o",
            root.join("other-key.txt").to_str().unwrap(),
        ],
        &[],
    );
    assert_code(&out, 0, "second keygen");
    let other_seed = fs::read_to_string(root.join("other-key.txt"))
        .unwrap()
        .trim()
        .to_string();
    let other_bundle = root.join("other.tar");
    let out = run_cage_with_env(
        &[
            "registry",
            "export",
            "common@0.1.0",
            "-o",
            other_bundle.to_str().unwrap(),
            "--sign",
            "--key-env",
            "CAGE_TEST_SIGNING_KEY",
            "--registry",
            &reg,
        ],
        &[("CAGE_TEST_SIGNING_KEY", &other_seed)],
    );
    assert_code(&out, 0, "export signed with the other key");
    let out = run_cage_with_env(
        &[
            "registry",
            "import",
            other_bundle.to_str().unwrap(),
            "--verify-sig",
            "--key-env",
            "CAGE_TEST_VERIFYING_KEY",
            "--registry",
            &reg,
            "--dry-run",
        ],
        &[("CAGE_TEST_VERIFYING_KEY", &public)],
    );
    assert_code(&out, 1, "foreign-key bundle refuses");
    assert!(stderr(&out).contains("E2107"), "{}", stderr(&out));

    // 7. Bad key material in the env: E2106 before any verification runs.
    let out = run_cage_with_env(
        &[
            "registry",
            "import",
            bundle.to_str().unwrap(),
            "--verify-sig",
            "--key-env",
            "CAGE_TEST_VERIFYING_KEY",
            "--registry",
            &reg,
            "--dry-run",
        ],
        &[("CAGE_TEST_VERIFYING_KEY", "not base64 ***")],
    );
    assert_code(&out, 1, "bad key material refuses");
    assert!(stderr(&out).contains("E2106"), "{}", stderr(&out));
}
