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
use crate::reference::DependencyGraph;
use crate::schema::Schema;
use crate::value::{Document, Row, SourceLocation, TypedValue, Value};
use std::collections::{HashMap, HashSet};

/// A game rule validator plugin. Receives the validated schema and document,
/// returns diagnostics (locations come from the document, codes from the
/// [`gamerule`] family). Registration order = run order.
pub trait GameRuleValidator: Send + Sync {
    /// Stable plugin name, shown in diagnostics and used for enable lists.
    fn name(&self) -> &'static str;

    /// Run the rule over the whole document.
    fn validate(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic>;
}

/// Stable identifier for a built-in declarative game rule. The rule engine
/// dispatches on this kind; each kind's parameters live in [`RuleParams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleKind {
    /// Skill rows must reference items whose category is in the allowed set.
    SkillItemCategory,
    /// Cross-table reference graph must be acyclic.
    ReferenceCycle,
    /// Drop rewards must reference known items and carry a valid quantity
    /// and weight.
    DropReward,
    /// Dungeon stages must reference existing monster groups and rewards.
    DungeonStage,
    /// A field visible to a profile must not depend on an entity hidden
    /// from that profile.
    ClientServerField,
}

/// Data-declared parameters for the skill/item-category rule.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillItemCategoryConfig {
    /// Table holding the skill rows.
    pub skill_table: String,
    /// Field in the skill table holding the referenced item id (scalar or array).
    pub item_field: String,
    /// Table holding the item definitions.
    pub item_table: String,
    /// Field in the item table holding the unique item id.
    pub item_id_field: String,
    /// Field in the item table holding the item's category.
    pub category_field: String,
    /// Categories a skill may reference.
    pub allowed_categories: Vec<String>,
}

/// Data-declared parameters for the reference-cycle rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceCycleConfig {
    /// Restrict the scan to these table names; empty means every schema table.
    pub table_filter: Vec<String>,
}

/// Data-declared parameters for the drop-reward rule.
#[derive(Debug, Clone, PartialEq)]
pub struct DropRewardConfig {
    /// Table holding the drop rows.
    pub drop_table: String,
    /// Field holding the dropped item id (scalar or array).
    pub item_field: String,
    /// Table holding the item definitions.
    pub item_table: String,
    /// Field holding the unique item id.
    pub item_id_field: String,
    /// Field holding the per-drop quantity (checked when the row has it).
    pub quantity_field: String,
    /// Field holding the drop probability weight (checked when the row has it).
    pub weight_field: String,
}

/// Data-declared parameters for the dungeon-stage rule.
#[derive(Debug, Clone, PartialEq)]
pub struct DungeonStageConfig {
    /// Table holding the stage rows.
    pub stage_table: String,
    /// Field holding the referenced monster-group id (scalar or array).
    pub group_field: String,
    /// Table holding the monster-group definitions.
    pub group_table: String,
    /// Field holding the unique monster-group id.
    pub group_id_field: String,
    /// Field holding the referenced reward id (scalar or array).
    pub reward_field: String,
    /// Table holding the reward definitions.
    pub reward_table: String,
    /// Field holding the unique reward id.
    pub reward_id_field: String,
}

/// Data-declared parameters for the client/server field rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientServerFieldConfig {
    /// The consuming profile (e.g. `"client"`): fields visible to it must
    /// not reference entities hidden from it.
    pub profile: String,
    /// Restrict the scan to these table names; empty means every schema table.
    pub table_filter: Vec<String>,
}

/// Parameters for one declarative rule instance.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleParams {
    /// [`RuleKind::SkillItemCategory`] parameters.
    SkillItemCategory(SkillItemCategoryConfig),
    /// [`RuleKind::ReferenceCycle`] parameters.
    ReferenceCycle(ReferenceCycleConfig),
    /// [`RuleKind::DropReward`] parameters.
    DropReward(DropRewardConfig),
    /// [`RuleKind::DungeonStage`] parameters.
    DungeonStage(DungeonStageConfig),
    /// [`RuleKind::ClientServerField`] parameters.
    ClientServerField(ClientServerFieldConfig),
}

/// A data-declared game rule: stable name, enable flag, deterministic
/// execution order, rule kind and its parameters. The same semantic model
/// drives both the rule engine and the visual rule builder — no second rule
/// format (design review P1-2).
#[derive(Debug, Clone, PartialEq)]
pub struct RuleConfig {
    /// Stable rule name (shown in diagnostics and enable lists).
    pub name: String,
    /// Whether the rule runs. Disabled rules are skipped but stay declared.
    pub enabled: bool,
    /// Execution order; lower runs first. Deterministic on ties (declaration order).
    pub order: u32,
    /// Which built-in check this configures.
    pub kind: RuleKind,
    /// Rule-specific parameters.
    pub params: RuleParams,
}

impl RuleConfig {
    /// A fully-enabled declarative rule with the given execution order.
    pub fn new(name: impl Into<String>, kind: RuleKind, params: RuleParams, order: u32) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            order,
            kind,
            params,
        }
    }

    /// Mark the rule disabled (declared but skipped at run time).
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// One executable rule in the registry: either an embedder-supplied plugin
/// (kept for the trait extension point / phase-2 dynamic libraries) or a
/// data-declared built-in rule. Both carry an explicit execution order.
enum RuleEntry {
    /// Plugin validator with the stable registration order (insertion index).
    Plugin {
        /// Deterministic run position.
        order: u32,
        /// The plugin validator.
        validator: Box<dyn GameRuleValidator>,
    },
    /// Data-declared rule; order and enable flag come from [`RuleConfig`].
    Declarative(RuleConfig),
}

/// Registry of game rule validators and declarative rules. `cage check
/// --level gamerule` runs every registered entry and merges its diagnostics
/// into the pipeline.
#[derive(Default)]
pub struct GameRuleRegistry {
    entries: Vec<RuleEntry>,
    next_plugin_order: u32,
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

    /// Registry preloaded with a data-declared list of game rules. Entries
    /// run in their declared `order`; disabled entries are skipped.
    pub fn with_rules(rules: Vec<RuleConfig>) -> Self {
        let mut reg = Self::new();
        for rule in rules {
            reg.register_rule(rule);
        }
        reg
    }

    /// Register one validator plugin (chainable).
    pub fn register(&mut self, validator: Box<dyn GameRuleValidator>) -> &mut Self {
        let order = self.next_plugin_order;
        self.next_plugin_order += 1;
        self.entries.push(RuleEntry::Plugin { order, validator });
        self
    }

    /// Declare one data-driven game rule (chainable).
    pub fn register_rule(&mut self, rule: RuleConfig) -> &mut Self {
        self.entries.push(RuleEntry::Declarative(rule));
        self
    }

    /// Run every registered entry, concatenating diagnostics in deterministic
    /// execution order (plugin registration order, then declarative `order`;
    /// the pipeline sorts by location afterwards).
    pub fn run(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic> {
        let mut order: Vec<&RuleEntry> = self.entries.iter().collect();
        order.sort_by_key(|entry| match entry {
            RuleEntry::Plugin { order, .. } => *order,
            RuleEntry::Declarative(config) => config.order,
        });

        let mut out = Vec::new();
        for entry in order {
            match entry {
                RuleEntry::Plugin { validator, .. } => {
                    out.extend(validator.validate(schema, document));
                }
                RuleEntry::Declarative(config) if config.enabled => {
                    out.extend(run_rule(config, schema, document));
                }
                RuleEntry::Declarative(_) => {}
            }
        }
        out
    }

    /// Number of registered entries (plugins + declarative rules).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no entry is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Dispatch a data-declared rule to its built-in implementation.
fn run_rule(config: &RuleConfig, schema: &Schema, document: &Document) -> Vec<Diagnostic> {
    match &config.params {
        RuleParams::SkillItemCategory(params) => {
            run_skill_item_category(config, schema, document, params)
        }
        RuleParams::ReferenceCycle(params) => run_reference_cycle(config, schema, document, params),
        RuleParams::DropReward(params) => run_drop_reward(config, document, params),
        RuleParams::DungeonStage(params) => run_dungeon_stage(config, document, params),
        RuleParams::ClientServerField(params) => {
            run_client_server_field(config, schema, document, params)
        }
    }
}

/// L7.1 declarative rule: every skill must reference an item whose category
/// is in the declared allowed set. Resolves the cross-table pair
/// (`skill_table.item_field` → `item_table.item_id_field`) then checks the
/// referenced item's `category_field`. Unresolved references are reported by
/// the L5 reference check, so they are skipped here.
fn run_skill_item_category(
    config: &RuleConfig,
    _schema: &Schema,
    document: &Document,
    params: &SkillItemCategoryConfig,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(skill_table) = document.tables.get(&params.skill_table) else {
        return out;
    };
    let Some(item_table) = document.tables.get(&params.item_table) else {
        return out;
    };

    // item id -> category; the first row wins.
    let mut item_category: HashMap<String, String> = HashMap::new();
    for row in &item_table.rows {
        let Some(id) = row
            .fields
            .get(&params.item_id_field)
            .and_then(|tv| tv.value.coerce_to_string())
        else {
            continue;
        };
        let Some(category) = row
            .fields
            .get(&params.category_field)
            .and_then(|tv| tv.value.coerce_to_string())
        else {
            continue;
        };
        item_category.entry(id).or_insert(category);
    }

    for row in &skill_table.rows {
        let Some(reference_field) = row.fields.get(&params.item_field) else {
            continue;
        };
        let referenced: Vec<Value> = match &reference_field.value {
            Value::Array(items) => items.clone(),
            single => vec![single.clone()],
        };
        for item_ref in referenced {
            let Some(item_id) = item_ref.coerce_to_string() else {
                continue;
            };
            let Some(category) = item_category.get(&item_id) else {
                continue;
            };
            if params.allowed_categories.iter().any(|c| c == category) {
                continue;
            }
            out.push(
                DiagnosticBuilder::error(
                    gamerule::E1604,
                    "Skill references an item in a disallowed category",
                )
                .location(field_location(reference_field, row))
                .table(&params.skill_table)
                .row(format!("{}", row.index))
                .field(&params.item_field)
                .value(item_id.clone())
                .hint(format!(
                    "{}: item '{item_id}' has category '{category}', allowed: {}",
                    config.name,
                    params.allowed_categories.join(", ")
                ))
                .build(),
            );
        }
    }
    out
}

/// L7.2 declarative rule: the cross-table reference graph (built from schema
/// `reference` definitions) must be acyclic. Each detected cycle reports the
/// edge that closes it, located at the referencing field/row.
fn run_reference_cycle(
    config: &RuleConfig,
    schema: &Schema,
    document: &Document,
    params: &ReferenceCycleConfig,
) -> Vec<Diagnostic> {
    let in_scope = |table: &str| {
        params.table_filter.is_empty() || params.table_filter.iter().any(|t| t == table)
    };

    let mut graph = DependencyGraph::new();
    // (from, to) -> the first schema field (deterministic) that declares the reference.
    let mut edge_field: HashMap<(String, String), String> = HashMap::new();
    let mut table_source: HashMap<String, (String, Option<String>)> = HashMap::new();

    for (table_name, table_schema) in &schema.tables {
        if !in_scope(table_name) {
            continue;
        }
        if let Some(table) = document.tables.get(table_name) {
            table_source.insert(
                table_name.clone(),
                (table.source_file.clone(), table.sheet.clone()),
            );
        }
        for (field_name, field_schema) in &table_schema.fields {
            let Some(reference) = &field_schema.reference else {
                continue;
            };
            if !in_scope(&reference.table) {
                continue;
            }
            graph.add_edge(table_name, &reference.table);
            edge_field
                .entry((table_name.clone(), reference.table.clone()))
                .or_insert_with(|| field_name.clone());
        }
    }

    // Canonical form: rotate so the lexicographically smallest table is first,
    // then dedup and sort — deterministic diagnostic order.
    let mut cycles = graph.find_cycles();
    for cycle in &mut cycles {
        if cycle.is_empty() {
            continue;
        }
        let Some(min_index) = cycle
            .iter()
            .enumerate()
            .min_by_key(|(_, name)| name.as_str())
            .map(|(index, _)| index)
        else {
            continue;
        };
        cycle.rotate_left(min_index);
    }
    cycles.sort();
    cycles.dedup();

    let mut out = Vec::new();
    for cycle in cycles {
        // The edge from the rotated last table back to the first closes the
        // cycle — that is the point of detection with a precise location.
        let from = cycle.last().cloned().unwrap_or_default();
        let to = cycle.first().cloned().unwrap_or_default();
        let Some(field) = edge_field.get(&(from.clone(), to.clone())).cloned() else {
            continue;
        };
        let location = reference_edge_location(document, &from, &field, &table_source);
        let mut cycle_text = cycle.join(" -> ");
        cycle_text.push_str(" -> ");
        cycle_text.push_str(&to);
        out.push(
            DiagnosticBuilder::error(
                gamerule::E1605,
                "Cross-table reference cycle detected",
            )
            .location(location)
            .table(&from)
            .field(field.clone())
            .value(field.clone())
            .hint(format!(
                "{}: {from} -> {to} closes the cycle ({cycle_text}); break the ring or drop the reference",
                config.name,
            ))
            .build(),
        );
    }
    out
}

/// L7.3 declarative rule: every drop row must reference a known item and,
/// when it declares them, carry a non-negative whole-number quantity and a
/// finite probability weight within `[0, 1]`. Unknown item references are
/// reported here (the L5 check covers schema-declared references; a
/// data-declared drop relation needs its own cross-table resolution).
fn run_drop_reward(
    config: &RuleConfig,
    document: &Document,
    params: &DropRewardConfig,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(drop_table) = document.tables.get(&params.drop_table) else {
        return out;
    };
    let Some(item_table) = document.tables.get(&params.item_table) else {
        return out;
    };
    let item_ids = id_set(item_table, &params.item_id_field);

    for row in &drop_table.rows {
        let Some(item_field) = row.fields.get(&params.item_field) else {
            continue;
        };
        for item_ref in as_values(&item_field.value) {
            let Some(item_id) = item_ref.coerce_to_string() else {
                continue;
            };
            if item_ids.contains(&item_id) {
                continue;
            }
            out.push(
                DiagnosticBuilder::error(gamerule::E1606, "Drop rewards an unknown item")
                    .location(field_location(item_field, row))
                    .table(&params.drop_table)
                    .row(format!("{}", row.index))
                    .field(&params.item_field)
                    .value(item_id.clone())
                    .hint(format!(
                        "{}: item '{item_id}' is not in {}.{}",
                        config.name, params.item_table, params.item_id_field
                    ))
                    .build(),
            );
        }

        if let Some(quantity) = row.fields.get(&params.quantity_field) {
            let valid = as_f64(&quantity.value)
                .is_some_and(|f| f.is_finite() && f >= 0.0 && f.fract() == 0.0);
            if !valid {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1606,
                        "Drop quantity must be a non-negative whole number",
                    )
                    .location(field_location(quantity, row))
                    .table(&params.drop_table)
                    .row(format!("{}", row.index))
                    .field(&params.quantity_field)
                    .value(quantity.value.to_string())
                    .hint(format!("{}: got {:?}", config.name, quantity.value))
                    .build(),
                );
            }
        }

        if let Some(weight) = row.fields.get(&params.weight_field) {
            let valid =
                as_f64(&weight.value).is_some_and(|f| f.is_finite() && (0.0..=1.0).contains(&f));
            if !valid {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1606,
                        "Drop weight must be a finite number within [0, 1]",
                    )
                    .location(field_location(weight, row))
                    .table(&params.drop_table)
                    .row(format!("{}", row.index))
                    .field(&params.weight_field)
                    .value(weight.value.to_string())
                    .hint(format!("{}: got {:?}", config.name, weight.value))
                    .build(),
                );
            }
        }
    }
    out
}

/// L7.4 declarative rule: every dungeon stage must reference a monster group
/// and a reward that actually exist in their definition tables. Both edges
/// are data-declared relations (not schema references), so they are resolved
/// and enforced here.
fn run_dungeon_stage(
    config: &RuleConfig,
    document: &Document,
    params: &DungeonStageConfig,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let Some(stage_table) = document.tables.get(&params.stage_table) else {
        return out;
    };
    let Some(group_table) = document.tables.get(&params.group_table) else {
        return out;
    };
    let Some(reward_table) = document.tables.get(&params.reward_table) else {
        return out;
    };
    let group_ids = id_set(group_table, &params.group_id_field);
    let reward_ids = id_set(reward_table, &params.reward_id_field);

    for row in &stage_table.rows {
        for (field_name, target_table, allowed) in [
            (&params.group_field, &params.group_table, &group_ids),
            (&params.reward_field, &params.reward_table, &reward_ids),
        ] {
            let Some(field) = row.fields.get(field_name) else {
                continue;
            };
            for reference in as_values(&field.value) {
                let Some(id) = reference.coerce_to_string() else {
                    continue;
                };
                if allowed.contains(&id) {
                    continue;
                }
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1607,
                        "Dungeon stage references an unknown monster group or reward",
                    )
                    .location(field_location(field, row))
                    .table(&params.stage_table)
                    .row(format!("{}", row.index))
                    .field(field_name)
                    .value(id.clone())
                    .hint(format!(
                        "{}: '{id}' is not a row of {}",
                        config.name, target_table
                    ))
                    .build(),
                );
            }
        }
    }
    out
}

/// L7.5 declarative rule: a field exposed to a profile must not reference an
/// entity (table or field) that the profile projection hides — otherwise the
/// profile build would silently dangle. Both the referenced table and the
/// referenced field are checked against the same visibility rule the profile
/// projection uses (empty targets = visible everywhere, `*` = wildcard).
fn run_client_server_field(
    config: &RuleConfig,
    schema: &Schema,
    document: &Document,
    params: &ClientServerFieldConfig,
) -> Vec<Diagnostic> {
    let in_scope = |table: &str| {
        params.table_filter.is_empty() || params.table_filter.iter().any(|t| t == table)
    };
    let mut table_source: HashMap<String, (String, Option<String>)> = HashMap::new();
    for (name, table) in &document.tables {
        table_source.insert(
            name.clone(),
            (table.source_file.clone(), table.sheet.clone()),
        );
    }

    let mut out = Vec::new();
    for (table_name, table_schema) in &schema.tables {
        if !in_scope(table_name) {
            continue;
        }
        for (field_name, field_schema) in &table_schema.fields {
            if !visible_to(&field_schema.targets, &params.profile) {
                continue;
            }
            let Some(reference) = &field_schema.reference else {
                continue;
            };
            // Unknown reference targets are the L5 reference check's report.
            let Some(target_schema) = schema.tables.get(&reference.table) else {
                continue;
            };
            if !visible_to(&target_schema.targets, &params.profile) {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1608,
                        "A field visible to the profile references a table hidden from it",
                    )
                    .location(reference_edge_location(
                        document,
                        table_name,
                        field_name,
                        &table_source,
                    ))
                    .table(table_name)
                    .field(field_name)
                    .value(format!("{}.{}", reference.table, reference.field))
                    .hint(format!(
                        "{}: {table_name}.{field_name} is visible to profile {:?} but \
                         references {}.{}, which is not (targets: {:?})",
                        config.name,
                        params.profile,
                        reference.table,
                        reference.field,
                        target_schema.targets,
                    ))
                    .build(),
                );
                continue;
            }
            let Some(target_field) = target_schema.fields.get(&reference.field) else {
                continue; // unknown referenced field: L5's job
            };
            if !visible_to(&target_field.targets, &params.profile) {
                out.push(
                    DiagnosticBuilder::error(
                        gamerule::E1608,
                        "A field visible to the profile references a field hidden from it",
                    )
                    .location(reference_edge_location(
                        document,
                        table_name,
                        field_name,
                        &table_source,
                    ))
                    .table(table_name)
                    .field(field_name)
                    .value(format!("{}.{}", reference.table, reference.field))
                    .hint(format!(
                        "{}: {table_name}.{field_name} is visible to profile {:?} but \
                         references {}.{}, which is not (targets: {:?})",
                        config.name,
                        params.profile,
                        reference.table,
                        reference.field,
                        target_field.targets,
                    ))
                    .build(),
                );
            }
        }
    }
    out
}

/// The set of string ids a table declares in one field (scalar or array
/// values, first row wins per id).
fn id_set(table: &crate::value::Table, id_field: &str) -> HashSet<String> {
    table
        .rows
        .iter()
        .filter_map(|row| row.fields.get(id_field))
        .flat_map(|tv| as_values(&tv.value))
        .filter_map(|value| value.coerce_to_string())
        .collect()
}

/// A scalar-or-array field's values, each value cloned (mirrors the
/// skill-item rule's handling of array item fields).
fn as_values(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        single => vec![single.clone()],
    }
}

/// Whether a target list exposes an entity to the given profile: empty
/// targets are visible everywhere and `*` is a wildcard — the same rule the
/// profile projection applies.
fn visible_to(targets: &[String], profile: &str) -> bool {
    targets.is_empty() || targets.iter().any(|t| t == profile || t == "*")
}

/// Choose the most precise location for a field: the typed value's own
/// location when it carries row/line/column info, otherwise the row location.
fn field_location(field: &TypedValue, row: &Row) -> SourceLocation {
    let location = &field.location;
    if location.row.is_some()
        || location.line.is_some()
        || location.col.is_some()
        || location.column.is_some()
    {
        location.clone()
    } else {
        row.location.clone()
    }
}

/// Locate a reference edge at the referencing field, falling back to the
/// table's source location when no row carries the field.
fn reference_edge_location(
    document: &Document,
    from_table: &str,
    field: &str,
    table_source: &HashMap<String, (String, Option<String>)>,
) -> SourceLocation {
    if let Some(table) = document.tables.get(from_table) {
        if let Some(row) = table.rows.iter().find(|row| row.fields.contains_key(field)) {
            return field_location(&row.fields[field], row);
        }
    }
    let (file, sheet) = table_source.get(from_table).cloned().unwrap_or_default();
    let mut location = SourceLocation::new(file);
    location.sheet = sheet;
    location.field = Some(field.to_string());
    location
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
    use crate::schema::{FieldSchema, FieldType, ReferenceSchema, TableSchema};
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

    /// Same shape as [`row`] but with caller-chosen value families — used to
    /// exercise the numeric views the power curve accepts.
    fn flex_row(index: usize, level: Value, attack: Value) -> Row {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "level".to_string(),
            TypedValue::new(level, SourceLocation::default()),
        );
        fields.insert(
            "attack".to_string(),
            TypedValue::new(attack, SourceLocation::default()),
        );
        Row {
            primary_key: vec![],
            fields,
            location: SourceLocation::default(),
            index,
        }
    }

    #[test]
    fn power_curve_checks_uint_and_float_stats() {
        let registry = GameRuleRegistry::with_builtins();
        let rows = vec![
            // level 5 → cap 550; 600 violates.
            flex_row(0, Value::UInt(5), Value::UInt(600)),
            // level 1.0 → cap 150; 200.0 violates.
            flex_row(1, Value::Float(1.0), Value::Float(200.0)),
        ];
        let diags = registry.run(&Schema::new(), &monster_doc(rows));
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.code == gamerule::E1601));
        assert!(diags.iter().any(|d| {
            d.hint
                .as_deref()
                .is_some_and(|h| h.contains("attack 600") && h.contains("level 5"))
        }));
    }

    #[test]
    fn power_curve_skips_rows_with_non_numeric_stats() {
        let registry = GameRuleRegistry::with_builtins();
        let rows = vec![
            flex_row(0, Value::String("high".to_string()), Value::Int(9)),
            flex_row(1, Value::Int(9), Value::String("max".to_string())),
            flex_row(2, Value::Null, Value::Null),
        ];
        assert!(registry.run(&Schema::new(), &monster_doc(rows)).is_empty());
    }

    #[test]
    fn builtin_validator_reports_its_name() {
        assert_eq!(PowerCurveValidator.name(), "power_curve");
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
        let marker = Box::new(Marker);
        assert_eq!(marker.name(), "marker");
        registry.register(marker);
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

    fn declarative_field(
        name: &str,
        value: Value,
        location: Option<SourceLocation>,
    ) -> (String, TypedValue) {
        (
            name.to_string(),
            TypedValue::new(value, location.unwrap_or_default()),
        )
    }

    fn declarative_row(
        fields: Vec<(String, TypedValue)>,
        location: Option<SourceLocation>,
        index: usize,
    ) -> Row {
        let mut map = indexmap::IndexMap::new();
        for (name, value) in fields {
            map.insert(name, value);
        }
        Row {
            primary_key: vec![],
            fields: map,
            location: location.unwrap_or_default(),
            index,
        }
    }

    fn declarative_table(name: &str, rows: Vec<Row>) -> Table {
        Table {
            name: name.to_string(),
            primary_key_fields: vec![],
            rows,
            source_file: format!("config/{name}.json"),
            sheet: None,
        }
    }

    fn plain_table_schema(
        name: &str,
        fields: indexmap::IndexMap<String, FieldSchema>,
    ) -> TableSchema {
        TableSchema {
            name: name.to_string(),
            description: None,
            primary_key: vec![],
            fields,
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: indexmap::IndexMap::new(),
        }
    }

    fn reference_field(name: &str, target: &str) -> FieldSchema {
        FieldSchema {
            name: name.to_string(),
            field_type: FieldType::UInt32,
            description: None,
            required: false,
            nullable: false,
            default: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            enum_values: None,
            min_items: None,
            max_items: None,
            items: None,
            properties: None,
            additional_properties: None,
            reference: Some(ReferenceSchema {
                table: target.to_string(),
                field: "id".to_string(),
                predicate: None,
                cardinality: "one".to_string(),
                compatible_with: None,
            }),
            targets: vec![],
            rules: vec![],
            metadata: indexmap::IndexMap::new(),
        }
    }

    fn item_category_rule() -> RuleConfig {
        RuleConfig::new(
            "skill_item_category",
            RuleKind::SkillItemCategory,
            RuleParams::SkillItemCategory(SkillItemCategoryConfig {
                skill_table: "Skill".to_string(),
                item_field: "item".to_string(),
                item_table: "Item".to_string(),
                item_id_field: "id".to_string(),
                category_field: "category".to_string(),
                allowed_categories: vec!["consumable".to_string(), "equipment".to_string()],
            }),
            10,
        )
    }

    #[test]
    fn skill_item_category_reports_violation_with_field_location() {
        let items = declarative_table(
            "Item",
            vec![
                declarative_row(
                    vec![
                        declarative_field("id", Value::UInt(100), None),
                        declarative_field("category", Value::String("equipment".into()), None),
                    ],
                    None,
                    0,
                ),
                declarative_row(
                    vec![
                        declarative_field("id", Value::UInt(200), None),
                        declarative_field("category", Value::String("pet".into()), None),
                    ],
                    None,
                    1,
                ),
            ],
        );
        let skill_location = SourceLocation::new("config/Skill.json")
            .with_line_col(7, 9)
            .with_field("item");
        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![
                    declarative_field("id", Value::UInt(1), None),
                    declarative_field("item", Value::UInt(200), Some(skill_location.clone())),
                ],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(skills);

        let registry = GameRuleRegistry::with_rules(vec![item_category_rule()]);
        let diags = registry.run(&Schema::new(), &doc);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, gamerule::E1604);
        assert_eq!(diags[0].table.as_deref(), Some("Skill"));
        assert_eq!(diags[0].row.as_deref(), Some("0"));
        assert_eq!(diags[0].field.as_deref(), Some("item"));
        assert_eq!(diags[0].location.as_ref(), Some(&skill_location));
        assert!(diags[0].hint.as_deref().is_some_and(|hint| {
            hint.contains("skill_item_category")
                && hint.contains("item '200'")
                && hint.contains("category 'pet'")
                && hint.contains("consumable, equipment")
        }));
    }

    #[test]
    fn skill_item_category_accepts_allowed_scalar_and_array_items() {
        let items = declarative_table(
            "Item",
            vec![
                declarative_row(
                    vec![
                        declarative_field("id", Value::UInt(100), None),
                        declarative_field("category", Value::String("equipment".into()), None),
                    ],
                    None,
                    0,
                ),
                declarative_row(
                    vec![
                        declarative_field("id", Value::UInt(200), None),
                        declarative_field("category", Value::String("consumable".into()), None),
                    ],
                    None,
                    1,
                ),
            ],
        );
        let skills = declarative_table(
            "Skill",
            vec![
                declarative_row(
                    vec![declarative_field("item", Value::UInt(100), None)],
                    None,
                    0,
                ),
                declarative_row(
                    vec![declarative_field(
                        "item",
                        Value::Array(vec![Value::UInt(200), Value::UInt(100)]),
                        None,
                    )],
                    None,
                    1,
                ),
            ],
        );
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(skills);

        let registry = GameRuleRegistry::with_rules(vec![item_category_rule()]);
        assert!(registry.run(&Schema::new(), &doc).is_empty());
    }

    #[test]
    fn disabled_skill_item_category_rule_is_declared_but_skipped() {
        let items = declarative_table(
            "Item",
            vec![declarative_row(
                vec![
                    declarative_field("id", Value::UInt(200), None),
                    declarative_field("category", Value::String("pet".into()), None),
                ],
                None,
                0,
            )],
        );
        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![declarative_field("item", Value::UInt(200), None)],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(skills);

        let rule = item_category_rule().disabled();
        let registry = GameRuleRegistry::with_rules(vec![rule]);
        assert_eq!(registry.len(), 1);
        assert!(registry.run(&Schema::new(), &doc).is_empty());
    }

    #[test]
    fn declarative_rules_run_in_declared_order() {
        let items = declarative_table(
            "Item",
            vec![declarative_row(
                vec![
                    declarative_field("id", Value::UInt(200), None),
                    declarative_field("category", Value::String("pet".into()), None),
                ],
                None,
                0,
            )],
        );
        let skill = |table: &str| {
            declarative_table(
                table,
                vec![declarative_row(
                    vec![declarative_field("item", Value::UInt(200), None)],
                    None,
                    0,
                )],
            )
        };
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(skill("SkillA"));
        doc.add_table(skill("SkillB"));

        // Declared late first, but order 10 must run before order 20.
        let late = RuleConfig::new(
            "skill_b",
            RuleKind::SkillItemCategory,
            RuleParams::SkillItemCategory(SkillItemCategoryConfig {
                skill_table: "SkillB".to_string(),
                item_field: "item".to_string(),
                item_table: "Item".to_string(),
                item_id_field: "id".to_string(),
                category_field: "category".to_string(),
                allowed_categories: vec!["equipment".to_string()],
            }),
            20,
        );
        let early = RuleConfig::new(
            "skill_a",
            RuleKind::SkillItemCategory,
            RuleParams::SkillItemCategory(SkillItemCategoryConfig {
                skill_table: "SkillA".to_string(),
                item_field: "item".to_string(),
                item_table: "Item".to_string(),
                item_id_field: "id".to_string(),
                category_field: "category".to_string(),
                allowed_categories: vec!["equipment".to_string()],
            }),
            10,
        );
        let registry = GameRuleRegistry::with_rules(vec![late, early]);
        let diags = registry.run(&Schema::new(), &doc);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].table.as_deref(), Some("SkillA"));
        assert_eq!(diags[1].table.as_deref(), Some("SkillB"));
    }

    fn cyclic_schema() -> Schema {
        let mut schema = Schema::new();
        for (table, field_name, target) in [
            ("Skill", "required_item", "Item"),
            ("Item", "required_skill", "Skill"),
        ] {
            let mut fields = indexmap::IndexMap::new();
            fields.insert(field_name.to_string(), reference_field(field_name, target));
            schema.add_table(plain_table_schema(table, fields));
        }
        schema
    }

    #[test]
    fn reference_cycle_reports_closing_edge_with_field_location() {
        let schema = cyclic_schema();
        let items = declarative_table(
            "Item",
            vec![declarative_row(
                vec![(
                    "required_skill".to_string(),
                    TypedValue::new(Value::UInt(1), SourceLocation::default()),
                )],
                None,
                0,
            )],
        );
        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![(
                    "required_item".to_string(),
                    TypedValue::new(
                        Value::UInt(1),
                        SourceLocation::new("config/Skill.json")
                            .with_line_col(5, 7)
                            .with_field("required_item"),
                    ),
                )],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(skills);
        let rule = RuleConfig::new(
            "reference_cycle",
            RuleKind::ReferenceCycle,
            RuleParams::ReferenceCycle(ReferenceCycleConfig {
                table_filter: vec![],
            }),
            20,
        );

        let registry = GameRuleRegistry::with_rules(vec![rule]);
        let diags = registry.run(&schema, &doc);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, gamerule::E1605);
        assert_eq!(diags[0].table.as_deref(), Some("Skill"));
        assert_eq!(diags[0].field.as_deref(), Some("required_item"));
        let location = diags[0].location.clone().unwrap();
        assert_eq!(location.file, "config/Skill.json");
        assert_eq!(location.line, Some(5));
        assert_eq!(location.col, Some(7));
        assert!(diags[0].hint.as_deref().is_some_and(|hint| {
            hint.contains("Skill -> Item") && hint.contains("Item -> Skill -> Item")
        }));
    }

    #[test]
    fn reference_cycle_accepts_acyclic_schema() {
        let mut schema = Schema::new();
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "required_item".to_string(),
            reference_field("required_item", "Item"),
        );
        schema.add_table(plain_table_schema("Skill", fields));
        schema.add_table(plain_table_schema("Item", indexmap::IndexMap::new()));
        let rule = RuleConfig::new(
            "reference_cycle",
            RuleKind::ReferenceCycle,
            RuleParams::ReferenceCycle(ReferenceCycleConfig {
                table_filter: vec![],
            }),
            0,
        );
        let registry = GameRuleRegistry::with_rules(vec![rule]);
        assert!(registry.run(&schema, &Document::new()).is_empty());
    }

    #[test]
    fn reference_cycle_detects_self_reference() {
        // A table referencing itself is a one-edge cycle.
        let mut fields = indexmap::IndexMap::new();
        fields.insert("parent".to_string(), reference_field("parent", "Skill"));
        let mut schema = Schema::new();
        schema.add_table(plain_table_schema("Skill", fields));
        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![(
                    "parent".to_string(),
                    TypedValue::new(
                        Value::UInt(1),
                        SourceLocation::new("config/Skill.json")
                            .with_line_col(9, 3)
                            .with_field("parent"),
                    ),
                )],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(skills);
        let rule = RuleConfig::new(
            "reference_cycle",
            RuleKind::ReferenceCycle,
            RuleParams::ReferenceCycle(ReferenceCycleConfig {
                table_filter: vec![],
            }),
            0,
        );
        let registry = GameRuleRegistry::with_rules(vec![rule]);
        let diags = registry.run(&schema, &doc);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, gamerule::E1605);
        assert_eq!(diags[0].table.as_deref(), Some("Skill"));
        assert_eq!(diags[0].field.as_deref(), Some("parent"));

        assert!(diags[0]
            .hint
            .as_deref()
            .is_some_and(|hint| { hint.contains("Skill -> Skill") }));
    }

    fn drop_reward_rule() -> RuleConfig {
        RuleConfig::new(
            "drop_reward",
            RuleKind::DropReward,
            RuleParams::DropReward(DropRewardConfig {
                drop_table: "Drop".to_string(),
                item_field: "item".to_string(),
                item_table: "Item".to_string(),
                item_id_field: "id".to_string(),
                quantity_field: "quantity".to_string(),
                weight_field: "weight".to_string(),
            }),
            20,
        )
    }

    #[test]
    fn drop_reward_reports_unknown_item_and_invalid_quantity_weight() {
        let items = declarative_table(
            "Item",
            vec![declarative_row(
                vec![declarative_field("id", Value::UInt(100), None)],
                None,
                0,
            )],
        );
        let drops = declarative_table(
            "Drop",
            vec![declarative_row(
                vec![
                    declarative_field("item", Value::UInt(999), None),
                    declarative_field("quantity", Value::Int(-1), None),
                    declarative_field("weight", Value::Float(1.5), None),
                ],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(drops);

        let registry = GameRuleRegistry::with_rules(vec![drop_reward_rule()]);
        let diags = registry.run(&Schema::new(), &doc);
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert!(diags.iter().all(|d| d.code == gamerule::E1606));
        assert!(diags.iter().all(|d| d.table.as_deref() == Some("Drop")));
        let fields: Vec<&str> = diags.iter().filter_map(|d| d.field.as_deref()).collect();
        assert_eq!(fields, ["item", "quantity", "weight"]);
    }

    #[test]
    fn drop_reward_accepts_known_item_and_valid_quantity_weight() {
        let items = declarative_table(
            "Item",
            vec![declarative_row(
                vec![declarative_field("id", Value::UInt(100), None)],
                None,
                0,
            )],
        );
        let drops = declarative_table(
            "Drop",
            vec![declarative_row(
                vec![
                    declarative_field("item", Value::Array(vec![Value::UInt(100)]), None),
                    declarative_field("quantity", Value::UInt(2), None),
                    declarative_field("weight", Value::Float(0.0), None),
                ],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(items);
        doc.add_table(drops);

        let registry = GameRuleRegistry::with_rules(vec![drop_reward_rule()]);
        assert!(registry.run(&Schema::new(), &doc).is_empty());
    }

    fn dungeon_stage_rule() -> RuleConfig {
        RuleConfig::new(
            "dungeon_stage",
            RuleKind::DungeonStage,
            RuleParams::DungeonStage(DungeonStageConfig {
                stage_table: "Stage".to_string(),
                group_field: "monsters".to_string(),
                group_table: "MonsterGroup".to_string(),
                group_id_field: "id".to_string(),
                reward_field: "reward".to_string(),
                reward_table: "Reward".to_string(),
                reward_id_field: "id".to_string(),
            }),
            30,
        )
    }

    #[test]
    fn dungeon_stage_reports_unresolved_group_and_reward() {
        let groups = declarative_table(
            "MonsterGroup",
            vec![declarative_row(
                vec![declarative_field("id", Value::String("g1".into()), None)],
                None,
                0,
            )],
        );
        let rewards = declarative_table(
            "Reward",
            vec![declarative_row(
                vec![declarative_field("id", Value::String("r1".into()), None)],
                None,
                0,
            )],
        );
        let stages = declarative_table(
            "Stage",
            vec![declarative_row(
                vec![
                    declarative_field(
                        "monsters",
                        Value::Array(vec![Value::String("g1".into()), Value::String("g2".into())]),
                        None,
                    ),
                    declarative_field("reward", Value::String("r9".into()), None),
                ],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(groups);
        doc.add_table(rewards);
        doc.add_table(stages);

        let registry = GameRuleRegistry::with_rules(vec![dungeon_stage_rule()]);
        let diags = registry.run(&Schema::new(), &doc);
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert!(diags.iter().all(|d| d.code == gamerule::E1607));
        assert_eq!(
            diags
                .iter()
                .filter_map(|d| d.field.as_deref())
                .collect::<Vec<_>>(),
            ["monsters", "reward"]
        );
    }

    #[test]
    fn dungeon_stage_accepts_resolved_group_and_reward() {
        let groups = declarative_table(
            "MonsterGroup",
            vec![declarative_row(
                vec![declarative_field("id", Value::String("g1".into()), None)],
                None,
                0,
            )],
        );
        let rewards = declarative_table(
            "Reward",
            vec![declarative_row(
                vec![declarative_field("id", Value::String("r1".into()), None)],
                None,
                0,
            )],
        );
        let stages = declarative_table(
            "Stage",
            vec![declarative_row(
                vec![
                    declarative_field("monsters", Value::String("g1".into()), None),
                    declarative_field("reward", Value::String("r1".into()), None),
                ],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(groups);
        doc.add_table(rewards);
        doc.add_table(stages);

        let registry = GameRuleRegistry::with_rules(vec![dungeon_stage_rule()]);
        assert!(registry.run(&Schema::new(), &doc).is_empty());
    }

    fn client_server_rule() -> RuleConfig {
        RuleConfig::new(
            "client_server_field",
            RuleKind::ClientServerField,
            RuleParams::ClientServerField(ClientServerFieldConfig {
                profile: "client".to_string(),
                table_filter: vec![],
            }),
            40,
        )
    }

    fn targeted_reference_field(name: &str, target: &str, targets: &[&str]) -> FieldSchema {
        let mut field = reference_field(name, target);
        field.targets = targets.iter().map(ToString::to_string).collect();
        field
    }

    #[test]
    fn client_server_field_reports_dependency_on_server_only_table() {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "client_item".to_string(),
            targeted_reference_field("client_item", "Item", &["client"]),
        );
        let mut schema = Schema::new();
        let mut skill_schema = plain_table_schema("Skill", fields);
        skill_schema.targets = vec!["client".to_string()];
        schema.add_table(skill_schema);
        let mut item_schema = plain_table_schema("Item", indexmap::IndexMap::new());
        item_schema.targets = vec!["server".to_string()];
        schema.add_table(item_schema);

        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![declarative_field("client_item", Value::UInt(1), None)],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(skills);

        let registry = GameRuleRegistry::with_rules(vec![client_server_rule()]);
        let diags = registry.run(&schema, &doc);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, gamerule::E1608);
        assert_eq!(diags[0].table.as_deref(), Some("Skill"));
        assert_eq!(diags[0].field.as_deref(), Some("client_item"));
    }

    #[test]
    fn client_server_field_reports_dependency_on_server_only_field() {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "item".to_string(),
            targeted_reference_field("item", "Item", &["client"]),
        );
        let mut schema = Schema::new();
        let mut skill_schema = plain_table_schema("Skill", fields);
        skill_schema.targets = vec!["client".to_string()];
        schema.add_table(skill_schema);
        let mut item_fields = indexmap::IndexMap::new();
        let mut id_field = reference_field("id", "Skill");
        id_field.targets = vec!["server".to_string()];
        item_fields.insert("id".to_string(), id_field);
        let mut item_schema = plain_table_schema("Item", item_fields);
        item_schema.targets = vec!["client".to_string()];
        schema.add_table(item_schema);

        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![declarative_field("item", Value::UInt(1), None)],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(skills);

        let registry = GameRuleRegistry::with_rules(vec![client_server_rule()]);
        let diags = registry.run(&schema, &doc);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, gamerule::E1608);
        assert_eq!(diags[0].field.as_deref(), Some("item"));
        assert!(diags[0]
            .hint
            .as_deref()
            .is_some_and(|hint| { hint.contains("Item.id") }));
    }

    #[test]
    fn client_server_field_accepts_shared_targets() {
        let mut fields = indexmap::IndexMap::new();
        fields.insert(
            "item".to_string(),
            targeted_reference_field("item", "Item", &["client"]),
        );
        let mut schema = Schema::new();
        let mut skill_schema = plain_table_schema("Skill", fields);
        skill_schema.targets = vec!["client".to_string()];
        schema.add_table(skill_schema);
        let mut item_fields = indexmap::IndexMap::new();
        item_fields.insert("id".to_string(), reference_field("id", "Skill"));
        let item_schema = plain_table_schema("Item", item_fields);
        schema.add_table(item_schema);

        let skills = declarative_table(
            "Skill",
            vec![declarative_row(
                vec![declarative_field("item", Value::UInt(1), None)],
                None,
                0,
            )],
        );
        let mut doc = Document::new();
        doc.add_table(skills);

        let registry = GameRuleRegistry::with_rules(vec![client_server_rule()]);
        assert!(registry.run(&schema, &doc).is_empty());
    }
}
