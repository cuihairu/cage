//! P0 三源一致性对拍套件（2026-10-26 Luban 对标跟踪清单 P0）。
//!
//! 同一逻辑配置在 Excel / CSV / JSON（及嵌套形专用的 YAML）多份 fixture 上
//! 编译，断言规范化 IR 经 JSON target 输出后逐字节一致。同时逐项断言：
//! - 默认值（显式写出的 schema default，且流入代码 target）
//! - 空值（`Any` 上的 null / 空单元格）
//! - 显式空串（JSON/YAML 可表示；CSV/Excel 空单元格坍缩为 null → 不可表示）
//! - 数值边界（`Int32` 上下界）
//! - 嵌套对象（Object / Array / Map；JSON vs YAML 对拍）
//! - 字符串枚举成员资格
//! - 外键引用解析（`Drop.id` 存在性）
//! - 三源诊断定位：Excel 文件+工作表+单元格 / CSV 文件+行号+列名 /
//!   JSON 文件+行号+JSON 路径

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CAGE_TOML: &str = r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "cross-source-parity"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
"#;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cross_source")
}

/// 一个临时对拍工程；持有 `TempDir` 直到测试结束。
struct Project {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    fn empty() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("schemas")).unwrap();
        fs::write(root.join("cage.toml"), CAGE_TOML).unwrap();
        fs::copy(
            fixtures().join("common-schema.yaml"),
            root.join("schemas/item.yaml"),
        )
        .unwrap();
        Self { _tmp: tmp, root }
    }

    fn copy_fixture(&self, name: &str) {
        let src = fixtures().join(name);
        let dst = self.root.join("config").join(name);
        fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("copy {} → {}: {e}", src.display(), dst.display()));
    }

    fn write_source(&self, name: &str, bytes: impl AsRef<[u8]>) {
        fs::write(self.root.join("config").join(name), bytes)
            .unwrap_or_else(|e| panic!("write {name}: {e}"));
    }

    fn new(sources: &[&str]) -> Self {
        let project = Self::empty();
        for name in sources {
            project.copy_fixture(name);
        }
        project
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

fn run_build(root: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_cage"))
        .args(["build", root.to_str().unwrap(), "--profile", "client"])
        .output()
        .expect("spawn cage build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn read_artifact(root: &Path, table: &str) -> Vec<u8> {
    fs::read(root.join("build/client/json").join(format!("{table}.json"))).unwrap()
}

fn assert_common_parity(roots: [&Path; 3], label: &str) {
    let [excel, csv, json] = roots;
    let ie = read_artifact(excel, "Item");
    let ic = read_artifact(csv, "Item");
    let ij = read_artifact(json, "Item");
    let de = read_artifact(excel, "Drop");
    let dc = read_artifact(csv, "Drop");
    let dj = read_artifact(json, "Drop");
    assert_eq!(ie, ic, "{label}: Item bytes diverge (xlsx vs csv)");
    assert_eq!(ie, ij, "{label}: Item bytes diverge (xlsx vs json)");
    assert_eq!(de, dc, "{label}: Drop bytes diverge (xlsx vs csv)");
    assert_eq!(de, dj, "{label}: Drop bytes diverge (xlsx vs json)");

    let text = String::from_utf8(ie).unwrap();
    for needle in [
        r#""tier": 3"#,
        r#""nullable": null"#,
        r#""bound": 2147483647"#,
        r#""bound": -2147483648"#,
        r#""weight": 1.5"#,
        r#""kind": "Sword""#,
        r#""drop_id": 1"#,
    ] {
        assert!(
            text.contains(needle),
            "{label}: missing {needle} in\n{text}"
        );
    }
}

#[test]
fn three_source_common_core_builds_byte_identical() {
    let excel = Project::new(&["data.xlsx"]);
    let csv = Project::new(&["Item.csv", "Drop.csv"]);
    let json = Project::new(&["Item.json", "Drop.json"]);

    for (label, p) in [("xlsx", &excel), ("csv", &csv), ("json", &json)] {
        let (ok, out) = run_build(p.root());
        assert!(ok, "{label} build failed:\n{out}");
    }

    assert_common_parity([excel.root(), csv.root(), json.root()], "common-core");
}

#[test]
fn nested_object_json_and_yaml_build_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let json_root = tmp.path().join("proj-nested-json");
    let yaml_root = tmp.path().join("proj-nested-yaml");

    for (dir, source) in [(&json_root, "nested.json"), (&yaml_root, "Nested.yaml")] {
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::create_dir_all(dir.join("schemas")).unwrap();
        fs::write(dir.join("cage.toml"), CAGE_TOML).unwrap();
        fs::copy(
            fixtures().join("nested-schema.yaml"),
            dir.join("schemas/nested.yaml"),
        )
        .unwrap();
        fs::copy(fixtures().join(source), dir.join("config").join(source)).unwrap();
    }

    let (ok, out) = run_build(&json_root);
    assert!(ok, "nested JSON build failed:\n{out}");
    let (ok, out) = run_build(&yaml_root);
    assert!(ok, "nested YAML build failed:\n{out}");

    let ja = fs::read(json_root.join("build/client/json/Item.json")).unwrap();
    let jb = fs::read(yaml_root.join("build/client/json/Item.json")).unwrap();
    assert_eq!(ja, jb, "nested object JSON/YAML bytes diverge");

    let text = String::from_utf8(ja).unwrap();
    for needle in [
        r#""stats""#,
        r#""hp": 100"#,
        r#""mp": 50"#,
        r#""tags""#,
        r#""drop_table""#,
        r#""common""#,
        r#""rare""#,
    ] {
        assert!(text.contains(needle), "missing {needle} in\n{text}");
    }
}

#[test]
fn explicit_empty_string_is_json_yaml_only_and_table_sources_reject_it() {
    // JSON preserves `""` as a String.
    let json = Project::new(&["Item-empty-string.json", "Drop.json"]);
    let (ok, out) = run_build(json.root());
    assert!(ok, "JSON empty-string build failed:\n{out}");
    let jt = String::from_utf8(read_artifact(json.root(), "Item")).unwrap();
    assert!(
        jt.contains(r#""note": """#),
        "JSON must keep \"\", got:\n{jt}"
    );

    // YAML has the same text path: `note: ""` stays a String.
    let yaml = Project::new(&["Item-empty-string.yaml", "Drop.json"]);
    let (ok, out) = run_build(yaml.root());
    assert!(ok, "YAML empty-string build failed:\n{out}");
    let yt = String::from_utf8(read_artifact(yaml.root(), "Item")).unwrap();
    assert!(
        yt.contains(r#""note": """#),
        "YAML must keep \"\", got:\n{yt}"
    );

    // CSV / Excel cannot spell `""`: an empty String cell becomes Null and the
    // field type rejects Null with E1101 → build fails. This pins the table
    // fidelity limit rather than pretending parity the adapters cannot deliver.
    // The fixture file names here are table-derived (CSV stem / Excel sheet),
    // so they must land in the project as `Item.csv` / `data.xlsx`.
    let csv_bytes = fs::read(fixtures().join("Item-unrepresentable-empty-string.csv")).unwrap();
    let csv = Project::empty();
    csv.write_source("Item.csv", csv_bytes);
    csv.copy_fixture("Drop.json");
    let (ok, out) = run_build(csv.root());
    assert!(!ok, "CSV empty String cell must fail with E1101");
    assert!(out.contains("E1101"), "CSV E1101 expected, got:\n{out}");

    let excel_bytes = fs::read(fixtures().join("data-unrepresentable-empty-string.xlsx")).unwrap();
    let excel = Project::empty();
    excel.write_source("data.xlsx", excel_bytes);
    let (ok, out) = run_build(excel.root());
    assert!(!ok, "Excel empty String cell must fail with E1101");
    assert!(out.contains("E1101"), "Excel E1101 expected, got:\n{out}");
}

#[test]
fn excel_empty_cell_fills_schema_default() {
    // 回流-1: an Excel empty cell collapses to Null, and a field with a schema
    // default is filled with it — the build succeeds and the output carries the
    // default, so an empty cell is no longer a hard type error for defaulted
    // fields. `tier` is empty in every row of `data-empty-tier.xlsx`.
    let excel = Project::new(&["data-empty-tier.xlsx", "Drop.json"]);
    let (ok, out) = run_build(excel.root());
    assert!(ok, "Excel empty-cell + default build failed:\n{out}");
    let text = String::from_utf8(read_artifact(excel.root(), "Item")).unwrap();
    assert!(
        text.contains(r#""tier": 3"#),
        "defaulted tier must be filled, got:\n{text}"
    );
    // The rest of the row is untouched by the fill.
    assert!(text.contains(r#""name": "Sword""#), "name lost:\n{text}");
    assert!(!text.contains("E1101"), "unexpected E1101 in\n{text}");
}

#[test]
fn excel_nullable_field_accepts_empty_cell() {
    // 回流-2: an Excel empty cell collapses to Null, and a `nullable: true`
    // typed field accepts it — the build succeeds without weakening the type
    // to Any. `note` is empty in every row of `data-nullable.xlsx`.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(root.join("cage.toml"), CAGE_TOML).unwrap();
    // common-schema with `note` made nullable.
    let schema = fs::read_to_string(fixtures().join("common-schema.yaml"))
        .unwrap()
        .replace(
            "note: { name: note, type: { kind: String } }",
            "note: { name: note, type: { kind: String }, nullable: true }",
        );
    fs::write(root.join("schemas/item.yaml"), schema).unwrap();
    fs::copy(
        fixtures().join("data-nullable.xlsx"),
        root.join("config/data.xlsx"),
    )
    .unwrap();
    fs::copy(fixtures().join("Drop.json"), root.join("config/Drop.json")).unwrap();

    let (ok, out) = run_build(&root);
    assert!(ok, "nullable empty-cell build failed:\n{out}");
    let text = String::from_utf8(read_artifact(&root, "Item")).unwrap();
    assert!(
        text.contains(r#""note": null"#),
        "nullable note must stay null, got:\n{text}"
    );
    assert!(!text.contains("E1101"), "unexpected E1101 in\n{text}");
}

#[test]
fn default_flows_from_schema_to_code_target_while_ir_keeps_explicit_value() {
    // Data IR: the common-core parity test pins `tier: 3` explicitly from all
    // three sources. The schema default is the authoring contract the code
    // targets render — build a python target and check the dataclass carries
    // `tier: int = 3`.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj-default");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("schemas")).unwrap();
    fs::write(
        root.join("cage.toml"),
        r#"output_dir = "build"
schema_path = "schemas"

[project]
name = "default"
version = "0.1.0"

[source_roots]
main = "config"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"

[[profiles.client.targets]]
format = "python"
output_dir = "build/client/py"
"#,
    )
    .unwrap();
    fs::copy(
        fixtures().join("common-schema.yaml"),
        root.join("schemas/item.yaml"),
    )
    .unwrap();
    fs::copy(fixtures().join("Item.json"), root.join("config/Item.json")).unwrap();
    fs::copy(fixtures().join("Drop.json"), root.join("config/Drop.json")).unwrap();

    let (ok, out) = run_build(&root);
    assert!(ok, "default build failed:\n{out}");
    let py = fs::read_to_string(root.join("build/client/py/Item.py")).unwrap();
    assert!(
        py.contains("tier: int = 3"),
        "generated Item.py must carry the default, got:\n{py}"
    );
}

#[test]
fn diagnostic_locations_excel_csv_json() {
    // Excel：文件 + 工作表 + 单元格。首个数据行在表头行（第 1 行）之下，
    // 因此是工作表第 2 行；price 列在 D 列。
    let excel = Project::empty();
    excel.write_source(
        "data.xlsx",
        fs::read(fixtures().join("data-bad-price.xlsx")).unwrap(),
    );
    let (ok, out) = run_build(excel.root());
    assert!(!ok, "Excel bad price must fail");
    assert!(out.contains("E1201"), "Excel E1201 expected:\n{out}");
    assert!(out.contains("data.xlsx"), "Excel file missing:\n{out}");
    assert!(out.contains("Sheet: Item"), "Excel sheet missing:\n{out}");
    assert!(out.contains("Row: 2"), "Excel sheet row missing:\n{out}");
    assert!(out.contains("Col: D"), "Excel cell column missing:\n{out}");

    // CSV：文件 + 行号 + 列名（首个数据记录 = 第 1 行）。
    let csv = Project::empty();
    csv.write_source("Item.csv", "id,name,kind,price,bound,weight,enabled,tier,note,nullable,drop_id\n1,Sword,Sword,-1,0,1.5,true,3,hello,x,1\n");
    csv.copy_fixture("Drop.json");
    let (ok, out) = run_build(csv.root());
    assert!(!ok, "CSV bad price must fail");
    assert!(out.contains("Item.csv"), "CSV file missing:\n{out}");
    assert!(out.contains("Row: 1"), "CSV row missing:\n{out}");
    assert!(out.contains("Col: price"), "CSV column missing:\n{out}");

    // JSON：文件 + 行号 + 路径（行号 + 字段列）。
    let json = Project::empty();
    json.write_source(
        "Item.json",
        r#"{"Item":[{"id":1,"name":"Sword","kind":"Sword","price":-1,"bound":0,"weight":1.5,"enabled":true,"tier":3,"note":"hello","nullable":"x","drop_id":1}]}"#,
    );
    json.copy_fixture("Drop.json");
    let (ok, out) = run_build(json.root());
    assert!(!ok, "JSON bad price must fail");
    assert!(out.contains("Item.json"), "JSON file missing:\n{out}");
    assert!(out.contains("Row: 1"), "JSON row missing:\n{out}");
    assert!(out.contains("Col: price"), "JSON path missing:\n{out}");
}
