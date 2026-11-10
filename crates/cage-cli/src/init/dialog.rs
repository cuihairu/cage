//! Interactive Q&A for `cage init` (split out of init.rs, LAY-028):
//! the step machine, the ask helpers, and the pending-file plan.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use super::{normalize_target_format, Envs, InitOptions, SourceKind, TARGET_CATALOG};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Step {
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
pub(crate) fn run_dialog<R: BufRead, W: Write>(
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
pub(crate) fn print_plan<W: Write>(
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
