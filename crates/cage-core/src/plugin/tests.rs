//! Worker-driver tests: manifest parsing, the seven failure classes against
//! real subprocesses, and finding → diagnostic location mapping.

use super::*;
use crate::error::codes::Severity;
use crate::value::{Row, SourceLocation, Table, TypedValue, Value};
use indexmap::IndexMap;
use std::path::PathBuf;

/// Repo root for a test running inside `crates/cage-core`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

/// Path of the bundled Python SDK (importable by test plugins).
fn sdk_path() -> PathBuf {
    repo_root().join("sdk").join("python")
}

/// Embed the absolute SDK path into a fixture script. The host never exports
/// an SDK path env var, so a fixture imports `cage_sdk` by absolute path.
fn sdk_script(source: &str) -> String {
    source.replace("__SDK__", &sdk_path().to_string_lossy())
}

fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Write a `plugin.toml` + entry script into `dir`.
fn write_plugin(dir: &Path, manifest: &str, script_rel: &str, script: &str) {
    std::fs::create_dir_all(dir).expect("plugin dir");
    std::fs::write(dir.join("plugin.toml"), manifest).expect("write manifest");
    let entry = dir.join(script_rel);
    if let Some(parent) = entry.parent() {
        std::fs::create_dir_all(parent).expect("entry dir");
    }
    std::fs::write(entry, script).expect("write entry");
}

/// A manifest for a plugin whose entry is `plugin.py`.
fn manifest_for(dir: &Path) -> PluginManifest {
    PluginManifest::load(dir).expect("manifest loads")
}

fn sample_document() -> Document {
    let loc = SourceLocation {
        file: "data/Monster.xlsx".to_string(),
        sheet: Some("Sheet1".to_string()),
        row: Some(2),
        ..SourceLocation::default()
    };
    let mut fields: IndexMap<String, TypedValue> = IndexMap::new();
    fields.insert(
        "id".to_string(),
        TypedValue::new(Value::Int(1), loc.clone()),
    );
    fields.insert(
        "level".to_string(),
        TypedValue::new(Value::Int(12), loc.clone()),
    );
    let row = Row {
        primary_key: vec![Value::Int(1)],
        fields,
        location: loc,
        index: 0,
    };
    let table = Table {
        name: "Monster".to_string(),
        primary_key_fields: vec!["id".to_string()],
        rows: vec![row],
        source_file: "data/Monster.xlsx".to_string(),
        sheet: Some("Sheet1".to_string()),
    };
    let mut doc = Document::new();
    doc.add_table(table);
    doc
}

fn sample_schema() -> crate::schema::Schema {
    crate::schema::Schema::new()
}

fn run_against(manifest: &PluginManifest, document: &Document) -> WorkerOutcome {
    let root = repo_root();
    let schema = sample_schema();
    let run = WorkerRun {
        request_id: "t-1".to_string(),
        schema: &schema,
        document,
        options: serde_json::json!({}),
    };
    run_plugin(&root, manifest, &run)
}

/// The failure behind a `Failed` outcome (tests then match it by reference so
/// the value stays owned for `to_diagnostic`).
fn failure_of(outcome: WorkerOutcome) -> WorkerFailure {
    match outcome {
        WorkerOutcome::Failed(failure) => failure,
        WorkerOutcome::Ok(_) => panic!("expected a failure, got Ok diagnostics"),
    }
}

// ---- manifest parsing ----

#[test]
fn manifest_loads_with_defaults() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"demo\"\nversion = \"1.0.0\"\nentry = \"plugin.py\"\n",
        "plugin.py",
        "print('placeholder')\n",
    );
    let manifest = PluginManifest::load(tmp.path()).expect("manifest");
    assert_eq!(manifest.id, "demo");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.runtime, "python");
    assert_eq!(manifest.entry, PathBuf::from("plugin.py"));
    assert_eq!(manifest.timeout, DEFAULT_PLUGIN_TIMEOUT);
    assert_eq!(manifest.python, None);
}

#[test]
fn manifest_loads_python_constraint_and_timeout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"demo\"\nversion = \"1.0.0\"\nentry = \"plugin.py\"\npython = \">=3.9\"\ntimeout_secs = 5\n",
        "plugin.py",
        "pass\n",
    );
    let manifest = PluginManifest::load(tmp.path()).expect("manifest");
    assert_eq!(
        manifest.python,
        Some(PythonConstraint {
            minimum: (3, 9),
            exact: None
        })
    );
    assert_eq!(manifest.timeout, Duration::from_secs(5));
}

#[test]
fn manifest_without_plugin_toml_reports_e1602() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let diags = PluginManifest::load(tmp.path()).expect_err("must fail");
    let d = &diags.errors()[0];
    assert_eq!(d.code, gamerule::E1602);
    assert!(d.message.contains("plugin.toml not found"));
}

#[test]
fn manifest_rejects_an_unsupported_runtime() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"demo\"\nversion = \"1\"\nentry = \"plugin.wasm\"\nruntime = \"wasm\"\n",
        "plugin.wasm",
        "",
    );
    let diags = PluginManifest::load(tmp.path()).expect_err("must fail");
    let d = &diags.errors()[0];
    assert_eq!(d.code, gamerule::E1602);
    assert!(d.message.contains("unsupported runtime 'wasm'"));
}

#[test]
fn manifest_rejects_a_bad_python_constraint() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"demo\"\nversion = \"1\"\nentry = \"plugin.py\"\npython = \"at least 3\"\n",
        "plugin.py",
        "",
    );
    let diags = PluginManifest::load(tmp.path()).expect_err("must fail");
    assert_eq!(diags.errors()[0].code, gamerule::E1602);
}

// ---- python constraint ----

#[test]
fn python_constraint_parses_the_accepted_forms() {
    assert_eq!(
        PythonConstraint::parse(">=3.9").unwrap(),
        PythonConstraint {
            minimum: (3, 9),
            exact: None
        }
    );
    assert_eq!(
        PythonConstraint::parse(">=3.12.1").unwrap(),
        PythonConstraint {
            minimum: (3, 12),
            exact: None
        }
    );
    assert_eq!(
        PythonConstraint::parse("3.12").unwrap().exact,
        Some((3, 12, 0))
    );
    assert_eq!(
        PythonConstraint::parse("==3.12.1").unwrap().exact,
        Some((3, 12, 1))
    );
    assert!(PythonConstraint::parse("").is_err());
    assert!(PythonConstraint::parse(">=x.y").is_err());
    assert!(PythonConstraint::parse(">=3").is_err());
    assert!(PythonConstraint::parse(">=3.9.0.1").is_err());
}

#[test]
fn python_constraint_matching_and_display() {
    let minimum = PythonConstraint::parse(">=3.9").unwrap();
    assert!(minimum.matches((3, 9, 0)));
    assert!(minimum.matches((3, 14, 4)));
    assert!(minimum.matches((4, 0, 0)));
    assert!(!minimum.matches((3, 8, 10)));
    assert!(!minimum.matches((2, 7, 18)));

    let exact = PythonConstraint::parse("==3.12.1").unwrap();
    assert!(exact.matches((3, 12, 1)));
    assert!(!exact.matches((3, 12, 0)));
    assert_eq!(minimum.display(), ">=3.9");
    assert_eq!(exact.display(), "3.12.1");
}

// ---- entry resolution ----

#[test]
fn entry_resolution_rejects_escape_and_absolute_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"d\"\nversion = \"1\"\nentry = \"../outside.py\"\n",
        "../outside.py",
        "",
    );
    let manifest = manifest_for(tmp.path());
    let err = manifest.entry_path().expect_err("traversal refused");
    assert!(err.errors()[0].message.contains("must not escape"));

    write_plugin(
        tmp.path(),
        "id = \"d\"\nversion = \"1\"\nentry = \"/etc/passwd\"\n",
        "plugin.py",
        "",
    );
    let manifest = manifest_for(tmp.path());
    let err = manifest.entry_path().expect_err("absolute refused");
    assert!(err.errors()[0].message.contains("relative"));
}

// ---- the failure table (real subprocesses) ----

#[test]
fn a_missing_entry_script_is_the_missing_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"ghost\"\nversion = \"1\"\nentry = \"plugin.py\"\n",
        "other.py",
        "",
    );
    let manifest = manifest_for(tmp.path());
    let failure = failure_of(run_against(&manifest, &sample_document()));
    match &failure {
        WorkerFailure::Missing { plugin_id, detail } => {
            assert_eq!(plugin_id, "ghost");
            assert!(
                detail.contains("entry script not found"),
                "detail: {detail}"
            );
            assert_eq!(failure.to_diagnostic().code, gamerule::E1602);
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn an_unsatisfiable_python_constraint_is_prechecked() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"versioned\"\nversion = \"1\"\nentry = \"plugin.py\"\npython = \">=99.0\"\n",
        "plugin.py",
        "print('never runs')\n",
    );
    let manifest = manifest_for(tmp.path());
    let failure = failure_of(run_against(&manifest, &sample_document()));
    match &failure {
        WorkerFailure::PythonVersionMismatch {
            plugin_id,
            expected,
            found,
        } => {
            assert_eq!(plugin_id, "versioned");
            assert_eq!(expected, ">=99.0");
            assert!(
                found.starts_with("Python ") || found.contains('.'),
                "found: {found}"
            );
            let diag = failure.to_diagnostic();
            assert_eq!(diag.code, gamerule::E1603);
            assert!(diag.message.contains("install a matching python"));
        }
        other => panic!("expected PythonVersionMismatch, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn a_crashing_worker_is_isolated_and_reported() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"crasher\"\nversion = \"1\"\nentry = \"plugin.py\"\n",
        "plugin.py",
        "import sys\nsys.stderr.write('boom')\nsys.exit(3)\n",
    );
    let manifest = manifest_for(tmp.path());
    let failure = failure_of(run_against(&manifest, &sample_document()));
    match &failure {
        WorkerFailure::Crash { plugin_id, detail } => {
            assert_eq!(plugin_id, "crasher");
            assert!(detail.contains('3'), "detail mentions exit code: {detail}");
            assert!(detail.contains("boom"), "stderr tail carried: {detail}");
            assert_eq!(failure.to_diagnostic().code, gamerule::E1603);
        }
        other => panic!("expected Crash, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn non_json_output_is_an_invalid_output_failure() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"noisy\"\nversion = \"1\"\nentry = \"plugin.py\"\n",
        "plugin.py",
        "print('not json')\n",
    );
    let manifest = manifest_for(tmp.path());
    match run_against(&manifest, &sample_document()) {
        WorkerOutcome::Failed(WorkerFailure::InvalidOutput { plugin_id, detail }) => {
            assert_eq!(plugin_id, "noisy");
            assert!(
                detail.contains("not json"),
                "stdout snippet carried: {detail}"
            );
        }
        other => panic!("expected InvalidOutput, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn a_silent_worker_times_out_and_is_killed() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"sleeper\"\nversion = \"1\"\nentry = \"plugin.py\"\ntimeout_secs = 2\n",
        "plugin.py",
        "import time\ntime.sleep(30)\n",
    );
    let manifest = manifest_for(tmp.path());
    let started = Instant::now();
    match run_against(&manifest, &sample_document()) {
        WorkerOutcome::Failed(WorkerFailure::Timeout { plugin_id, timeout }) => {
            assert_eq!(plugin_id, "sleeper");
            assert_eq!(timeout, Duration::from_secs(2));
            // Killed near the deadline, not after the full 30s sleep.
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "worker was killed promptly (elapsed {:?})",
                started.elapsed()
            );
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

// ---- the happy path: a real SDK plugin over a real subprocess ----

const CURVE_PLUGIN: &str = r#"
import sys
sys.path.insert(0, "__SDK__")
from cage_sdk import Plugin, run

class Curve(Plugin):
    id = "curve"
    version = "1.0.0"
    def validate(self, ctx):
        cap = ctx.options().get("cap", 10)
        table = ctx.table("Monster")
        for row in table["rows"]:
            level = row["fields"].get("level")
            if isinstance(level, (int, float)) and level > cap:
                ctx.error("level too high", table="Monster", row=row["index"], field="level", hint="lower it")
                ctx.warning("high level", table="Monster", row=row["index"])
                ctx.info("checked", table="Monster", row=row["index"])

run(Curve())
"#;

#[test]
#[cfg(unix)]
fn a_real_sdk_plugin_maps_findings_to_locations() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    write_plugin(
        tmp.path(),
        "id = \"curve\"\nversion = \"1.0.0\"\nentry = \"plugin.py\"\n",
        "plugin.py",
        &sdk_script(CURVE_PLUGIN),
    );
    let manifest = manifest_for(tmp.path());

    let document = sample_document();
    let schema = sample_schema();
    let run = WorkerRun {
        request_id: "t-curve".to_string(),
        schema: &schema,
        document: &document,
        options: serde_json::json!({"cap": 10}),
    };
    let outcome = run_plugin(&repo_root(), &manifest, &run);
    let WorkerOutcome::Ok(diags) = outcome else {
        panic!("expected Ok, got {outcome:?}");
    };
    assert_eq!(diags.errors().len(), 1);
    assert_eq!(diags.warnings().len(), 1);
    assert_eq!(diags.infos().len(), 1);

    let error = diags.errors()[0];
    assert_eq!(error.code, "E1601");
    assert_eq!(error.message, "level too high");
    assert_eq!(error.table.as_deref(), Some("Monster"));
    assert_eq!(error.row.as_deref(), Some("0"));
    assert_eq!(error.field.as_deref(), Some("level"));
    assert_eq!(error.hint.as_deref(), Some("lower it"));
    // The finding resolved back to the row's real source location.
    let loc = error.location.as_ref().expect("location resolved");
    assert_eq!(loc.file, "data/Monster.xlsx");
    assert_eq!(loc.sheet.as_deref(), Some("Sheet1"));
    assert_eq!(loc.row, Some(2));
    assert_eq!(error.source, "data/Monster.xlsx");

    let warning = diags.warnings()[0];
    assert_eq!(warning.table.as_deref(), Some("Monster"));
    assert!(warning.field.is_none());

    let info = diags.infos()[0];
    assert_eq!(info.severity, Severity::Info);
}

#[test]
#[cfg(unix)]
fn a_plugin_exception_surfaces_as_a_plugin_error() {
    if !python3_available() {
        eprintln!("skip: no python3");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = r#"
import sys
sys.path.insert(0, "__SDK__")
from cage_sdk import Plugin, run

class Thrower(Plugin):
    id = "thrower"
    version = "1"
    def validate(self, ctx):
        raise ValueError("bad options")

run(Thrower())
"#;
    write_plugin(
        tmp.path(),
        "id = \"thrower\"\nversion = \"1\"\nentry = \"plugin.py\"\n",
        "plugin.py",
        &sdk_script(script),
    );
    let manifest = manifest_for(tmp.path());
    let failure = failure_of(run_against(&manifest, &sample_document()));
    match &failure {
        WorkerFailure::PluginError { plugin_id, detail } => {
            assert_eq!(plugin_id, "thrower");
            assert!(detail.contains("ValueError"), "detail: {detail}");
            assert_eq!(failure.to_diagnostic().code, gamerule::E1603);
        }
        other => panic!("expected PluginError, got {other:?}"),
    }
}

// ---- protocol-error mapping (no subprocess needed) ----

#[test]
fn protocol_errors_map_onto_the_failure_table() {
    use cage_plugin_protocol as proto;

    let outcome = failure_from_protocol_error(
        "p",
        &proto::ProtocolError::version_mismatch(9, proto::PROTOCOL_VERSION),
    );
    let failure = failure_of(outcome);
    match &failure {
        WorkerFailure::ProtocolVersionMismatch {
            plugin_id,
            request_protocol,
            supported,
        } => {
            assert_eq!(plugin_id, "p");
            assert_eq!(*request_protocol, 9);
            assert_eq!(*supported, proto::PROTOCOL_VERSION);
            let diag = failure.to_diagnostic();
            assert_eq!(diag.code, gamerule::E1603);
            assert!(diag.message.contains("rebuild the plugin"));
        }
        other => panic!("expected ProtocolVersionMismatch, got {other:?}"),
    }

    let outcome = failure_from_protocol_error("p", &proto::ProtocolError::plugin_error("x"));
    assert!(matches!(
        outcome,
        WorkerOutcome::Failed(WorkerFailure::PluginError { .. })
    ));

    let outcome = failure_from_protocol_error("p", &proto::ProtocolError::invalid_result("x"));
    assert!(matches!(
        outcome,
        WorkerOutcome::Failed(WorkerFailure::InvalidOutput { .. })
    ));

    // Unknown codes degrade to invalid output, never silently pass.
    let outcome = failure_from_protocol_error(
        "p",
        &proto::ProtocolError {
            code: "SOMETHING_ELSE".to_string(),
            message: "m".to_string(),
        },
    );
    assert!(matches!(
        outcome,
        WorkerOutcome::Failed(WorkerFailure::InvalidOutput { .. })
    ));
}

#[test]
fn version_parsing_helpers() {
    assert_eq!(parse_python_version("Python 3.14.4").unwrap(), (3, 14, 4));
    assert_eq!(parse_python_version("3.12").unwrap(), (3, 12, 0));
    assert_eq!(
        parse_python_version("Python 3.14.0rc1").unwrap(),
        (3, 14, 0)
    );
    assert!(parse_python_version("nope").is_err());
    assert_eq!(format_version((3, 9, 0)), "3.9.0");
    assert_eq!(trim_stdout(b"{\"a\":1}\n"), b"{\"a\":1}");
    assert_eq!(trim_stdout(b"{\"a\":1}\n\n"), b"{\"a\":1}");
    assert_eq!(truncate_stderr("a".repeat(500).as_str()).len(), 400);
    // CJK truncation must not split a character: the cut stays <= the cap
    // bytes and decodes cleanly (an is_char_boundary cut would panic).
    let cjk = "爆".repeat(300);
    assert_eq!(cjk.len(), 900);
    let cut = truncate_stderr(&cjk);
    assert!(cut.len() <= 400);
    assert!(cut.chars().all(|c| c == '爆'));
}
