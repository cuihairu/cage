//! Rule-file rendering of one migration step: the single-key wire form
//! and the indentation helper (split out of diff.rs, LAY-025).

use super::values::{has_unsigned, quote_string, value_to_yaml_block};
use crate::migrate::Step;
use std::fmt::Write as _;

/// Render one step in the single-key wire form the rule parser decodes.
/// `None` when the step carries a value with no inline-or-block YAML form
/// (a bytes default) — the caller turns that into a `# TODO`.
pub(crate) fn render_step(step: &Step) -> Option<String> {
    match step {
        Step::RenameField { table, from, to } => Some(format!(
            "  - rename_field:\n      table: {table}\n      from: {from}\n      to: {to}\n"
        )),
        Step::SetDefault {
            table,
            field,
            value,
        } => {
            // A bare scalar cannot carry the UInt family (plain `5` reads
            // back as `Int`): unsigned values ride the canonical tagged
            // form, which `yaml_to_value` decodes first.
            if has_unsigned(value) {
                let rendered = serde_yaml::to_string(value).ok()?;
                return Some(format!(
                    "  - set_default:\n      table: {table}\n      field: {field}\n      value:\n{}",
                    indent(8, rendered.trim_end())
                ));
            }
            let rendered = value_to_yaml_block(value, 0)?;
            let trimmed = rendered.trim_end();
            // Scalars ride inline; sequences and mappings need the block
            // form (`value: - 1` is not YAML).
            if trimmed.contains('\n') {
                Some(format!(
                    "  - set_default:\n      table: {table}\n      field: {field}\n      value:\n{}",
                    indent(8, trimmed)
                ))
            } else {
                Some(format!(
                    "  - set_default:\n      table: {table}\n      field: {field}\n      value: {trimmed}\n"
                ))
            }
        }
        Step::RemoveField { table, field } => Some(format!(
            "  - remove_field:\n      table: {table}\n      field: {field}\n"
        )),
        Step::WidenType { table, field, to } => {
            // FieldType is adjacently tagged for serde, so serde_yaml can
            // render it — block form, one `kind:`/`value:` pair per line.
            Some(format!(
                "  - widen_type:\n      table: {table}\n      field: {field}\n      to:\n{}",
                indent(8, &serde_yaml::to_string(to).unwrap_or_default())
            ))
        }
        Step::RemapValues { table, field, map } => {
            let mut out = format!(
                "  - remap_values:\n      table: {table}\n      field: {field}\n      map:\n"
            );
            for (k, v) in map {
                let _ = writeln!(out, "        {}: {}", quote_string(k)?, quote_string(v)?);
            }
            Some(out)
        }
        Step::RenameTable { from, to } => Some(format!(
            "  - rename_table:\n      from: {from}\n      to: {to}\n"
        )),
    }
}

fn indent(n: usize, text: &str) -> String {
    let pad = " ".repeat(n);
    let mut out = String::new();
    for line in text.lines() {
        let _ = writeln!(out, "{pad}{line}");
    }
    out
}
