//! AI 助手（MCP）server — `cage mcp --stdio`.
//!
//! Wraps the *existing* check / build / inspect pipelines as MCP tools so an
//! AI client can drive the real compiler instead of guessing at its output:
//!
//! * `check_schema` — structured validation errors (code + `file:line` +
//!   actionable suggestion) instead of the human-facing render;
//! * `list_tables` / `list_fields` / `list_envs` — schema structure
//!   (and the per-environment override diff);
//! * `build_dry_run` — the artifact plan a build would write, without
//!   touching the disk;
//! * `explain_config` — what `cage.toml` means and the precedence chain
//!   every value resolves through.
//!
//! Everything is JSON: an AI can read the result and tell the user exactly
//! what to change. Nothing here re-implements validation or generation —
//! the tools call [`validate_project`], [`generate_target_items`] and the
//! same loaders `cage check` / `cage build` use.

use std::path::{Path, PathBuf};

use cage_core::error::codes::{editor, registry, remote};
use cage_core::manifest::{ManifestGenerator, ProjectConfig};
use cage_core::schema::{FieldSchema, ReferenceSchema, TableSchema};
use cage_core::{
    build, error_title, gamerule, internal, normalize_document, parse, ref_codes, schema_codes,
    semantic, table, type_val, value_codes, Diagnostic, Schema,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    filter_by_profile, generate_target_items, load_project, load_project_config, validate_project,
};

/// Tool argument shapes share one `path` convention: the configuration
/// project root (a directory holding cage.toml), default `.`.
mod params {
    use super::{schemars, Deserialize};

    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    pub(crate) struct CheckSchema {
        /// 配置工程根目录（含 cage.toml），默认当前目录
        #[serde(default)]
        pub path: Option<String>,
        /// 校验环境（表 `env_overrides` 里声明的名字，如 dev/test/staging/prod）
        #[serde(default)]
        pub env: Option<String>,
        /// 构建 profile（默认 client）
        #[serde(default)]
        pub profile: Option<String>,
        /// 最高校验级别（parse/schema/type/value/table/reference/semantic/gamerule，默认 semantic）
        #[serde(default)]
        pub level: Option<String>,
        /// 拒绝远程源离线缓存回退
        #[serde(default)]
        pub no_cache: bool,
    }

    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    pub(crate) struct ListTables {
        /// 配置工程根目录（含 cage.toml），默认当前目录
        #[serde(default)]
        pub path: Option<String>,
        /// 拒绝远程源离线缓存回退
        #[serde(default)]
        pub no_cache: bool,
    }

    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    pub(crate) struct ListFields {
        /// 配置工程根目录（含 cage.toml），默认当前目录
        #[serde(default)]
        pub path: Option<String>,
        /// 只看这张表；缺省返回全部表的字段
        #[serde(default)]
        pub table: Option<String>,
        /// 拒绝远程源离线缓存回退
        #[serde(default)]
        pub no_cache: bool,
    }

    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    pub(crate) struct ListEnvs {
        /// 配置工程根目录（含 cage.toml），默认当前目录
        #[serde(default)]
        pub path: Option<String>,
        /// 拒绝远程源离线缓存回退
        #[serde(default)]
        pub no_cache: bool,
    }

    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    pub(crate) struct BuildDryRun {
        /// 配置工程根目录（含 cage.toml），默认当前目录
        #[serde(default)]
        pub path: Option<String>,
        /// 构建 profile（默认 client）
        #[serde(default)]
        pub profile: Option<String>,
        /// 校验环境（表 `env_overrides` 里声明的名字）
        #[serde(default)]
        pub env: Option<String>,
        /// 最高校验级别（默认 semantic）
        #[serde(default)]
        pub level: Option<String>,
        /// 拒绝远程源离线缓存回退
        #[serde(default)]
        pub no_cache: bool,
    }

    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    pub(crate) struct ExplainConfig {
        /// 配置工程根目录（含 cage.toml），默认当前目录
        #[serde(default)]
        pub path: Option<String>,
        /// 只解释这一节（profiles / `source_roots` / registry / remote /
        /// dependencies / `output_dir` / `schema_path` / `warnings_as_errors`）；
        /// 缺省返回全部
        #[serde(default)]
        pub section: Option<String>,
    }
}

/// One `cage mcp --stdio` session: tools only, no state beyond the stdio pipe.
#[derive(Debug, Default)]
pub(crate) struct CageMcpServer;

// rmcp 的 `#[tool]` 宏要求方法带 `&self`，工具本身无状态。
#[allow(clippy::unused_self)]
#[tool_router]
impl CageMcpServer {
    #[tool(
        description = "校验配置工程（cage check 的结构化版）：返回错误码、file:line、原因与可操作建议；改完配置先跑它"
    )]
    fn check_schema(&self, Parameters(p): Parameters<params::CheckSchema>) -> String {
        let path = p.path.as_deref().unwrap_or(".");
        let profile = p.profile.as_deref().unwrap_or("client");
        let level = p.level.as_deref().unwrap_or("semantic");
        match run_check_schema(path, p.env.as_deref(), profile, level, p.no_cache) {
            Ok(v) => render(&v),
            Err(e) => render(&load_error_json(&e, path)),
        }
    }

    #[tool(
        description = "列出 schema 的全部表：主键、字段、行数、引用与各表声明的环境；先看结构再改数据"
    )]
    fn list_tables(&self, Parameters(p): Parameters<params::ListTables>) -> String {
        let path = p.path.as_deref().unwrap_or(".");
        match run_list_tables(path, p.no_cache) {
            Ok(v) => render(&v),
            Err(e) => render(&load_error_json(&e, path)),
        }
    }

    #[tool(
        description = "列出表的字段定义：类型、必填、默认值、范围/长度/正则、枚举、引用、profile 可见性与环境覆盖"
    )]
    fn list_fields(&self, Parameters(p): Parameters<params::ListFields>) -> String {
        let path = p.path.as_deref().unwrap_or(".");
        match run_list_fields(path, p.table.as_deref(), p.no_cache) {
            Ok(v) => render(&v),
            Err(e) => render(&load_error_json(&e, path)),
        }
    }

    #[tool(
        description = "列出全部已声明环境（--env 合法值）与各表的环境差异（env_overrides）；四套 dev/test/staging/prod 常见于此"
    )]
    fn list_envs(&self, Parameters(p): Parameters<params::ListEnvs>) -> String {
        let path = p.path.as_deref().unwrap_or(".");
        match run_list_envs(path, p.no_cache) {
            Ok(v) => render(&v),
            Err(e) => render(&load_error_json(&e, path)),
        }
    }

    #[tool(
        description = "预演构建（不写盘）：产物清单（路径/大小/格式/所属表）、目标列表与全部哈希；真实构建仍走 cage build"
    )]
    fn build_dry_run(&self, Parameters(p): Parameters<params::BuildDryRun>) -> String {
        let path = p.path.as_deref().unwrap_or(".");
        let profile = p.profile.as_deref().unwrap_or("client");
        let level = p.level.as_deref().unwrap_or("semantic");
        match run_build_dry_run(path, profile, p.env.as_deref(), level, p.no_cache) {
            Ok(v) => render(&v),
            Err(e) => render(&load_error_json(&e, path)),
        }
    }

    #[tool(
        description = "解释 cage.toml 每一节的语义、本项目实际取值，与配置优先级链（发现顺序/级别/profile/环境/源合并顺序）"
    )]
    fn explain_config(&self, Parameters(p): Parameters<params::ExplainConfig>) -> String {
        let path = p.path.as_deref().unwrap_or(".");
        match run_explain_config(path, p.section.as_deref()) {
            Ok(v) => render(&v),
            Err(e) => render(&load_error_json(&e, path)),
        }
    }
}

// `#[tool_handler]` 宏展开出 async trait impl，方法体没有 `.await`。
#[allow(clippy::unused_async_trait_impl)]
#[tool_handler(
    name = "cage",
    instructions = "Cage 配置编译器的 AI 助手：check_schema 校验并给结构化错误与修复建议；list_tables/list_fields/list_envs 看结构与环境差异；build_dry_run 预演构建产物；explain_config 解释 cage.toml 语义与优先级。工具参数 path 一律是配置工程根目录（含 cage.toml），默认当前目录。"
)]
impl ServerHandler for CageMcpServer {}

/// Entry: run the stdio server until the client disconnects. Returns the
/// process exit code.
pub(crate) fn serve_stdio() -> i32 {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build();
    let runtime = match runtime {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cage mcp: 无法启动运行时：{e}");
            return 1;
        }
    };
    runtime.block_on(async {
        match CageMcpServer.serve(stdio()).await {
            Ok(service) => match service.waiting().await {
                Ok(_quit) => 0,
                Err(e) => {
                    eprintln!("cage mcp: server 结束：{e}");
                    1
                }
            },
            Err(e) => {
                eprintln!("cage mcp: 无法在 stdio 上启动 server：{e}");
                1
            }
        }
    })
}

/// `cage mcp`（不带 --stdio）：打印工具清单，方便人先看有什么。
pub(crate) fn print_tools() -> i32 {
    println!("cage mcp —— AI 助手（MCP）server，stdio JSON-RPC");
    println!();
    println!("启动：cage mcp --stdio");
    println!();
    println!("工具（{} 个）：", 6);
    let tools: [(&str, &str); 6] = [
        ("check_schema", "校验配置：错误码 + file:line + 原因 + 建议"),
        ("list_tables", "全部表：主键/字段/行数/引用/环境"),
        ("list_fields", "字段定义：类型/必填/约束/枚举/引用/可见性"),
        ("list_envs", "已声明环境与各表 env_overrides 差异"),
        ("build_dry_run", "构建预演：产物清单与哈希，不写盘"),
        ("explain_config", "cage.toml 语义与配置优先级链"),
    ];
    for (name, desc) in tools {
        println!("  {name:<16} {desc}");
    }
    println!();
    println!("MCP 客户端配置片段与说明见 README「AI 助手（MCP）」。");
    0
}

// ---- tool implementations (also the unit-test surface) ----

fn run_check_schema(
    path: &str,
    env: Option<&str>,
    profile: &str,
    level: &str,
    no_cache: bool,
) -> Result<Value, String> {
    let root = resolve_root(path)?;
    let outcome = validate_project(&root, level, env, profile, no_cache)?;
    let diagnostics = &outcome.diagnostics;
    let errors: Vec<Value> = diagnostics.errors().iter().map(|d| diag_json(d)).collect();
    let warnings: Vec<Value> = diagnostics
        .warnings()
        .iter()
        .map(|d| diag_json(d))
        .collect();
    let ok = !diagnostics.has_errors();
    Ok(json!({
        "ok": ok,
        "path": root.display().to_string(),
        "profile": profile,
        "env": env,
        "level": level,
        "tables_checked": outcome.visible_tables,
        "error_count": errors.len(),
        "warning_count": warnings.len(),
        "errors": errors,
        "warnings": warnings,
        "next": if ok {
            format!("校验通过。构建产物：cage build {} --profile {profile}", root.display())
        } else {
            format!("先修上面的错误再构建；重跑校验：cage check {} --env {}", root.display(), env.unwrap_or("<环境>"))
        },
    }))
}

fn run_list_tables(path: &str, no_cache: bool) -> Result<Value, String> {
    let root = resolve_root(path)?;
    let project = load_project(&root, no_cache)?;
    let schema = &project.schema;
    let tables: Vec<Value> = schema
        .tables
        .values()
        .map(|t| table_json(t, project.document.tables.get(&t.name)))
        .collect();
    let enums: Vec<Value> = schema
        .enums
        .iter()
        .map(|(name, e)| {
            json!({
                "name": name,
                "description": e.description,
                "value_count": e.values.len(),
                "values": e.values.iter().map(|v| &v.name).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "path": root.display().to_string(),
        "table_count": tables.len(),
        "tables": tables,
        "enums": enums,
    }))
}

fn run_list_fields(path: &str, table: Option<&str>, no_cache: bool) -> Result<Value, String> {
    let root = resolve_root(path)?;
    let project = load_project(&root, no_cache)?;
    let schema = &project.schema;
    let selected: Vec<&TableSchema> = match table {
        Some(name) => {
            let t = schema.tables.get(name).ok_or_else(|| {
                format!(
                    "表 {name:?} 不在 schema 里（可用：{}）",
                    table_names(schema)
                )
            })?;
            vec![t]
        }
        None => schema.tables.values().collect(),
    };
    let tables: Vec<Value> = selected
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "primary_key": t.primary_key,
                "field_count": t.fields.len(),
                "fields": t.fields.values().map(field_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "path": root.display().to_string(),
        "table_count": tables.len(),
        "tables": tables,
    }))
}

fn run_list_envs(path: &str, no_cache: bool) -> Result<Value, String> {
    let root = resolve_root(path)?;
    let project = load_project(&root, no_cache)?;
    let schema = &project.schema;
    let declared: Vec<String> = schema.declared_envs().into_iter().collect();
    let mut tables = serde_json::Map::new();
    for (name, t) in &schema.tables {
        if t.env_overrides.is_empty() {
            continue;
        }
        let mut envs = serde_json::Map::new();
        for (env, fields) in &t.env_overrides {
            let mut field_map = serde_json::Map::new();
            for (field, patch) in fields {
                // FieldOverride 只带覆盖到的约束键，序列化即差异本身
                field_map.insert(
                    field.clone(),
                    serde_json::to_value(patch).unwrap_or(Value::Null),
                );
            }
            envs.insert(env.clone(), Value::Object(field_map));
        }
        tables.insert(
            name.clone(),
            json!({
                "primary_key": t.primary_key,
                "env_overrides": Value::Object(envs),
            }),
        );
    }
    Ok(json!({
        "path": root.display().to_string(),
        "declared": declared,
        "declared_count": schema.declared_envs().len(),
        "tables_with_overrides": tables.len(),
        "tables": Value::Object(tables),
        "how_to_run": {
            "check": "cage check <path> --env <环境>",
            "build": "cage build <path> --profile <profile> --env <环境>",
        },
        "note": "--env 只接受这里列出的已声明环境；未声明的名字会直接报错。环境覆盖整条替换字段约束（不改类型）。",
    }))
}

fn run_build_dry_run(
    path: &str,
    profile: &str,
    env: Option<&str>,
    level: &str,
    no_cache: bool,
) -> Result<Value, String> {
    let root = resolve_root(path)?;
    // 与 build_project 相同的顺序：先校验（失败给结构化错误，不进入生成）
    let outcome = validate_project(&root, level, env, profile, no_cache)?;
    if outcome.diagnostics.has_errors() {
        let errors: Vec<Value> = outcome
            .diagnostics
            .errors()
            .iter()
            .map(|d| diag_json(d))
            .collect();
        return Ok(json!({
            "ok": false,
            "stage": "validate",
            "path": root.display().to_string(),
            "profile": profile,
            "env": env,
            "error_count": errors.len(),
            "errors": errors,
            "next": "校验未通过，构建不会进行；修好错误后重跑 build_dry_run。",
        }));
    }
    let project = load_project(&root, no_cache)?;
    let project = super::project_with_env(project, env)?;
    let Some(build_profile) = project.config.profiles.get(profile) else {
        return Err(format!(
            "未知 profile {profile:?}（可用：{}）",
            project
                .config
                .profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let (schema, document) = filter_by_profile(&project.schema, &project.document, profile);
    let normalized = normalize_document(&document);
    let (schema_hash, source_hash) = ManifestGenerator::input_hashes(&schema, &normalized);

    // 逐 target 生成到内存——与 cage build 同一份 generate_target_items，
    // 只是不调用写盘路径。
    let mut artifacts: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    for target in &build_profile.targets {
        let items = match generate_target_items(target, &schema, &normalized, &schema_hash, &root) {
            Ok(items) => items,
            Err(super::BuildFailure::Validation(diags)) => {
                let errors: Vec<Value> = diags.errors().iter().map(|d| diag_json(d)).collect();
                return Ok(json!({
                    "ok": false,
                    "stage": "generate",
                    "path": root.display().to_string(),
                    "profile": profile,
                    "env": env,
                    "error_count": errors.len(),
                    "errors": errors,
                    "next": "目标生成失败；按错误修 schema/模板后重跑。",
                }));
            }
            Err(super::BuildFailure::Io(msg)) => return Err(msg),
            Err(super::BuildFailure::UpToDate { .. }) => {
                return Err("增量跳过路径不会出现在 dry run 里".to_string())
            }
        };
        for (rel_path, content) in items {
            let table = rel_path
                .rsplit('/')
                .next()
                .and_then(|f| f.rsplit_once('.'))
                .map(|(stem, _)| stem.to_string());
            artifacts.push((rel_path, content, target.format.clone(), table));
        }
    }
    // 与真实构建一致的规范顺序（路径排序）
    artifacts.sort_by(|a, b| a.0.cmp(&b.0));

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .with_environment(env)
    .with_targets(&build_profile.targets)
    .generate(&schema, &normalized, &artifacts);

    let artifact_list: Vec<Value> = artifacts
        .iter()
        .map(|(path, content, format, table)| {
            json!({
                "path": path,
                "size_bytes": content.len(),
                "format": format,
                "table": table,
            })
        })
        .collect();
    let targets: Vec<Value> = build_profile
        .targets
        .iter()
        .map(|t| json!({"format": t.format, "output_dir": t.output_dir}))
        .collect();
    Ok(json!({
        "ok": true,
        "path": root.display().to_string(),
        "profile": profile,
        "env": env,
        "level": level,
        "output_dir": project.config.output_dir.as_deref().unwrap_or("build"),
        "artifact_count": artifacts.len(),
        "artifacts": artifact_list,
        "targets": targets,
        "schema_hash": schema_hash,
        "source_hash": source_hash,
        "build_id": manifest.build_id,
        "table_hashes": manifest.table_hashes,
        "written": false,
        "next": format!("真实构建（写盘）：cage build {} --profile {profile}", root.display()),
    }))
}

fn run_explain_config(path: &str, section: Option<&str>) -> Result<Value, String> {
    let root = resolve_root(path)?;
    let config_file = find_config_file(&root)?;
    let config = load_project_config(&root)?;
    let full = config_json(&root, &config_file, &config);
    match section {
        None => Ok(full),
        Some(sec) => {
            let key = section_key(sec);
            let value = full.get(&key).cloned().ok_or_else(|| {
                format!("未知 section {sec:?}（可用：profiles / source_roots / registry / remote / dependencies / output_dir / schema_path / warnings_as_errors / precedence）")
            })?;
            Ok(json!({
                "path": root.display().to_string(),
                "section": key,
                "value": value,
            }))
        }
    }
}

// ---- JSON shaping helpers ----

fn render(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn resolve_root(path: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(path);
    if !root.is_dir() {
        return Err(format!(
            "目录不存在：{}（path 应为含 cage.toml 的工程根目录）",
            root.display()
        ));
    }
    Ok(root)
}

fn find_config_file(root: &Path) -> Result<String, String> {
    for name in ["cage.toml", "cage.yaml", "cage.yml", "cage.json"] {
        if root.join(name).is_file() {
            return Ok(name.to_string());
        }
    }
    Err(format!(
        "{} 下没有 cage.toml / cage.yaml / cage.yml / cage.json",
        root.display()
    ))
}

fn section_key(section: &str) -> String {
    // 常见别名归一（schema-draft 风格的写法也接受）
    match section {
        "sources" | "source" => "source_roots".to_string(),
        "deps" => "dependencies".to_string(),
        "output-dir" => "output_dir".to_string(),
        "schema-path" => "schema_path".to_string(),
        other => other.to_string(),
    }
}

fn config_json(root: &Path, config_file: &str, config: &ProjectConfig) -> Value {
    let profiles: serde_json::Map<String, Value> = config
        .profiles
        .iter()
        .map(|(name, p)| {
            (
                name.clone(),
                json!({
                    "name": p.name,
                    "description": p.description,
                    "targets": p.targets.iter().map(|t| json!({
                        "format": t.format,
                        "output_dir": t.output_dir,
                        "options": t.options,
                    })).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    let source_roots: Vec<Value> = config
        .source_roots
        .iter()
        .map(|(name, root)| {
            json!({
                "name": name,
                "path": root,
                "kind": source_root_kind(root),
            })
        })
        .collect();
    json!({
        "path": root.display().to_string(),
        "config_file": config_file,
        "project": {
            "name": config.project.name,
            "version": config.project.version,
        },
        "output_dir": config.output_dir.as_deref().unwrap_or("build"),
        "schema_path": config.schema_path,
        "source_roots": source_roots,
        "profiles": Value::Object(profiles),
        "dependencies": config.dependencies,
        "registry": config.registry.as_ref().map(|r| json!({
            "path": r.path,
            "auth_env": r.auth_env,
        })),
        "remote": config.remote.iter().map(|(name, r)| json!({
            "name": name,
            "dsn_env": r.dsn_env,
            "credential_env": r.credential_env,
            "queries": r.queries,
        })).collect::<Vec<_>>(),
        "warnings_as_errors": config.warnings_as_errors,
        "precedence": precedence_chain(),
    })
}

/// 配置解析的优先级链——`explain_config` 的静态部分，和 docs 里
/// 的约定一致（发现顺序 → 级别 → profile → 环境 → 源合并 → 产物）。
fn precedence_chain() -> Vec<Value> {
    [
        ("config_discovery", "配置文件发现顺序",
         "同一目录下 cage.toml > cage.yaml > cage.yml > cage.json，取第一个命中的；其余忽略。"),
        ("validation_level", "校验级别",
         "--level parse|schema|type|value|table|reference|semantic|gamerule，默认 semantic；逐级向下全跑。"),
        ("profile", "profile 选择",
         "--profile client|server|…（默认 client）。表/字段的 targets 决定该 profile 的可见性；空 targets = 全部可见。"),
        ("environment", "环境覆盖",
         "--env <已声明环境>。表级 env_overrides 的 key 是合法环境名（未声明的直接报错）；覆盖整条替换该字段约束，类型永不被环境改写。"),
        ("field_constraints", "字段约束取值",
         "基础 schema 约束 ← env 覆盖（required/min/max/min_length/max_length/pattern/enum_values…）。无 --env 时用基础约束，并只警告不报错的 lint 环境结构问题。"),
        ("source_merge", "源合并顺序",
         "source_roots 按 cage.toml 里的书写顺序装载；同名表后装的覆盖先装的；表名按文件名（CSV/JSON）或根键（YAML）。"),
        ("output", "产物落盘",
         "--output-dir 或 cage.toml output_dir（默认 build）；profile.targets 决定生成哪些格式到哪个子目录。"),
        ("incremental", "增量构建",
         "cage build --incremental 三层：整体指纹命中跳过 → 表哈希 + 依赖图传播 → target 指纹传播；确定性保证增量与全量字节一致。"),
    ]
        .iter()
        .enumerate()
        .map(|(i, (key, what, rule))| {
            json!({"step": i + 1, "key": key, "what": what, "rule": rule})
        })
        .collect()
}

fn source_root_kind(root: &str) -> &'static str {
    if root.starts_with("registry:") {
        "registry"
    } else if root.starts_with("mysql:") || root.starts_with("pg:") {
        "database"
    } else if root.starts_with("gsheet:") {
        "sheets"
    } else if super::remote::is_remote_root(root) {
        "http"
    } else {
        "local"
    }
}

fn table_names(schema: &Schema) -> String {
    schema.tables.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn table_json(t: &TableSchema, data: Option<&cage_core::value::Table>) -> Value {
    let references: Vec<Value> = t
        .fields
        .values()
        .filter_map(|f| f.reference.as_ref().map(|r| field_reference_json(f, r)))
        .collect();
    json!({
        "name": t.name,
        "description": t.description,
        "primary_key": t.primary_key,
        "field_count": t.fields.len(),
        "fields": t.fields.keys().collect::<Vec<_>>(),
        "row_count": data.map_or(0, |d| d.rows.len()),
        "targets": t.targets,
        "env_overrides": t.env_overrides.keys().collect::<Vec<_>>(),
        "unique_constraints": t.unique_constraints.iter().map(|u| u.name.clone()).collect::<Vec<_>>(),
        "references": references,
    })
}

fn field_json(f: &FieldSchema) -> Value {
    json!({
        "name": f.name,
        "type": serde_json::to_value(&f.field_type).unwrap_or(Value::Null),
        "description": f.description,
        "required": f.required,
        "default": f.default,
        "min": f.min,
        "max": f.max,
        "min_length": f.min_length,
        "max_length": f.max_length,
        "pattern": f.pattern,
        "enum_values": f.enum_values,
        "reference": f.reference.as_ref().map(reference_json),
        "targets": f.targets,
    })
}

fn reference_json(r: &ReferenceSchema) -> Value {
    json!({
        "table": r.table,
        "field": r.field,
        "cardinality": r.cardinality,
    })
}

fn field_reference_json(f: &FieldSchema, r: &ReferenceSchema) -> Value {
    let mut v = reference_json(r);
    if let Value::Object(map) = &mut v {
        map.insert("field_name".to_string(), json!(f.name));
    }
    v
}

/// 诊断 → 结构化 JSON：错误码、标题、severity、定位（file:line，文本
/// 源无行号时给 `file:row N[:col]`）、表/字段/值、消息（原因）与建议。
/// 建议优先用诊断自带的 hint，缺 hint 时按错误码族给一条可操作的话。
fn diag_json(d: &Diagnostic) -> Value {
    let loc = d.location.as_ref();
    json!({
        "code": d.code,
        "title": error_title(&d.code),
        "severity": d.severity.as_str().to_lowercase(),
        "file": d.source,
        "where": location_string(d),
        "line": loc.and_then(|l| l.line),
        "sheet": loc.and_then(|l| l.sheet.clone()),
        "row": loc.and_then(|l| l.row),
        "column": loc.and_then(|l| l.column.clone()),
        "table": d.table,
        "field": d.field,
        "value": d.value,
        "message": d.message,
        "suggestion": d.hint.clone().or_else(|| suggestion_for(d)),
    })
}

/// 复合定位串：`file:line`（文本源）或 `file:row N[:col]` / `file[sheet]:row N`
/// ——cage 的源定位以行/列为主，文本行号只在有记录时出现。
fn location_string(d: &Diagnostic) -> String {
    use std::fmt::Write as _;
    let loc = d.location.as_ref();
    let mut s = d.source.clone();
    if let Some(l) = loc {
        if let Some(sheet) = &l.sheet {
            let _ = write!(s, "[{sheet}]");
        }
        if let Some(line) = l.line {
            let _ = write!(s, ":{line}");
        } else if let Some(row) = l.row {
            let _ = write!(s, ":row {row}");
            if let Some(col) = &l.column {
                let _ = write!(s, ":{col}");
            }
        } else if let Some(col) = &l.column {
            let _ = write!(s, ":{col}");
        }
    }
    s
}

/// 按错误码族的兜底建议（诊断没带 hint 时）。目标：AI 读完就知道
/// 让用户改哪个文件哪一行。
fn suggestion_for(d: &Diagnostic) -> Option<String> {
    let s: &str = match d.code.as_str() {
        // L0 解析
        parse::E0001 => "源文件语法错误：按 message 指出的位置修文件本身（YAML/JSON 缩进、CSV 列数）。",
        parse::E0002 => "文件意外截断：补全或恢复文件。",
        parse::E0003 => "编码问题：把文件转成 UTF-8。",
        parse::E0004 => "结构不合法：YAML 映射/数组形状与预期不符，按 message 修。",
        // L1 schema：缺字段 / 多字段
        schema_codes::E1001 => "缺字段：给数据补上该字段（或这一列），或在 schema 里给该字段设默认值 default；若字段可缺，把 required 改为 false。",
        schema_codes::E1002 => "多余字段：schema 没定义它——从数据源删掉这一列/键，或在 schema 里补上定义。",
        schema_codes::E1003 => "schema 里字段重复定义：删掉重复的那条。",
        schema_codes::E1004 => "schema 定义非法：检查引用的枚举/规则/谓词是否存在、字段名是否拼对。",
        schema_codes::E1005 => "schema 文件缺失或不可读：确认 schema_path 指向存在的文件/目录。",
        // L2 类型：类型不符
        type_val::E1101 => "类型不符：把 schema 里该字段的 kind 改成数据实际类型（CSV 里的整数按 Int32/Int64 声明，不是 UInt），或把数据改成 schema 声明的类型。",
        type_val::E1102 => "无法转换成目标类型：改 kind 或改数据；Excel 布尔/文本混排最常见。",
        type_val::E1103 => "数值溢出：换更宽的类型（Int32→Int64）或收紧数据范围。",
        type_val::E1104 => "枚举变体不合法：在 schema 的枚举定义里补上该值，或改数据。",
        // L3 值约束
        value_codes::E1201 => "超出范围：把值收回 schema 的 min/max 区间，或按需放宽约束。",
        value_codes::E1202 => "字符串长度越界：调整数据长度或 min_length/max_length。",
        value_codes::E1203 => "不匹配正则：改数据或调整 pattern。",
        value_codes::E1204 => "值不在枚举内：补 enum_values 或改数据。",
        value_codes::E1205 => "数组长度越界：增删元素或调整 min_items/max_items。",
        // L4 表级
        table::E1301 => "主键重复：改掉或删掉重复行；主键由 schema 的 primary_key 决定。",
        table::E1302 => "唯一约束冲突：调整数据或 unique_constraints。",
        table::E1303 => "被引用的行缺失：补上目标行，或把引用值改成已存在的 id。",
        table::E1304 => "行顺序违反 order_by：按声明字段排序数据。",
        table::E1305 => "表为空但需要数据：补数据或放宽该约束。",
        // L5 引用
        ref_codes::E1401 => "引用目标不存在：确认被引用的表/行存在。表名按源文件名（Item.csv→表 Item）或 YAML 根键。",
        ref_codes::E1402 => "引用了已删除的实体：改指向有效目标。",
        ref_codes::E1403 => "循环引用：打破环（A→B→A）。",
        ref_codes::E1404 => "基数不符：one/optional 要单值（可空用 optional），many 要数组。",
        ref_codes::E1410 => "引用对象不满足谓词：检查 reference.predicate 引用的字段与写法。",
        ref_codes::E1411 => "引用对象字段违反约束：修目标行的那个字段。",
        // L6 语义
        semantic::E1501 => "断言表达式为假：调整数据或 assert 表达式；只警告可加 warning_only。",
        semantic::E1502 => "跨字段约束违反：按 message 修相关字段。",
        semantic::E1503 => "表达式求值错误（如除零）：改数据或表达式。",
        // L7 游戏规则
        gamerule::E1601 => "自定义校验器报错：看 message 里校验器给的具体原因。",
        gamerule::E1602 => "校验器未找到：确认插件名与注册表。",
        gamerule::E1603 => "校验器执行失败（panic/超时）：看校验器实现。",
        gamerule::E1604 => "技能引用了不允许类别的道具：把 item 改成允许类别的道具，或扩 allowed_categories。",
        gamerule::E1605 => "跨表引用成环：打破环（A→B→A）或移除该条引用。",
        // 编辑器交换
        editor::E1701 => "编辑器交换格式非法：检查 JSON/YAML 是否能解析成 Schema。",
        // registry
        registry::E1801 => "registry 发布冲突：换一个版本号重新 publish。",
        registry::E1802 => "registry 引用解析失败：确认包名/版本存在；source_roots 里的 registry:<包>[@版本] 需要 [registry] path 与 [dependencies] 的版本 pin。",
        registry::E1803 => "registry 条目账本校验失败：bundle 可能被篡改，重新导出/导入。",
        // 远程源
        remote::E1901 => "远程源抓取失败：检查网络/URL/超时；确有缓存时可容忍（E1906）。",
        remote::E1902 => "远程源认证失败（401/403）：检查凭证。",
        remote::E1903 => "远程源响应形状非法：确认首行是表头且返回行集。",
        remote::E1904 => "远程源凭证缺失：设置 [remote] 里 dsn_env/credential_env 指定的环境变量。",
        remote::E1905 => "远程源查询非法：只允许单条 SELECT 或已声明的具名查询。",
        remote::E1906 => "远程源不可达但返回了缓存副本：要强制联网用 --no-cache，要离线容忍保持默认。",
        // 构建/生成
        build::E9001 => "目标生成器不存在：检查 profile.targets 里 format 的拼写。",
        build::E9002 => "目标生成失败：模板变量不存在时，确认模板里引用的变量都在 schema/数据里；看 message 指出的模板位置。",
        build::E9003 => "非确定性构建：构建输入含随机因素（时间戳/随机迭代序）。",
        build::E9004 => "manifest 生成失败：看 message。",
        build::E9005 => "profile 不存在：用 explain_config 的 profiles 节看可用名字。",
        build::E9006 => "字段可见性冲突：profile 的 targets 过滤把结构上必需的字段滤掉了——调整表/字段的 targets。",
        // 内部
        internal::E9901 => "内部不变量被破坏：这可能是 cage 的 bug，请提 issue 附复现。",
        internal::E9902 => "读源文件 I/O 错误：检查文件权限与路径。",
        internal::E9903 => "配置错误：CLI 参数或文件缺失，按 message 修。",
        internal::E9904 => "插件加载失败：检查插件路径与格式。",
        _ => return None,
    };
    Some(s.to_string())
}

/// 装载失败（路径不存在 / 配置坏 / 环境未声明）也走 JSON：带建议，
/// AI 能直接转述。环境未声明是最常见的一种，给专门的修法。
fn load_error_json(err: &str, path: &str) -> Value {
    let suggestion = if err.starts_with("unknown environment")
        || err.contains("declares no environments")
    {
        Some("环境未声明：在某个表的 env_overrides 里声明这个环境（schemas/<表>.yaml 的 env_overrides.<环境>: {<字段>: {…}}），或改用已声明的环境名；list_envs 可查全部已声明环境。".to_string())
    } else if err.starts_with("invalid env_overrides") {
        Some("env_overrides 结构非法：环境覆盖只接受约束键（required/min/max/min_length/max_length/pattern/enum_values…），不能改类型。".to_string())
    } else if err.contains("path not found")
        || err.contains("no such file")
        || err.starts_with("目录不存在")
    {
        Some("路径不存在：检查 source_roots / schema_path / 工程根目录（path 应为含 cage.toml 的目录）。".to_string())
    } else {
        None
    };
    json!({
        "ok": false,
        "error": {
            "kind": "config",
            "code": null,
            "message": err,
            "suggestion": suggestion,
            "path": path,
        },
    })
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(v["tables_checked"], json!(5));
        assert_eq!(v["error_count"], json!(0));
    }

    #[test]
    fn check_schema_structures_a_broken_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        write_min_project(&root);
        // price=-5（E1201）+ price=oops（E1101 类型不符）
        std::fs::write(root.join("config/Item.csv"), "id,price\n1,-5\n2,oops\n").unwrap();
        let v = run_check_schema(root.to_str().unwrap(), None, "client", "semantic", false)
            .expect("loads");
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
        let v = run_list_fields(example_project().to_str().unwrap(), Some("Item"), false)
            .expect("loads");
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
}
