//! Project scaffold and config generation for `cage init` (split out
//! of init.rs, LAY-028): the file writes and the toml/schema/sample/readme
//! renderers.

use std::fs;
use std::path::{Path, PathBuf};

use super::{catalog_entry, Envs, InitOptions, SourceKind};

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Write the scaffold. Refuses (Err) when the target directory already
/// holds a project file — re-running never overwrites user files.
pub(crate) fn generate(opts: &InitOptions) -> Result<Vec<PathBuf>, String> {
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
pub(crate) fn render_toml(opts: &InitOptions) -> String {
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
pub(crate) fn render_schema(opts: &InitOptions) -> String {
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
pub(crate) fn git_init(dir: &Path) {
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
