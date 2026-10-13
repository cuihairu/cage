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

use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

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

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Dir,
    Name,
    Source,
    Envs,
    Targets,
    Template,
    Git,
    Examples,
    Confirm,
}

const STEPS: &[Step] = &[
    Step::Dir,
    Step::Name,
    Step::Source,
    Step::Envs,
    Step::Targets,
    Step::Template,
    Step::Git,
    Step::Examples,
    Step::Confirm,
];

/// Run the interactive Q&A. Returns `Ok(None)` when the user declines the
/// final confirmation — nothing is written in that case.
fn run_dialog<R: BufRead, W: Write>(
    read: &mut R,
    out: &mut W,
) -> std::io::Result<Option<InitOptions>> {
    let mut dir = PathBuf::from(".");
    let mut name = String::new();
    let mut source = SourceKind::Json;
    let mut envs = Envs::Four;
    let mut targets: Vec<&'static str> = vec!["json", "csv"];
    let mut git = false;
    let mut examples = true;

    let mut i = 0usize;
    loop {
        let step = STEPS[i];
        match step {
            Step::Dir => match ask_line(read, out, "项目目录", ".")? {
                Ask::Value(v) => dir = PathBuf::from(v),
                Ask::Back => continue, // nothing before the first question
                Ask::Eof => return Ok(None),
            },
            Step::Name => {
                let default = dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("cage-project")
                    .to_string();
                match ask_line(read, out, "项目名", &default)? {
                    Ask::Value(v) => name = v,
                    Ask::Back => {
                        i = i.saturating_sub(1);
                        continue;
                    }
                    Ask::Eof => return Ok(None),
                }
            }
            Step::Source => match ask_choice(
                read,
                out,
                "配置源类型 [1]excel [2]csv [3]json [4]yaml [5]sheets [6]db [7]http",
                "3",
                |n| match n {
                    1 => Some(SourceKind::Excel),
                    2 => Some(SourceKind::Csv),
                    3 => Some(SourceKind::Json),
                    4 => Some(SourceKind::Yaml),
                    5 => Some(SourceKind::Sheets),
                    6 => Some(SourceKind::Db),
                    7 => Some(SourceKind::Http),
                    _ => None,
                },
            )? {
                Ask::Value(v) => source = v,
                Ask::Back => {
                    i = i.saturating_sub(1);
                    continue;
                }
                Ask::Eof => return Ok(None),
            },
            Step::Envs => match ask_choice(
                read,
                out,
                "环境套 [1]four(dev/test/staging/prod) [2]none",
                "1",
                |n| match n {
                    1 => Some(Envs::Four),
                    2 => Some(Envs::None),
                    _ => None,
                },
            )? {
                Ask::Value(v) => envs = v,
                Ask::Back => {
                    i = i.saturating_sub(1);
                    continue;
                }
                Ask::Eof => return Ok(None),
            },
            Step::Targets => {
                let list = TARGET_CATALOG
                    .iter()
                    .map(|(f, _, _)| *f)
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(out, "可用目标生成器：{list}（逗号分隔多选）")?;
                let mut going_back = false;
                loop {
                    match ask_line(read, out, "目标生成器", "json,csv")? {
                        Ask::Value(v) => {
                            let mut picked = Vec::new();
                            let mut bad = None;
                            for part in v.split(',') {
                                let part = part.trim();
                                if part.is_empty() {
                                    continue;
                                }
                                match normalize_target_format(part) {
                                    Some(fmt) => {
                                        if !picked.contains(&fmt) {
                                            picked.push(fmt);
                                        }
                                    }
                                    None => bad = Some(part.to_string()),
                                }
                            }
                            if let Some(b) = bad {
                                writeln!(out, "未知目标生成器 {b:?}，请重选。")?;
                                continue;
                            }
                            targets = if picked.is_empty() {
                                vec!["json", "csv"]
                            } else {
                                picked
                            };
                            break;
                        }
                        Ask::Back => {
                            i = i.saturating_sub(1);
                            going_back = true;
                            break;
                        }
                        Ask::Eof => return Ok(None),
                    }
                }
                if going_back {
                    continue;
                }
            }
            Step::Template => match ask_choice(read, out, "模板风格 [1]game", "1", |n| match n {
                1 => Some("game"),
                _ => None,
            })? {
                Ask::Value(_) => {}
                Ask::Back => {
                    i = i.saturating_sub(1);
                    continue;
                }
                Ask::Eof => return Ok(None),
            },
            Step::Git => match ask_yn(read, out, "生成后执行 git init？", false)? {
                Ask::Value(v) => git = v,
                Ask::Back => {
                    i = i.saturating_sub(1);
                    continue;
                }
                Ask::Eof => return Ok(None),
            },
            Step::Examples => match ask_yn(
                read,
                out,
                "带示例表（schemas/item.yaml + config/Item.*）？",
                true,
            )? {
                Ask::Value(v) => examples = v,
                Ask::Back => {
                    i = i.saturating_sub(1);
                    continue;
                }
                Ask::Eof => return Ok(None),
            },
            Step::Confirm => {
                print_plan(out, &dir, &name, source, envs, &targets, git, examples)?;
                match ask_yn(read, out, "确认生成？", true)? {
                    Ask::Value(true) => {
                        return Ok(Some(InitOptions {
                            dir,
                            name,
                            source,
                            envs,
                            targets,
                            git,
                            examples,
                        }))
                    }
                    Ask::Value(false) | Ask::Eof => return Ok(None),
                    Ask::Back => {
                        i = i.saturating_sub(1);
                        continue;
                    }
                }
            }
        }
        i += 1;
        if i >= STEPS.len() {
            break;
        }
    }
    Ok(None)
}

/// One answer to a dialog question. `Back` is an explicit `b` from the
/// user; `Eof` is a closed stdin, which must abort (a `Back` at the end
/// of a script would otherwise loop forever re-asking).
enum Ask<T> {
    Value(T),
    Back,
    Eof,
}

/// Ask a free-text question.
fn ask_line<R: BufRead, W: Write>(
    read: &mut R,
    out: &mut W,
    prompt: &str,
    default: &str,
) -> std::io::Result<Ask<String>> {
    write!(out, "{prompt} [{default}]: ")?;
    out.flush()?;
    let mut line = String::new();
    if read.read_line(&mut line)? == 0 {
        return Ok(Ask::Eof);
    }
    let v = line.trim();
    if v.eq_ignore_ascii_case("b") || v.eq_ignore_ascii_case("back") {
        return Ok(Ask::Back);
    }
    Ok(Ask::Value(if v.is_empty() {
        default.to_string()
    } else {
        v.to_string()
    }))
}

/// Ask a numbered-choice question; invalid input re-asks (without
/// consuming a back).
fn ask_choice<R: BufRead, W: Write, T>(
    read: &mut R,
    out: &mut W,
    prompt: &str,
    default: &str,
    resolve: impl Fn(u32) -> Option<T>,
) -> std::io::Result<Ask<T>> {
    loop {
        write!(out, "{prompt} [{default}]: ")?;
        out.flush()?;
        let mut line = String::new();
        if read.read_line(&mut line)? == 0 {
            return Ok(Ask::Eof);
        }
        let v = line.trim();
        if v.eq_ignore_ascii_case("b") || v.eq_ignore_ascii_case("back") {
            return Ok(Ask::Back);
        }
        let n: u32 = if v.is_empty() {
            default.parse().unwrap_or(0)
        } else if let Ok(n) = v.parse() {
            n
        } else {
            writeln!(out, "请输入选项编号。")?;
            continue;
        };
        match resolve(n) {
            Some(t) => return Ok(Ask::Value(t)),
            None => writeln!(out, "无效选项，请重选。")?,
        }
    }
}

/// Ask a yes/no question.
fn ask_yn<R: BufRead, W: Write>(
    read: &mut R,
    out: &mut W,
    prompt: &str,
    default: bool,
) -> std::io::Result<Ask<bool>> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        write!(out, "{prompt} {hint}: ")?;
        out.flush()?;
        let mut line = String::new();
        if read.read_line(&mut line)? == 0 {
            return Ok(Ask::Eof);
        }
        let v = line.trim().to_lowercase();
        if v == "b" || v == "back" {
            return Ok(Ask::Back);
        }
        if v.is_empty() {
            return Ok(Ask::Value(default));
        }
        match v.as_str() {
            "y" | "yes" | "是" => return Ok(Ask::Value(true)),
            "n" | "no" | "否" => return Ok(Ask::Value(false)),
            _ => writeln!(out, "请输入 y 或 n。")?,
        }
    }
}

/// Print the pending file list before writing anything.
#[allow(clippy::too_many_arguments)]
fn print_plan<W: Write>(
    out: &mut W,
    dir: &Path,
    name: &str,
    source: SourceKind,
    envs: Envs,
    targets: &[&str],
    git: bool,
    examples: bool,
) -> std::io::Result<()> {
    writeln!(out)?;
    writeln!(out, "待生成清单：")?;
    writeln!(out, "  项目目录   {}", dir.display())?;
    writeln!(out, "  项目名     {name}")?;
    writeln!(out, "  配置源     {}", source.label())?;
    writeln!(
        out,
        "  环境套     {}",
        match envs {
            Envs::Four => "dev/test/staging/prod",
            Envs::None => "无（不生成 env_overrides）",
        }
    )?;
    writeln!(out, "  目标生成器 {}", targets.join(", "))?;
    writeln!(out, "  示例表     {}", if examples { "是" } else { "否" })?;
    writeln!(out, "  git init   {}", if git { "是" } else { "否" })?;
    writeln!(out, "文件：")?;
    writeln!(out, "  {}/cage.toml", dir.display())?;
    if examples {
        writeln!(out, "  {}/schemas/item.yaml", dir.display())?;
        writeln!(
            out,
            "  {}/config/Item.{}",
            dir.display(),
            source.sample_ext()
        )?;
    }
    writeln!(out, "  {}/README.md", dir.display())?;
    writeln!(out, "  {}/.gitignore", dir.display())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Write the scaffold. Refuses (Err) when the target directory already
/// holds a project file — re-running never overwrites user files.
fn generate(opts: &InitOptions) -> Result<Vec<PathBuf>, String> {
    let collisions = [
        "cage.toml",
        "cage.yaml",
        "cage.yml",
        "cage.json",
        "schemas",
        "config",
        "README.md",
        ".gitignore",
    ]
    .iter()
    .filter(|p| opts.dir.join(p).exists())
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    if !collisions.is_empty() {
        return Err(format!(
            "{} 已存在项目文件（{}）——拒绝覆盖。换一个空目录，或手动删除后重试。",
            opts.dir.display(),
            collisions.join(", ")
        ));
    }

    let mut files = Vec::new();
    fs::create_dir_all(opts.dir.join("schemas")).map_err(|e| format!("无法创建 schemas/：{e}"))?;
    fs::create_dir_all(opts.dir.join("config")).map_err(|e| format!("无法创建 config/：{e}"))?;

    let toml = render_toml(opts);
    write_file(&opts.dir.join("cage.toml"), &toml)?;
    files.push(opts.dir.join("cage.toml"));

    if opts.examples {
        let schema = render_schema(opts);
        write_file(&opts.dir.join("schemas/item.yaml"), &schema)?;
        files.push(opts.dir.join("schemas/item.yaml"));

        let sample = render_sample(opts);
        let ext = opts.source.sample_ext();
        write_file(&opts.dir.join(format!("config/Item.{ext}")), &sample)?;
        files.push(opts.dir.join(format!("config/Item.{ext}")));
    }

    let readme = render_readme(opts);
    write_file(&opts.dir.join("README.md"), &readme)?;
    files.push(opts.dir.join("README.md"));

    write_file(&opts.dir.join(".gitignore"), GITIGNORE)?;
    files.push(opts.dir.join(".gitignore"));

    Ok(files)
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("无法写入 {}：{e}", path.display()))
}

const GITIGNORE: &str = "# Cage 构建产物与本地 registry
build/
reg/
.cage-cache/
";

/// Render `cage.toml` in the game-config seed shape.
fn render_toml(opts: &InitOptions) -> String {
    use std::fmt::Write as _;
    let mut t = String::new();
    t.push_str("# Cage 项目配置（cage init 生成）。完整示例见 examples/game-config/cage.toml\n");
    t.push_str("output_dir = \"build\"\n");
    t.push_str("schema_path = \"schemas\"\n\n");
    t.push_str("[project]\n");
    let _ = writeln!(t, "name = \"{}\"", opts.name);
    t.push_str("version = \"0.1.0\"\n");
    let _ = write!(t, "description = \"{} 配置工程\"\n\n", opts.name);
    t.push_str("[source_roots]\nmain = \"config\"\n");
    match opts.source {
        SourceKind::Excel => t.push_str(
            "# Excel 源：把 .xlsx 放进 config/ 即按扩展名自动识别（示例数据先用 CSV 跑通）\n",
        ),
        SourceKind::Sheets => t.push_str(
            "# Sheets 源：把 main 换成 gsheet:<id>/<tab> 并按文档配置 [remote.gsheet] 凭据\n",
        ),
        SourceKind::Db => t.push_str(
            "# DB 源：把 main 换成 mysql://user:pass@host/db 并按文档配置 [remote.mysql]\n",
        ),
        SourceKind::Http => t.push_str(
            "# HTTP 源：把 main 换成 https://host/config/index.json 并按文档配置 [remote.http]\n",
        ),
        _ => {}
    }
    t.push_str("\n[profiles.client]\nname = \"client\"\ndescription = \"全量数据 + 语言绑定\"\n");
    t.push_str("targets = [\n");
    for fmt in &opts.targets {
        if let Some((_, subdir, options)) = catalog_entry(fmt) {
            match options {
                Some((k, v)) => {
                    let _ = writeln!(
                        t,
                        "  {{ format = \"{fmt}\", output_dir = \"build/client/{subdir}\", options = {{ {k} = \"{v}\" }} }},"
                    );
                }
                None => {
                    let _ = writeln!(
                        t,
                        "  {{ format = \"{fmt}\", output_dir = \"build/client/{subdir}\" }},"
                    );
                }
            }
        }
    }
    t.push_str("]\n\n");
    t.push_str("[profiles.server]\nname = \"server\"\ndescription = \"服务端视图：json + csv 数据 target\"\n");
    t.push_str("targets = [\n");
    t.push_str("  { format = \"json\", output_dir = \"build/server/json\" },\n");
    t.push_str("  { format = \"csv\", output_dir = \"build/server/csv\" },\n");
    t.push_str("]\n\n");
    t.push_str("# 本地 registry 依赖解析（可选，产物目录不入版本控制）\n");
    t.push_str("# [registry]\n# path = \"reg\"\n");
    t
}

/// Render the example schema. With `Envs::Four` the price ceiling
/// tightens per environment (dev 1000 → prod 500); the sample data stays
/// under every lane so all four environments validate.
fn render_schema(opts: &InitOptions) -> String {
    let env_overrides = match opts.envs {
        Envs::Four => {
            "\n    env_overrides:\n      dev:\n        price: { max: 1000 }\n      test:\n        price: { max: 800 }\n      staging:\n        price: { max: 600 }\n      prod:\n        price: { max: 500 }\n"
        }
        Envs::None => "",
    };
    format!(
        "tables:\n  Item:\n    name: Item\n    description: 示例道具表（id 主键；price 上限按环境收紧）\n    primary_key: [id]\n    fields:\n      id: {{ name: id, type: {{ kind: Int32 }}, required: true, min: 1 }}\n      name: {{ name: name, type: {{ kind: String }}, required: true, max_length: 64 }}\n      price: {{ name: price, type: {{ kind: Int32 }}, required: true, min: 0 }}{env_overrides}\nenums: {{}}\n"
    )
}

/// Render the sample data file for the chosen source kind.
fn render_sample(opts: &InitOptions) -> String {
    match opts.source.sample_ext() {
        "json" => {
            "[\n  { \"id\": 1, \"name\": \"wooden-sword\", \"price\": 10 },\n  { \"id\": 2, \"name\": \"iron-shield\", \"price\": 25 },\n  { \"id\": 3, \"name\": \"healing-potion\", \"price\": 8 }\n]\n".to_string()
        }
        "yaml" => {
            "Item:\n  - id: 1\n    name: wooden-sword\n    price: 10\n  - id: 2\n    name: iron-shield\n    price: 25\n  - id: 3\n    name: healing-potion\n    price: 8\n".to_string()
        }
        _ => "id,name,price\n1,wooden-sword,10\n2,iron-shield,25\n3,healing-potion,8\n".to_string(),
    }
}

fn render_readme(opts: &InitOptions) -> String {
    let env_line = match opts.envs {
        Envs::Four => "cage build . --env dev    # 校验 + 生成（dev/test/staging/prod 四套环境）",
        Envs::None => "cage build .            # 校验 + 生成（无环境套，不传 --env）",
    };
    format!(
        "# {}\n\nCage 配置工程（由 `cage init` 生成）。\n\n## 结构\n\n- `cage.toml` — 项目配置（源、profiles、targets）\n- `schemas/` — 结构定义（YAML）\n- `config/` — 源数据\n- `build/` — 构建产物（已 gitignore）\n\n## 命令\n\n```sh\ncage check .              # 只校验\n{env_line}\ncage gen .                # 只生成代码绑定，不校验数据\ncage inspect .            # 查看表结构\n```\n\n环境套在 `schemas/` 的 `env_overrides` 声明，`--env <名>` 选档。\n\n文档：<https://cuihairu.github.io/cage/>\n",
        opts.name
    )
}

/// `git init` in the scaffolded directory. Failure is a warning, not a
/// fatal error — the scaffold itself is already complete.
fn git_init(dir: &Path) {
    match std::process::Command::new("git")
        .arg("init")
        .arg("--quiet")
        .current_dir(dir)
        .status()
    {
        Ok(status) if status.success() => println!("  已 git init"),
        Ok(status) => {
            eprintln!("cage init: 警告：git init 退出码 {status}（文件已生成，可手动 git init）");
        }
        Err(e) => {
            eprintln!("cage init: 警告：无法执行 git init：{e}（文件已生成，可手动 git init）");
        }
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn dialog_eof_aborts() {
        // A script that runs dry mid-dialog aborts instead of looping.
        assert!(dialog_answers(&["my-game"]).is_none());
    }

    #[test]
    fn dialog_no_examples_and_git() {
        let opts =
            dialog_answers(&["", "", "", "", "", "", "y", "n", "y"]).expect("confirmed plan");
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
}
