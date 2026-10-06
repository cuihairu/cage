//! `format = "template"` targets (design §22 G3): user `.tera` templates
//! render against the schema IR. The template directory resolves against
//! the project root — `options.template_dir`, defaulting to the
//! `.cage/templates` convention — and template faults (missing directory,
//! syntax errors) fail the run with an error naming the offending path.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write_project(root: &Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::create_dir_all(root.join(".cage/templates")).unwrap();
    fs::create_dir_all(root.join("my_templates")).unwrap();

    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "tplgen"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "template"
output_dir = "build/tpl"

[profiles.custom]
name = "custom"

[[profiles.custom.targets]]
format = "template"
output_dir = "build/custom"
options = { template_dir = "my_templates" }
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
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
"#,
    )
    .unwrap();

    fs::write(
        root.join("config/item.json"),
        r#"{"Item": [{"id": 1, "name": "Sword"}]}"#,
    )
    .unwrap();

    fs::write(
        root.join(".cage/templates/{table}.tpl.tera"),
        "table={{ table.name }} pk-snake={{ table.name | snake_case }} hash={{ schema_hash | default(value=\"(unavailable)\") }}\n",
    )
    .unwrap();
    fs::write(
        root.join(".cage/templates/defs.tera"),
        "tables={{ tables | length }} hash={{ schema_hash | default(value=\"(unavailable)\") }}\n",
    )
    .unwrap();

    fs::write(
        root.join("my_templates/{table}.tpl.tera"),
        "custom table={{ table.name }} fields={{ table.fields | length }}\n",
    )
    .unwrap();
}

fn run_cage(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn gen_renders_user_templates_from_the_convention_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_success(&out, "gen");

    // Per-table template: `{table}` in the file name renders once per table
    // (name order); the hash stamp comes from the manifest's schema hash.
    let tpl = fs::read_to_string(root.join("build/tpl/Item.tpl")).unwrap();
    assert!(tpl.starts_with("table=Item "));
    assert!(tpl.contains("pk-snake=item"));
    assert!(!tpl.contains("(unavailable)"));
    assert!(tpl.contains("hash="));

    // Global template (no `{table}` placeholder) renders exactly once.
    let defs = fs::read_to_string(root.join("build/tpl/defs")).unwrap();
    assert!(defs.contains("tables=1"));
    assert!(defs.contains("hash="));
}

#[test]
fn build_writes_template_artifacts_into_the_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "client"]);
    assert_success(&out, "build");

    let manifest = fs::read_to_string(root.join("build/manifest.json")).unwrap();
    assert!(manifest.contains("\"format\": \"template\""));
    assert!(manifest.contains("build/tpl/Item.tpl"));

    let out = run_cage(&["build", root.to_str().unwrap(), "--profile", "custom"]);
    assert_success(&out, "build custom");
    let custom = fs::read_to_string(root.join("build/custom/Item.tpl")).unwrap();
    assert_eq!(custom, "custom table=Item fields=2\n");
}

#[test]
fn missing_template_dir_fails_naming_the_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    fs::remove_dir_all(root.join(".cage/templates")).unwrap();

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot read"), "{stderr}");
    assert!(stderr.contains(".cage/templates"), "{stderr}");

    // An existing but empty directory is the other flavor of "no templates".
    fs::create_dir_all(root.join(".cage/templates")).unwrap();
    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no *.tera templates"), "{stderr}");
}

#[test]
fn template_syntax_fault_names_the_offending_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);
    fs::write(
        root.join(".cage/templates/broken.tera"),
        "{% for t in tables %}\n{{ t.name }}\n",
    )
    .unwrap();

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("broken.tera"), "{stderr}");
}

#[test]
fn template_render_is_byte_deterministic_across_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_project(root);

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_success(&out, "gen");
    let first = fs::read_to_string(root.join("build/tpl/Item.tpl")).unwrap();

    let out = run_cage(&["gen", root.to_str().unwrap(), "--profile", "client"]);
    assert_success(&out, "gen 2");
    let second = fs::read_to_string(root.join("build/tpl/Item.tpl")).unwrap();

    assert_eq!(first, second);
}
