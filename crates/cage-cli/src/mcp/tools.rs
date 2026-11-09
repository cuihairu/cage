//! MCP tool executors: the `run_*` implementations that drive the real
//! check / build / inspect pipelines (split out of mcp.rs, LAY-026).

use super::views::{
    config_json, diag_json, field_json, find_config_file, resolve_root, section_key, table_json,
    table_names,
};
use crate::{
    filter_by_profile, generate_target_items, load_project, load_project_config, validate_project,
};
use cage_core::manifest::ManifestGenerator;
use cage_core::normalize_document;
use cage_core::schema::TableSchema;
use serde_json::{json, Value};

pub(crate) fn run_check_schema(
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

pub(crate) fn run_list_tables(path: &str, no_cache: bool) -> Result<Value, String> {
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

pub(crate) fn run_list_fields(
    path: &str,
    table: Option<&str>,
    no_cache: bool,
) -> Result<Value, String> {
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

pub(crate) fn run_list_envs(path: &str, no_cache: bool) -> Result<Value, String> {
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

pub(crate) fn run_build_dry_run(
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
    let project = crate::project_with_env(project, env)?;
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
    let source_extra = crate::source_script_fingerprint(&root, &project.config)?;
    let (schema_hash, source_hash) =
        ManifestGenerator::input_hashes(&schema, &normalized, source_extra.as_deref());

    // 逐 target 生成到内存——与 cage build 同一份 generate_target_items，
    // 只是不调用写盘路径。
    let mut artifacts: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    for target in &build_profile.targets {
        let items = match generate_target_items(target, &schema, &normalized, &schema_hash, &root) {
            Ok(items) => items,
            Err(crate::BuildFailure::Validation(diags)) => {
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
            Err(crate::BuildFailure::Io(msg)) => return Err(msg),
            Err(crate::BuildFailure::UpToDate { .. }) => {
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
    .with_source_extra(source_extra)
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

pub(crate) fn run_explain_config(path: &str, section: Option<&str>) -> Result<Value, String> {
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
