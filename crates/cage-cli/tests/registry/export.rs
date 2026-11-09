//! `cage registry export` tests: byte-deterministic tar output, the
//! zstd compressed roundtrip, and unknown-format rejection.

use super::*;
use std::fs;

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
