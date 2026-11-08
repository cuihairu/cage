//! Process-level coverage for the subcommand surfaces the other suites
//! miss: `snapshot --list`, the `web` command end to end (a real server
//! process serving both faces over loopback), `graph`/`assets`/`verify`
//! error branches, `verify` on an environment-tagged build, and the
//! `schema-draft` refusal paths (a workbook with no meta rows, an
//! unwritable `-o` target).

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

const CAGE_TOML: &str = r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "surfaces"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/json"
file_template = "{table}.json"
"#;

const ITEM_SCHEMA: &str = r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
enums: {}
"#;

const ITEM_DATA: &str = r#"{"Item": [{"id": 1, "name": "Sword"}]}"#;

/// Minimal valid project like cli_paths writes: config + schema + source.
fn write_project(root: &Path) {
    write(root, "cage.toml", CAGE_TOML);
    write(root, "schemas/item.yaml", ITEM_SCHEMA);
    write(root, "config/Item.json", ITEM_DATA);
}

// ------------------------------------------------------------ snapshot ---

/// `snapshot --list` inventories the packed snapshots under the output
/// directory — the command half of the pack/list pair the web API covers.
#[test]
fn snapshot_list_reports_packed_snapshots() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let empty = run_cage(&["snapshot", root.to_str().unwrap(), "--list"]);
    assert_code(&empty, 0, "list with no packs");
    assert!(
        stdout(&empty).contains("no snapshots"),
        "empty listing:\n{}",
        stdout(&empty)
    );

    let out = run_cage(&["snapshot", root.to_str().unwrap(), "--profile", "client"]);
    assert_code(&out, 0, "pack");
    assert!(stdout(&out).contains("verified"), "{}", stdout(&out));

    let out = run_cage(&["snapshot", root.to_str().unwrap(), "--list"]);
    assert_code(&out, 0, "list after pack");
    let body = stdout(&out);
    assert!(
        body.contains("client"),
        "listing names the profile:\n{body}"
    );
}

// ----------------------------------------------------------------- web ---

/// A root without cage.toml is refused before any socket is bound.
#[test]
fn web_command_refuses_a_root_without_config() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run_cage(&["web", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "web without cage.toml");
    let err = stderr(&out);
    assert!(err.contains("cage web:"), "stderr:\n{err}");
}

/// Push one raw HTTP request at the running server and return the reply.
/// The very first request can land while the serve loop is still spinning
/// up (the banner precedes the accept loop): the connection layer then
/// answers with a bare 500 that is not the route's verdict, or nothing at
/// all. The faces answer 200 when healthy, so only a 200 ends the retry;
/// anything else is kept as `last` for the assertion message.
fn http_get(port: u16, path: &str) -> String {
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    let mut last = String::new();
    for _ in 0..25 {
        if let Ok(mut client) = TcpStream::connect(("127.0.0.1", port)) {
            client
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            if client.write_all(request.as_bytes()).is_err() {
                continue;
            }
            let mut reply = Vec::new();
            if client.read_to_end(&mut reply).is_ok() && !reply.is_empty() {
                let reply = String::from_utf8_lossy(&reply).into_owned();
                if reply.starts_with("HTTP/1.1 200") {
                    return reply;
                }
                last = reply;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    last
}

/// The real `cage web` process serves the editor face, its assets, and the
/// status face over loopback — the serve loop itself, not just `handle`.
#[test]
fn web_command_serves_the_faces_until_killed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    // Bind, then release: the port is free but observable.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut child = Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(["web", root.to_str().unwrap(), "--port", &port.to_string()])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn cage web");
    // The banner pipe must stay open for the child's whole lifetime: the
    // server prints a second line ("Ctrl+C to stop") after the banner, and
    // a closed read end would EPIPE-panic the child mid-startup.
    let mut banner_pipe = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut banner = String::new();
    banner_pipe.read_line(&mut banner).expect("banner read");
    assert!(
        banner.contains(&format!("http://127.0.0.1:{port}")),
        "banner names the bound port: {banner}"
    );

    let index = http_get(port, "/");
    assert!(index.starts_with("HTTP/1.1 200"), "index head:\n{index}");
    assert!(index.contains("html"), "editor page body:\n{index}");

    let script = http_get(port, "/app.js");
    assert!(script.starts_with("HTTP/1.1 200"), "app.js head:\n{script}");
    let status_face = http_get(port, "/status/");
    assert!(
        status_face.starts_with("HTTP/1.1 200"),
        "status face head:\n{status_face}"
    );

    let api = http_get(port, "/api/status");
    assert!(api.starts_with("HTTP/1.1 200"), "status api head:\n{api}");
    assert!(api.contains("\"ok\""), "status api body:\n{api}");

    child.kill().expect("kill serve loop");
    child.wait().expect("reap");
    drop(banner_pipe);
}

// --------------------------------------------------------------- graph ---

/// A schema that fails to load is an exit-2 usage error, not a panic.
#[test]
fn graph_bad_schema_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    fs::write(root.join("schemas/item.yaml"), "tables: [unterminated").unwrap();
    let out = run_cage(&["graph", root.to_str().unwrap()]);
    assert_code(&out, 2, "graph with broken schema");
    let err = stderr(&out);
    assert!(err.contains("error:"), "stderr:\n{err}");
}

// --------------------------------------------------------------- assets ---

/// `assets` fails closed: one unreadable source file is a hard error, and
/// `--write`/`--check` each report their own I/O refusals.
#[test]
fn assets_reports_write_and_check_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    // --write onto an existing directory: fs::write refuses → exit 2.
    fs::create_dir_all(root.join("baseline.json")).unwrap();
    let out = run_cage(&[
        "assets",
        root.to_str().unwrap(),
        "--write",
        root.join("baseline.json").to_str().unwrap(),
    ]);
    assert_code(&out, 2, "--write onto a directory");
    let err = stderr(&out);
    assert!(err.contains("cannot write"), "stderr:\n{err}");

    // --check with a parseable baseline but no cage.toml in the root →
    // exit 2 (the config is loaded after the baseline parses).
    let bare = tempfile::tempdir().unwrap();
    let baseline = bare.path().join("baseline.json");
    fs::write(
        &baseline,
        r#"{"project":"surfaces","cage_version":"0.0.0","assets":[]}"#,
    )
    .unwrap();
    let empty_root = tempfile::tempdir().unwrap();
    let out = run_cage(&[
        "assets",
        empty_root.path().to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&out, 2, "--check without a project config");

    // --check against the real project with an empty ledger: every current
    // asset is unlisted → exit 1 with the problems listed.
    let out = run_cage(&[
        "assets",
        root.to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let body = stdout(&out);
    assert!(body.contains("problem(s)"), "report:\n{body}");
    assert!(
        body.contains("unlisted"),
        "report names unlisted files:\n{body}"
    );

    // A clean write + check round trip exits 0.
    let baseline = root.join("ledger.json");
    let out = run_cage(&[
        "assets",
        root.to_str().unwrap(),
        "--write",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&out, 0, "--write baseline");
    let out = run_cage(&[
        "assets",
        root.to_str().unwrap(),
        "--check",
        baseline.to_str().unwrap(),
    ]);
    assert_code(&out, 0, "--check clean");
    assert!(
        stdout(&out).contains("asset(s) verified"),
        "{}",
        stdout(&out)
    );

    // One unreadable source file fails the inventory outright (fail closed).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let secret = root.join("config/secret.json");
        fs::write(&secret, r#"{"Item": [{"id": 2}]}"#).unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        let out = run_cage(&["assets", root.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

// --------------------------------------------------------------- verify ---

/// `verify` in a project root without a build hints at `cage build`; after
/// a build it audits the ledger — tampered bytes and unreadable artifacts
/// are named with their verdicts.
#[test]
fn verify_hints_build_first_and_flags_tampering() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    // No manifest yet → exit 2 with the build-first hint.
    let out = run_cage(&["verify", root.to_str().unwrap()]);
    assert_code(&out, 2, "verify before any build");
    let err = stderr(&out);
    assert!(err.contains("hint: build first"), "stderr:\n{err}");

    let build = run_cage(&["build", root.to_str().unwrap()]);
    assert!(
        build.status.success(),
        "{}{}",
        stdout(&build),
        stderr(&build)
    );

    // Same-length byte swap → hash mismatch, exit 1.
    let artifact = root.join("build/json/Item.json");
    let original = fs::read(&artifact).unwrap();
    let mut tampered = original.clone();
    let last = tampered.len() - 1;
    tampered[last] = if tampered[last] == b'}' { b'!' } else { b'}' };
    fs::write(&artifact, &tampered).unwrap();
    let out = run_cage(&["verify", root.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let body = stdout(&out);
    assert!(body.contains("hash "), "mismatch report:\n{body}");
    assert!(body.contains("rebuild"), "rebuild advice:\n{body}");

    // Restore content but make the file unreadable → reported missing.
    fs::write(&artifact, &original).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o000)).unwrap();
        let out = run_cage(&["verify", root.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
        assert!(
            stdout(&out).contains("FAILED"),
            "unreadable artifact is flagged:\n{}",
            stdout(&out)
        );
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o644)).unwrap();
    }

    // Clean again → exit 0.
    let out = run_cage(&["verify", root.to_str().unwrap()]);
    assert_code(&out, 0, "clean verify");
    assert!(stdout(&out).contains("0 problems"), "{}", stdout(&out));
}

/// An environment-tagged build names its environment in the verify report.
#[test]
fn verify_names_the_environment_of_the_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    write(
        root,
        "schemas/item.yaml",
        r#"tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true }
      name: { name: name, type: { kind: String } }
    env_overrides:
      prod:
        name:
          required: true
enums: {}
"#,
    );

    let out = run_cage(&[
        "build",
        root.to_str().unwrap(),
        "--profile",
        "client",
        "--env",
        "prod",
    ]);
    assert_code(&out, 0, "env build");

    let out = run_cage(&["verify", root.to_str().unwrap()]);
    assert_code(&out, 0, "verify env build");
    let body = stdout(&out);
    assert!(
        body.contains("environment: prod"),
        "verify names the environment:\n{body}"
    );
}

// --------------------------------------------------------- schema-draft ---

/// A workbook without convention meta rows decodes no metadata: the draft
/// refuses with E2201 instead of emitting an empty schema.
#[test]
fn schema_draft_excel_without_meta_rows_is_e2201() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cage-source-excel/tests/fixtures/merged_cells.xlsx");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::copy(&fixture, root.join("config/Plain.xlsx")).unwrap();
    write(
        root,
        "cage.toml",
        r###"[project]
name = "drafter"

[source_roots]
main = "config"

[source.excel]
preset = "marker_rows"
marker = "##"

[profiles.client]
name = "client"
targets = []
"###,
    );

    let out = run_cage(&[
        "schema-draft",
        root.to_str().unwrap(),
        "excel:config/Plain.xlsx",
    ]);
    assert_code(&out, 2, "no meta rows");
    let err = stderr(&out);
    assert!(err.contains("E2201"), "stderr:\n{err}");
    assert!(err.contains("Plain.xlsx"), "names the file:\n{err}");
}

/// `-o` onto an existing directory is a write failure, exit 2 — the parent
/// creation succeeds, the final write does not.
#[test]
fn schema_draft_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema_draft/Items.xlsx");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::copy(&fixture, root.join("config/Items.xlsx")).unwrap();
    write(
        root,
        "cage.toml",
        r###"[project]
name = "drafter"

[source_roots]
main = "config"

[source.excel]
preset = "marker_rows"
marker = "##"

[profiles.client]
name = "client"
targets = []
"###,
    );
    fs::create_dir_all(root.join("drafts/out.yaml")).unwrap();

    let out = run_cage(&[
        "schema-draft",
        root.to_str().unwrap(),
        "excel:config/Items.xlsx",
        "-o",
        root.join("drafts/out.yaml").to_str().unwrap(),
    ]);
    assert_code(&out, 2, "-o onto a directory");
    let err = stderr(&out);
    assert!(err.contains("cannot write"), "stderr:\n{err}");
}
