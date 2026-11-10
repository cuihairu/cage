//! `cage init` — scaffold a new configuration project.
//!
//! Two modes: with a terminal on stdin and no flags, an interactive Q&A
//! walks through the choices (every question can go back with `b` or be
//! skipped with Enter for the default); without a terminal (or with any
//! flag) the flags alone drive generation, so CI and scripts can run it
//! unattended.
//!
//! The generated `cage.toml` follows the same shape as
//! `examples/game-config/cage.toml` — there is deliberately no second
//! configuration format.

use std::io::IsTerminal;
use std::path::PathBuf;

/// Target generators offered by `cage init`, mirroring the formats the
/// build dispatches on (`code_target_items` in main.rs) — the same
/// catalog the game-config example builds.
///
/// One catalog row: (format, output subdir, default options shown in the
/// scaffold).
type TargetEntry = (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
);

const TARGET_CATALOG: &[TargetEntry] = &[
    // (format, output subdir, default options shown in the scaffold)
    ("json", "json", None),
    ("csv", "csv", None),
    ("msgpack", "msgpack", None),
    ("proto", "proto", Some(("package", "game.config"))),
    ("flatbuffers", "fbs", Some(("namespace", "game.config"))),
    ("csharp", "cs", Some(("namespace", "Game.Config"))),
    ("python", "py", None),
    ("lua", "lua", None),
    ("typescript", "ts", Some(("enums_file", "cage_enums.ts"))),
    ("javascript", "js", None),
    ("cpp", "cpp", Some(("namespace", "game::config"))),
    ("go", "go", Some(("package", "config"))),
    ("java", "java", Some(("package", "game.config"))),
    ("jsonschema", "jsonschema", None),
    ("template", "template", None),
];

/// Source types the scaffold can be seeded with. Excel/sheets/db/http
/// cannot ship a zero-touch sample (binary workbook / external service),
/// so those keep a local text sample plus a commented config stanza —
/// the scaffold still checks clean out of the box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SourceKind {
    Excel,
    Csv,
    Json,
    Yaml,
    Sheets,
    Db,
    Http,
}

impl SourceKind {
    fn parse(raw: &str) -> Option<SourceKind> {
        match raw.to_lowercase().as_str() {
            "excel" | "xlsx" => Some(SourceKind::Excel),
            "csv" => Some(SourceKind::Csv),
            "json" => Some(SourceKind::Json),
            "yaml" | "yml" => Some(SourceKind::Yaml),
            "sheets" | "gsheet" | "google-sheets" => Some(SourceKind::Sheets),
            "db" | "database" | "mysql" | "postgres" | "pg" => Some(SourceKind::Db),
            "http" | "https" | "remote" => Some(SourceKind::Http),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SourceKind::Excel => "excel",
            SourceKind::Csv => "csv",
            SourceKind::Json => "json",
            SourceKind::Yaml => "yaml",
            SourceKind::Sheets => "sheets",
            SourceKind::Db => "db",
            SourceKind::Http => "http",
        }
    }

    /// Extension of the sample data file written under `config/`.
    fn sample_ext(self) -> &'static str {
        match self {
            SourceKind::Json => "json",
            SourceKind::Yaml => "yaml",
            _ => "csv",
        }
    }
}

/// Environment sets: the built-in dev/test/staging/prod four-pack (the
/// same set the game-config example declares) or none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Envs {
    Four,
    None,
}

impl Envs {
    fn parse(raw: &str) -> Option<Envs> {
        match raw.to_lowercase().as_str() {
            "four" | "4" | "dev,test,staging,prod" => Some(Envs::Four),
            "none" | "0" | "no" => Some(Envs::None),
            _ => None,
        }
    }
}

/// Everything the scaffold needs to write the project.
#[derive(Debug)]
pub(crate) struct InitOptions {
    pub dir: PathBuf,
    pub name: String,
    pub source: SourceKind,
    pub envs: Envs,
    pub targets: Vec<&'static str>,
    pub git: bool,
    pub examples: bool,
}

/// Resolve a target-format spelling (aliases included) to a canonical
/// catalog entry.
pub(crate) fn normalize_target_format(raw: &str) -> Option<&'static str> {
    match raw.to_lowercase().as_str() {
        "json" => Some("json"),
        "csv" => Some("csv"),
        "msgpack" => Some("msgpack"),
        "proto" | "protobuf" => Some("proto"),
        "flatbuffers" | "fbs" => Some("flatbuffers"),
        "cs" | "csharp" => Some("csharp"),
        "python" | "py" => Some("python"),
        "lua" => Some("lua"),
        "typescript" | "ts" => Some("typescript"),
        "javascript" | "js" => Some("javascript"),
        "cpp" | "c++" | "cxx" => Some("cpp"),
        "go" | "golang" => Some("go"),
        "java" => Some("java"),
        "jsonschema" | "json-schema" | "json_schema" => Some("jsonschema"),
        "template" => Some("template"),
        _ => None,
    }
}

fn catalog_entry(format: &str) -> Option<&TargetEntry> {
    TARGET_CATALOG.iter().find(|(f, _, _)| *f == format)
}

/// Entry point for `cage init`. Returns the process exit code.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_init(
    path: Option<PathBuf>,
    name: Option<String>,
    source: Option<String>,
    envs: Option<String>,
    targets: Option<String>,
    template: Option<String>,
    git: bool,
    no_examples: bool,
) -> i32 {
    // Interactive only when nothing was pinned by a flag and stdin is a
    // terminal; a piped/CI stdin must never hang on a prompt.
    let interactive = std::io::stdin().is_terminal()
        && path.is_none()
        && name.is_none()
        && source.is_none()
        && envs.is_none()
        && targets.is_none()
        && template.is_none()
        && !git
        && !no_examples;

    let opts = if interactive {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        match run_dialog(&mut stdin, &mut stdout) {
            Ok(Some(opts)) => opts,
            Ok(None) => {
                println!("已取消，未生成任何文件。");
                return 0;
            }
            Err(e) => {
                eprintln!("cage init: 交互问答失败：{e}");
                return 1;
            }
        }
    } else {
        match build_options(
            path,
            name,
            source,
            envs,
            targets,
            template,
            git,
            no_examples,
        ) {
            Ok(opts) => opts,
            Err(msg) => {
                eprintln!("cage init: {msg}");
                eprintln!(
                    "用法：cage init [目录] [--name N] [--source excel|csv|json|yaml|sheets|db|http] \
                     [--envs four|none] [--targets json,csv,...] [--template game] [--git] [--no-examples]"
                );
                return 2;
            }
        }
    };

    match generate(&opts) {
        Ok(files) => {
            for f in &files {
                println!("  生成 {}", f.display());
            }
            if opts.git {
                git_init(&opts.dir);
            }
            println!("完成：{}", opts.dir.display());
            println!(
                "下一步：cd {} && cage check . && cage build . --env dev",
                opts.dir.display()
            );
            0
        }
        Err(msg) => {
            eprintln!("cage init: {msg}");
            1
        }
    }
}

/// Parse `--source`, defaulting to a local JSON sample.
fn parse_source(raw: Option<String>) -> Result<SourceKind, String> {
    let Some(v) = raw else {
        return Ok(SourceKind::Json);
    };
    SourceKind::parse(&v)
        .ok_or_else(|| format!("未知配置源类型 {v:?}（可选：excel/csv/json/yaml/sheets/db/http）"))
}

/// Parse `--envs`, defaulting to the built-in four environments.
fn parse_envs(raw: Option<String>) -> Result<Envs, String> {
    let Some(v) = raw else {
        return Ok(Envs::Four);
    };
    Envs::parse(&v).ok_or_else(|| format!("未知环境套 {v:?}（可选：four/none）"))
}

/// Parse `--targets` (comma-separated, aliases allowed), defaulting to
/// the json + csv pair the example builds.
fn parse_targets(raw: Option<String>) -> Result<Vec<&'static str>, String> {
    let Some(raw) = raw else {
        return Ok(vec!["json", "csv"]);
    };
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match normalize_target_format(part) {
            Some(fmt) => {
                if !out.contains(&fmt) {
                    out.push(fmt);
                }
            }
            None => {
                return Err(format!(
                    "未知目标生成器 {part:?}（可选：{}）",
                    TARGET_CATALOG
                        .iter()
                        .map(|(f, _, _)| *f)
                        .collect::<Vec<_>>()
                        .join("/")
                ))
            }
        }
    }
    Ok(if out.is_empty() {
        vec!["json", "csv"]
    } else {
        out
    })
}

/// Assemble options from flags (or defaults) for the non-interactive path.
#[allow(clippy::too_many_arguments)]
fn build_options(
    path: Option<PathBuf>,
    name: Option<String>,
    source: Option<String>,
    envs: Option<String>,
    targets: Option<String>,
    template: Option<String>,
    git: bool,
    no_examples: bool,
) -> Result<InitOptions, String> {
    let dir = path.unwrap_or_else(|| PathBuf::from("."));
    let source = parse_source(source)?;
    let envs = parse_envs(envs)?;
    let targets = parse_targets(targets)?;
    if let Some(t) = template {
        if !t.eq_ignore_ascii_case("game") {
            return Err(format!("未知模板风格 {t:?}（可选：game）"));
        }
    }
    let name = name.unwrap_or_else(|| {
        dir.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("cage-project")
            .to_string()
    });
    Ok(InitOptions {
        dir,
        name,
        source,
        envs,
        targets,
        git,
        examples: !no_examples,
    })
}

// ---------------------------------------------------------------------------
// Interactive dialog
// ---------------------------------------------------------------------------

mod dialog;
mod scaffold;
#[cfg(test)]
mod tests;

use dialog::run_dialog;
use scaffold::{generate, git_init};
