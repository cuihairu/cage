//! L7 Game Rule Validators — the plugin API for game-specific business rules.
//!
//! Layering (design §17): Schema handles structure, L6 expressions handle
//! simple per-row assertions, and everything that is genuinely *game logic*
//! (power curves, drop-table economics, quest consistency, …) belongs in a
//! code validator behind this trait — so Cage Core never has to understand
//! any specific game's business.
//!
//! Execution model (finalized in design §17「插件沙箱方案定稿」): in-process
//! trait objects for built-ins and embedder-supplied validators today;
//! third-party distribution moves to dynamic libraries with a C-ABI shim;
//! untrusted code (Registry) is never executed — see the design doc for the
//! full comparison and the migration path.

use crate::diagnostics::{Diagnostic, DiagnosticBuilder};
use crate::error::codes::gamerule;
use crate::schema::Schema;
use crate::value::{Document, Value};

/// A game rule validator plugin. Receives the validated schema and document,
/// returns diagnostics (locations come from the document, codes from the
/// [`gamerule`] family). Registration order = run order.
pub trait GameRuleValidator: Send + Sync {
    /// Stable plugin name, shown in diagnostics and used for enable lists.
    fn name(&self) -> &'static str;

    /// Run the rule over the whole document.
    fn validate(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic>;
}

/// Registry of game rule validators. `cage check --level gamerule` runs
/// every registered validator and merges its diagnostics into the pipeline.
#[derive(Default)]
pub struct GameRuleRegistry {
    validators: Vec<Box<dyn GameRuleValidator>>,
}

impl GameRuleRegistry {
    /// Empty registry (no built-ins).
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry preloaded with Cage's built-in sample rules.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(PowerCurveValidator));
        reg
    }

    /// Register one validator (chainable).
    pub fn register(&mut self, validator: Box<dyn GameRuleValidator>) -> &mut Self {
        self.validators.push(validator);
        self
    }

    /// Run every registered validator, concatenating diagnostics in
    /// registration order (the pipeline sorts by location afterwards).
    pub fn run(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for validator in &self.validators {
            out.extend(validator.validate(schema, document));
        }
        out
    }

    /// Number of registered validators.
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Whether no validator is registered.
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }
}

/// Built-in sample rule: **power curve** — in any table that has both a
/// `level` and an `attack` field, `attack` must not exceed
/// `level * 100 + 50`. Demonstrates the full plugin shape end to end:
/// name → cross-field row scan → `E1601` diagnostics with row locations.
///
/// Tables without both fields are skipped, so the rule is safe to run on
/// any schema. Game projects replace/extend it by registering their own
/// validators — Cage Core never ships game-specific assumptions beyond
/// this one sample.
pub struct PowerCurveValidator;

/// Cap the attack stat may reach at a given level: level * 100 + 50.
fn power_cap(level: f64) -> f64 {
    level * 100.0 + 50.0
}

/// Numeric view of a value (int/uint/float); `None` for anything else.
fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(i) => Some(*i as f64),
        Value::UInt(u) => Some(*u as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

impl GameRuleValidator for PowerCurveValidator {
    fn name(&self) -> &'static str {
        "power_curve"
    }

    fn validate(&self, _schema: &Schema, document: &Document) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for table in document.tables.values() {
            for row in &table.rows {
                let (Some(level), Some(attack)) = (
                    row.fields.get("level").map(|tv| &tv.value),
                    row.fields.get("attack").map(|tv| &tv.value),
                ) else {
                    continue;
                };
                let (Some(level), Some(attack)) = (as_f64(level), as_f64(attack)) else {
                    continue;
                };
                let cap = power_cap(level);
                if attack > cap {
                    out.push(
                        DiagnosticBuilder::error(
                            gamerule::E1601,
                            "Game rule violation: power curve",
                        )
                        .location(row.location.clone())
                        .table(&table.name)
                        .row(format!("{}", row.index))
                        .hint(format!(
                            "power_curve: attack {attack} exceeds the level {level} cap {cap} (level * 100 + 50)"
                        ))
                        .build(),
                    );
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Row, SourceLocation, Table, TypedValue};

    fn row(index: usize, level: i64, attack: i64) -> Row {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "level".to_string(),
            TypedValue::new(Value::Int(level), SourceLocation::default()),
        );
        fields.insert(
            "attack".to_string(),
            TypedValue::new(Value::Int(attack), SourceLocation::default()),
        );
        Row {
            primary_key: vec![Value::Int(i64::try_from(index).unwrap_or(0))],
            fields,
            location: SourceLocation::default(),
            index,
        }
    }

    fn monster_doc(rows: Vec<Row>) -> Document {
        let mut doc = Document::new();
        doc.add_table(Table {
            name: "Monster".to_string(),
            primary_key_fields: vec!["id".to_string()],
            rows,
            source_file: "config/monster.json".to_string(),
            sheet: None,
        });
        doc
    }

    #[test]
    fn registry_runs_builtin_and_reports_e1601() {
        let registry = GameRuleRegistry::with_builtins();
        assert_eq!(registry.len(), 1);

        // level 1 caps attack at 150 — 500 violates; level 5 caps at 550 — passes.
        let doc = monster_doc(vec![row(0, 1, 500), row(1, 5, 400)]);
        let diags = registry.run(&Schema::new(), &doc);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "E1601");
        assert!(diags[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("power_curve") && h.contains("500")));
    }

    #[test]
    fn tables_without_both_fields_are_untouched() {
        let registry = GameRuleRegistry::with_builtins();
        let doc = monster_doc(vec![row(0, 1, 500)]);
        // Rename attack away — the rule no longer applies, no diagnostics.
        let mut doc = doc;
        let table = doc.get_table_mut("Monster").unwrap();
        for r in &mut table.rows {
            r.fields.shift_remove("attack");
        }
        assert!(registry.run(&Schema::new(), &doc).is_empty());
    }

    #[test]
    fn custom_validator_registers_and_runs() {
        struct Marker;
        impl GameRuleValidator for Marker {
            fn name(&self) -> &'static str {
                "marker"
            }
            fn validate(&self, _s: &Schema, _d: &Document) -> Vec<Diagnostic> {
                vec![DiagnosticBuilder::error(gamerule::E1601, "marker fired")
                    .hint("marker".to_string())
                    .build()]
            }
        }

        let mut registry = GameRuleRegistry::new();
        assert!(registry.is_empty());
        registry.register(Box::new(Marker));
        let diags = registry.run(&Schema::new(), &Document::new());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "E1601");
    }

    #[test]
    fn boundary_value_passes() {
        let registry = GameRuleRegistry::with_builtins();
        // Exactly at the cap (level 1 → 150) is allowed.
        let doc = monster_doc(vec![row(0, 1, 150)]);
        assert!(registry.run(&Schema::new(), &doc).is_empty());
    }
}
