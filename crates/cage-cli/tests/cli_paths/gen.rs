use super::*;

#[test]
fn gen_nonexistent_root_exits_2() {
    let ghost = std::env::temp_dir().join("cage-cli-ghost-gen");
    let out = run_cage(&["gen", ghost.to_str().unwrap()]);
    assert_code(&out, 2, "nonexistent project root");
    assert!(
        stderr(&out).contains("no project config found"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn gen_unknown_profile_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_project(tmp.path());
    let out = run_cage(&["gen", tmp.path().to_str().unwrap(), "--profile", "server"]);
    assert_code(&out, 2, "unknown profile");
    assert!(
        stderr(&out).contains("unknown profile 'server' (available: client)"),
        "stderr:\n{}",
        stderr(&out)
    );
}

fn write_codegen_project(root: &Path, cage_toml: &str) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(root.join("cage.toml"), cage_toml).unwrap();
    fs::write(root.join("schemas/item.yaml"), ITEM_SCHEMA).unwrap();
    fs::write(root.join("config/item.json"), ITEM_DATA).unwrap();
}

#[test]
fn gen_artifact_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    write_codegen_project(
        tmp.path(),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "genfail"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "cs"
output_dir = "gout"
"#,
    );
    // The code artifact's output dir is a regular file → create_dir_all fails.
    fs::write(tmp.path().join("gout"), "blocker").unwrap();
    let out = run_cage(&["gen", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "gen artifact parent blocked");
    assert!(
        stderr(&out).contains("cannot create"),
        "stderr:\n{}",
        stderr(&out)
    );
}

#[test]
fn gen_manifest_write_failure_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    // Manifest lands in output_dir = "gmout", which is a regular file, while
    // the code artifacts go to "gout" and succeed.
    write_codegen_project(
        tmp.path(),
        r#"output_dir = "gmout"
schema_path = "schemas"

[project]
name = "genfail"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "cs"
output_dir = "gout"
"#,
    );
    fs::write(tmp.path().join("gmout"), "blocker").unwrap();
    let out = run_cage(&["gen", tmp.path().to_str().unwrap()]);
    assert_code(&out, 2, "gen manifest dir blocked");
    assert!(
        stderr(&out).contains("cannot create"),
        "stderr:\n{}",
        stderr(&out)
    );
}
