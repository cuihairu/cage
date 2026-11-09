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

use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};

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

mod params;
#[cfg(test)]
mod tests;
mod tools;
mod views;

use tools::{
    run_build_dry_run, run_check_schema, run_explain_config, run_list_envs, run_list_fields,
    run_list_tables,
};
use views::{load_error_json, render};
