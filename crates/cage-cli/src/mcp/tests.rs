//! MCP tool and JSON-view tests (split out of mcp.rs, LAY-026).

use super::params;
use super::tools::{
    run_build_dry_run, run_check_schema, run_explain_config, run_list_envs, run_list_fields,
    run_list_tables,
};
use super::views::{
    diag_json, find_config_file, load_error_json, location_string, precedence_chain, render,
    section_key, source_root_kind, suggestion_for,
};
use super::{print_tools, CageMcpServer};
use cage_core::error::codes::{editor, registry, remote};
use cage_core::{
    build, gamerule, internal, parse, ref_codes, schema_codes, semantic, table, type_val,
    value_codes,
};
use serde_json::json;
use std::path::{Path, PathBuf};

fn example_project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/game-config")
}

#[test]
fn check_schema_green_project_is_ok() {
    let v = run_check_schema(
        example_project().to_str().unwrap(),
        Some("dev"),
        "client",
        "semantic",
        false,
    )
    .expect("example project loads");
    assert_eq!(v["ok"], json!(true), "output: {v}");
    assert_eq!(v["tables_checked"], json!(6));
    assert_eq!(v["error_count"], json!(0));
}

#[test]
fn check_schema_structures_a_broken_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_min_project(&root);
    // price=-5（E1201）+ price=oops（E1101 类型不符）
    std::fs::write(root.join("config/Item.csv"), "id,price\n1,-5\n2,oops\n").unwrap();
    let v =
        run_check_schema(root.to_str().unwrap(), None, "client", "semantic", false).expect("loads");
    assert_eq!(v["ok"], json!(false), "output: {v}");
    assert_eq!(v["error_count"], json!(2));
    let codes: Vec<&str> = v["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["code"].as_str())
        .collect();
    assert!(codes.contains(&"E1101"), "codes: {codes:?}");
    assert!(codes.contains(&"E1201"), "codes: {codes:?}");
    for e in v["errors"].as_array().unwrap() {
        assert!(
            e["file"].as_str().is_some_and(|f| f.ends_with("Item.csv")),
            "file: {e}"
        );
        assert!(
            e["where"].as_str().is_some_and(|w| w.contains("Item.csv")),
            "where: {e}"
        );
        assert!(
            e["suggestion"].as_str().is_some_and(|s| !s.is_empty()),
            "suggestion: {e}"
        );
    }
}

#[test]
fn check_schema_flags_missing_file_and_missing_path() {
    // 工程目录不存在 → 结构化错误 + 建议
    let err = run_check_schema(
        "/nonexistent-cage-project",
        None,
        "client",
        "semantic",
        false,
    )
    .expect_err("missing dir must fail");
    let v = load_error_json(&err, "/nonexistent-cage-project");
    assert_eq!(v["ok"], json!(false));
    assert!(v["error"]["suggestion"].is_string());
    // 非 carry 的装载错误也带 message
    let v = run_list_tables("/nonexistent-cage-project", false)
        .unwrap_or_else(|e| load_error_json(&e, "/nonexistent-cage-project"));
    assert_eq!(v["ok"], json!(false));
}

#[test]
fn list_tables_reports_structure_and_envs() {
    let v = run_list_tables(example_project().to_str().unwrap(), false).expect("loads");
    let tables = v["tables"].as_array().expect("tables array");
    let names: Vec<&str> = tables.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Item"), "tables: {names:?}");
    assert!(names.contains(&"Character"), "tables: {names:?}");
    let item = tables.iter().find(|t| t["name"] == json!("Item")).unwrap();
    assert!(item["row_count"].as_u64().unwrap() > 0);
    // 四环境演示在 Shop（discount 逐环境收紧）——list_tables 里能看到
    // （键序按 schema 文档书写序，不排序）
    let shop = tables.iter().find(|t| t["name"] == json!("Shop")).unwrap();
    let envs: Vec<&str> = shop["env_overrides"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e.as_str())
        .collect();
    for env in ["dev", "test", "staging", "prod"] {
        assert!(envs.contains(&env), "envs: {envs:?}");
    }
}

#[test]
fn list_fields_unknown_table_names_alternatives() {
    let err = run_list_fields(example_project().to_str().unwrap(), Some("Nope"), false)
        .expect_err("unknown table");
    assert!(err.contains("不在 schema 里"), "err: {err}");
    assert!(err.contains("Item"), "alternatives listed: {err}");
    let v =
        run_list_fields(example_project().to_str().unwrap(), Some("Item"), false).expect("loads");
    let fields = v["tables"][0]["fields"].as_array().unwrap();
    assert!(fields.len() >= 3);
    let id = fields.iter().find(|f| f["name"] == json!("id")).unwrap();
    assert_eq!(id["type"]["kind"], json!("Int32"), "kind surfaces: {id}");
    let item_type = fields.iter().find(|f| f["name"] == json!("type")).unwrap();
    assert_eq!(item_type["type"]["kind"], json!("Enum"));
    assert_eq!(item_type["type"]["value"], json!("ItemType"));
}

#[test]
fn list_envs_declares_the_four_environments() {
    let v = run_list_envs(example_project().to_str().unwrap(), false).expect("loads");
    assert_eq!(
        v["declared"],
        json!(["dev", "prod", "staging", "test"]),
        "declared set: {}",
        v["declared"]
    );
    // 四环境差异：Shop.discount 逐环境收紧（0.9 → 0.5）
    let shop = &v["tables"]["Shop"];
    assert!(shop.is_object(), "Shop carries overrides: {shop}");
    assert_eq!(shop["env_overrides"]["prod"]["discount"]["max"], json!(0.5));
    assert_eq!(shop["env_overrides"]["dev"]["discount"]["max"], json!(0.9));
}

#[test]
fn build_dry_run_plans_without_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_min_project(&root);
    let v = run_build_dry_run(root.to_str().unwrap(), "client", None, "semantic", false)
        .expect("dry run computes");
    assert_eq!(v["ok"], json!(true), "output: {v}");
    assert_eq!(v["written"], json!(false));
    assert!(
        v["artifact_count"].as_u64().unwrap() >= 2,
        "artifacts planned: {}",
        v["artifacts"]
    );
    assert!(v["schema_hash"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(v["build_id"].as_str().is_some_and(|s| !s.is_empty()));
    // 预演不落盘：输出目录没有 build/
    assert!(
        !root.join("build").exists(),
        "dry run must not create build/"
    );
}

/// 最小可构建工程：json+csv 两个 target，一张 Item 表。
/// 注意 TOML 里 `schema_path` 必须在任何 `[table]` 头之前（顶层键）。
fn write_min_project(root: &Path) {
    std::fs::write(
            root.join("cage.toml"),
            "schema_path = \"schemas\"\n[project]\nname = \"dry\"\n[source_roots]\nmain = \"config\"\n[profiles.client]\nname = \"client\"\ntargets = [{ format = \"json\", output_dir = \"build/client/json\" }, { format = \"csv\", output_dir = \"build/client/csv\" }]\n",
        )
        .unwrap();
    std::fs::create_dir(root.join("schemas")).unwrap();
    std::fs::write(
            root.join("schemas/item.yaml"),
            "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\n      price: { name: price, type: { kind: Int32 }, required: true, min: 0 }\nenums: {}\n",
        )
        .unwrap();
    std::fs::create_dir(root.join("config")).unwrap();
    std::fs::write(root.join("config/Item.csv"), "id,price\n1,10\n2,25\n").unwrap();
}

#[test]
fn build_dry_run_reports_validation_errors_structured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_min_project(&root);
    // price=-5 违反 min: 0 → E1201
    std::fs::write(root.join("config/Item.csv"), "id,price\n1,-5\n").unwrap();
    let v = run_build_dry_run(root.to_str().unwrap(), "client", None, "semantic", false)
        .expect("dry run computes");
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["stage"], json!("validate"));
    let errs = v["errors"].as_array().unwrap();
    let e1201 = errs
        .iter()
        .find(|e| e["code"] == json!("E1201"))
        .expect("range error present");
    assert!(
        e1201["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("Item.csv")),
        "file: {e1201}"
    );
    assert!(
        e1201["where"]
            .as_str()
            .is_some_and(|w| w.contains("Item.csv")),
        "composite location: {e1201}"
    );
    // E1201 自带 hint（"Minimum allowed: 0"）时优先于按码兜底的建议
    assert!(
        e1201["suggestion"]
            .as_str()
            .is_some_and(|s| s.to_lowercase().contains("min")),
        "suggestion: {e1201}"
    );
}

#[test]
fn explain_config_carries_actual_values_and_chain() {
    let v = run_explain_config(example_project().to_str().unwrap(), None).expect("loads");
    assert_eq!(v["config_file"], json!("cage.toml"));
    assert!(v["profiles"]
        .as_object()
        .is_some_and(|p| p.contains_key("client")));
    assert!(v["precedence"].as_array().is_some_and(|p| p.len() >= 5));
    assert!(v["precedence"][0]["rule"]
        .as_str()
        .is_some_and(|r| r.contains("cage.toml")));
    // section 过滤 + 别名
    let v = run_explain_config(example_project().to_str().unwrap(), Some("source_roots"))
        .expect("section loads");
    assert!(v["value"].as_array().is_some_and(|a| !a.is_empty()));
    let err = run_explain_config(example_project().to_str().unwrap(), Some("bogus"))
        .expect_err("unknown section");
    assert!(err.contains("未知 section"));
}

#[test]
fn suggestion_table_covers_the_common_families() {
    // 缺字段 / 类型不符 / 环境未声明 / 模板变量不存在 —— 用户点名的四类
    let cases = [
        (schema_codes::E1001, "required"),
        (type_val::E1101, "kind"),
        (build::E9002, "模板变量"),
    ];
    for (code, needle) in cases {
        let d = cage_core::Diagnostic {
            code: code.to_string(),
            ..empty_diag()
        };
        let s = suggestion_for(&d).unwrap_or_else(|| panic!("{code} needs a suggestion"));
        assert!(s.contains(needle), "{code}: {s}");
    }
    // 未知码没有兜底（返回 None 而不是胡编）
    let d = cage_core::Diagnostic {
        code: "E7777".to_string(),
        ..empty_diag()
    };
    assert!(suggestion_for(&d).is_none());
}

#[test]
fn load_error_recognizes_undeclared_env() {
    let v = load_error_json(
        "unknown environment 'qa' (declared: dev, prod, staging, test)",
        ".",
    );
    let s = v["error"]["suggestion"].as_str().expect("suggestion");
    assert!(s.contains("env_overrides"), "suggestion: {s}");
}

#[test]
fn precedence_chain_steps_are_numbered() {
    let chain = precedence_chain();
    let steps: Vec<u64> = chain.iter().filter_map(|s| s["step"].as_u64()).collect();
    assert_eq!(steps, (1..=8).collect::<Vec<u64>>());
}

/// 构造一个最小诊断（`suggestion_for` 只看 code，其余字段无意义）
fn empty_diag() -> cage_core::Diagnostic {
    cage_core::Diagnostic {
        code: String::new(),
        severity: cage_core::Severity::Error,
        title: None,
        source: String::new(),
        location: None,
        table: None,
        row: None,
        column: None,
        field: None,
        value: None,
        message: String::new(),
        hint: None,
        related: Vec::new(),
        metadata: indexmap::IndexMap::new(),
    }
}

/// 表格源的定位串：有行号给 `:row N`，行上的列追加 `:col`；无行号
/// 时裸列名也单独成立（文本源无行记录的场景）。
#[test]
fn diag_location_renders_row_then_column() {
    let d = cage_core::Diagnostic {
        code: "E1201".to_string(),
        source: "config/Item.xlsx".to_string(),
        location: Some(cage_core::SourceLocation {
            row: Some(3),
            column: Some("C".to_string()),
            ..cage_core::SourceLocation::new("config/Item.xlsx")
        }),
        ..empty_diag()
    };
    let v = diag_json(&d);
    assert_eq!(
        v["where"],
        json!("config/Item.xlsx:row 3:C"),
        "row then column: {v}"
    );

    let d = cage_core::Diagnostic {
        code: "E1201".to_string(),
        source: "config/Item.xlsx".to_string(),
        location: Some(cage_core::SourceLocation {
            column: Some("D".to_string()),
            ..cage_core::SourceLocation::new("config/Item.xlsx")
        }),
        ..empty_diag()
    };
    let v = diag_json(&d);
    assert_eq!(v["where"], json!("config/Item.xlsx:D"), "bare column: {v}");
}

/// `[registry]` / `[remote.*]` 的实际值进入解释输出，section 过滤
/// 同样成立。
#[test]
fn explain_config_reports_registry_and_remote_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_min_project(&root);
    std::fs::write(
            root.join("cage.toml"),
            "schema_path = \"schemas\"\n[project]\nname = \"dry\"\n[source_roots]\nmain = \"config\"\n[profiles.client]\nname = \"client\"\ntargets = [{ format = \"json\", output_dir = \"build/client/json\" }]\n[registry]\npath = \"vendor/reg\"\nauth_env = \"CAGE_REG_TOKEN\"\n[remote.sheets]\ncredential_env = \"CAGE_SHEETS_KEY\"\n[remote.mysql]\ndsn_env = \"CAGE_MYSQL_URL\"\nqueries = { top = \"SELECT id FROM Item\" }\n",
        )
        .unwrap();

    let v = run_explain_config(root.to_str().unwrap(), None).expect("loads");
    assert_eq!(v["registry"]["path"], json!("vendor/reg"));
    assert_eq!(v["registry"]["auth_env"], json!("CAGE_REG_TOKEN"));
    // remote 是带 name 的数组（IndexMap 保序展开）
    let remotes = v["remote"].as_array().expect("remote list");
    let sheets = remotes
        .iter()
        .find(|r| r["name"] == json!("sheets"))
        .expect("sheets entry");
    assert_eq!(sheets["credential_env"], json!("CAGE_SHEETS_KEY"));
    let mysql = remotes
        .iter()
        .find(|r| r["name"] == json!("mysql"))
        .expect("mysql entry");
    assert_eq!(mysql["dsn_env"], json!("CAGE_MYSQL_URL"));
    assert_eq!(mysql["queries"], json!({ "top": "SELECT id FROM Item" }));

    let v = run_explain_config(root.to_str().unwrap(), Some("remote")).expect("section");
    assert_eq!(v["section"], json!("remote"));
    assert!(v["value"].as_array().is_some_and(|a| !a.is_empty()));
}

/// target 格式没人认识时 dry run 在生成阶段报 Io 失败（校验已放行，
/// 生成器拒收）。
#[test]
fn build_dry_run_unknown_target_format_is_io_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_min_project(&root);
    std::fs::write(
            root.join("cage.toml"),
            "schema_path = \"schemas\"\n[project]\nname = \"dry\"\n[source_roots]\nmain = \"config\"\n[profiles.client]\nname = \"client\"\ntargets = [{ format = \"parquet\", output_dir = \"build/client/parquet\" }]\n",
        )
        .unwrap();
    let err = run_build_dry_run(root.to_str().unwrap(), "client", None, "semantic", false)
        .expect_err("unknown format refused");
    assert!(err.contains("unsupported target format 'parquet'"), "{err}");
}

#[test]
fn list_fields_without_table_lists_all_tables() {
    let v = run_list_fields(example_project().to_str().unwrap(), None, false).expect("loads");
    assert!(v["table_count"].as_u64().unwrap() >= 6, "all tables: {v}");
    let names: Vec<&str> = v["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Item") && names.contains(&"Shop"),
        "{names:?}"
    );
}

#[test]
fn source_root_kind_classifies_roots() {
    assert_eq!(source_root_kind("registry:reg"), "registry");
    assert_eq!(source_root_kind("mysql:db"), "database");
    assert_eq!(source_root_kind("pg:db"), "database");
    assert_eq!(source_root_kind("gsheet:spreadsheet"), "sheets");
    assert_eq!(source_root_kind("https://example.com/data"), "http");
    assert_eq!(source_root_kind("config"), "local");
}

#[test]
fn location_string_covers_sheet_line_row_and_column() {
    let loc = |sheet: Option<&str>, line: Option<usize>, row: Option<usize>, col: Option<usize>| {
        let mut location = cage_core::value::SourceLocation::new("data/Item.csv");
        if let Some(s) = sheet {
            location = location.with_sheet(s);
        }
        if let Some(r) = row {
            location = location.with_row(r);
        }
        if let Some(l) = line {
            location = location.with_line_col(l, 1);
        }
        if let Some(c) = col {
            location = location.with_column(c.to_string());
        }
        let d = cage_core::Diagnostic {
            code: "E1201".to_string(),
            source: "data/Item.csv".to_string(),
            location: Some(location),
            ..empty_diag()
        };
        location_string(&d)
    };
    assert!(loc(None, None, None, None).contains("data/Item.csv"));
    assert!(loc(Some("Sheet1"), None, None, None).contains("[Sheet1]"));
    assert!(loc(None, Some(3), None, None).contains(":3"));
    assert!(loc(None, None, Some(4), None).contains(":row 4"));
    assert!(loc(None, None, Some(4), Some(5)).contains(":5"));
    assert!(loc(None, None, None, Some(7)).contains(":7"));
}

#[test]
fn load_error_json_recognizes_all_suggestion_families() {
    let cases = [
        (
            "unknown environment 'qa' (declared: dev, prod)",
            "env_overrides",
        ),
        ("declares no environments", "env_overrides"),
        (
            "invalid env_overrides: type cannot be changed",
            "env_overrides",
        ),
        ("path not found: config/x.csv", "路径不存在"),
        ("no such file or directory", "路径不存在"),
        ("目录不存在：/tmp/nope", "路径不存在"),
    ];
    for (msg, needle) in cases {
        let v = load_error_json(msg, ".");
        let s = v["error"]["suggestion"].as_str().expect("suggestion");
        assert!(s.contains(needle), "{msg}: {s}");
    }
    let v = load_error_json("unknown profile 'x'", ".");
    assert!(v["error"]["suggestion"].is_null());
}

#[test]
fn build_dry_run_unknown_profile_and_generate_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_min_project(&root);
    let err = run_build_dry_run(root.to_str().unwrap(), "server", None, "semantic", false)
        .expect_err("unknown profile");
    assert!(err.contains("未知 profile"), "err: {err}");
    assert!(err.contains("client"), "alternatives: {err}");

    // Unsupported target format → hard error (the Io branch of the
    // generator), still surfaced as an Err string.
    std::fs::write(
            root.join("cage.toml"),
            "schema_path = \"schemas\"\n[project]\nname = \"dry\"\n[source_roots]\nmain = \"config\"\n[profiles.client]\nname = \"client\"\ntargets = [{ format = \"unsupported\", output_dir = \"build/client/x\" }]\n",
        )
        .unwrap();
    let err = run_build_dry_run(root.to_str().unwrap(), "client", None, "semantic", false)
        .expect_err("unsupported format is a hard error");
    assert!(err.contains("unsupported target format"), "err: {err}");
}

#[test]
fn print_tools_returns_zero() {
    assert_eq!(print_tools(), 0);
}

#[test]
fn suggestion_covers_every_code_family() {
    let codes = [
        parse::E0001,
        parse::E0002,
        parse::E0003,
        parse::E0004,
        schema_codes::E1001,
        schema_codes::E1002,
        schema_codes::E1003,
        schema_codes::E1004,
        schema_codes::E1005,
        type_val::E1101,
        type_val::E1102,
        type_val::E1103,
        type_val::E1104,
        value_codes::E1201,
        value_codes::E1202,
        value_codes::E1203,
        value_codes::E1204,
        value_codes::E1205,
        table::E1301,
        table::E1302,
        table::E1303,
        table::E1304,
        table::E1305,
        ref_codes::E1401,
        ref_codes::E1402,
        ref_codes::E1403,
        ref_codes::E1404,
        ref_codes::E1410,
        ref_codes::E1411,
        semantic::E1501,
        semantic::E1502,
        semantic::E1503,
        gamerule::E1601,
        gamerule::E1602,
        gamerule::E1603,
        gamerule::E1604,
        gamerule::E1605,
        editor::E1701,
        registry::E1801,
        registry::E1802,
        registry::E1803,
        remote::E1901,
        remote::E1902,
        remote::E1903,
        remote::E1904,
        remote::E1905,
        remote::E1906,
        build::E9001,
        build::E9002,
        build::E9003,
        build::E9004,
        build::E9005,
        build::E9006,
        internal::E9901,
        internal::E9902,
        internal::E9903,
        internal::E9904,
    ];
    for code in codes {
        let d = cage_core::Diagnostic {
            code: code.to_string(),
            ..empty_diag()
        };
        assert!(
            suggestion_for(&d).is_some(),
            "{code} should have a suggestion"
        );
    }
}

#[test]
fn section_key_normalizes_aliases_and_find_config_prefers_toml() {
    assert_eq!(section_key("sources"), "source_roots");
    assert_eq!(section_key("source"), "source_roots");
    assert_eq!(section_key("deps"), "dependencies");
    assert_eq!(section_key("output-dir"), "output_dir");
    assert_eq!(section_key("schema-path"), "schema_path");
    assert_eq!(section_key("profiles"), "profiles");

    let dir = tempfile::tempdir().expect("tempdir");
    assert!(find_config_file(dir.path()).is_err());
    std::fs::write(dir.path().join("cage.yaml"), "x").unwrap();
    assert_eq!(find_config_file(dir.path()).unwrap(), "cage.yaml");
    std::fs::write(dir.path().join("cage.toml"), "x").unwrap();
    assert_eq!(find_config_file(dir.path()).unwrap(), "cage.toml");
}

#[test]
fn render_pretty_json_and_tool_entries_run_through_macros() {
    let v = json!({"ok": true, "n": 1});
    assert!(render(&v).contains("{\n  \"ok\": true"));
    // The tool methods are thin wrappers over run_*; drive the success
    // and error arms through a real project.
    let dir = tempfile::tempdir().expect("tempdir");
    write_min_project(dir.path());
    let server = CageMcpServer;
    let s = server.check_schema(rmcp::handler::server::wrapper::Parameters(
        params::CheckSchema {
            path: Some(dir.path().to_str().unwrap().to_string()),
            profile: None,
            env: None,
            level: None,
            no_cache: false,
        },
    ));
    assert!(s.contains("\"ok\": true"), "check_schema: {s}");
    let s = server.check_schema(rmcp::handler::server::wrapper::Parameters(
        params::CheckSchema {
            path: Some("/nope".to_string()),
            profile: None,
            env: None,
            level: None,
            no_cache: false,
        },
    ));
    assert!(s.contains("\"ok\": false"), "error arm: {s}");

    // Drive the remaining tool methods through the macro wrappers so
    // every #[tool] arm gets real execution, success and error sides.
    let root = Some(dir.path().to_str().unwrap().to_string());
    let ok = server.list_tables(rmcp::handler::server::wrapper::Parameters(
        params::ListTables {
            path: root.clone(),
            no_cache: false,
        },
    ));
    assert!(ok.contains("\"tables\""), "list_tables: {ok}");
    let err = server.list_tables(rmcp::handler::server::wrapper::Parameters(
        params::ListTables {
            path: Some("/nope".into()),
            no_cache: false,
        },
    ));
    assert!(err.contains("\"ok\": false"), "list_tables err: {err}");

    let ok = server.list_fields(rmcp::handler::server::wrapper::Parameters(
        params::ListFields {
            path: root.clone(),
            table: None,
            no_cache: false,
        },
    ));
    assert!(ok.contains("\"tables\""), "list_fields: {ok}");
    let err = server.list_fields(rmcp::handler::server::wrapper::Parameters(
        params::ListFields {
            path: Some("/nope".into()),
            table: None,
            no_cache: false,
        },
    ));
    assert!(err.contains("\"ok\": false"), "list_fields err: {err}");

    let ok = server.list_envs(rmcp::handler::server::wrapper::Parameters(
        params::ListEnvs {
            path: root.clone(),
            no_cache: false,
        },
    ));
    assert!(ok.contains("\"declared\""), "list_envs: {ok}");
    let err = server.list_envs(rmcp::handler::server::wrapper::Parameters(
        params::ListEnvs {
            path: Some("/nope".into()),
            no_cache: false,
        },
    ));
    assert!(err.contains("\"ok\": false"), "list_envs err: {err}");

    let ok = server.build_dry_run(rmcp::handler::server::wrapper::Parameters(
        params::BuildDryRun {
            path: root.clone(),
            profile: None,
            env: None,
            level: None,
            no_cache: false,
        },
    ));
    assert!(ok.contains("\"artifact_count\""), "build_dry_run: {ok}");
    let err = server.build_dry_run(rmcp::handler::server::wrapper::Parameters(
        params::BuildDryRun {
            path: Some("/nope".into()),
            profile: None,
            env: None,
            level: None,
            no_cache: false,
        },
    ));
    assert!(err.contains("\"ok\": false"), "build_dry_run err: {err}");

    let ok = server.explain_config(rmcp::handler::server::wrapper::Parameters(
        params::ExplainConfig {
            path: root.clone(),
            section: None,
        },
    ));
    assert!(ok.contains("precedence"), "explain_config: {ok}");
    let err = server.explain_config(rmcp::handler::server::wrapper::Parameters(
        params::ExplainConfig {
            path: Some("/nope".into()),
            section: None,
        },
    ));
    assert!(err.contains("\"ok\": false"), "explain_config err: {err}");
}
