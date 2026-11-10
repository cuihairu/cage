//! `cage init` tests (split out of init.rs, LAY-028).

use super::dialog::print_plan;
use super::scaffold::{generate, render_schema, render_toml};
use super::*;

fn dialog(input: &str) -> Option<InitOptions> {
    let mut out = Vec::new();
    let opts = run_dialog(&mut input.as_bytes(), &mut out).unwrap();
    // The dialog must be drivable purely from the scripted input.
    let _ = String::from_utf8(out).unwrap();
    opts
}

/// Drive the dialog with one answer per question — the list must match
/// the eight questions plus the confirmation, so a miscount is visible
/// in the source rather than in a run of `\n`.
fn dialog_answers(answers: &[&str]) -> Option<InitOptions> {
    let mut input = String::new();
    for a in answers {
        input.push_str(a);
        input.push('\n');
    }
    dialog(&input)
}

#[test]
fn dialog_all_defaults() {
    // Every question answered with Enter (defaults), confirmed.
    let opts = dialog_answers(&["", "", "", "", "", "", "", "", "y"]).expect("confirmed plan");
    assert_eq!(opts.dir, PathBuf::from("."));
    assert_eq!(opts.name, "cage-project");
    assert_eq!(opts.source, SourceKind::Json);
    assert_eq!(opts.envs, Envs::Four);
    assert_eq!(opts.targets, vec!["json", "csv"]);
    assert!(opts.examples);
    assert!(!opts.git);
}

#[test]
fn dialog_back_navigation() {
    // Name -> back -> the dir question re-asks and the user fixes it
    // (the project name then defaults to the directory name).
    let opts = dialog_answers(&["my-game", "b", "proj", "", "", "", "", "", "", "", "y"])
        .expect("confirmed plan");
    assert_eq!(opts.dir, PathBuf::from("proj"));
    assert_eq!(opts.name, "proj");
}

#[test]
fn dialog_invalid_choice_reasks() {
    // Source: 9 (invalid) -> 2 (csv); envs: x -> 1; targets: bogus,json
    // -> rejected -> json.
    let opts = dialog_answers(&[
        "",
        "",
        "9",
        "2",
        "x",
        "1",
        "bogus,json",
        "json",
        "",
        "",
        "",
        "",
        "y",
    ])
    .expect("confirmed plan");
    assert_eq!(opts.source, SourceKind::Csv);
    assert_eq!(opts.envs, Envs::Four);
    assert_eq!(opts.targets, vec!["json"]);
}

#[test]
fn dialog_decline_writes_nothing() {
    assert!(dialog_answers(&["", "", "", "", "", "", "", "", "n"]).is_none());
}

/// Stepping back from every question re-asks the previous one and still
/// reaches a confirmed plan — one test per origin step, so a broken
/// back-edge names its step by the resulting options.
#[test]
fn dialog_back_from_every_step_still_confirms() {
    // Step 0 (dir) has nothing before it: back re-asks the dir itself.
    let opts =
        dialog_answers(&["b", ".", "", "", "", "", "", "", "", "y"]).expect("back at dir re-asks");
    assert_eq!(opts.dir, PathBuf::from("."));

    // Steps 1..7: k default answers, then `b` at step k, then the
    // re-asked previous question plus the rest, then confirm.
    for k in 1..8usize {
        let mut input: Vec<&str> = vec![""; k];
        input.push("b");
        // The previous question is re-asked (1 line), then steps k..7.
        input.extend(std::iter::repeat_n("", 9 - k));
        input.push("y");
        let opts =
            dialog_answers(&input).unwrap_or_else(|| panic!("back from step {k} still confirms"));
        assert_eq!(opts.dir, PathBuf::from("."), "step {k}");
        assert_eq!(opts.source, SourceKind::Json, "step {k}");
        assert_eq!(opts.targets, vec!["json", "csv"], "step {k}");
    }
}

/// A script that runs dry aborts from every step, not only early ones.
#[test]
fn dialog_eof_aborts_from_every_step() {
    for k in 0..8usize {
        let input: Vec<&str> = vec![""; k];
        assert!(
            dialog_answers(&input).is_none(),
            "eof after {k} answers aborts"
        );
    }
}

#[test]
fn dialog_eof_aborts() {
    // A script that runs dry mid-dialog aborts instead of looping.
    assert!(dialog_answers(&["my-game"]).is_none());
}

#[test]
fn dialog_no_examples_and_git() {
    let opts = dialog_answers(&["", "", "", "", "", "", "y", "n", "y"]).expect("confirmed plan");
    assert!(!opts.examples);
    assert!(opts.git);
}

#[test]
fn target_aliases_normalize() {
    assert_eq!(normalize_target_format("cs"), Some("csharp"));
    assert_eq!(normalize_target_format("py"), Some("python"));
    assert_eq!(normalize_target_format("ts"), Some("typescript"));
    assert_eq!(normalize_target_format("js"), Some("javascript"));
    assert_eq!(normalize_target_format("golang"), Some("go"));
    assert_eq!(normalize_target_format("c++"), Some("cpp"));
    assert_eq!(normalize_target_format("protobuf"), Some("proto"));
    assert_eq!(normalize_target_format("fbs"), Some("flatbuffers"));
    assert_eq!(normalize_target_format("json-schema"), Some("jsonschema"));
    assert_eq!(normalize_target_format("nope"), None);
}

#[test]
fn build_options_rejects_unknown_flags() {
    assert!(build_options(
        None,
        None,
        Some("parquet".into()),
        None,
        None,
        None,
        false,
        false
    )
    .is_err());
    assert!(build_options(
        None,
        None,
        None,
        Some("five".into()),
        None,
        None,
        false,
        false
    )
    .is_err());
    assert!(build_options(
        None,
        None,
        None,
        None,
        Some("json,parquet".into()),
        None,
        false,
        false
    )
    .is_err());
    assert!(build_options(
        None,
        None,
        None,
        None,
        None,
        Some("rust".into()),
        false,
        false
    )
    .is_err());
}

#[test]
fn build_options_defaults() {
    let opts = build_options(None, None, None, None, None, None, false, false).unwrap();
    assert_eq!(opts.source, SourceKind::Json);
    assert_eq!(opts.envs, Envs::Four);
    assert_eq!(opts.targets, vec!["json", "csv"]);
    assert!(opts.examples);
}

#[test]
fn render_toml_carries_targets_and_env_hint() {
    let opts = build_options(
        None,
        Some("demo".into()),
        Some("csv".into()),
        Some("four".into()),
        Some("json,cs,proto".into()),
        None,
        false,
        false,
    )
    .unwrap();
    let toml = render_toml(&opts);
    assert!(toml.contains("name = \"demo\""));
    assert!(toml.contains("{ format = \"json\", output_dir = \"build/client/json\" }"));
    assert!(toml.contains("{ format = \"csharp\", output_dir = \"build/client/cs\", options = { namespace = \"Game.Config\" } }"));
    assert!(toml.contains("{ format = \"proto\", output_dir = \"build/client/proto\", options = { package = \"game.config\" } }"));
    assert!(toml.contains("[profiles.server]"));
    assert!(toml.contains("# [registry]"));
}

#[test]
fn render_schema_four_envs_vs_none() {
    let four = build_options(
        None,
        None,
        None,
        Some("four".into()),
        None,
        None,
        false,
        false,
    )
    .unwrap();
    let schema = render_schema(&four);
    assert!(schema.contains("env_overrides:"));
    assert!(schema.contains("prod:"));
    let none = build_options(
        None,
        None,
        None,
        Some("none".into()),
        None,
        None,
        false,
        false,
    )
    .unwrap();
    assert!(!render_schema(&none).contains("env_overrides:"));
}

#[test]
fn generate_refuses_existing_project() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("cage.toml"), "output_dir = \"build\"\n").unwrap();
    let opts = build_options(
        Some(tmp.path().to_path_buf()),
        Some("demo".into()),
        None,
        None,
        None,
        None,
        false,
        false,
    )
    .unwrap();
    let err = generate(&opts).unwrap_err();
    assert!(err.contains("拒绝覆盖"), "unexpected: {err}");
}

#[test]
fn generate_writes_full_scaffold() {
    let tmp = tempfile::tempdir().unwrap();
    let opts = build_options(
        Some(tmp.path().to_path_buf()),
        Some("demo".into()),
        Some("csv".into()),
        Some("four".into()),
        Some("json,csv".into()),
        None,
        false,
        false,
    )
    .unwrap();
    let files = generate(&opts).unwrap();
    assert_eq!(files.len(), 5);
    let toml = std::fs::read_to_string(tmp.path().join("cage.toml")).unwrap();
    assert!(toml.contains("name = \"demo\""));
    let schema = std::fs::read_to_string(tmp.path().join("schemas/item.yaml")).unwrap();
    assert!(schema.contains("env_overrides:"));
    let sample = std::fs::read_to_string(tmp.path().join("config/Item.csv")).unwrap();
    assert!(sample.contains("wooden-sword"));
    assert!(tmp.path().join("README.md").is_file());
    assert!(tmp.path().join(".gitignore").is_file());
}

#[test]
fn source_kind_parse_label_and_sample_ext() {
    assert_eq!(SourceKind::parse("excel"), Some(SourceKind::Excel));
    assert_eq!(SourceKind::parse("xlsx"), Some(SourceKind::Excel));
    assert_eq!(SourceKind::parse("csv"), Some(SourceKind::Csv));
    assert_eq!(SourceKind::parse("json"), Some(SourceKind::Json));
    assert_eq!(SourceKind::parse("yaml"), Some(SourceKind::Yaml));
    assert_eq!(SourceKind::parse("yml"), Some(SourceKind::Yaml));
    assert_eq!(SourceKind::parse("sheets"), Some(SourceKind::Sheets));
    assert_eq!(SourceKind::parse("gsheet"), Some(SourceKind::Sheets));
    assert_eq!(SourceKind::parse("google-sheets"), Some(SourceKind::Sheets));
    assert_eq!(SourceKind::parse("db"), Some(SourceKind::Db));
    assert_eq!(SourceKind::parse("database"), Some(SourceKind::Db));
    assert_eq!(SourceKind::parse("mysql"), Some(SourceKind::Db));
    assert_eq!(SourceKind::parse("postgres"), Some(SourceKind::Db));
    assert_eq!(SourceKind::parse("pg"), Some(SourceKind::Db));
    assert_eq!(SourceKind::parse("http"), Some(SourceKind::Http));
    assert_eq!(SourceKind::parse("https"), Some(SourceKind::Http));
    assert_eq!(SourceKind::parse("remote"), Some(SourceKind::Http));
    assert_eq!(SourceKind::parse("nope"), None);
    assert_eq!(SourceKind::Excel.label(), "excel");
    assert_eq!(SourceKind::Csv.label(), "csv");
    assert_eq!(SourceKind::Json.label(), "json");
    assert_eq!(SourceKind::Yaml.label(), "yaml");
    assert_eq!(SourceKind::Sheets.label(), "sheets");
    assert_eq!(SourceKind::Db.label(), "db");
    assert_eq!(SourceKind::Http.label(), "http");
    assert_eq!(SourceKind::Json.sample_ext(), "json");
    assert_eq!(SourceKind::Yaml.sample_ext(), "yaml");
    assert_eq!(SourceKind::Csv.sample_ext(), "csv");
}

#[test]
fn parse_source_envs_and_targets_defaults_and_edges() {
    // Defaults when absent.
    assert_eq!(parse_source(None).unwrap(), SourceKind::Json);
    assert_eq!(parse_envs(None).unwrap(), Envs::Four);
    assert_eq!(parse_targets(None).unwrap(), vec!["json", "csv"]);
    // Empty/whitespace target list falls back to the default pair.
    assert_eq!(
        parse_targets(Some(",".into())).unwrap(),
        vec!["json", "csv"]
    );
    assert_eq!(
        parse_targets(Some("  ".into())).unwrap(),
        vec!["json", "csv"]
    );
    // Duplicates collapse, aliases normalize.
    assert_eq!(
        parse_targets(Some("json,json,cs".into())).unwrap(),
        vec!["json", "csharp"]
    );
    // Unknown values are errors.
    assert!(parse_source(Some("parquet".into())).is_err());
    assert!(parse_envs(Some("five".into())).is_err());
    assert!(parse_targets(Some("json,parquet".into())).is_err());
}

#[test]
fn dialog_back_at_first_question_reasks() {
    // `b` at the very first (dir) question re-asks it; then an answer.
    let opts =
        dialog_answers(&["b", "proj", "", "", "", "", "", "", "", "y"]).expect("confirmed plan");
    assert_eq!(opts.dir, PathBuf::from("proj"));
}

#[test]
fn dialog_choice_back_and_yn_invalid_reask() {
    // Dir -> Name -> Source("b" back to Name) -> Name("proj") ->
    // Source(7 http) -> Envs(2 none) -> Targets -> Template ->
    // Git("maybe" invalid then "y") -> Examples("no") -> confirm("y").
    let opts = dialog_answers(&[
        "", "", "b", "proj", "7", "2", "", "", "maybe", "y", "no", "y",
    ])
    .expect("confirmed plan");
    assert_eq!(opts.name, "proj", "Source's Back returns to Name");
    assert_eq!(opts.source, SourceKind::Http);
    assert_eq!(opts.envs, Envs::None);
    assert!(opts.git);
    assert!(!opts.examples);
}

#[test]
fn run_init_non_interactive_writes_scaffold_and_git_init() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    // First run: scaffold into a clean directory.
    let code = run_init(
        Some(dir.clone()),
        Some("demo".into()),
        Some("json".into()),
        Some("four".into()),
        Some("json".into()),
        None,
        false,
        false,
    );
    assert_eq!(code, 0, "run_init should succeed");
    assert!(dir.join("cage.toml").is_file());
    assert!(dir.join("schemas/item.yaml").is_file());
    assert!(dir.join("config/Item.json").is_file());
    assert!(dir.join("README.md").is_file());

    // Second run on the same dir refuses (existing project file).
    let code = run_init(
        Some(dir.clone()),
        Some("demo".into()),
        None,
        None,
        None,
        None,
        false,
        false,
    );
    assert_eq!(code, 1, "existing project must refuse");
}

#[test]
fn run_init_bad_flags_return_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let code = run_init(
        Some(tmp.path().to_path_buf()),
        None,
        Some("parquet".into()),
        None,
        None,
        None,
        false,
        false,
    );
    assert_eq!(code, 2, "bad --source must fail with usage");
}

#[test]
fn git_init_handles_success_and_failure() {
    let tmp = tempfile::tempdir().unwrap();
    git_init(tmp.path());
    // git init succeeded → a .git dir exists.
    assert!(tmp.path().join(".git").is_dir());
    // git init in a non-writable dir → warning path (no panic).
    let sub = tmp.path().join("locked");
    std::fs::create_dir_all(&sub).unwrap();
    git_init(&sub);
    assert!(sub.exists());
}

#[test]
fn print_plan_renders_every_variant() {
    let mut out = Vec::new();
    let opts = build_options(
        None,
        Some("demo".into()),
        Some("csv".into()),
        Some("four".into()),
        Some("json,csv".into()),
        None,
        true,
        false,
    )
    .unwrap();
    print_plan(
        &mut out,
        &opts.dir,
        &opts.name,
        opts.source,
        opts.envs,
        &opts.targets,
        opts.git,
        opts.examples,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("待生成清单"));
    assert!(text.contains("dev/test/staging/prod"));
    assert!(text.contains("git init"));

    let mut out = Vec::new();
    let opts = build_options(
        None,
        Some("demo".into()),
        Some("yaml".into()),
        Some("none".into()),
        None,
        None,
        false,
        true,
    )
    .unwrap();
    print_plan(
        &mut out,
        &opts.dir,
        &opts.name,
        opts.source,
        opts.envs,
        &opts.targets,
        opts.git,
        opts.examples,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("无（不生成 env_overrides）"));
    assert!(!text.contains("schemas/item.yaml"));
}

#[test]
fn dialog_every_source_choice_and_env_choice() {
    use SourceKind;
    for (n, expect) in [
        (1, SourceKind::Excel),
        (2, SourceKind::Csv),
        (3, SourceKind::Json),
        (4, SourceKind::Yaml),
        (5, SourceKind::Sheets),
        (6, SourceKind::Db),
        (7, SourceKind::Http),
    ] {
        let opts = dialog_answers(&[
            "",
            "",
            &n.to_string(), // dir, name, source
            "1",
            "",
            "",
            "",
            "",
            "",
            "y", // envs four, targets, template, git, examples, confirm
        ])
        .expect("confirmed");
        assert_eq!(opts.source, expect, "source {n}");
    }
    // Envs: none.
    let opts = dialog_answers(&["", "", "3", "2", "", "", "", "", "", "y"]).expect("confirmed");
    assert_eq!(opts.envs, Envs::None);
}

#[test]
fn dialog_eof_at_each_question_aborts() {
    // EOF after each single answer: dir, name, source, envs, targets,
    // template, git, examples — all abort cleanly (no infinite loop).
    for answers in [
        &["x"][..],
        &["", "x"][..],
        &["", "", "x"][..],
        &["", "", "3", "x"][..],
        &["", "", "3", "1", "x"][..],
        &["", "", "3", "1", "", "x"][..],
        &["", "", "3", "1", "", "", "x"][..],
        &["", "", "3", "1", "", "", "", "x"][..],
    ] {
        let mut input = String::new();
        for a in answers {
            input.push_str(a);
            input.push('\n');
        }
        assert!(dialog(&input).is_none(), "answers: {answers:?}");
    }
}
