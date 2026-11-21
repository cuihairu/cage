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

use crate::diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics};
use crate::error::codes::gamerule;
use crate::plugin::{run_plugin, PluginManifest, WorkerOutcome, WorkerRun};
use crate::schema::Schema;
use crate::value::{Document, Value};
use std::path::{Path, PathBuf};

use declarative::{
    run_client_server_field, run_drop_reward, run_dungeon_stage, run_reference_cycle,
    run_skill_item_category,
};
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

/// A worker plugin (an independent-process validator) registered with the
/// unified L7 executor, together with the project context it needs to run.
struct WorkerRule {
    /// Deterministic run position, from the shared registration order.
    order: u32,
    /// The project root the worker process is spawned in.
    root: PathBuf,
    /// The loaded manifest, or the load failure to report at run time — a
    /// missing or unreadable plugin fails validation rather than being
    /// silently skipped (review §四.2 插件缺失 → 校验失败).
    manifest: Result<PluginManifest, Diagnostics>,
    /// Project options forwarded to the plugin (read-only for it).
    options: serde_json::Value,
}

/// One executable rule in the registry: an in-process Rust validator, a
/// data-declared built-in rule, or an independent-process worker plugin. All
/// three are driven by the same `run` and merge into one diagnostics surface
/// (review P2-3: Rust built-ins and worker plugins share the L7 surface).
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
    /// Independent-process worker plugin (P2-2); runs as `python3 <entry>`.
    Worker(WorkerRule),
}

/// The unified L7 executor: runs in-process Rust validators, data-declared
/// built-in rules and independent-process worker plugins, merging their
/// diagnostics into one surface for the build gate. `cage check
/// --level gamerule` runs every registered entry through a single `run`.
#[derive(Default)]
pub struct GameRuleRegistry {
    entries: Vec<RuleEntry>,
    next_order: u32,
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
        let order = self.next_order;
        self.next_order += 1;
        self.entries.push(RuleEntry::Plugin { order, validator });
        self
    }

    /// Declare one data-driven game rule (chainable).
    pub fn register_rule(&mut self, rule: RuleConfig) -> &mut Self {
        self.entries.push(RuleEntry::Declarative(rule));
        self
    }

    /// Register a loaded worker plugin (chainable). The worker runs as an
    /// independent process at `run` time, in the same ordered pass as the
    /// in-process validators.
    pub fn register_worker(
        &mut self,
        root: impl Into<PathBuf>,
        manifest: PluginManifest,
        options: serde_json::Value,
    ) -> &mut Self {
        self.push_worker(root.into(), Ok(manifest), options);
        self
    }

    /// Register a worker plugin from a `plugins/<id>` directory, keeping a
    /// load failure as a run-time E1602 instead of dropping the plugin.
    pub fn register_worker_dir(
        &mut self,
        root: impl Into<PathBuf>,
        dir: &Path,
        options: serde_json::Value,
    ) -> &mut Self {
        let root = root.into();
        self.push_worker(root.clone(), PluginManifest::load(dir), options);
        self
    }

    /// Discover every `plugins/<id>/plugin.toml` under `root` and register it
    /// as a worker. A missing `plugins/` directory is not an error (no plugins
    /// is a valid project); a malformed manifest surfaces as E1602 at `run`
    /// time.
    pub fn discover_workers(&mut self, root: impl AsRef<Path>) -> &mut Self {
        let root = root.as_ref();
        let plugins_dir = root.join("plugins");
        let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
            return self;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        // Deterministic discovery order: id order, not filesystem order.
        dirs.sort();
        for dir in dirs {
            self.register_worker_dir(root, &dir, serde_json::json!({}));
        }
        self
    }

    fn push_worker(
        &mut self,
        root: PathBuf,
        manifest: Result<PluginManifest, Diagnostics>,
        options: serde_json::Value,
    ) {
        let order = self.next_order;
        self.next_order += 1;
        self.entries.push(RuleEntry::Worker(WorkerRule {
            order,
            root,
            manifest,
            options,
        }));
    }

    /// Run every registered entry, concatenating diagnostics in deterministic
    /// execution order (shared registration order for plugins and workers,
    /// then declarative `order`; the pipeline sorts by location afterwards).
    /// Worker findings and worker failures land on the same diagnostics
    /// surface as the in-process rules.
    pub fn run(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic> {
        let mut order: Vec<&RuleEntry> = self.entries.iter().collect();
        order.sort_by_key(|entry| match entry {
            RuleEntry::Plugin { order, .. } | RuleEntry::Worker(WorkerRule { order, .. }) => *order,
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
                RuleEntry::Worker(worker) => run_worker(worker, schema, document, &mut out),
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

/// Execute one worker plugin and append its diagnostics to `out`. A worker
/// that fails is reported as its failure diagnostic (E1602/E1603) — never a
/// silent skip — so the build gate treats plugin failures as validation
/// failures on the same surface.
fn run_worker(
    worker: &WorkerRule,
    schema: &Schema,
    document: &Document,
    out: &mut Vec<Diagnostic>,
) {
    let manifest = match &worker.manifest {
        Ok(manifest) => manifest,
        Err(load_errors) => {
            out.extend(load_errors.iter().cloned());
            return;
        }
    };
    let run = WorkerRun {
        request_id: format!("l7-{}", manifest.id),
        schema,
        document,
        options: worker.options.clone(),
    };
    match run_plugin(&worker.root, manifest, &run) {
        WorkerOutcome::Ok(findings) => out.extend(findings.iter().cloned()),
        WorkerOutcome::Failed(failure) => out.push(failure.to_diagnostic()),
    }
}

/// L7.1 declarative rule: every skill must reference an item whose category
/// is in the declared allowed set. Resolves the cross-table pair
/// (`skill_table.item_field` → `item_table.item_id_field`) then checks the
/// referenced item's `category_field`. Unresolved references are reported by
/// the L5 reference check, so they are skipped here.
pub struct PowerCurveValidator;

/// Cap the attack stat may reach at a given level: level * 100 + 50.
fn power_cap(level: f64) -> f64 {
    level * 100.0 + 50.0
}

/// Numeric view of a value (int/uint/float); `None` for anything else.
pub(crate) fn as_f64(value: &Value) -> Option<f64> {
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

mod declarative;
#[cfg(test)]
mod executor;
#[cfg(test)]
mod tests;
