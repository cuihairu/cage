//! Template Target Generator — Tera (Jinja-style) templates rendering the
//! Schema IR into code bindings and data artifacts (design §22 Template
//! Target, todo G series).
//!
//! Three shapes, one engine:
//!
//! - Official templates shipped with each target crate (G2, per-language
//!   `templates/` directories)
//! - User templates: `options.template_dir` points at a directory of
//!   `*.tera` files (G3 wires the CLI; `.cage/templates/` convention)
//! - The IR is exposed to templates as a whole: the full [`Schema`]
//!   serialization (tables / fields / types / descriptions all
//!   referencable), plus `schema_hash`, plus — for per-table templates —
//!   the current `table`.
//!
//! Determinism: the template file name (relative to `template_dir`) is the
//! output-name template — `{table}.py.tera` renders once per table in
//! table-name order, a name without `{table}` renders once globally with
//! output name = file name minus `.tera`. Tera is workspace-locked and
//! templates travel with the source, so the same schema plus the same
//! templates produce byte-identical files.
//!
//! Logic stays out of templates: naming conventions are Tera filters here
//! (G4 grows the library into per-language type maps and literal renderers),
//! so a template expresses text shape while decisions remain in Rust.

#![warn(clippy::all, clippy::pedantic)]
// Stage: crate-prefixed type names (TemplateTargetGenerator, ...) are
// idiomatic across the cage-target-* family.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines
)]

use cage_core::{manifest::TargetConfig, schema::Schema, schema::TableSchema};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tera::{Context as TeraContext, Tera};

/// Template Target Generator: renders every `*.tera` template under
/// `template_dir` against the schema IR.
pub struct TemplateTargetGenerator {
    /// Output directory — files land at `output_dir/<rendered name>`
    pub output_dir: PathBuf,
    /// Directory of `*.tera` templates; relative file names become output
    /// names (minus the `.tera` suffix)
    pub template_dir: PathBuf,
}

impl Default for TemplateTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/template"),
            template_dir: PathBuf::from("templates"),
        }
    }
}

impl TemplateTargetGenerator {
    /// Create from target config. `options.template_dir` (string) overrides
    /// the default `templates`.
    pub fn from_config(config: &TargetConfig) -> Self {
        let mut gen = Self {
            output_dir: PathBuf::from(&config.output_dir),
            ..Self::default()
        };
        if let Some(opts) = &config.options {
            if let Some(d) = opts.get("template_dir").and_then(Value::as_str) {
                gen.template_dir = PathBuf::from(d);
            }
        }
        gen
    }

    /// Render all templates against the schema IR.
    ///
    /// Per-table templates (output name contains `{table}`) render once per
    /// table in table-name order; global templates render once. Artifact
    /// order is template-name order, then table order — deterministic for
    /// identical input. Template syntax faults and filter failures surface
    /// as `Err` pointing at the offending template.
    pub fn generate(
        &self,
        schema: &Schema,
        schema_hash: Option<&str>,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let mut tera = Tera::default();
        // Code templates are text, not HTML: escape nothing, so output
        // bytes match the template's own rendering decisions.
        tera.autoescape_on(Vec::<&str>::new());

        let mut files = Vec::new();
        collect_templates(&self.template_dir, &mut files)?;
        if files.is_empty() {
            return Err(format!(
                "template target: no *.tera templates found under {}",
                self.template_dir.display()
            ));
        }
        files.sort();
        for path in &files {
            let rel = path
                .strip_prefix(&self.template_dir)
                .map_err(|e| format!("template target: {}: {e}", path.display()))?
                .to_string_lossy()
                .into_owned();
            tera.add_template_file(path, Some(&rel))
                .map_err(|e| format!("template target: {}: {e}", path.display()))?;
        }
        register_convention_filters(&mut tera);

        let mut names: Vec<String> = tera.get_template_names().map(str::to_owned).collect();
        names.sort();

        let mut table_names: Vec<&String> = schema.tables.keys().collect();
        table_names.sort();

        let mut artifacts = Vec::new();
        for rel in &names {
            let out_pattern = rel.strip_suffix(".tera").unwrap_or(rel);
            if out_pattern.contains("{table}") {
                for table_name in &table_names {
                    let table = &schema.tables[table_name.as_str()];
                    let ctx = ir_context(schema, schema_hash, Some(table))?;
                    let rendered = render(&tera, rel, &ctx)?;
                    artifacts.push((
                        self.join_output(&out_pattern.replace("{table}", table_name)),
                        rendered.into_bytes(),
                    ));
                }
            } else {
                let ctx = ir_context(schema, schema_hash, None)?;
                let rendered = render(&tera, rel, &ctx)?;
                artifacts.push((self.join_output(out_pattern), rendered.into_bytes()));
            }
        }
        Ok(artifacts)
    }

    fn join_output(&self, rel: &str) -> String {
        self.output_dir.join(rel).to_string_lossy().into_owned()
    }
}

fn render(tera: &Tera, name: &str, ctx: &TeraContext) -> Result<String, String> {
    tera.render(name, ctx)
        .map_err(|e| format!("template target: {name}: {e}"))
}

/// Template context: the schema serialization as-is (tables, fields, types,
/// descriptions all referencable), `schema_hash` on top, and `table` for
/// per-table templates.
fn ir_context(
    schema: &Schema,
    schema_hash: Option<&str>,
    table: Option<&TableSchema>,
) -> Result<TeraContext, String> {
    let mut value = serde_json::to_value(schema)
        .map_err(|e| format!("template target: schema serialization failed: {e}"))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| "template target: schema serialization is not an object".to_string())?;
    obj.insert(
        "schema_hash".to_string(),
        schema_hash.map_or(Value::Null, |h| Value::String(h.to_string())),
    );
    if let Some(t) = table {
        let t = serde_json::to_value(t)
            .map_err(|e| format!("template target: table serialization failed: {e}"))?;
        obj.insert("table".to_string(), t);
    }
    TeraContext::from_value(value).map_err(|e| format!("template target: context: {e}"))
}

/// Recursively collect `*.tera` files under `dir`.
fn collect_templates(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("template target: cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("template target: {}: {e}", dir.display()))?
            .path();
        if path.is_dir() {
            collect_templates(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "tera") {
            out.push(path);
        }
    }
    Ok(())
}

// --- naming-convention filters -------------------------------------------

/// Split an identifier into words: on non-alphanumeric separators, on
/// lower/digit → upper boundaries, and before an uppercase letter that
/// starts a new capitalized word after an acronym run (`HTTPServer` →
/// `HTTP`, `Server`).
fn words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric() {
            let boundary = !cur.is_empty()
                && c.is_uppercase()
                && cur.chars().next_back().is_some_and(|p| {
                    p.is_lowercase()
                        || p.is_numeric()
                        || (p.is_uppercase() && chars.get(i + 1).is_some_and(|n| n.is_lowercase()))
                });
            if boundary {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `first` uppercased, remainder lowercased (`sword` → `Sword`).
fn capitalized(w: &str) -> String {
    let mut cs = w.chars();
    match cs.next() {
        Some(f) => f.to_uppercase().collect::<String>() + cs.as_str().to_lowercase().as_str(),
        None => String::new(),
    }
}

fn as_identifier(value: &Value) -> tera::Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        _ => Err("naming filters expect a string".into()),
    }
}

struct SnakeCaseFilter;

impl tera::Filter for SnakeCaseFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let s = as_identifier(value)?;
        Ok(Value::String(
            words(&s)
                .iter()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join("_"),
        ))
    }
}

struct CamelCaseFilter;

impl tera::Filter for CamelCaseFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let s = as_identifier(value)?;
        let mut ws = words(&s).into_iter();
        let first = ws.next().map(|w| w.to_lowercase()).unwrap_or_default();
        let rest: String = ws.map(|w| capitalized(&w)).collect();
        Ok(Value::String(format!("{first}{rest}")))
    }
}

struct PascalCaseFilter;

impl tera::Filter for PascalCaseFilter {
    fn filter(&self, value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
        let s = as_identifier(value)?;
        Ok(Value::String(
            words(&s).iter().map(|w| capitalized(w)).collect(),
        ))
    }
}

/// Register the naming-convention filters (G4 grows this into the full
/// per-language library).
fn register_convention_filters(tera: &mut Tera) {
    tera.register_filter("snake_case", SnakeCaseFilter);
    tera.register_filter("camelCase", CamelCaseFilter);
    tera.register_filter("PascalCase", PascalCaseFilter);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_schema() -> Schema {
        serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    description: Equipment definitions.
    primary_key: [id]
    fields:
      name: { name: name, type: { kind: String }, required: true, description: Display name }
      id: { name: id, type: { kind: Int32 }, required: true }
      price: { name: price, type: { kind: Int32 }, default: 10 }
  Aura:
    name: Aura
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int64 }, required: true }
enums:
  ItemKind:
    name: ItemKind
    values:
      - { name: Sword, value: 1 }
",
        )
        .expect("test schema must parse")
    }

    fn write_template(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("rel has a parent")).expect("mkdirs");
        fs::write(path, content).expect("write template");
    }

    #[test]
    fn per_table_template_renders_every_table_in_name_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_template(
            tmp.path(),
            "data/{table}.txt.tera",
            "table={{ table.name }}\n{% for fname, f in table.fields -%}\n{{ fname }}:{{ f.type.kind }}\n{% endfor -%}\n",
        );
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let artifacts = gen.generate(&test_schema(), None).expect("render");
        assert_eq!(artifacts.len(), 2);
        // Aura < Item in name order; the subdir pattern carries through.
        assert_eq!(artifacts[0].0, "build/template/data/Aura.txt");
        assert_eq!(artifacts[1].0, "build/template/data/Item.txt");
        // Field order follows schema declaration order (preserve_order).
        assert_eq!(
            std::str::from_utf8(&artifacts[1].1).expect("utf8"),
            "table=Item\nname:String\nid:Int32\nprice:Int32\n"
        );
        assert_eq!(
            std::str::from_utf8(&artifacts[0].1).expect("utf8"),
            "table=Aura\nid:Int64\n"
        );
    }

    #[test]
    fn global_template_renders_once_with_ir_exposed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_template(
            tmp.path(),
            "enums.txt.tera",
            "tables={{ tables | length }} first={{ tables.Item.fields.id.type.kind }} enum0={{ enums.ItemKind.values[0].name }}\n",
        );
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let artifacts = gen.generate(&test_schema(), None).expect("render");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].0, "build/template/enums.txt");
        assert_eq!(
            std::str::from_utf8(&artifacts[0].1).expect("utf8"),
            "tables=2 first=Int32 enum0=Sword\n"
        );
    }

    #[test]
    fn schema_hash_is_exposed_to_templates() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_template(tmp.path(), "hash.txt.tera", "hash={{ schema_hash }}\n");
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let artifacts = gen
            .generate(&test_schema(), Some("abc123"))
            .expect("render");
        assert_eq!(
            std::str::from_utf8(&artifacts[0].1).expect("utf8"),
            "hash=abc123\n"
        );
    }

    #[test]
    fn naming_filters_convert_identifiers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_template(
            tmp.path(),
            "names.txt.tera",
            "{{ 'ItemPrice' | snake_case }} {{ 'item_price' | camelCase }} {{ 'http_server' | PascalCase }}\n",
        );
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let artifacts = gen.generate(&test_schema(), None).expect("render");
        assert_eq!(
            std::str::from_utf8(&artifacts[0].1).expect("utf8"),
            "item_price itemPrice HttpServer\n"
        );
    }

    #[test]
    fn field_descriptions_and_defaults_are_referencable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_template(
            tmp.path(),
            "{table}.txt.tera",
            "{{ table.name }}: {{ table.description }}; {{ table.fields.name.description | default(value=\"none\") }} default={{ table.fields.price.default | default(value=\"-\") }}\n",
        );
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let artifacts = gen.generate(&test_schema(), None).expect("render");
        // Only Item has fields name/price; Aura renders the fallbacks.
        let item = &artifacts[1];
        assert_eq!(item.0, "build/template/Item.txt");
        assert_eq!(
            std::str::from_utf8(&item.1).expect("utf8"),
            "Item: Equipment definitions.; Display name default=10\n"
        );
        let aura = &artifacts[0];
        assert_eq!(
            std::str::from_utf8(&aura.1).expect("utf8"),
            "Aura: ; none default=-\n"
        );
    }

    #[test]
    fn missing_template_dir_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().join("absent"),
        };
        let err = gen.generate(&test_schema(), None).expect_err("must fail");
        assert!(err.contains("cannot read"), "{err}");
    }

    #[test]
    fn empty_template_dir_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let err = gen.generate(&test_schema(), None).expect_err("must fail");
        assert!(err.contains("no *.tera"), "{err}");
    }

    #[test]
    fn broken_template_syntax_is_an_error_naming_the_template() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_template(tmp.path(), "bad.txt.tera", "{% for %}");
        let gen = TemplateTargetGenerator {
            output_dir: PathBuf::from("build/template"),
            template_dir: tmp.path().to_path_buf(),
        };
        let err = gen.generate(&test_schema(), None).expect_err("must fail");
        assert!(err.contains("bad.txt.tera"), "{err}");
    }

    #[test]
    fn from_config_reads_template_dir_option() {
        let config = TargetConfig {
            format: "template".to_string(),
            output_dir: "build/gen".to_string(),
            file_template: None,
            options: Some(
                [(
                    "template_dir".to_string(),
                    Value::String(".cage/templates".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
        };
        let gen = TemplateTargetGenerator::from_config(&config);
        assert_eq!(gen.output_dir, PathBuf::from("build/gen"));
        assert_eq!(gen.template_dir, PathBuf::from(".cage/templates"));
    }

    #[test]
    fn words_split_on_case_digit_and_separator_boundaries() {
        assert_eq!(words("ItemPrice"), vec!["Item", "Price"]);
        assert_eq!(words("item_price"), vec!["item", "price"]);
        assert_eq!(words("HTTPServer"), vec!["HTTP", "Server"]);
        assert_eq!(words("item2Price"), vec!["item2", "Price"]);
        assert_eq!(words("a_b-c"), vec!["a", "b", "c"]);
        assert_eq!(words(""), Vec::<&str>::new());
    }
}
