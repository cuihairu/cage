//! Process-level dispatch coverage: the `main()` match arms that unit
//! tests cannot reach (they need a real `cage` binary) — `graph`,
//! `verify`, and `mcp` without `--stdio`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

fn minimal_project(dir: &Path) {
    fs::write(
        dir.join("cage.toml"),
        "schema_path = \"schemas\"\n[project]\nname = \"dispatch\"\n[source_roots]\nmain = \"config\"\n[profiles.client]\nname = \"client\"\ntargets = [{ format = \"json\", output_dir = \"build/client/json\" }]\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("schemas")).unwrap();
    fs::write(
        dir.join("schemas/item.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\nenums: {}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("config")).unwrap();
    fs::write(dir.join("config/Item.json"), r#"{"Item": [{"id": 1}]}"#).unwrap();
}

#[test]
fn graph_command_runs_all_formats_and_reports_bad_format() {
    let dir = tempfile::tempdir().unwrap();
    minimal_project(dir.path());
    for format in ["text", "dot", "json"] {
        let out = run_cage(&["graph", dir.path().to_str().unwrap(), "--format", format]);
        assert!(out.status.success(), "{format}: {:?}", out);
        let body = String::from_utf8_lossy(&out.stdout);
        match format {
            "text" => assert!(body.contains("Item"), "{format}: {body}"),
            "dot" => assert!(body.contains("digraph"), "{format}: {body}"),
            "json" => assert!(body.contains("Item"), "{format}: {body}"),
            _ => unreachable!(),
        }
    }
    let out = run_cage(&["graph", dir.path().to_str().unwrap(), "--format", "bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown format"), "{err}");
}

#[test]
fn verify_command_checks_a_built_tree_and_missing_manifest() {
    let dir = tempfile::tempdir().unwrap();
    minimal_project(dir.path());
    let build = run_cage(&["build", dir.path().to_str().unwrap(), "--profile", "client"]);
    assert!(build.status.success(), "build: {:?}", build);

    let out = run_cage(&["verify", dir.path().to_str().unwrap()]);
    assert!(out.status.success(), "verify: {:?}", out);
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(body.contains("manifest"), "{body}");

    // Missing manifest → exit 2.
    let empty = tempfile::tempdir().unwrap();
    let out = run_cage(&["verify", empty.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn mcp_without_stdio_prints_tools() {
    let out = run_cage(&["mcp"]);
    assert!(out.status.success(), "mcp: {:?}", out);
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(body.contains("cage mcp"), "{body}");
    assert!(body.contains("check_schema"), "{body}");
}
