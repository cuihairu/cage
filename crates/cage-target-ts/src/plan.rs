//! Per-file plan: import surface, enum spellings and template precomputation (split out of lib.rs, LAY-010).

use cage_core::schema::{EnumValue, Schema, TableSchema};
use serde_json::{json, Value};
use std::collections::HashSet;

use super::docfmt::{jsdoc_desc_safe, EnumStyle, Member};
use super::idents::{sanitize_ident, ts_ident, unique_ident};
use super::literals::{field_doc, render_default, ts_string_literal};
use super::types::ts_type;
use super::{
    banner_line, collect_enum_refs, enum_exports, jsdoc_import_path, module_specifier,
    sorted_fields,
};
/// Everything a table file needs, computed once per file so interface,
/// defaults and factory can never disagree.
pub(crate) struct TablePlan<'a> {
    /// Keyword-escaped binding for the interface / typedef / factory.
    table_ident: String,
    /// (export ident, local binding) pairs for the type-only import.
    import_rows: Vec<(String, String)>,
    /// Enum schema name → type expression in this file.
    enum_ty: std::collections::HashMap<String, String>,
    /// Properties in field name order.
    members: Vec<Member<'a>>,
}

/// Compute the per-file plan: binding namespace, enum imports/aliases and
/// members in field name order.
pub(crate) fn plan_table<'a>(
    schema: &'a Schema,
    table: &'a TableSchema,
    style: EnumStyle,
    enums_file: &str,
) -> TablePlan<'a> {
    let table_ident = ts_ident(&table.name);
    let mut used_bindings: HashSet<String> = HashSet::new();
    let mut used_props: HashSet<String> = HashSet::new();

    let exports = enum_exports(schema);
    let mut used_enums = std::collections::BTreeSet::new();
    for (_, field) in sorted_fields(table) {
        collect_enum_refs(&field.field_type, schema, &mut used_enums);
    }

    let mut import_rows = Vec::new();
    let mut enum_ty = std::collections::HashMap::new();
    match style {
        EnumStyle::Local => {
            used_bindings.insert(table_ident.clone());
            used_bindings.insert(format!("{table_ident}Defaults"));
            used_bindings.insert(format!("new{table_ident}"));
            for (name, export) in &exports {
                if !used_enums.contains(*name) {
                    continue;
                }
                let local = unique_ident(export.clone(), &mut used_bindings);
                import_rows.push((export.clone(), local.clone()));
                enum_ty.insert((*name).to_string(), local);
            }
        }
        EnumStyle::InlineImport => {
            let path = jsdoc_import_path(enums_file);
            for (name, export) in &exports {
                if used_enums.contains(*name) {
                    enum_ty.insert((*name).to_string(), format!("import(\"{path}\").{export}"));
                }
            }
        }
    }

    let mut members = Vec::with_capacity(table.fields.len());
    for (field_name, field) in sorted_fields(table) {
        let default = field
            .default
            .as_ref()
            .filter(|v| !v.is_null())
            .and_then(|d| render_default(d, &field.field_type, schema));
        members.push(Member {
            prop: unique_ident(sanitize_ident(field_name), &mut used_props),
            ft: &field.field_type,
            doc: field_doc(schema, field),
            required: field.required,
            default,
        });
    }

    TablePlan {
        table_ident,
        import_rows,
        enum_ty,
        members,
    }
}

/// String-bucket value for one enum member: `String` as-is, `Number` as
/// its decimal string, `Bool` as `"true"`/`"false"`, anything else (or
/// missing) as the member name (Cage compares enums as strings).
pub(crate) fn enum_string_value(v: &EnumValue) -> String {
    match &v.value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        _ => v.name.clone(),
    }
}

/// Language precomputation for the official TypeScript/JavaScript
/// templates (design §22 G2): every decision the legacy renderer made —
/// import surface, both enum spellings (Local binding vs inline
/// `import("…")`), `JSDoc` scanner safety, defaults and factory inlining —
/// is computed here; the templates only express text shape. The context
/// serves all registered flavors of one mode: `.ts` uses the Local `ty`,
/// `.js` the braced `ty_jsdoc`, `.d.ts` the Local `ty` again.
//
// Contract: returns `Result` to match the `extras` hook signature even
// though this precomputation is infallible.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn ts_extras(
    enums_file: &str,
    schema: &Schema,
    table: Option<&TableSchema>,
) -> Result<Value, String> {
    if let Some(t) = table {
        // Both enum spellings, so every registered flavor can pick its
        // own: `.ts`/`.d.ts` render Local types, `.js` inline imports.
        let plan_local = plan_table(schema, t, EnumStyle::Local, enums_file);
        let plan_inline = plan_table(schema, t, EnumStyle::InlineImport, enums_file);

        // Type-only import of the enums actually used (TS table files and
        // `.d.ts`; JSDoc spells enums through inline `import("…")` types).
        let import_line: Option<String> = if plan_local.import_rows.is_empty() {
            None
        } else {
            let names = plan_local
                .import_rows
                .iter()
                .map(|(export, local)| {
                    if export == local {
                        export.clone()
                    } else {
                        format!("{export} as {local}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "import type {{ {names} }} from \"{}\";",
                module_specifier(enums_file)
            ))
        };

        let members: Vec<Value> = plan_local
            .members
            .iter()
            .zip(plan_inline.members.iter())
            .map(|(m, mj)| {
                let doc = m.doc.clone();
                // Bracketed names re-lex the description's first
                // character; unsafe starters move to a continuation line.
                let doc_on_line = match &doc {
                    Some(d) => !m.is_optional() || jsdoc_desc_safe(d),
                    None => true,
                };
                json!({
                    "prop": m.prop,
                    "opt": if m.is_optional() { "?" } else { "" },
                    "ty": ts_type(m.ft, schema, &plan_local.enum_ty),
                    "ty_jsdoc": format!("{{{}}}", ts_type(mj.ft, schema, &plan_inline.enum_ty)),
                    "jsdoc_name": m.jsdoc_name(),
                    "doc": doc,
                    "doc_on_line": doc_on_line,
                })
            })
            .collect();

        // Defaults const and factory are rendered from the same collection
        // so they can never drift; the factory inlines fresh literals so
        // rows never share arrays.
        let default_rows: Vec<Value> = plan_local
            .members
            .iter()
            .filter_map(|m| {
                m.default
                    .as_ref()
                    .map(|d| json!({ "prop": m.prop, "expr": d }))
            })
            .collect();
        let factory_inline = plan_local
            .members
            .iter()
            .filter_map(|m| m.default.as_ref().map(|d| format!("{}: {d}", m.prop)))
            .collect::<Vec<_>>()
            .join(", ");

        Ok(json!({
            "header_what": format!("table:  {}", t.name),
            "description": t.description,
            "banner": banner_line(t),
            "ident": plan_local.table_ident,
            "import_line": import_line,
            "interface_empty": plan_local.members.is_empty(),
            "defaults_empty": default_rows.is_empty(),
            "factory_inline": factory_inline,
            "default_rows": default_rows,
            "members": members,
        }))
    } else {
        let enums: Vec<Value> = enum_exports(schema)
            .into_iter()
            .map(|(name, export)| {
                // `emitted_enums` already dropped empty enums.
                let e = &schema.enums[name];
                // Numeric bucket only when every member carries an
                // integral value; otherwise members are stringified (name
                // as fallback). The bucket literal is shared by the
                // runtime rows and the `.d.ts` readonly pairs (the only
                // textual divergence, a non-number value in the integral
                // bucket, is unreachable by construction).
                let all_integral = e.values.iter().all(|v| {
                    matches!(
                        &v.value,
                        Some(serde_json::Value::Number(n)) if n.is_i64() || n.is_u64()
                    )
                });
                let mut used: HashSet<String> = HashSet::new();
                let members: Vec<Value> = e
                    .values
                    .iter()
                    .map(|v| {
                        let key = unique_ident(sanitize_ident(&v.name), &mut used);
                        let value = if all_integral {
                            match &v.value {
                                Some(serde_json::Value::Number(n)) => n.to_string(),
                                _ => v.name.clone(),
                            }
                        } else {
                            ts_string_literal(&enum_string_value(v))
                        };
                        json!({ "key": key, "value": value, "desc": v.description })
                    })
                    .collect();
                let base = if all_integral { "number" } else { "string" };
                let typedef_line = format!("/** @typedef {{{base}}} {export} */");
                json!({
                    "name": e.name,
                    "export": export,
                    "description": e.description,
                    "typedef_line": typedef_line,
                    "members": members,
                })
            })
            .collect();
        Ok(json!({
            "header_what": "enums:  shared definitions",
            "emitted_enums": enums,
        }))
    }
}
