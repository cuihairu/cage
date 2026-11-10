//! Registry behavior tests.

use super::signing::base64_decode_32;
use super::version::render_req;
use super::*;
use std::fs;
use std::io::Cursor;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Build a tiny self-verifying snapshot directory on the fly. The
/// ledger's `content_hash` derives from `content`, so different contents
/// really conflict and equal contents really dedupe.
fn make_snapshot(dir: &Path, content: &str) {
    make_snapshot_with_build(dir, content, "b111111111111111111111111");
}

/// Same, with an explicit `build_id` — the second axis of pack
/// identity (same data, different schema/environment).
fn make_snapshot_with_build(dir: &Path, content: &str, build_id: &str) {
    let artifacts = vec![(
        "build/client/json/Item.json".to_string(),
        content.as_bytes().to_vec(),
        "json".to_string(),
        Some("Item".to_string()),
    )];
    let content_hash = blake3::hash(content.as_bytes()).to_hex()[..24].to_string();
    let files = crate::snapshot::snapshot_files(
        br#"{"generator_version":"1.0.0"}"#,
        br#"{"tables":{},"enums":{},"metadata":null}"#,
        &artifacts,
        "build",
        build_id,
        &content_hash,
    );
    for (rel, bytes) in &files {
        let abs = dir.join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, bytes).unwrap();
    }
}

#[test]
fn version_order_is_dotted_numeric() {
    assert!(version_key("1.9") < version_key("1.10"));
    assert!(version_key("1.0.0") > version_key("1.0"));
    // A label suffix sorts after the bare release (shorter-first rule).
    assert!(version_key("1.0.0") < version_key("1.0.0-beta"));
    assert!(version_key("1.0.0-beta2") > version_key("1.0.0-beta"));
    assert_eq!(
        latest_version(&[
            "1.0.0".to_string(),
            "1.9.0".to_string(),
            "1.10.0".to_string()
        ]),
        Some("1.10.0")
    );
    // same-key tie ("1.0" vs "1.0.0") breaks on the raw string
    assert_eq!(
        latest_version(&["1.0.0".to_string(), "1.0".to_string()]),
        Some("1.0.0")
    );
}

#[test]
fn parse_spec_accepts_bare_and_pinned_versions() {
    let (p, v) = parse_spec("registry:common").unwrap();
    assert_eq!(p, "common");
    assert_eq!(v, None);
    let (p, v) = parse_spec("registry:common@1.2.0").unwrap();
    assert_eq!(p, "common");
    assert_eq!(v.as_deref(), Some("1.2.0"));
}

#[test]
fn parse_spec_rejects_path_escape_attempts() {
    // Only the registry core enforces this — consumers render these as
    // E1802 resolution errors, never as paths.
    assert!(parse_spec("registry:../secret").is_err());
    assert!(parse_spec("registry:a/b").is_err());
    assert!(parse_spec("registry:ok@1.0/../../x").is_err());
    assert!(parse_spec("registry:ok@..").is_err());
    assert!(parse_spec("registry:").is_err());
    assert!(parse_spec("plainroot").is_err());
}

#[test]
fn publish_resolves_latest_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");

    let r = publish(&root, "common", "1.0.0", &snap).unwrap();
    assert!(!r.already_identical);
    assert_eq!(r.entry_path, "common/1.0.0");
    assert!(root.join("common/1.0.0/HASHES.json").is_file());
    assert!(root
        .join("common/1.0.0/data/client/json/Item.json")
        .is_file());

    // Re-publish identical bytes (same version = same content_hash) → no-op.
    let r2 = publish(&root, "common", "1.0.0", &snap).unwrap();
    assert!(r2.already_identical);

    // Publish 1.9.0 and 1.10.0 with different content → latest is 1.10.0.
    let snap2 = tmp.path().join("snap2");
    make_snapshot(&snap2, "two");
    publish(&root, "common", "1.9.0", &snap2).unwrap();
    publish(&root, "common", "1.10.0", &snap2).unwrap();

    let dir = resolve(&root, "common", None).unwrap();
    assert!(dir.ends_with("1.10.0"));
    let dir = resolve(&root, "common", Some("1.0.0")).unwrap();
    assert!(dir.ends_with("1.0.0"));

    // Index is deterministic: sorted ascending, stable parse.
    let index = read_index(&root, "common").unwrap();
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .collect::<Vec<_>>(),
        ["1.0.0", "1.9.0", "1.10.0"]
    );
}

#[test]
fn export_bundle_is_deterministic_tar_with_entry_files_and_excerpt() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&root, "common", "1.0.0", &snap).unwrap();
    let snap2 = tmp.path().join("snap2");
    make_snapshot(&snap2, "two");
    publish(&root, "common", "1.9.0", &snap2).unwrap();

    // Two exports of the same entry are byte-identical.
    let out1 = tmp.path().join("b1.tar");
    let report = export_bundle(
        &root,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &out1,
    )
    .unwrap();
    assert_eq!(report.package, "common");
    assert_eq!(report.version, "1.0.0");
    let out2 = tmp.path().join("b2.tar");
    export_bundle(
        &root,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &out2,
    )
    .unwrap();
    assert_eq!(fs::read(&out1).unwrap(), fs::read(&out2).unwrap());

    // A missing version picks the dotted-numeric latest (1.9.0).
    let out3 = tmp.path().join("b3.tar");
    let latest = export_bundle(&root, "common", None, BundleCompression::Plain, &out3).unwrap();
    assert_eq!(latest.version, "1.9.0");

    // The bundle parses as a tar carrying entry files + index excerpt,
    // with the determinism contract visible in the headers.
    let mut archive = tar::Archive::new(fs::File::open(&out1).unwrap());
    let mut names = Vec::new();
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        names.push(entry.path().unwrap().to_string_lossy().into_owned());
        assert_eq!(entry.header().mtime().unwrap(), 0, "mtime must be zero");
        assert_eq!(entry.header().uid().unwrap(), 0);
        assert_eq!(entry.header().gid().unwrap(), 0);
        assert_eq!(entry.header().mode().unwrap(), 0o644);
    }
    names.sort();
    assert_eq!(
        names,
        vec![
            "common/1.0.0/HASHES.json".to_string(),
            "common/1.0.0/data/client/json/Item.json".to_string(),
            "common/1.0.0/manifest.json".to_string(),
            "common/1.0.0/schema.json".to_string(),
            "index.json".to_string(),
        ]
    );
    // The excerpt records exactly the exported entry.
    let mut ar2 = tar::Archive::new(fs::File::open(&out1).unwrap());
    for entry in ar2.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap() == std::path::Path::new("index.json") {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            let excerpt: RegistryIndex = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(excerpt.package, "common");
            assert_eq!(excerpt.entries.len(), 1);
            assert_eq!(excerpt.entries[0].version, "1.0.0");
        }
    }
}

#[test]
fn export_bundle_zstd_is_byte_deterministic_and_wraps_the_plain_tar() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&root, "common", "1.0.0", &snap).unwrap();

    let plain = tmp.path().join("plain.tar");
    export_bundle(
        &root,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &plain,
    )
    .unwrap();
    let z1 = tmp.path().join("z1.tar.zst");
    let z2 = tmp.path().join("z2.tar.zst");
    export_bundle(&root, "common", Some("1.0.0"), BundleCompression::Zstd, &z1).unwrap();
    export_bundle(&root, "common", Some("1.0.0"), BundleCompression::Zstd, &z2).unwrap();

    // Fixed-level determinism: two exports of the same entry land
    // byte-identical container bytes (within the pinned zstd library —
    // the same scope as the plain-tar golden contract).
    assert_eq!(
        fs::read(&z1).unwrap(),
        fs::read(&z2).unwrap(),
        "fixed level must compress identically every run"
    );

    // The container wraps exactly the plain tar: frame magic at the
    // head, and the decoded payload is byte-identical to the plain
    // export of the same entry.
    let wrapped = fs::read(&z1).unwrap();
    assert!(wrapped.starts_with(&ZSTD_MAGIC), "zstd frame magic");
    assert_ne!(wrapped, fs::read(&plain).unwrap(), "wrapped, not plain");
    let payload = zstd::stream::decode_all(Cursor::new(&wrapped[..])).unwrap();
    assert_eq!(payload, fs::read(&plain).unwrap());
}

#[test]
fn import_bundle_sniffs_zstd_container_and_gates_on_decompressed_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();
    let plain = tmp.path().join("plain.tar");
    export_bundle(
        &source,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &plain,
    )
    .unwrap();
    let wrapped = tmp.path().join("wrapped.tar.zst");
    export_bundle(
        &source,
        "common",
        Some("1.0.0"),
        BundleCompression::Zstd,
        &wrapped,
    )
    .unwrap();

    // The compressed bundle imports through the same trust gate: verify
    // clean, resolve, and entry bytes matching the plain form — the
    // ledger hashes uncompressed content, so the container never shows.
    let target = tmp.path().join("regB");
    let report = import_bundle(&target, &wrapped, false).unwrap();
    assert_eq!(report.package, "common");
    assert_eq!(report.version, "1.0.0");
    assert!(!report.already_identical);
    let audit = verify_registry(&target).unwrap();
    assert!(audit.ok(), "{:?}", audit.problems);
    let dir = resolve(&target, "common", Some("1.0.0")).unwrap();
    let source_artifact = source.join("common/1.0.0/data/client/json/Item.json");
    assert_eq!(
        fs::read(dir.join("data/client/json/Item.json")).unwrap(),
        fs::read(source_artifact).unwrap()
    );

    // Dry run of a compressed bundle: full gate, nothing written.
    let dry_target = tmp.path().join("regC");
    let report = import_bundle(&dry_target, &wrapped, true).unwrap();
    assert!(report.dry_run);
    assert!(!dry_target.join("common").exists());

    // Content-based sniff, extension-blind: the compressed form under a
    // plain `.tar` name and the plain form under a `.zst` name both
    // import — the file head, not the suffix, decides.
    let lying_tar = tmp.path().join("actually-zstd.tar");
    fs::copy(&wrapped, &lying_tar).unwrap();
    let lying_zst = tmp.path().join("actually-plain.tar.zst");
    fs::copy(&plain, &lying_zst).unwrap();
    let target_tar = tmp.path().join("regD");
    let report = import_bundle(&target_tar, &lying_tar, false).unwrap();
    assert_eq!(report.version, "1.0.0");
    let target_zst = tmp.path().join("regE");
    let report = import_bundle(&target_zst, &lying_zst, false).unwrap();
    assert_eq!(report.version, "1.0.0");

    // A head claiming the zstd frame but not decoding is a refused
    // bundle (E2103), not a panic and not a silent plain-tar parse.
    let mut corrupt = fs::read(&wrapped).unwrap();
    corrupt.truncate(8);
    let corrupt_path = tmp.path().join("corrupt.tar.zst");
    fs::write(&corrupt_path, &corrupt).unwrap();
    let err = import_bundle(&tmp.path().join("regF"), &corrupt_path, false).unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::distribution::E2103),
        "{err}"
    );
}

#[test]
fn export_bundle_missing_entry_or_bad_name_is_e2101() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&root, "common", "1.0.0", &snap).unwrap();
    let out = tmp.path().join("x.tar");
    let e2101 = crate::error::codes::distribution::E2101;

    // Unknown package / unknown version / path-escape version.
    let err = export_bundle(&root, "ghost", None, BundleCompression::Plain, &out).unwrap_err();
    assert!(err.starts_with(e2101), "{err}");
    let err = export_bundle(
        &root,
        "common",
        Some("9.9.9"),
        BundleCompression::Plain,
        &out,
    )
    .unwrap_err();
    assert!(err.starts_with(e2101), "{err}");
    let err = export_bundle(
        &root,
        "common",
        Some("../evil"),
        BundleCompression::Plain,
        &out,
    )
    .unwrap_err();
    assert!(err.starts_with(e2101), "{err}");
    let err = export_bundle(&root, "../evil", None, BundleCompression::Plain, &out).unwrap_err();
    assert!(err.starts_with(e2101), "{err}");

    // An entry stripped of its ledger is not exportable.
    let version_dir = root.join("common/1.0.0");
    fs::remove_file(version_dir.join(LEDGER_FILE)).unwrap();
    let err = export_bundle(
        &root,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &out,
    )
    .unwrap_err();
    assert!(err.starts_with(e2101), "{err}");
}

/// Rebuild a bundle with one member's bytes replaced (test tampering:
/// the content file or the index excerpt) — same deterministic headers.
fn repack_with_member(src: &Path, dst: &Path, member_suffix: &str, new_bytes: &[u8]) {
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
fn import_bundle_enters_verified_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();
    let bundle = tmp.path().join("b.tar");
    export_bundle(
        &source,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &bundle,
    )
    .unwrap();

    let target = tmp.path().join("regB");
    let report = import_bundle(&target, &bundle, false).unwrap();
    assert_eq!(report.package, "common");
    assert_eq!(report.version, "1.0.0");
    assert!(!report.already_identical);
    assert!(!report.dry_run);

    // The imported entry is a first-class citizen: full-registry audit
    // clean, resolve works, bytes match the source entry.
    let audit = verify_registry(&target).unwrap();
    assert!(audit.problems.is_empty(), "{:?}", audit.problems);
    let dir = resolve(&target, "common", Some("1.0.0")).unwrap();
    assert!(dir.ends_with("common/1.0.0"));
    let artifact = fs::read(dir.join("data/client/json/Item.json")).unwrap();
    assert_eq!(artifact, b"one".to_vec());

    // Re-importing the same bundle is an idempotent no-op.
    let again = import_bundle(&target, &bundle, false).unwrap();
    assert!(again.already_identical);
}

#[test]
fn import_bundle_refuses_tamper_conflict_and_respects_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();
    let bundle = tmp.path().join("b.tar");
    export_bundle(
        &source,
        "common",
        Some("1.0.0"),
        BundleCompression::Plain,
        &bundle,
    )
    .unwrap();
    let e2103 = crate::error::codes::distribution::E2103;

    // Tampered entry bytes fail the riding ledger at the trust gate.
    let tampered = tmp.path().join("tampered.tar");
    repack_with_member(&bundle, &tampered, "Item.json", b"hacked");
    let target = tmp.path().join("regB");
    let err = import_bundle(&target, &tampered, false).unwrap_err();
    assert!(err.starts_with(e2103), "{err}");
    assert!(
        !target.join("common").exists(),
        "refused bytes must not touch the registry"
    );

    // An index excerpt doctored to claim another content_hash also
    // fails (excerpt ⇔ ledger cross-check) — the JSON itself is valid,
    // so only the cross-check can catch this one.
    let doctored = tmp.path().join("doctored.tar");
    let data = fs::read(&bundle).unwrap();
    let mut ar = tar::Archive::new(&data[..]);
    let mut fake_excerpt_bytes = Vec::new();
    for e in ar.entries().unwrap() {
        let mut e = e.unwrap();
        if e.path().unwrap() == std::path::Path::new(INDEX_FILE) {
            std::io::Read::read_to_end(&mut e, &mut fake_excerpt_bytes).unwrap();
        }
    }
    let mut fake: RegistryIndex = serde_json::from_slice(&fake_excerpt_bytes).unwrap();
    fake.entries[0].content_hash = "0".repeat(24);
    let mut fake_bytes = serde_json::to_vec_pretty(&fake).unwrap();
    fake_bytes.push(b'\n');
    repack_with_member(&bundle, &doctored, INDEX_FILE, &fake_bytes);
    let err = import_bundle(&target, &doctored, false).unwrap_err();
    assert!(err.starts_with(e2103), "{err}");

    // Same version, different bytes already in the target → E1801.
    let other = tmp.path().join("snap2");
    make_snapshot(&other, "two");
    publish(&target, "common", "1.0.0", &other).unwrap();
    let err = import_bundle(&target, &bundle, false).unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::registry::E1801),
        "{err}"
    );

    // Dry run reports without writing: a fresh target stays empty even
    // though the real import would succeed.
    let dry_target = tmp.path().join("regC");
    let report = import_bundle(&dry_target, &bundle, true).unwrap();
    assert!(report.dry_run);
    assert!(!report.already_identical);
    assert!(!dry_target.join("common").exists());
}

/// A stand-in remote registry for `push_entry`: stores PUT bodies,
/// serves GETs from the store, records (method, path, auth header) per
/// request, and can be forced to answer a fixed status for everything
/// (401 = auth rejection, 405 = no write channel). `std::net` only.
struct PushServer {
    port: u16,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PushServer {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn shutdown(self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.handle.map(std::thread::JoinHandle::join);
    }
}

type PushServerState = (
    PushServer,
    Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    Arc<Mutex<Vec<String>>>,
);

/// A stand-in that refuses every write with `force_status` (reads still
/// serve from the store).
fn start_push_server(force_status: Option<u16>) -> PushServerState {
    start_push_server_inner(force_status, None)
}

/// A stand-in that refuses every request — reads included — with
/// `force_status`: a registry that demands credentials even for the
/// anonymous state probe (the probe-rejection path).
fn start_push_server_refusing_reads(force_status: u16) -> PushServerState {
    start_push_server_inner(None, Some(force_status))
}

fn start_push_server_inner(force_put: Option<u16>, force_get: Option<u16>) -> PushServerState {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    let store: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let store2 = store.clone();
    let requests2 = requests.clone();
    let handle = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !stop2.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            // Windows accepts inherit the listener's non-blocking mode;
            // restore blocking so the request read waits.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            // Read the head, then exactly Content-Length body bytes.
            let mut buf = Vec::new();
            // Large heap read chunk: the >1 MiB head guard admits
            // oversized uploads, and a 4 KiB step would rescan the
            // buffer per read.
            let mut chunk = vec![0u8; 256 * 1024];
            let head_end = loop {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break buf.len(),
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        // Size check first: a >1 MiB head is junk either
                        // way, and scanning its windows per read would
                        // make the truncation quadratic.
                        if buf.len() > 1024 * 1024 {
                            break buf.len();
                        }
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break pos + 4;
                        }
                    }
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end.min(buf.len())]).into_owned();
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap_or("");
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("/").to_string();
            let mut content_length = 0usize;
            let mut auth = String::new();
            for line in lines {
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    auth = line["authorization:".len()..].trim().to_string();
                }
            }
            let mut body = buf[head_end.min(buf.len())..].to_vec();
            while body.len() < content_length {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                }
            }
            requests2
                .lock()
                .unwrap()
                .push(format!("{method} {path} {auth}"));
            let (status, reason, payload): (u16, &str, Vec<u8>) =
                match (force_put, force_get, method.as_str()) {
                    // GETs serve from the store unless the read side is
                    // forced too — a static host reads fine even when it
                    // refuses writes.
                    (_, Some(code), "GET") | (Some(code), _, "PUT") => {
                        (code, "Refused", b"refused".to_vec())
                    }
                    (_, None, "GET") => match store2.lock().unwrap().get(&path) {
                        Some(bytes) => (200, "OK", bytes.clone()),
                        None => (404, "Not Found", b"not found".to_vec()),
                    },
                    (None, _, "PUT") => {
                        store2.lock().unwrap().insert(path.clone(), body);
                        (200, "OK", Vec::new())
                    }
                    _ => (405, "Method Not Allowed", b"method".to_vec()),
                };
            let head = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&payload);
            let _ = stream.flush();
        }
    });
    (
        PushServer {
            port,
            stop,
            handle: Some(handle),
        },
        store,
        requests,
    )
}

fn put_count(requests: &[String]) -> usize {
    requests.iter().filter(|r| r.starts_with("PUT ")).count()
}

/// The stand-in tolerates raw misbehaving clients: a truncated head, an
/// oversized head without terminator, and a body shorter than its
/// Content-Length all end their read loops without panicking the server
/// thread; a non-GET/PUT method answers 405.
#[test]
fn push_server_tolerates_misbehaving_clients() {
    use std::io::{Read, Write};
    use std::net::Shutdown;

    // Poll the reply until the response head is complete and its
    // Content-Length bytes arrived — the server may still be finishing
    // the previous client, and it never closes first.
    fn read_reply(mut c: &std::net::TcpStream) -> String {
        let _ = c.set_read_timeout(Some(std::time::Duration::from_millis(100)));
        let mut reply = Vec::new();
        let mut buf = [0u8; 4096];
        for _ in 0..60 {
            match c.read(&mut buf) {
                Ok(0) | Err(_) => {}
                Ok(n) => reply.extend_from_slice(&buf[..n]),
            }
            if let Some(head_end) = reply
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
            {
                let complete = reply[head_end..].len()
                    >= String::from_utf8_lossy(&reply)
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("Content-Length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                if complete {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&reply).into_owned()
    }

    let (server, _store, requests) = start_push_server(None);

    // Truncated head: EOF ends the head loop and the request still lands.
    let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    c.write_all(b"GET /half HTTP/1.1\r\n").unwrap();
    c.shutdown(Shutdown::Write).unwrap();
    let reply = read_reply(&c);
    assert!(
        reply.contains("404"),
        "truncated head still serves: {reply}"
    );
    drop(c);

    // Oversized head without terminator: the >1 MiB guard breaks the
    // loop (and the junk method answers 405).
    let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let junk = vec![b'a'; 1024 * 1024 + 16];
    c.write_all(&junk).unwrap();
    let reply = read_reply(&c);
    assert!(reply.contains("405"), "junk method is refused: {reply}");
    drop(c);

    // Body shorter than Content-Length: EOF ends the body read.
    let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    c.write_all(b"PUT /f HTTP/1.1\r\nContent-Length: 100\r\n\r\nshort")
        .unwrap();
    c.shutdown(Shutdown::Write).unwrap();
    let reply = read_reply(&c);
    assert!(reply.contains("200"), "short body still stores: {reply}");
    drop(c);

    // Body split across writes: the body loop keeps reading until
    // Content-Length bytes arrived.
    let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    c.write_all(b"PUT /g HTTP/1.1\r\nContent-Length: 100\r\n\r\nshort")
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    c.write_all(&[b'x'; 95]).unwrap();
    let reply = read_reply(&c);
    assert!(reply.contains("200"), "split body still stores: {reply}");
    drop(c);

    // Non-GET/PUT method: 405.
    let mut c = std::net::TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    c.write_all(b"DELETE /x HTTP/1.1\r\nHost: t\r\n\r\n")
        .unwrap();
    let reply = read_reply(&c);
    assert!(reply.contains("405"), "non-GET/PUT is refused: {reply}");
    drop(c);

    server.shutdown();
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 5, "every client must be tracked: {seen:?}");
    assert!(seen.iter().any(|r| r.starts_with("GET /half ")), "{seen:?}");
    assert!(seen.iter().any(|r| r.starts_with("PUT /f ")), "{seen:?}");
    assert!(seen.iter().any(|r| r.starts_with("DELETE /x ")), "{seen:?}");
}

#[test]
fn push_entry_uploads_files_then_merged_index() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();
    let snap2 = tmp.path().join("snap2");
    make_snapshot(&snap2, "two");
    publish(&source, "common", "0.2.0", &snap2).unwrap();

    let (server, store, requests) = start_push_server(None);
    std::env::set_var("CAGE_TOK_B", "tok-b");
    let report = push_entry(
        &source,
        &server.url(),
        "common",
        Some("0.2.0"),
        Some("CAGE_TOK_B"),
        false,
    )
    .unwrap();
    assert_eq!(report.package, "common");
    assert_eq!(report.version, "0.2.0");
    assert!(!report.already_identical);
    assert!(!report.dry_run);
    assert_eq!(report.files, 4);

    // Order: index GET first (anonymous — the read protocol is), entry
    // files next, the package index PUT dead last; the token rides only
    // the PUTs.
    let log = requests.lock().unwrap().clone();
    assert_eq!(log[0], "GET /common/index.json ");
    assert_eq!(log.last().unwrap(), "PUT /common/index.json Bearer tok-b");
    assert_eq!(put_count(&log), 5, "4 entry files + merged index");
    assert!(log
        .iter()
        .filter(|r| r.starts_with("PUT "))
        .all(|r| r.ends_with("Bearer tok-b")));

    // The merged index holds exactly the pushed entry (remote was
    // empty), and a second push of another version merges over it.
    let stored = store
        .lock()
        .unwrap()
        .get("/common/index.json")
        .cloned()
        .unwrap();
    let index: RegistryIndex = serde_json::from_slice(&stored).unwrap();
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .collect::<Vec<_>>(),
        ["0.2.0"]
    );
    push_entry(
        &source,
        &server.url(),
        "common",
        Some("1.0.0"),
        Some("CAGE_TOK_B"),
        false,
    )
    .unwrap();
    let stored = store
        .lock()
        .unwrap()
        .get("/common/index.json")
        .cloned()
        .unwrap();
    let index: RegistryIndex = serde_json::from_slice(&stored).unwrap();
    // dotted-numeric order: 0.2.0 sorts before 1.0.0
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .collect::<Vec<_>>(),
        ["0.2.0", "1.0.0"]
    );

    // Re-pushing identical bytes is a zero-PUT no-op (one GET for the
    // remote state, nothing else).
    let before = requests.lock().unwrap().len();
    let again = push_entry(
        &source,
        &server.url(),
        "common",
        Some("0.2.0"),
        Some("CAGE_TOK_B"),
        false,
    )
    .unwrap();
    assert!(again.already_identical);
    let log = requests.lock().unwrap().clone();
    assert_eq!(log.len(), before + 1);
    assert_eq!(put_count(&log[before..]), 0);
    server.shutdown();
    std::env::remove_var("CAGE_TOK_B");
}

#[test]
fn push_entry_maps_auth_refusals_and_missing_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    // 401 from the remote → E2102, credentials riding every request.
    let (server, _store, requests) = start_push_server(Some(401));
    std::env::set_var("CAGE_TOK_A", "sekrit-token");
    let err = push_entry(
        &source,
        &server.url(),
        "common",
        Some("1.0.0"),
        Some("CAGE_TOK_A"),
        false,
    )
    .unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::distribution::E2102),
        "{err}"
    );
    assert!(
        !err.contains("sekrit-token"),
        "the token value never leaks into errors: {err}"
    );
    // Anonymous state probe first, then the token rides the refused PUT;
    // the push fails fast — exactly one PUT attempted.
    let log = requests.lock().unwrap().clone();
    assert_eq!(log[0], "GET /common/index.json ");
    assert_eq!(put_count(&log), 1);
    assert!(log[1..]
        .iter()
        .all(|r| r.starts_with("PUT ") && r.ends_with("Bearer sekrit-token")));
    server.shutdown();
    std::env::remove_var("CAGE_TOK_A");

    // 405 → E2104: the server has no write channel.
    let (server, _store, _requests) = start_push_server(Some(405));
    let err = push_entry(&source, &server.url(), "common", Some("1.0.0"), None, false).unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::distribution::E2104),
        "{err}"
    );
    server.shutdown();

    // auth_env unset → E2105 before any network contact: the target is
    // a closed port, so any connection attempt would surface as E2101.
    let err = push_entry(
        &source,
        "http://127.0.0.1:1",
        "common",
        Some("1.0.0"),
        Some("CAGE_TOK_MISSING"),
        false,
    )
    .unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::distribution::E2105),
        "{err}"
    );
    // An empty token is as good as absent.
    std::env::set_var("CAGE_TOK_EMPTY", "");
    let err = push_entry(
        &source,
        "http://127.0.0.1:1",
        "common",
        Some("1.0.0"),
        Some("CAGE_TOK_EMPTY"),
        false,
    )
    .unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::distribution::E2105),
        "{err}"
    );
    std::env::remove_var("CAGE_TOK_EMPTY");
}

#[test]
fn push_entry_conflict_refuses_and_dry_run_uploads_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    // Dry run: the remote state check happens, nothing is stored.
    let (server, store, requests) = start_push_server(None);
    let report = push_entry(&source, &server.url(), "common", Some("1.0.0"), None, true).unwrap();
    assert!(report.dry_run);
    assert!(!report.already_identical);
    assert!(store.lock().unwrap().is_empty(), "dry run stores nothing");
    assert_eq!(put_count(&requests.lock().unwrap()), 0);
    server.shutdown();

    // Same version, different bytes already on the remote → E1801.
    let (server, store, _requests) = start_push_server(None);
    push_entry(&source, &server.url(), "common", Some("1.0.0"), None, false).unwrap();
    let other = tmp.path().join("snap2");
    make_snapshot(&other, "hacked");
    let conflicting = tmp.path().join("regB");
    publish(&conflicting, "common", "1.0.0", &other).unwrap();
    let err = push_entry(
        &conflicting,
        &server.url(),
        "common",
        Some("1.0.0"),
        None,
        false,
    )
    .unwrap_err();
    assert!(
        err.starts_with(crate::error::codes::registry::E1801),
        "{err}"
    );
    // The remote index was not rewritten by the refused push.
    let index: RegistryIndex =
        serde_json::from_slice(store.lock().unwrap().get("/common/index.json").unwrap()).unwrap();
    assert_eq!(index.entries.len(), 1);
    assert_ne!(index.entries[0].content_hash, "");
    server.shutdown();
}

/// A presign map pointing at the stand-in server, covering exactly
/// the published entry's files (keys carry a leading `/` on purpose —
/// normalization must tolerate it). `with_get` controls whether the
/// map knows about an existing remote index.
fn presign_map_for(
    base: &str,
    package: &str,
    source: &Path,
    version: Option<&str>,
    with_get: bool,
) -> (PresignMap, String) {
    let (version, _dir, rels) = resolve_entry(source, package, version).unwrap();
    let mut uploads = BTreeMap::new();
    for rel in &rels {
        let key = format!("/{package}/{version}/{}", rel.to_string_lossy());
        uploads.insert(
            key,
            format!("{base}/{package}/{version}/{}", rel.to_string_lossy()),
        );
    }
    let map = PresignMap {
        uploads,
        index_get: with_get.then(|| format!("{base}/{package}/index.json")),
        index_put: format!("{base}/{package}/index.json"),
    };
    (map, version)
}

#[test]
fn parse_presign_map_accepts_and_enforces_the_shape() {
    let ok = parse_presign_map(
        br#"{"uploads": {"/cfg/1.0.0/manifest.json": "https://s3/put-one"},
                 "index": {"get": "https://s3/get-index", "put": "https://s3/put-index"}}"#,
    )
    .unwrap();
    // Leading `/` normalized away on keys.
    assert_eq!(
        ok.uploads
            .get("cfg/1.0.0/manifest.json")
            .map(String::as_str),
        Some("https://s3/put-one")
    );
    assert_eq!(ok.index_get.as_deref(), Some("https://s3/get-index"));
    assert_eq!(ok.index_put, "https://s3/put-index");

    // index.get is optional (fresh package, nothing to merge yet).
    let fresh =
        parse_presign_map(br#"{"uploads": {}, "index": {"put": "https://s3/put-index"}}"#).unwrap();
    assert_eq!(fresh.index_get, None);
    assert!(fresh.uploads.is_empty());

    // Missing index object → E2101.
    let err = parse_presign_map(br#"{"uploads": {}}"#).unwrap_err();
    assert!(err.starts_with(E2101), "{err}");
    // Missing index.put → E2101 (a push without an index write is
    // invisible to consumers).
    let err =
        parse_presign_map(br#"{"uploads": {}, "index": {"get": "https://s3/get"}}"#).unwrap_err();
    assert!(err.starts_with(E2101) && err.contains("index.put"), "{err}");
    // Any non-absolute URL (bare path, ftp) → E2101.
    for bad in [
        br#"{"uploads": {"cfg/1.0.0/f": "cfg/1.0.0/f"}, "index": {"put": "https://x"}}"#.as_slice(),
        br#"{"uploads": {}, "index": {"put": "ftp://s3/put"}}"#,
    ] {
        let err = parse_presign_map(bad).unwrap_err();
        assert!(err.starts_with(E2101), "{err}");
    }
    // Malformed JSON → E2101.
    let err = parse_presign_map(b"{not json").unwrap_err();
    assert!(err.starts_with(E2101), "{err}");
}

#[test]
fn push_entry_presigned_follows_the_same_discipline_without_a_token() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    let (server, store, requests) = start_push_server(None);
    let (map, version) = presign_map_for(&server.url(), "common", &source, Some("1.0.0"), true);
    assert_eq!(version, "1.0.0");
    let report = push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap();
    assert_eq!(report.files, 4);
    assert!(!report.already_identical);

    // Order preserved: index GET first, entry files, merged index PUT
    // last; no request carries an Authorization header — the URLs are
    // the grant, no bearer token is read or sent.
    let log = requests.lock().unwrap().clone();
    assert_eq!(log[0], "GET /common/index.json ");
    assert_eq!(log.last().unwrap(), "PUT /common/index.json ");
    assert_eq!(put_count(&log), 5, "4 entry files + merged index");
    assert!(
        log.iter().all(|r| !r.ends_with("Bearer")),
        "presigned requests must stay anonymous: {log:?}"
    );

    // The stored merged index holds the pushed entry.
    let stored = store
        .lock()
        .unwrap()
        .get("/common/index.json")
        .cloned()
        .unwrap();
    let index: RegistryIndex = serde_json::from_slice(&stored).unwrap();
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .collect::<Vec<_>>(),
        ["1.0.0"]
    );

    // A second push of another version merges over the remote index
    // read through the presigned GET.
    let snap2 = tmp.path().join("snap2");
    make_snapshot(&snap2, "two");
    publish(&source, "common", "0.2.0", &snap2).unwrap();
    let (map2, _) = presign_map_for(&server.url(), "common", &source, Some("0.2.0"), true);
    push_entry_presigned(&source, &map2, "common", Some("0.2.0"), false).unwrap();
    let stored = store
        .lock()
        .unwrap()
        .get("/common/index.json")
        .cloned()
        .unwrap();
    let index: RegistryIndex = serde_json::from_slice(&stored).unwrap();
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .collect::<Vec<_>>(),
        ["0.2.0", "1.0.0"]
    );

    // Re-pushing identical bytes through presigned URLs is the same
    // zero-PUT no-op: one GET, nothing stored twice.
    let before = requests.lock().unwrap().len();
    let again = push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap();
    assert!(again.already_identical);
    let log = requests.lock().unwrap().clone();
    assert_eq!(log.len(), before + 1);
    assert_eq!(put_count(&log[before..]), 0);
    server.shutdown();
}

#[test]
fn push_entry_presigned_fresh_package_skips_the_get_and_conflicts_through_it() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    // No index.get in the map = fresh package: the push goes straight
    // to the PUTs (the remote index is treated as empty).
    let (server, _store, requests) = start_push_server(None);
    let (map, _) = presign_map_for(&server.url(), "common", &source, Some("1.0.0"), false);
    push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap();
    let log = requests.lock().unwrap().clone();
    assert!(
        !log.iter().any(|r| r.starts_with("GET ")),
        "no index_get means no probe GET: {log:?}"
    );
    assert_eq!(put_count(&log), 5);
    server.shutdown();

    // The conflict guard survives the presigned route: same version,
    // different bytes already on the remote (read via the presigned
    // GET) → E1801, nothing uploaded.
    let (server, store, requests) = start_push_server(None);
    push_entry(&source, &server.url(), "common", Some("1.0.0"), None, false).unwrap();
    let other = tmp.path().join("snap2");
    make_snapshot(&other, "hacked");
    let conflicting = tmp.path().join("regB");
    publish(&conflicting, "common", "1.0.0", &other).unwrap();
    let (bad_map, _) = presign_map_for(&server.url(), "common", &conflicting, Some("1.0.0"), true);
    let err =
        push_entry_presigned(&conflicting, &bad_map, "common", Some("1.0.0"), false).unwrap_err();
    assert!(err.starts_with(E1801), "{err}");
    let log = requests.lock().unwrap().clone();
    assert_eq!(
        put_count(&log),
        5,
        "the seeding push's 5 PUTs only — the refused push uploaded nothing"
    );
    let index: RegistryIndex =
        serde_json::from_slice(store.lock().unwrap().get("/common/index.json").unwrap()).unwrap();
    assert_eq!(index.entries.len(), 1, "the refused push changed nothing");
    server.shutdown();
}

#[test]
fn push_entry_presigned_coverage_gap_and_dry_run_upload_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    // A map missing one entry file fails closed BEFORE any PUT — a
    // partial upload would leave the index pointing at nothing.
    let (server, store, requests) = start_push_server(None);
    let (map, version) = presign_map_for(&server.url(), "common", &source, Some("1.0.0"), true);
    let mut short = map.clone();
    let dropped = short.uploads.keys().next().cloned().unwrap();
    short.uploads.remove(&dropped);
    let err = push_entry_presigned(&source, &short, "common", Some("1.0.0"), false).unwrap_err();
    assert!(err.starts_with(E2101), "{err}");
    assert!(err.contains("nothing uploaded"), "{err}");
    assert!(err.contains(&dropped), "{err}");
    let log = requests.lock().unwrap().clone();
    assert_eq!(log[0], "GET /common/index.json ", "the probe still ran");
    assert_eq!(put_count(&log), 0, "coverage gap → zero PUTs");
    assert!(store.lock().unwrap().is_empty());
    assert_eq!(version, "1.0.0");

    // Dry run through the presigned route: state check + coverage
    // check pass, zero PUTs.
    let report = push_entry_presigned(&source, &map, "common", Some("1.0.0"), true).unwrap();
    assert!(report.dry_run);
    assert!(!report.already_identical);
    let log = requests.lock().unwrap().clone();
    assert_eq!(put_count(&log[1..]), 0, "dry run PUTs nothing");
    assert!(store.lock().unwrap().is_empty());
    server.shutdown();
}

/// The presigned route maps refusals through the same shared failure
/// mapping as the direct route (E2102 auth / E2104 no write channel),
/// and never carries a bearer token even when the server refuses.
#[test]
fn push_entry_presigned_maps_refusals_to_e2102_and_e2104() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("regA");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    // 401 on the writes: the presigned index probe (anonymous, served
    // from the empty store → 404) passes, the first PUT is refused.
    let (server, _store, requests) = start_push_server(Some(401));
    let (map, _) = presign_map_for(&server.url(), "common", &source, Some("1.0.0"), true);
    let err = push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap_err();
    assert!(err.starts_with(E2102), "{err}");
    let log = requests.lock().unwrap().clone();
    assert_eq!(put_count(&log), 1, "fail fast — one PUT attempted");
    assert!(
        log.iter().all(|r| !r.ends_with("Bearer")),
        "presigned requests stay anonymous even on refusal: {log:?}"
    );
    server.shutdown();

    // 405 on the writes → E2104 (the server has no write channel).
    let (server, _store, _requests) = start_push_server(Some(405));
    let (map, _) = presign_map_for(&server.url(), "common", &source, Some("1.0.0"), true);
    let err = push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap_err();
    assert!(err.starts_with(E2104), "{err}");
    server.shutdown();

    // A presigned probe that answers 401 is an auth rejection, not a
    // transport failure — mapped before the generic case, and nothing
    // is uploaded afterwards.
    let (server, _store, requests) = start_push_server_refusing_reads(401);
    let (map, _) = presign_map_for(&server.url(), "common", &source, Some("1.0.0"), true);
    let err = push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap_err();
    assert!(err.starts_with(E2102), "{err}");
    let log = requests.lock().unwrap().clone();
    assert_eq!(log.len(), 1, "the refused probe is the only request");
    assert_eq!(put_count(&log), 0);
    server.shutdown();

    // Probe unreachable (closed port) → E2101, and the message names the
    // presigned GET so an operator knows which URL to reissue.
    let (map, _) = presign_map_for("http://127.0.0.1:1", "common", &source, Some("1.0.0"), true);
    let err = push_entry_presigned(&source, &map, "common", Some("1.0.0"), false).unwrap_err();
    assert!(err.starts_with(E2101), "{err}");
    assert!(err.contains("presigned index GET"), "{err}");
}

#[test]
fn publish_same_version_different_bytes_is_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&root, "common", "1.0.0", &snap).unwrap();

    let snap2 = tmp.path().join("snap2");
    make_snapshot(&snap2, "two");
    let err = publish(&root, "common", "1.0.0", &snap2).unwrap_err();
    assert!(err.contains("E1801"), "{err}");
    assert!(err.contains("conflict"), "{err}");
}

/// Pack identity spans the whole pack: identical artifact bytes but a
/// different `build_id` (schema-only change, e.g. an environment-ized
/// repack) must conflict — a content_hash-only check would silently
/// no-op onto the old pack and the env bytes would never land.
#[test]
fn publish_same_artifacts_different_build_is_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot_with_build(&snap, "one", "baaaaaaaaaaaaaaaaaaaaaa1");
    let report = publish(&root, "common", "1.0.0", &snap).unwrap();
    assert!(!report.already_identical);

    let snap2 = tmp.path().join("snap2");
    make_snapshot_with_build(&snap2, "one", "baaaaaaaaaaaaaaaaaaaaaa2");
    let err = publish(&root, "common", "1.0.0", &snap2).unwrap_err();
    assert!(err.contains("E1801"), "{err}");
    assert!(err.contains("build_id"), "{err}");

    // Same build_id and content: the true no-op.
    let report = publish(&root, "common", "1.0.0", &snap).unwrap();
    assert!(report.already_identical);
}

#[test]
fn resolve_missing_package_or_version_is_e1802() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&root, "common", "1.0.0", &snap).unwrap();

    let err = resolve(&root, "ghost", None).unwrap_err();
    assert!(err.contains("E1802"), "{err}");
    assert!(err.contains("not found"), "{err}");

    let err = resolve(&root, "common", Some("99.0.0")).unwrap_err();
    assert!(err.contains("E1802"), "{err}");
    assert!(err.contains("99.0.0"), "{err}");
}

#[test]
fn resolve_tampered_entry_is_e1803() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    publish(&root, "common", "1.0.0", &snap).unwrap();

    // Tamper with the published data file — the ledger catches it.
    let data = root.join("common/1.0.0/data/client/json/Item.json");
    fs::write(&data, "tampered").unwrap();
    let err = resolve(&root, "common", None).unwrap_err();
    assert!(err.contains("E1803"), "{err}");
    assert!(err.contains("hash mismatch"), "{err}");
}

#[test]
fn publish_refuses_unverifiable_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    // Break the ledger: the snapshot is no longer self-verifying.
    fs::write(snap.join("HASHES.json"), "{}").unwrap();
    let err = publish(&root, "common", "1.0.0", &snap).unwrap_err();
    assert!(err.contains("E1803"), "{err}");
    assert!(!root.join("common").exists(), "nothing may be written");
}

fn comparators(spec: &str) -> Vec<(CmpOp, String)> {
    parse_version_req(spec)
        .unwrap()
        .comparators
        .into_iter()
        .map(|c| (c.op, c.version))
        .collect()
}

fn req(spec: &str) -> VersionReq {
    parse_version_req(spec).unwrap()
}

#[test]
fn version_req_expands_caret_and_tilde() {
    use CmpOp::{Eq, Ge, Gt, Le, Lt};
    // A bare spec is exact; explicit operators pass through.
    assert_eq!(comparators("1.2.3"), [(Eq, "1.2.3".to_string())]);
    assert_eq!(
        comparators(">=1.0, <2.0"),
        [(Ge, "1.0".to_string()), (Lt, "2.0".to_string())]
    );
    assert_eq!(comparators(">1.2"), [(Gt, "1.2".to_string())]);
    assert_eq!(comparators("<=1.2"), [(Le, "1.2".to_string())]);
    // Caret: bump the left-most non-zero, everything after drops to zero.
    assert_eq!(
        comparators("^1.2"),
        [(Ge, "1.2".to_string()), (Lt, "2".to_string())]
    );
    assert_eq!(
        comparators("^0.2.3"),
        [(Ge, "0.2.3".to_string()), (Lt, "0.3".to_string())]
    );
    assert_eq!(
        comparators("^0.0.3"),
        [(Ge, "0.0.3".to_string()), (Lt, "0.0.4".to_string())]
    );
    // Tilde: lock every component but the second-to-last given.
    assert_eq!(
        comparators("~1.2"),
        [(Ge, "1.2".to_string()), (Lt, "1.3".to_string())]
    );
    assert_eq!(
        comparators("~1"),
        [(Ge, "1.0".to_string()), (Lt, "2".to_string())]
    );
    assert_eq!(
        comparators("~1.2.3"),
        [(Ge, "1.2.3".to_string()), (Lt, "1.3".to_string())]
    );
}

#[test]
fn version_req_satisfies_uses_zero_padded_order() {
    let caret = req("^1.2");
    assert!(satisfies("1.2.0", &caret));
    assert!(satisfies("1.9.0", &caret));
    assert!(satisfies("1.10.0", &caret)); // dotted-numeric: 1.10 > 1.9
    assert!(!satisfies("1.1.0", &caret));
    assert!(!satisfies("2.0.0", &caret));

    let zero_caret = req("^0.2.3");
    assert!(satisfies("0.2.3", &zero_caret));
    assert!(satisfies("0.2.9", &zero_caret));
    assert!(!satisfies("0.3.0", &zero_caret));

    let tilde = req("~1.2");
    assert!(satisfies("1.2.0", &tilde));
    assert!(satisfies("1.2.7", &tilde));
    assert!(!satisfies("1.3.0", &tilde));

    let exact = req("1.2.3");
    assert!(satisfies("1.2.3", &exact));
    assert!(!satisfies("1.2.30", &exact));

    // >=1.2 must not exclude 1.2.0 (comparisons zero-pad).
    assert!(satisfies("1.2.0", &req(">=1.2")));
    assert!(satisfies("1.2.0", &req("<=1.2")));
    assert!(satisfies("2.0.0", &req(">1.9, <=2.0")));
    assert!(!satisfies("2.0.1", &req(">1.9, <=2.0")));
}

#[test]
fn version_req_rejects_malformed_specs() {
    for spec in ["", "   ", "abc", ">= 1.0,", "^x", "~", "1..0"] {
        let err = parse_version_req(spec).unwrap_err();
        assert!(err.contains("E1802"), "{spec}: {err}");
    }
}

#[test]
fn resolve_pinned_picks_max_satisfying_and_gates_explicit_pins() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "one");
    for v in ["1.0.0", "1.9.0", "2.0.0"] {
        publish(&root, "common", v, &snap).unwrap();
    }

    // Versionless + requirement → highest satisfying version.
    let dir = resolve_pinned(&root, "common", None, Some(&req(">=1.0, <2.0"))).unwrap();
    assert!(dir.ends_with("1.9.0"));
    let dir = resolve_pinned(&root, "common", None, Some(&req("^1.0.0"))).unwrap();
    assert!(dir.ends_with("1.9.0"));
    let dir = resolve_pinned(&root, "common", None, Some(&req("1.0.0"))).unwrap();
    assert!(dir.ends_with("1.0.0"));
    // Explicit version still inside the requirement passes.
    let dir = resolve_pinned(&root, "common", Some("1.0.0"), Some(&req("^1.0"))).unwrap();
    assert!(dir.ends_with("1.0.0"));

    // Explicit version outside the requirement → E1802.
    let err = resolve_pinned(&root, "common", Some("2.0.0"), Some(&req("<2.0"))).unwrap_err();
    assert!(err.contains("E1802"), "{err}");
    assert!(err.contains("does not satisfy"), "{err}");

    // Requirement nothing satisfies → E1802 with the published list.
    let err = resolve_pinned(&root, "common", None, Some(&req(">=3.0"))).unwrap_err();
    assert!(err.contains("E1802"), "{err}");
    assert!(err.contains("satisfies requirement"), "{err}");
    assert!(err.contains("2.0.0"), "{err}");

    // No pin, no requirement → plain latest (unchanged R1 behavior).
    let dir = resolve_pinned(&root, "common", None, None).unwrap();
    assert!(dir.ends_with("2.0.0"));
}

#[test]
fn select_version_is_pure_and_drives_the_remote_fetcher() {
    let index = RegistryIndex {
        package: "common".to_string(),
        entries: vec![
            IndexEntry {
                version: "1.0.0".to_string(),
                build_id: "b".to_string(),
                content_hash: "c".to_string(),
                files: 3,
            },
            IndexEntry {
                version: "1.9.0".to_string(),
                build_id: "b".to_string(),
                content_hash: "c".to_string(),
                files: 3,
            },
            IndexEntry {
                version: "2.0.0".to_string(),
                build_id: "b".to_string(),
                content_hash: "c".to_string(),
                files: 3,
            },
        ],
    };
    // Same decision table as the local resolver — the remote fetcher
    // (R3) runs this exact function on the fetched index.
    assert_eq!(
        select_version(&index, None, Some(&req(">=1.0, <2.0")), "https://r").unwrap(),
        "1.9.0"
    );
    assert_eq!(
        select_version(&index, None, None, "https://r").unwrap(),
        "2.0.0"
    );
    assert_eq!(
        select_version(&index, Some("1.0.0"), None, "https://r").unwrap(),
        "1.0.0"
    );
    let err = select_version(&index, Some("2.0.0"), Some(&req("<2.0")), "https://r").unwrap_err();
    assert!(err.contains("E1802"), "{err}");
    assert!(err.contains("does not satisfy"), "{err}");
    let err = select_version(&index, None, Some(&req(">=3.0")), "https://r").unwrap_err();
    assert!(
        err.contains("E1802") && err.contains("satisfies requirement"),
        "{err}"
    );
    let empty = RegistryIndex {
        package: "ghost".to_string(),
        entries: Vec::new(),
    };
    let err = select_version(&empty, None, None, "https://r").unwrap_err();
    assert!(err.contains("not found"), "{err}");

    // Cache keys are deterministic and short enough for a path segment.
    assert_eq!(
        cache_key("https://r/example"),
        cache_key("https://r/example")
    );
    assert_ne!(cache_key("https://r/a"), cache_key("https://r/b"));
    assert_eq!(cache_key("https://r/example").len(), 12);
}

#[test]
fn verify_gc_and_remove_govern_the_registry_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("reg");

    // Three versions of one package.
    for (v, content) in [("1.0.0", "one"), ("1.9.0", "two"), ("2.0.0", "three")] {
        let snap = tmp.path().join("snap");
        if snap.exists() {
            fs::remove_dir_all(&snap).unwrap();
        }
        make_snapshot(&snap, content);
        publish(&root, "common", v, &snap).unwrap();
    }

    // Clean registry verifies clean.
    let report = verify_registry(&root).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(report.packages, 1);
    assert_eq!(report.entries_checked, 3);

    // A hand-edit inside an entry breaks its byte hashes; the index
    // record now disagrees with the (self-inconsistent) ledger.
    fs::write(
        root.join("common/1.0.0/data/client/json/Item.json"),
        "forged",
    )
    .unwrap();
    let report = verify_registry(&root).unwrap();
    assert!(!report.ok(), "tampered entry must be reported");
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("E1803") && p.contains("common/1.0.0")),
        "{:?}",
        report.problems
    );

    // gc keeps the newest window (default surfaced as 3; here keep 2)
    // and sweeps an orphan directory with no index record. The dry run
    // computes the identical removal list first and touches nothing.
    fs::create_dir_all(root.join("common/0.5.0")).unwrap();
    let plan = gc_registry(&root, 2, true).unwrap();
    assert_eq!(
        plan.removed,
        vec![
            "common/1.0.0".to_string(),
            "common/0.5.0 (orphan)".to_string()
        ]
    );
    assert_eq!(plan.rewritten, 1);
    assert!(root.join("common/1.0.0").is_dir());
    assert!(root.join("common/0.5.0").is_dir());
    let report = gc_registry(&root, 2, false).unwrap();
    assert_eq!(
        report.removed,
        vec![
            "common/1.0.0".to_string(),
            "common/0.5.0 (orphan)".to_string()
        ]
    );
    assert_eq!(report.rewritten, 1);
    let index = read_index(&root, "common").unwrap();
    let versions: Vec<String> = index.entries.iter().map(|e| e.version.clone()).collect();
    assert_eq!(versions, vec!["1.9.0", "2.0.0"]);
    assert!(root.join("common/1.9.0").is_dir());
    assert!(!root.join("common/1.0.0").exists());

    // keep is floored at one — a registry never loses its whole history.
    gc_registry(&root, 0, false).unwrap();
    assert!(root.join("common/2.0.0").is_dir());
    let index = read_index(&root, "common").unwrap();
    assert_eq!(index.entries.len(), 1);

    // The GC'd window is really gone for consumers.
    let err = resolve(&root, "common", Some("1.0.0")).unwrap_err();
    assert!(err.contains("E1802"), "{err}");

    // Explicit remove takes one version out; the package index stays
    // (empty), and the same snapshot republishes cleanly afterwards —
    // removal is explicit history editing, re-publication is not a
    // conflict.
    remove_entry(&root, "common", "2.0.0", false).unwrap();
    let index = read_index(&root, "common").unwrap();
    assert_eq!(index.entries.len(), 0);
    assert!(!root.join("common/2.0.0").exists());
    let report = verify_registry(&root).unwrap();
    assert!(report.ok(), "{:?}", report.problems);

    let err = remove_entry(&root, "common", "9.9.9", false).unwrap_err();
    assert!(err.contains("E1802"), "{err}");
    let err = remove_entry(&root, "../escape", "1.0.0", false).unwrap_err();
    assert!(err.contains("E1801"), "{err}");

    let snap = tmp.path().join("snap2");
    make_snapshot(&snap, "three");
    publish(&root, "common", "2.0.0", &snap).unwrap();
    let index = read_index(&root, "common").unwrap();
    assert_eq!(index.entries.len(), 1);
}

// --- A6 signed bundles ------------------------------------------------

fn keypair_from_seed_bytes(seed: [u8; 32]) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&seed)
}

#[test]
fn sign_verify_roundtrip_tamper_and_untrusted_key_refuse() {
    let key = generate_signing_key().unwrap();
    let (seed_b64, public_b64) = key_material(&key);
    let sig = sign_bundle_bytes(b"bundle-bytes", &key);
    assert_eq!(sig.algorithm, BUNDLE_SIG_ALGORITHM);
    // The sidecar advertises exactly the key that can verify.
    assert_eq!(sig.public_key, public_b64);
    // The seed decodes to 32 bytes (what --key-env must carry).
    assert_eq!(base64_decode_32(&seed_b64).unwrap(), key.to_bytes());

    verify_bundle_bytes(b"bundle-bytes", &sig, &key.verifying_key()).unwrap();

    // Any byte flip breaks attribution.
    let mut tampered = b"bundle-bytes".to_vec();
    tampered[0] ^= 1;
    let err = verify_bundle_bytes(&tampered, &sig, &key.verifying_key()).unwrap_err();
    assert!(err.contains("E2107"), "{err}");

    // A bundle from a different key is refused under the trusted key.
    let other = keypair_from_seed_bytes([9u8; 32]);
    let err = verify_bundle_bytes(b"bundle-bytes", &sig, &other.verifying_key()).unwrap_err();
    assert!(err.contains("E2107"), "{err}");

    // Unknown algorithm and non-64-byte signatures are E2107 too.
    let mut bad_algo = sig.clone();
    bad_algo.algorithm = "rsa".into();
    assert!(
        verify_bundle_bytes(b"bundle-bytes", &bad_algo, &key.verifying_key())
            .unwrap_err()
            .contains("E2107")
    );
    let mut bad_len = sig.clone();
    bad_len.signature = "AAAA".into();
    assert!(
        verify_bundle_bytes(b"bundle-bytes", &bad_len, &key.verifying_key())
            .unwrap_err()
            .contains("E2107")
    );
}

#[test]
fn key_env_resolves_seed_and_rejects_bad_material() {
    let key = generate_signing_key().unwrap();
    let (seed_b64, public_b64) = key_material(&key);

    std::env::set_var("CAGE_SEED_OK", &seed_b64);
    let parsed = signing_key_from_env("CAGE_SEED_OK").unwrap();
    assert_eq!(
        parsed.verifying_key().to_bytes(),
        key.verifying_key().to_bytes()
    );

    std::env::set_var("CAGE_PUB_OK", &public_b64);
    let trusted = verifying_key_from_env("CAGE_PUB_OK").unwrap();
    assert_eq!(trusted.to_bytes(), key.verifying_key().to_bytes());

    for (name, value) in [
        ("CAGE_KEY_EMPTY", ""),
        ("CAGE_KEY_NOTB64", "not base64 ***"),
        ("CAGE_KEY_SHORT", &seed_b64[..8]),
    ] {
        std::env::set_var(name, value);
        let err = signing_key_from_env(name).unwrap_err();
        assert!(err.contains("E2106"), "{name}: {err}");
    }
    let err = signing_key_from_env("CAGE_KEY_NEVER_SET_ANYWHERE").unwrap_err();
    assert!(err.contains("E2106"), "{err}");
}

#[test]
fn sidecar_roundtrips_and_missing_sidecar_is_e2107() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("bundle.tar");
    std::fs::write(&bundle, b"bundle-bytes").unwrap();
    let key = generate_signing_key().unwrap();
    let sig = sign_bundle_bytes(b"bundle-bytes", &key);
    let path = write_bundle_signature(&bundle, &sig).unwrap();
    assert_eq!(path, sidecar_path(&bundle));
    assert_eq!(path.file_name().unwrap(), "bundle.tar.sig");
    let read_back = read_bundle_signature(&bundle).unwrap();
    assert_eq!(read_back, sig);

    let err = read_bundle_signature(&tmp.path().join("absent.tar")).unwrap_err();
    assert!(err.contains("E2107"), "{err}");

    // Owner-only permissions on the written key file (unix).
    let key_file = tmp.path().join("seed.txt");
    std::fs::write(&key_file, format!("{}\n", key_material(&key).0)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mode = std::fs::metadata(&key_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// Assemble an in-memory tar bundle from named members — the raw
/// material for `import_bundle`'s malformed-member guards.
fn tar_bytes(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name.as_str(), data.as_slice())
            .unwrap();
    }
    builder.into_inner().unwrap()
}

#[test]
fn parse_version_req_covers_eq_caret_zero_and_tilde_label() {
    // the `=` comparator arm
    let req = parse_version_req("=1.2.3").unwrap();
    assert_eq!(render_req(&req), "=1.2.3");

    // an all-zero caret base bumps the last given component: ^0 → [0, 1)
    let req = parse_version_req("^0").unwrap();
    assert_eq!(render_req(&req), ">=0, <1");

    // a caret base carrying a label renders the label back into bounds
    let req = parse_version_req("^1.0-beta").unwrap();
    assert!(req.comparators[0].version.contains("beta"));

    // a tilde whose second-to-last slot is a label is a bad tilde version
    let err = parse_version_req("~beta").unwrap_err();
    assert!(err.contains("bad tilde version 'beta'"), "{err}");

    // the > and <= rendering arms
    let req = parse_version_req(">2.0,<=1.0").unwrap();
    assert_eq!(render_req(&req), ">2.0, <=1.0");
}

#[test]
fn read_index_rejects_unreadable_and_corrupt_index() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("common");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join(INDEX_FILE), b"not-json").unwrap();
    let err = read_index(tmp.path(), "common").unwrap_err();
    assert!(err.contains("corrupt registry index"), "{err}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(pkg.join(INDEX_FILE), fs::Permissions::from_mode(0o000)).unwrap();
        let err = read_index(tmp.path(), "common").unwrap_err();
        assert!(err.contains("cannot read registry index"), "{err}");
    }
}

#[test]
fn write_index_reports_create_and_write_failures() {
    let index = RegistryIndex {
        package: "common".to_string(),
        entries: Vec::new(),
    };
    // the package path occupied by a file blocks directory creation
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("common"), b"file").unwrap();
    let err = write_index(tmp.path(), &index).unwrap_err();
    assert!(err.contains("cannot create"), "{err}");

    // an index.json occupied by a directory fails the write itself
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("common").join(INDEX_FILE)).unwrap();
    let err = write_index(tmp.path(), &index).unwrap_err();
    assert!(err.contains("cannot write registry index"), "{err}");
}

#[test]
fn publish_rejects_bad_names_unverifiable_packs_and_blocked_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "alpha");

    // invalid package / version components
    let err = publish(tmp.path(), "bad/name", "1.0.0", &snap).unwrap_err();
    assert!(err.contains("invalid registry package name"), "{err}");
    let err = publish(tmp.path(), "common", "bad version", &snap).unwrap_err();
    assert!(err.contains("invalid registry version"), "{err}");
    // a lone `.` (or `-` / `_`) is a path no-op, not a package name.
    let err = publish(tmp.path(), ".", "1.0.0", &snap).unwrap_err();
    assert!(err.contains("invalid registry package name"), "{err}");
    let err = publish(tmp.path(), "common", ".", &snap).unwrap_err();
    assert!(err.contains("invalid registry version"), "{err}");
    let err = publish(tmp.path(), "-", "1.0.0", &snap).unwrap_err();
    assert!(err.contains("invalid registry package name"), "{err}");

    // a tampered pack fails the publish trust gate
    let snap2 = tmp.path().join("snap2");
    make_snapshot(&snap2, "beta");
    fs::write(snap2.join("data/client/json/Item.json"), b"tampered").unwrap();
    let err = publish(tmp.path(), "common", "1.0.0", &snap2).unwrap_err();
    assert!(err.contains("E1803"), "{err}");

    // the entry path occupied by a file fails the copy staging
    let root2 = tempfile::tempdir().unwrap();
    fs::create_dir_all(root2.path().join("common")).unwrap();
    fs::write(root2.path().join("common").join("1.0.0"), b"occupied").unwrap();
    let err = publish(root2.path(), "common", "1.0.0", &snap).unwrap_err();
    assert!(err.contains("cannot create"), "{err}");
}

#[test]
fn packages_lists_only_index_backed_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "alpha");
    publish(tmp.path(), "common", "1.0.0", &snap).unwrap();
    fs::create_dir_all(tmp.path().join("not-a-package")).unwrap();
    let names: Vec<String> = packages(tmp.path())
        .unwrap()
        .into_iter()
        .map(|i| i.package)
        .collect();
    assert_eq!(names, ["common"]);
}

#[test]
fn resolve_entry_reports_missing_entry_directory() {
    let tmp = tempfile::tempdir().unwrap();
    write_index(
        tmp.path(),
        &RegistryIndex {
            package: "common".to_string(),
            entries: vec![IndexEntry {
                version: "2.0.0".to_string(),
                build_id: "b2222222222222222222222".to_string(),
                content_hash: "c".to_string(),
                files: 1,
            }],
        },
    )
    .unwrap();
    let err = resolve_entry(tmp.path(), "common", Some("2.0.0")).unwrap_err();
    assert!(err.contains("entry directory missing"), "{err}");
}

#[test]
fn import_bundle_rejects_malformed_member_structures() {
    let tmp = tempfile::tempdir().unwrap();
    let index_empty = br#"{"package":"common","entries":[]}"#.to_vec();
    let entry = br#"{"package":"common","entries":[{"version":"1.0.0","build_id":"b","content_hash":"c","files":1}]}"#.to_vec();

    // a non-plain member path (`.` — the tar writer's builder refuses
    // `..` outright) is refused on import
    let bundle = tmp.path().join("escape.tar");
    fs::write(&bundle, tar_bytes(&[(".".to_string(), b"x".to_vec())])).unwrap();
    let err = import_bundle(tmp.path(), &bundle, false).unwrap_err();
    assert!(err.contains("unsafe bundle member path"), "{err}");

    // two riding index excerpts
    let bundle = tmp.path().join("dup.tar");
    fs::write(
        &bundle,
        tar_bytes(&[
            ("index.json".to_string(), index_empty.clone()),
            ("index.json".to_string(), index_empty.clone()),
        ]),
    )
    .unwrap();
    let err = import_bundle(tmp.path(), &bundle, false).unwrap_err();
    assert!(err.contains("duplicate index.json"), "{err}");

    // an excerpt without exactly one entry
    let bundle = tmp.path().join("empty.tar");
    fs::write(
        &bundle,
        tar_bytes(&[("index.json".to_string(), index_empty)]),
    )
    .unwrap();
    let err = import_bundle(tmp.path(), &bundle, false).unwrap_err();
    assert!(err.contains("exactly one entry, got 0"), "{err}");

    // a member outside the excerpt's package/version prefix
    let bundle = tmp.path().join("outside.tar");
    fs::write(
        &bundle,
        tar_bytes(&[
            ("index.json".to_string(), entry.clone()),
            ("other/1.0.0/x".to_string(), b"x".to_vec()),
        ]),
    )
    .unwrap();
    let err = import_bundle(tmp.path(), &bundle, false).unwrap_err();
    assert!(err.contains("is outside common/1.0.0/"), "{err}");

    // prefix honored but no riding ledger
    let bundle = tmp.path().join("noledger.tar");
    fs::write(
        &bundle,
        tar_bytes(&[
            ("index.json".to_string(), entry),
            ("common/1.0.0/data.json".to_string(), b"x".to_vec()),
        ]),
    )
    .unwrap();
    let err = import_bundle(tmp.path(), &bundle, false).unwrap_err();
    assert!(err.contains("bundle has no HASHES.json ledger"), "{err}");
}

#[test]
fn import_bundle_rejects_file_count_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "gamma");

    // members carry the real pack verbatim; the excerpt claims far too
    // many files — the trust gate passes, the count audit refuses
    let ledger: serde_json::Value =
        serde_json::from_slice(&fs::read(snap.join(LEDGER_FILE)).unwrap()).unwrap();
    let build_id = ledger["build_id"].as_str().unwrap();
    let content_hash = ledger["content_hash"].as_str().unwrap();
    let excerpt = format!(
        r#"{{"package":"common","entries":[{{"version":"1.0.0","build_id":"{build_id}","content_hash":"{content_hash}","files":99999}}]}}"#
    );
    let mut members = vec![("index.json".to_string(), excerpt.into_bytes())];
    let mut stack = vec![snap.clone()];
    while let Some(cur) = stack.pop() {
        for e in fs::read_dir(&cur).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(&snap)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                members.push((format!("common/1.0.0/{rel}"), fs::read(p).unwrap()));
            }
        }
    }
    let bundle = tmp.path().join("count.tar");
    fs::write(&bundle, tar_bytes(&members)).unwrap();
    let err = import_bundle(tmp.path(), &bundle, false).unwrap_err();
    assert!(
        err.contains("record claims 99999 files, ledger covers"),
        "{err}"
    );
}

#[test]
fn push_entry_requires_http_root_and_maps_transport_failure() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let tmp = tempfile::tempdir().unwrap();
    let err = push_entry(
        tmp.path(),
        "ftp://example.com/reg",
        "common",
        None,
        None,
        false,
    )
    .unwrap_err();
    assert!(err.contains("remote http(s) registry root"), "{err}");

    // a server that serves reads but black-holes PUTs exhausts the
    // transport retries → E2101 on the first entry file
    let source = tmp.path().join("reg");
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "delta");
    publish(&source, "common", "1.0.0", &snap).unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let handle = std::thread::spawn(move || {
        use std::io::{Read, Write};
        listener.set_nonblocking(true).unwrap();
        while !stop2.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            // Windows accepts inherit the listener's non-blocking mode;
            // restore blocking so the request read waits.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            // Read just the request head — the method decides the fate:
            // a GET gets a 404 (fresh remote), a PUT gets silence (the
            // socket closes under the in-flight request and ureq
            // reports a transport failure).
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            if buf.starts_with(b"GET ") {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        }
    });
    // A client that drops before sending a full head ends the head read
    // loop without panicking the server thread.
    drop(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap());
    let err = push_entry(
        &source,
        &format!("http://127.0.0.1:{port}"),
        "common",
        Some("1.0.0"),
        None,
        false,
    )
    .unwrap_err();
    assert!(
        err.contains("E2101") && err.contains("push transport failed"),
        "{err}"
    );
    stop.store(true, Ordering::SeqCst);
    handle.join().ok();
}

// Both scenario levers are unix-only: a 0o000 ledger and a non-UTF-8
// ghost directory — Windows can represent neither, so there is nothing
// to test there.
#[cfg(unix)]
#[test]
fn verify_registry_reports_unreadable_ledger_and_skips_unnameable_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "epsilon");
    publish(tmp.path(), "common", "1.0.0", &snap).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let entry = tmp.path().join("common").join("1.0.0");
        fs::set_permissions(entry.join(LEDGER_FILE), fs::Permissions::from_mode(0o000)).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let ghost = std::ffi::OsStr::from_bytes(b"ghost-\xff");
        fs::create_dir_all(tmp.path().join("common").join(ghost)).unwrap();
    }
    let report = verify_registry(tmp.path()).unwrap();
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("common/1.0.0") && p.contains("E1803")),
        "{report:?}"
    );
}

#[test]
fn verify_registry_clean_pass_ledger_mismatch_and_named_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "eta");
    publish(tmp.path(), "common", "1.0.0", &snap).unwrap();

    // a clean registry: the entry verifies and its ledger matches the
    // index record — checked, no problems
    let report = verify_registry(tmp.path()).unwrap();
    assert_eq!(report.packages, 1);
    assert_eq!(report.entries_checked, 1);
    assert_eq!(report.problems.len(), 0);

    // the pack stays valid but the index record now disagrees with the
    // ledger — flagged as a record mismatch
    let idx_path = tmp.path().join("common").join(INDEX_FILE);
    let mut index: RegistryIndex = serde_json::from_slice(&fs::read(&idx_path).unwrap()).unwrap();
    index.entries[0].content_hash = "deadbeef".to_string();
    fs::write(&idx_path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

    // and an entry directory on disk no index record names
    fs::create_dir_all(tmp.path().join("common").join("0.9.0")).unwrap();

    let report = verify_registry(tmp.path()).unwrap();
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("index record does not match ledger")),
        "{report:?}"
    );
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.contains("0.9.0: entry directory without index record")),
        "{report:?}"
    );
}

#[test]
fn gc_registry_reports_removals_and_skips_unchanged_packages() {
    let tmp = tempfile::tempdir().unwrap();
    for (v, tag) in [("0.1.0", "one"), ("0.2.0", "two"), ("1.0.0", "three")] {
        let snap = tmp.path().join(format!("snap-{v}"));
        make_snapshot(&snap, tag);
        publish(tmp.path(), "common", v, &snap).unwrap();
    }

    // keep 2: the oldest version is dropped, the index rewritten
    let report = gc_registry(tmp.path(), 2, false).unwrap();
    assert_eq!(report.removed, ["common/0.1.0"]);
    assert_eq!(report.rewritten, 1);
    assert!(!tmp.path().join("common").join("0.1.0").exists());
    let index = read_index(tmp.path(), "common").unwrap();
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.version.clone())
            .collect::<Vec<_>>(),
        ["0.2.0", "1.0.0"]
    );

    // a window that keeps everything changes nothing
    let report = gc_registry(tmp.path(), 8, false).unwrap();
    assert_eq!(report.removed.len(), 0);
    assert_eq!(report.rewritten, 0);
    assert!(tmp.path().join("common").join("0.2.0").exists());
}

#[test]
fn verifying_key_from_env_rejects_empty_and_malformed_values() {
    std::env::set_var("CAGE_TEST_VK_EMPTY", "   ");
    let err = verifying_key_from_env("CAGE_TEST_VK_EMPTY").unwrap_err();
    assert!(err.contains("is empty"), "{err}");

    std::env::set_var("CAGE_TEST_VK_BAD", "not-base64!!");
    let err = verifying_key_from_env("CAGE_TEST_VK_BAD").unwrap_err();
    assert!(err.contains("not base64 of a 32-byte key"), "{err}");

    std::env::remove_var("CAGE_TEST_VK_EMPTY");
    std::env::remove_var("CAGE_TEST_VK_BAD");
}

#[test]
fn remove_entry_rejects_bad_version_and_tolerates_missing_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let err = remove_entry(tmp.path(), "common", "1 0", false).unwrap_err();
    assert!(err.contains("invalid registry version"), "{err}");

    let snap = tmp.path().join("snap");
    make_snapshot(&snap, "zeta");
    publish(tmp.path(), "common", "1.0.0", &snap).unwrap();

    // a dry run validates but removes nothing
    remove_entry(tmp.path(), "common", "1.0.0", true).unwrap();
    assert!(tmp.path().join("common").join("1.0.0").exists());
    let index = read_index(tmp.path(), "common").unwrap();
    assert_eq!(index.entries.len(), 1);

    // the version directory vanished out from under the index — the
    // record is still removable
    fs::remove_dir_all(tmp.path().join("common").join("1.0.0")).unwrap();
    remove_entry(tmp.path(), "common", "1.0.0", false).unwrap();
    let index = read_index(tmp.path(), "common").unwrap();
    assert_eq!(index.entries.len(), 0);
}

#[test]
fn read_bundle_signature_rejects_malformed_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("b.tar");
    fs::write(&bundle, b"payload").unwrap();
    fs::write(sidecar_path(&bundle), b"{\"nope\":1}").unwrap();
    let err = read_bundle_signature(&bundle).unwrap_err();
    assert!(err.contains("malformed"), "{err}");
}
