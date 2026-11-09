//! Project configuration model: build profiles, target/source/registry/remote config
//! and the project info shape (split out of mod.rs, LAY-013).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Profile configuration for build
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildProfile {
    /// Profile name (e.g., client / server)
    pub name: String,
    /// Target generators to run
    pub targets: Vec<TargetConfig>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Template variables available to targets
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub variables: IndexMap<String, String>,
}

/// Target configuration within a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetConfig {
    /// Output format (json, csv, csharp, etc.)
    pub format: String,
    /// Output directory
    pub output_dir: String,
    /// File name template, e.g., "{table}.json"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_template: Option<String>,
    /// Format-specific options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<IndexMap<String, serde_json::Value>>,
}

/// Project configuration (cage.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project identity
    pub project: ProjectInfo,
    /// Build profiles by name
    pub profiles: IndexMap<String, BuildProfile>,
    /// Logical source name -> path mapping
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub source_roots: IndexMap<String, String>,
    /// Directory containing schema files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_path: Option<String>,
    /// Whether warnings are escalated to errors
    #[serde(default, skip_serializing_if = "is_false")]
    pub warnings_as_errors: bool,
    /// Default output directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// Local Configuration Registry root (R-series): consumers that resolve
    /// `registry:<package>[@<version>]` source roots must declare it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryConfig>,
    /// Dependency pins (`[dependencies]` in cage.toml): registry package
    /// name → version requirement (`"1.2.0"`, `">=1.0, <2.0"`, `"^1.2"`,
    /// `"~1.2"`). A `registry:<package>` reference without an explicit
    /// `@<version>` resolves through this pin.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub dependencies: IndexMap<String, String>,
    /// Remote source connection settings (`[remote.<scheme>]` in
    /// cage.toml, design §45): only environment variable NAMES and named
    /// read-only queries live here — DSNs and keys stay out of the
    /// config file entirely.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub remote: IndexMap<String, RemoteSourceConfig>,
    /// The `[source.<format>]` meta-row convention tables (design §51).
    /// A container struct rather than flat `source_excel` fields: the
    /// documented key is the nested table `[source.excel]`, and only a
    /// matching struct makes an unknown sub-table error instead of
    /// silently meaning "no convention".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceConfig>,
}

/// The `[source.<format>]` tables in cage.toml (design §51).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Excel meta-row convention (`[source.excel]`). `None` is the
    /// baseline: every Excel file reads exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excel: Option<crate::convention::ExcelConvention>,
    /// Sheets meta-row convention (`[source.sheets]`, design §51 Q7).
    /// `None` is the baseline: every `gsheet:` source reads exactly as
    /// before (first fetched row = header, no convention rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheets: Option<crate::convention::SheetsConvention>,
}

impl ProjectConfig {
    /// The Excel meta-row convention, when the project declares one.
    pub fn source_excel(&self) -> Option<&crate::convention::ExcelConvention> {
        self.source.as_ref().and_then(|s| s.excel.as_ref())
    }

    /// The Sheets meta-row convention, when the project declares one.
    pub fn source_sheets(&self) -> Option<&crate::convention::SheetsConvention> {
        self.source.as_ref().and_then(|s| s.sheets.as_ref())
    }
}

/// Local Configuration Registry declaration (`[registry]` in cage.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Registry root directory. A relative path is resolved against the
    /// project root — the consumer project pins which registry it reads.
    pub path: String,
    /// Name of the environment variable carrying the bearer token that
    /// `cage registry push` sends to the remote root (§47 A3). The config
    /// carries only the *name* — the token resolves from the environment at
    /// push time (missing → E2105), never logged, never stored anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_env: Option<String>,
}

/// Remote source connection settings (`[remote.<scheme>]`, design §45).
/// The config file carries only the *names* of environment variables —
/// DSNs and credentials resolve from the environment at load time
/// (missing → E1904, never guessed, never logged).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteSourceConfig {
    /// Name of the env var carrying the DSN / connection string
    /// (`[remote.mysql] dsn_env = "CAGE_MYSQL_URL"`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsn_env: Option<String>,
    /// Name of the env var carrying a credential — API key or service
    /// account JSON path (Google Sheets, S3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_env: Option<String>,
    /// Named read-only queries: `mysql:top_items` resolves here first,
    /// then falls back to a table-name expansion
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub queries: IndexMap<String, String>,
}

/// Project identity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Project name
    pub name: String,
    /// Project version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for ProjectConfig {
    fn default() -> Self {
        let mut profiles = IndexMap::new();
        profiles.insert(
            "client".to_string(),
            BuildProfile {
                name: "client".to_string(),
                targets: vec![TargetConfig {
                    format: "json".to_string(),
                    output_dir: "build/client".to_string(),
                    file_template: Some("{table}.json".to_string()),
                    options: None,
                }],
                description: Some("Client build profile".to_string()),
                variables: IndexMap::new(),
            },
        );
        profiles.insert(
            "server".to_string(),
            BuildProfile {
                name: "server".to_string(),
                targets: vec![TargetConfig {
                    format: "json".to_string(),
                    output_dir: "build/server".to_string(),
                    file_template: Some("{table}.json".to_string()),
                    options: None,
                }],
                description: Some("Server build profile".to_string()),
                variables: IndexMap::new(),
            },
        );

        Self {
            project: ProjectInfo {
                name: "game".to_string(),
                version: Some("0.1.0".to_string()),
                description: None,
            },
            profiles,
            source_roots: IndexMap::new(),
            schema_path: Some("schemas".to_string()),
            warnings_as_errors: false,
            output_dir: Some("build".to_string()),
            registry: None,
            dependencies: IndexMap::new(),
            remote: IndexMap::new(),
            source: None,
        }
    }
}
