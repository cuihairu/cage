//! JSON shaping helpers: the schema / config / diagnostic views an MCP
//! tool returns (split out of mcp.rs, LAY-026).

use cage_core::diagnostics::Diagnostic;
use cage_core::error::codes::{editor, registry, remote};
use cage_core::error_title;
use cage_core::manifest::ProjectConfig;
use cage_core::schema::{FieldSchema, ReferenceSchema, Schema, TableSchema};
use cage_core::{
    build, gamerule, internal, parse, ref_codes, schema_codes, semantic, table, type_val,
    value_codes,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ---- JSON shaping helpers ----

pub(crate) fn render(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

pub(crate) fn resolve_root(path: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(path);
    if !root.is_dir() {
        return Err(format!(
            "目录不存在：{}（path 应为含 cage.toml 的工程根目录）",
            root.display()
        ));
    }
    Ok(root)
}

pub(crate) fn find_config_file(root: &Path) -> Result<String, String> {
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

pub(crate) fn section_key(section: &str) -> String {
    // 常见别名归一（schema-draft 风格的写法也接受）
    match section {
        "sources" | "source" => "source_roots".to_string(),
        "deps" => "dependencies".to_string(),
        "output-dir" => "output_dir".to_string(),
        "schema-path" => "schema_path".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn config_json(root: &Path, config_file: &str, config: &ProjectConfig) -> Value {
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
pub(crate) fn precedence_chain() -> Vec<Value> {
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

pub(crate) fn source_root_kind(root: &str) -> &'static str {
    if root.starts_with("registry:") {
        "registry"
    } else if root.starts_with("mysql:") || root.starts_with("pg:") {
        "database"
    } else if root.starts_with("gsheet:") {
        "sheets"
    } else if crate::remote::is_remote_root(root) {
        "http"
    } else {
        "local"
    }
}

pub(crate) fn table_names(schema: &Schema) -> String {
    schema.tables.keys().cloned().collect::<Vec<_>>().join(", ")
}

pub(crate) fn table_json(t: &TableSchema, data: Option<&cage_core::value::Table>) -> Value {
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

pub(crate) fn field_json(f: &FieldSchema) -> Value {
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

pub(crate) fn reference_json(r: &ReferenceSchema) -> Value {
    json!({
        "table": r.table,
        "field": r.field,
        "cardinality": r.cardinality,
    })
}

pub(crate) fn field_reference_json(f: &FieldSchema, r: &ReferenceSchema) -> Value {
    let mut v = reference_json(r);
    if let Value::Object(map) = &mut v {
        map.insert("field_name".to_string(), json!(f.name));
    }
    v
}

/// 诊断 → 结构化 JSON：错误码、标题、severity、定位（file:line，文本
/// 源无行号时给 `file:row N[:col]`）、表/字段/值、消息（原因）与建议。
/// 建议优先用诊断自带的 hint，缺 hint 时按错误码族给一条可操作的话。
pub(crate) fn diag_json(d: &Diagnostic) -> Value {
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
pub(crate) fn location_string(d: &Diagnostic) -> String {
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
pub(crate) fn suggestion_for(d: &Diagnostic) -> Option<String> {
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
pub(crate) fn load_error_json(err: &str, path: &str) -> Value {
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
