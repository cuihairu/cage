//! `cage gen`: schema-driven code generation for a profile's code targets,
//! plus the manifest writer shared with `cage build`. Split out of the CLI
//! entry.

use std::path::{Path, PathBuf};

use cage_core::manifest::{BuildManifest, ManifestGenerator, TargetConfig};
use cage_core::normalize::normalize_document;
use cage_core::reference::DependencyGraph;
use cage_core::schema::{Schema, ValidatedSchema};
use cage_core::validation::ValidationLevel;
use cage_core::value::Document;

use crate::{filter_by_profile, load_project, project_with_env, BuildFailure};

/// `cage gen` — schema-driven code generation only. Writes the code-target
/// artifacts (cs/python/lua/ts/js/cpp/go/java) of a profile without running
/// data validation:
/// types and metadata all come from the Schema, so source rows are not
/// needed. Data targets (json/csv) in the profile are skipped — run
/// `cage build` for those. The manifest is written like a build's, so gen
/// and build manifests share the same 口径 (schema/source/content hashes).
pub(crate) fn run_gen(path: &Path, profile: &str, env: Option<&str>, no_cache: bool) -> i32 {
    let project = match load_project(path, no_cache) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_with_env(project, env) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let Some(build_profile) = project.config.profiles.get(profile) else {
        eprintln!(
            "error: unknown profile '{profile}' (available: {})",
            project
                .config
                .profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        return 2;
    };

    // Schema-only pass with profile semantics: a projection that would strip
    // required/key/unique/reference-critical fields is E9006, not a silent
    // generation (code targets render field metadata from the view).
    let validated = ValidatedSchema {
        schema: project.schema.clone(),
        dependency_graph: DependencyGraph::from_schema(&project.schema),
    };
    let diagnostics = cage_core::validation::validate_with_profile(
        &validated,
        &project.document,
        ValidationLevel::Schema,
        false,
        Some(profile),
    );
    if !diagnostics.is_empty() {
        println!("{}", diagnostics.render(false));
    }
    if diagnostics.has_errors() {
        println!(
            "cage gen: FAILED schema validation ({} errors)",
            diagnostics.errors().len()
        );
        return 1;
    }

    let (schema, document) = filter_by_profile(&project.schema, &project.document, profile);
    let normalized = normalize_document(&document);
    let (schema_hash, _) = ManifestGenerator::input_hashes(&schema, &normalized, None);

    let mut artifacts: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
    for target in &build_profile.targets {
        match code_target_items(target, &schema, &schema_hash, path) {
            Some(Ok(items)) => {
                if let Err(e) = write_artifact_files(path, items, &target.format, &mut artifacts) {
                    eprintln!("error: {e}");
                    return 2;
                }
            }
            Some(Err(e)) => {
                eprintln!("error: {e}");
                return 2;
            }
            None => {}
        }
    }
    if artifacts.is_empty() {
        eprintln!(
            "error: profile '{profile}' has no code targets (cs/python/lua/ts/js/cpp/go/java/proto/flatbuffers/jsonschema/template)"
        );
        return 2;
    }

    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest = ManifestGenerator::new(
        project.config.project.name.clone(),
        profile.to_string(),
        version,
    )
    .with_environment(env)
    .with_targets(&build_profile.targets)
    .generate(&schema, &normalized, &artifacts);
    let output_dir = project.config.output_dir.as_deref().unwrap_or("build");
    let manifest_path = match write_manifest(&path.join(output_dir), &manifest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    println!(
        "cage gen: OK (profile '{profile}', {} artifacts, manifest {})",
        artifacts.len(),
        manifest_path.display()
    );
    0
}

/// Generate code-target artifacts for one target config; `None` when the
/// format is a data target (json/csv) rather than a code target. The
/// bundled language generators are schema-driven, deterministic, and
/// infallible; the user-template target (`format = "template"`) renders
/// `.tera` files from disk and can fail — its `Err` names the offending
/// template file or the missing template directory. Each file header is
/// stamped with the manifest's schema hash.
type CodeTargetItems = Result<Vec<(String, Vec<u8>)>, String>;

pub(crate) fn code_target_items(
    target: &TargetConfig,
    schema: &Schema,
    schema_hash: &str,
    root: &Path,
) -> Option<CodeTargetItems> {
    // "csharp"/"python"/"typescript" per docs; short aliases ("cs"/"py"/
    // "ts"/"js") and the common alternates ("golang", "c++"/"cxx") accepted.
    match target.format.as_str() {
        "cs" | "csharp" => Some(Ok(cage_target_cs::CsTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "python" | "py" => Some(Ok(cage_target_py::PyTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "lua" => Some(Ok(cage_target_lua::LuaTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "typescript" | "ts" | "javascript" | "js" => {
            Some(Ok(cage_target_ts::TsTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash))))
        }
        "cpp" | "c++" | "cxx" => Some(Ok(cage_target_cpp::CppTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "go" | "golang" => Some(Ok(cage_target_go::GoTargetGenerator::from_config(target)
            .generate(schema, Some(schema_hash)))),
        "java" => Some(Ok(cage_target_java::JavaTargetGenerator::from_config(
            target,
        )
        .generate(schema, Some(schema_hash)))),
        "proto" | "protobuf" => Some(Ok(cage_target_proto::ProtoTargetGenerator::from_config(
            target,
        )
        .generate(schema, Some(schema_hash)))),
        "flatbuffers" | "fbs" => Some(Ok(
            cage_target_flatbuffers::FlatbuffersTargetGenerator::from_config(target)
                .generate(schema, Some(schema_hash)),
        )),
        // Standard JSON Schema documents (draft-07 default, 2020-12 via
        // options.draft) — the one bundled target that can fail on config
        // alone (an unknown draft value), like the template target.
        "jsonschema" | "json-schema" | "json_schema" => {
            let gen = cage_target_jsonschema::JsonSchemaTargetGenerator::from_config(target);
            Some(gen.generate(schema, Some(schema_hash)))
        }
        "template" => {
            let mut gen = cage_target_template::TemplateTargetGenerator::from_config(target);
            // `options.template_dir` (default `.cage/templates`) is relative
            // to the project root, like output_dir — not the process CWD.
            if gen.template_dir.is_relative() {
                gen.template_dir = root.join(&gen.template_dir);
            }
            // G4 filter libraries: `options.lang_filters = "py,go"` mounts
            // the named languages' type/default filters so user templates
            // can reuse the exact decisions the official generators make.
            let langs = match lang_filters(target) {
                Ok(langs) => langs,
                Err(e) => return Some(Err(e)),
            };
            let holder = java_holder(target);
            Some(
                gen.generate_with_setup(schema, Some(schema_hash), |tera, schema| {
                    for lang in &langs {
                        match lang.as_str() {
                            "py" | "python" => cage_target_py::register_filters(tera, schema),
                            "cs" | "csharp" => cage_target_cs::register_filters(tera, schema),
                            "ts" | "typescript" => cage_target_ts::register_filters(tera, schema),
                            "go" | "golang" => cage_target_go::register_filters(tera, schema),
                            "java" => cage_target_java::register_filters(
                                tera,
                                schema,
                                Some(holder.as_str()),
                            ),
                            "cpp" | "c++" | "cxx" => {
                                cage_target_cpp::register_filters(tera, schema);
                            }
                            "lua" => cage_target_lua::register_filters(tera, schema),
                            _ => {}
                        }
                    }
                }),
            )
        }
        _ => None,
    }
}

/// Generate one target's artifacts in memory — the per-target half of the
/// build loop, shared by `cage build` and the MCP `build_dry_run` tool so
/// both compute the same bytes from the same inputs. Bundled code targets
/// (cs/python/lua/ts/…) are schema-driven and infallible; the user-template
/// target can fail (template fault or missing directory); data targets
/// serialize the (possibly profile-filtered) document.
pub(crate) fn generate_target_items(
    target: &TargetConfig,
    gen_schema: &Schema,
    gen_doc: &Document,
    schema_hash: &str,
    path: &Path,
) -> Result<Vec<(String, Vec<u8>)>, BuildFailure> {
    match code_target_items(target, gen_schema, schema_hash, path) {
        Some(Ok(items)) => Ok(items),
        Some(Err(e)) => Err(BuildFailure::Io(e)),
        None => match target.format.as_str() {
            "json" => cage_target_json::JsonTargetGenerator::from_config(target)
                .generate(gen_doc, &[])
                .map_err(BuildFailure::Validation),
            "csv" => cage_target_csv::CsvTargetGenerator::from_config(target)
                .generate(gen_doc, &[])
                .map_err(BuildFailure::Validation),
            "msgpack" | "messagepack" => {
                cage_target_msgpack::MsgPackTargetGenerator::from_config(target)
                    .generate(gen_doc, &[])
                    .map_err(BuildFailure::Validation)
            }
            other => Err(BuildFailure::Io(format!(
                "unsupported target format '{other}'"
            ))),
        },
    }
}

/// `options.lang_filters` (design §22 G4): comma-separated language keys
/// whose filter libraries mount into the user-template engine. Unknown
/// keys fail up front, before any rendering.
pub(crate) fn lang_filters(target: &TargetConfig) -> Result<Vec<String>, String> {
    let Some(v) = target.options.as_ref().and_then(|o| o.get("lang_filters")) else {
        return Ok(Vec::new());
    };
    let s = v
        .as_str()
        .ok_or_else(|| "template target: options.lang_filters must be a string".to_string())?;
    let mut langs = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "py" | "python" | "cs" | "csharp" | "ts" | "typescript" | "go" | "golang" | "java"
            | "cpp" | "c++" | "cxx" | "lua" => langs.push(part.to_string()),
            other => {
                return Err(format!(
                    "template target: unknown lang_filters entry '{other}' \
                     (supported: py/cs/ts/go/java/cpp/lua)"
                ));
            }
        }
    }
    Ok(langs)
}

/// The shared-enums holder class stem for the Java filter library — the
/// same derivation `JavaTargetGenerator::from_config` applies
/// (`options.enums_file`'s file stem, default `CageEnums`).
pub(crate) fn java_holder(target: &TargetConfig) -> String {
    let f = target
        .options
        .as_ref()
        .and_then(|o| o.get("enums_file"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("CageEnums.java");
    Path::new(f)
        .file_stem()
        .map_or_else(|| f.to_string(), |s| s.to_string_lossy().into_owned())
}

/// Manifest-facing form of a target's relative output path: portable `/`
/// separators. The path is a contract key downstream (manifest artifacts,
/// snapshot ledger, `api_artifact` lookups, diff target mapping) — every
/// consumer compares it against `/`-separated strings, so the platform
/// separator must never leak into the recorded form.
pub(crate) fn normalize_artifact_rel(rel: &str) -> String {
    rel.replace(std::path::MAIN_SEPARATOR, "/")
}

/// Write generated artifacts under `root`, recording manifest entries
/// (relative path, content, raw format string, table stem when derivable).
pub(crate) fn write_artifact_files(
    root: &Path,
    items: Vec<(String, Vec<u8>)>,
    format: &str,
    artifacts: &mut Vec<(String, Vec<u8>, String, Option<String>)>,
) -> Result<(), String> {
    for (rel_path, content) in items {
        let rel_path = normalize_artifact_rel(&rel_path);
        let abs = root.join(&rel_path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&abs, &content)
            .map_err(|e| format!("cannot write {}: {e}", abs.display()))?;
        let table = rel_path
            .rsplit('/')
            .next()
            .and_then(|f| f.rsplit_once('.'))
            .map(|(stem, _)| stem.to_string());
        artifacts.push((rel_path, content, format.to_string(), table));
    }
    Ok(())
}

/// Persist the build manifest as `manifest.json` under `manifest_dir`,
/// returning its full path.
pub(crate) fn write_manifest(
    manifest_dir: &Path,
    manifest: &BuildManifest,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(manifest_dir)
        .map_err(|e| format!("cannot create {}: {e}", manifest_dir.display()))?;
    let manifest_path = manifest_dir.join("manifest.json");
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| format!("manifest serialization failed: {e}"))?;
    std::fs::write(&manifest_path, bytes)
        .map_err(|e| format!("cannot write {}: {e}", manifest_path.display()))?;
    Ok(manifest_path)
}
