use super::super::*;
use crate::error::codes::source_convention::{E2202, E2203, E2206, E2207};
// `E2204` / `E2205` / `Duration` only feed the unix-gated subprocess tests
// below; on Windows they would dangle unused.
#[cfg(unix)]
use crate::error::codes::source_convention::{E2204, E2205};
#[cfg(unix)]
use std::time::Duration;

/// `parse_response` gates every malformed script answer with the right code
/// (E2206 for shape, E2207 for identity, E2202 for vocabulary).
#[test]
fn parse_response_gates_malformed_answers() {
    let headers = ["id", "name"].map(String::from).to_vec();
    let ok =
        |v: serde_json::Value| parse_response(&v, "Skill", &headers, "config/Skill.xlsx", "Skill");

    let cases = [
        (serde_json::json!([1, 2]), E2206),
        (serde_json::json!({"header_row": 0, "columns": {}}), E2206),
        (serde_json::json!({"table": "Skill", "columns": {}}), E2206),
        (
            serde_json::json!({"table": "Skill", "header_row": 0}),
            E2206,
        ),
        (
            serde_json::json!({"table": "Skill", "header_row": 0, "columns": {}}),
            E2206,
        ),
    ];
    for (v, code) in cases {
        let err = ok(v).unwrap_err();
        assert!(
            err.errors().iter().any(|d| d.code == code),
            "{err:?} for {code}"
        );
    }

    let wrong_table = serde_json::json!({
        "table": "Other", "header_row": 0,
        "columns": {"id": {"cell": "A1", "type": {"kind": "Int32"}}}
    });
    assert!(ok(wrong_table)
        .unwrap_err()
        .errors()
        .iter()
        .any(|d| d.code == E2207));

    let bad_a1 = serde_json::json!({
        "table": "Skill", "header_row": 0,
        "columns": {"id": {"cell": "1A", "type": {"kind": "Int32"}}}
    });
    assert!(ok(bad_a1)
        .unwrap_err()
        .errors()
        .iter()
        .any(|d| d.code == E2206));

    let sub_cols = serde_json::json!({
        "table": "Skill", "header_row": 0,
        "columns": {"id": {"cell": "A1", "type": {"kind": "Int32"},
                    "sub_columns": {"x": {"cell": "B1"}}}}
    });
    assert!(ok(sub_cols)
        .unwrap_err()
        .errors()
        .iter()
        .any(|d| d.code == E2206));

    let multi = serde_json::json!({
        "table": "Skill", "header_row": 0,
        "columns": {"id": {"cell": "A1", "type": {"kind": "Int32"}, "multi_row": true}}
    });
    assert!(ok(multi)
        .unwrap_err()
        .errors()
        .iter()
        .any(|d| d.code == E2206));

    let not_in_header = serde_json::json!({
        "table": "Skill", "header_row": 0,
        "columns": {"ghost": {"cell": "A1", "type": {"kind": "Int32"}}}
    });
    assert!(ok(not_in_header)
        .unwrap_err()
        .errors()
        .iter()
        .any(|d| d.code == E2207));

    let invented = serde_json::json!({
        "table": "Skill", "header_row": 0,
        "columns": {"id": {"cell": "A1", "type": {"kind": "Int32"}, "note": "invented"}}
    });
    assert!(ok(invented)
        .unwrap_err()
        .errors()
        .iter()
        .any(|d| d.code == E2202));

    let good = serde_json::json!({
        "table": "Skill", "header_row": 0,
        "columns": {
            "id": {"cell": "A1", "type": {"kind": "Int32"}, "required": true},
            "name": {"cell": "B1", "type": {"kind": "String"}, "description": "武器名"}
        }
    });
    let meta = ok(good).expect("valid answer");
    assert_eq!(meta.header_row, 0);
    assert_eq!(meta.columns["id"].field_type, Some(FieldType::Int32));
    assert_eq!(meta.columns["id"].required, Some(true));
    assert_eq!(meta.columns["name"].description.as_deref(), Some("武器名"));
}

/// `resolve_script` refuses absolute paths, `.`/`..` segments, missing
/// files, symlinks escaping the root, and non-file targets — all E2203 —
/// and returns the canonical path for a good relative script.
#[test]
fn resolve_script_rejects_unsafe_paths() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(root.path().join("tools")).expect("tools");
    let script = root.path().join("tools/echo.py");
    std::fs::write(&script, "#!/bin/sh\necho ok\n").expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
    }

    let bad = [
        "/abs/meta.py",
        "tools/../tools/echo.py",
        "./tools/echo.py",
        "tools/absent.py",
        "tools",
    ];
    for rel in bad {
        let err = resolve_script(root.path(), rel).unwrap_err();
        assert!(
            err.errors().iter().any(|d| d.code == E2203),
            "{rel}: {err:?}"
        );
    }

    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("meta.py"), "#!/bin/sh\necho ok\n")
            .expect("outside script");
        let link = root.path().join("escape.py");
        std::os::unix::fs::symlink(outside.path().join("meta.py"), &link).expect("symlink");
        let err = resolve_script(root.path(), "escape.py").unwrap_err();
        assert!(
            err.errors()
                .iter()
                .any(|d| d.code == E2203 && d.message.contains("symlink")),
            "{err:?}"
        );
    }

    let resolved = resolve_script(root.path(), "tools/echo.py").expect("good script");
    assert!(resolved.is_file());
    // The resolved path is canonical, which on Windows means the verbatim
    // `\\?\C:\…` spelling — compare against a canonicalized root so both
    // sides carry the same prefix.
    assert!(
        resolved.starts_with(root.path().canonicalize().expect("root canon")),
        "{resolved:?}"
    );
}

// ---- run_script: real subprocess paths (python3 fixture scripts) ----

#[cfg(unix)]
fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(unix)]
fn write_script(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("script dir");
    }
    std::fs::write(&path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
}

#[test]
#[cfg(unix)]
fn run_script_round_trips_and_gates_output() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_script(
        tmp.path(),
        "tools/echo.py",
        "#!/usr/bin/env python3\nimport json, sys\nreq = json.load(sys.stdin)\nprint(json.dumps({\"contract\": \"cage.excel-meta/1\", \"echo\": req}))\n",
    );
    let ok = run_script(
        tmp.path(),
        "tools/echo.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({"x": 1}),
    )
    .expect("round-trip");
    assert_eq!(ok["contract"], "cage.excel-meta/1");
    assert_eq!(ok["echo"]["x"], 1);

    write_script(
        tmp.path(),
        "tools/bad.py",
        "#!/usr/bin/env python3\nprint('not json')\n",
    );
    let err = run_script(
        tmp.path(),
        "tools/bad.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({}),
    )
    .expect_err("invalid JSON");
    assert!(err.errors().iter().any(|d| d.code == E2206), "{err:?}");

    write_script(
        tmp.path(),
        "tools/contract.py",
        "#!/usr/bin/env python3\nprint('{\"contract\": \"cage.excel-meta/9\"}')\n",
    );
    let err = run_script(
        tmp.path(),
        "tools/contract.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({}),
    )
    .expect_err("contract mismatch");
    assert!(err.errors().iter().any(|d| d.code == E2206), "{err:?}");

    write_script(
        tmp.path(),
        "tools/fail.py",
        "#!/usr/bin/env python3\nimport sys\nsys.stderr.write('boom')\nsys.exit(3)\n",
    );
    let err = run_script(
        tmp.path(),
        "tools/fail.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({}),
    )
    .expect_err("non-zero exit");
    assert!(
        err.errors()
            .iter()
            .any(|d| d.code == E2205 && d.message.contains("boom")),
        "{err:?}"
    );

    write_script(
        tmp.path(),
        "tools/slow.py",
        "#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n",
    );
    let err = run_script(
        tmp.path(),
        "tools/slow.py",
        &[],
        Duration::from_millis(300),
        &serde_json::json!({}),
    )
    .expect_err("timeout");
    assert!(err.errors().iter().any(|d| d.code == E2204), "{err:?}");
}

#[test]
#[cfg(unix)]
fn run_script_reports_broken_pipe_and_spawn_failure() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_script(
        tmp.path(),
        "tools/exit.py",
        "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n",
    );
    let err = run_script(
        tmp.path(),
        "tools/exit.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({"payload": "x".repeat(65536)}),
    )
    .expect_err("broken pipe or quick exit");
    assert!(
        err.errors()
            .iter()
            .any(|d| d.code == E2205 || d.code == E2206 || d.code == E2203),
        "{err:?}"
    );

    write_script(
        tmp.path(),
        "tools/noexec.py",
        "#!/usr/bin/env python3\nprint('{}')\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let path = tmp.path().join("tools/noexec.py");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    let err = run_script(
        tmp.path(),
        "tools/noexec.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({}),
    )
    .expect_err("spawn failure");
    assert!(err.errors().iter().any(|d| d.code == E2203), "{err:?}");
}

#[test]
#[cfg(unix)]
fn run_script_truncates_long_stderr_at_a_char_boundary() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    // A script that fails with >400 bytes of stderr ending in a multi-byte
    // character. The truncation must land on a char boundary (byte 400 could
    // slice a CJK char) and must not panic, preserving the E2205 diagnostic.
    write_script(
        tmp.path(),
        "tools/loud.py",
        "#!/usr/bin/env python3\nimport sys\nsys.stderr.write('x' * 500 + '中文尾部' + '\\n')\nsys.exit(1)\n",
    );
    let err = run_script(
        tmp.path(),
        "tools/loud.py",
        &[],
        Duration::from_secs(5),
        &serde_json::json!({}),
    )
    .expect_err("non-zero exit");
    let errors = err.errors();
    let d = errors
        .iter()
        .find(|d| d.code == E2205)
        .expect("E2205 present");
    assert!(
        d.message.len() < 500,
        "stderr must be truncated to ~400 bytes, got {} bytes",
        d.message.len()
    );
    assert!(
        d.message.is_char_boundary(d.message.len()),
        "truncation must be safe"
    );
    assert!(
        !d.message.contains('中'),
        "truncation must cut before the multi-byte tail, got: {}",
        d.message
    );
}

#[test]
fn preset_for_script_and_empty_script_validate() {
    let scripted = ExcelConvention {
        script: Some("tools/gen.py".to_string()),
        ..ExcelConvention::default()
    };
    assert!(scripted.preset_for("anything.xlsx").is_data());
    assert!(scripted.uses_script());

    let empty = ExcelConvention {
        script: Some(String::new()),
        ..ExcelConvention::default()
    };
    let err = empty.validate().expect_err("empty script refused");
    assert!(err.contains("script must not be empty"), "{err}");
}

#[test]
fn script_contract_read_failure_is_e2203() {
    // A script path that is a directory resolves, then the read fails.
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(root.path().join("tools/dir")).expect("dir");
    let cfg = ExcelConvention {
        script: Some("tools/dir".to_string()),
        ..ExcelConvention::default()
    };
    let diags = script_fingerprint(root.path(), &cfg).expect_err("read refused");
    assert!(
        diags.errors().iter().any(|d| d.message.contains("E2203")),
        "{:?}",
        diags.errors()
    );
}
