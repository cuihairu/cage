//! MCP tool parameter structs (serde + schemars, one per tool) — the
//! wire shape the `#[tool]` handlers receive (split out of mcp.rs,
//! LAY-026).

use rmcp::schemars;
use serde::Deserialize;

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
