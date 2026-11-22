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
use crate::plugin::attestation::{
    dir_digest, value_digest, AttestationOutcome, PluginAttestation, ValidationAttestation,
    ValidationEnvironment,
};
use crate::plugin::{run_plugin, PluginManifest, WorkerOutcome, WorkerRun};
use crate::schema::Schema;
use crate::value::{Document, Value};
use std::collections::BTreeMap;
use std::fmt;
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

/// An explicitly-selectable built-in rule package (review P3-1, extended by
/// P3-2).
///
/// Cage is a generic configuration tool: a game-specific *sample* rule (the
/// `power_curve` formula) keys on field names and a numeric curve, so it must
/// not run on every project that reaches the L7 pass. Such rules live in
/// named packages that a project selects explicitly — via `[validation]
/// rule_packages` in cage.toml — instead of being baked into the default
/// registry. Each package ships a version ([`RulePackage::version`]),
/// declares the fields it needs ([`RulePackage::required_fields`]) and lets a
/// project pin its version and override its parameters
/// ([`crate::manifest::RulePackageConfig`]); a package is
/// name-addressed and stateless, so one selection block is reusable across
/// projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RulePackage {
    /// The `power_curve` sample: `attack <= level * 100 + 50`.
    SamplePowerCurve,
}

impl RulePackage {
    /// The stable name used in `[validation] rule_packages`.
    pub fn name(&self) -> &'static str {
        match self {
            RulePackage::SamplePowerCurve => "power_curve",
        }
    }

    /// Parse a package by its `[validation] rule_packages` name. `None` for
    /// an unknown name — the caller reports a hard error, never a silent skip
    /// (a declared-but-unrecognized package must not quietly run nothing).
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "power_curve" => Some(RulePackage::SamplePowerCurve),
            _ => None,
        }
    }

    /// Every selectable package name (for error messages and discovery).
    pub fn all_names() -> &'static [&'static str] {
        &["power_curve"]
    }

    /// The version this cage ships for the package (review P3-2). A project
    /// may pin a version via [`crate::manifest::RulePackageConfig`];
    /// a pin on a different *major* version is refused as an upgrade-
    /// compatibility error, while a pin on the same major is accepted (semver:
    /// the same major stays behavior-compatible).
    pub fn version(&self) -> &'static str {
        match self {
            RulePackage::SamplePowerCurve => "1.0.0",
        }
    }

    /// The schema fields this package needs to be meaningful (review P3-2,
    /// capability declaration). When a selected package is run against a
    /// schema that declares none of them, the run reports `E1609` instead of
    /// silently doing nothing — the project asked for a rule the schema
    /// cannot support, and that must be visible, not vacuous.
    pub fn required_fields(&self) -> &'static [&'static str] {
        match self {
            RulePackage::SamplePowerCurve => &["level", "attack"],
        }
    }
}

/// One selected rule package plus its project-side configuration (review
/// P3-2). The selection carries the resolved package, an optional version pin
/// and the parameter overrides; `GameRuleRegistry::with_package_selections`
/// validates both before building the validator.
#[derive(Debug, Clone, PartialEq)]
pub struct RulePackageSelection {
    /// The resolved package to run.
    pub package: RulePackage,
    /// Version pin (`"1"`, `"1.0.0"`, `"*"`); `None` accepts the shipped one.
    pub version: Option<String>,
    /// Parameter overrides, validated by the package itself.
    pub params: BTreeMap<String, serde_json::Value>,
}

impl RulePackageSelection {
    /// A selection with the default version (no pin) and no parameter
    /// overrides — the plain `[validation] rule_packages = ["power_curve"]`
    /// form of review P3-1.
    pub fn new(package: RulePackage) -> Self {
        Self {
            package,
            version: None,
            params: BTreeMap::new(),
        }
    }
}

/// Why a rule package selection is refused (review P3-2): an incompatible
/// version pin or an invalid parameter. These are *usage* errors — the
/// project declared a package it cannot actually run — so the CLI reports
/// them as hard errors, never as a silent skip.
#[derive(Debug, Clone, PartialEq)]
pub enum RulePackageError {
    /// The version pin targets a different major than the shipped version —
    /// a breaking change the project must migrate to.
    IncompatibleVersion {
        /// Package name.
        name: String,
        /// The pin the project wrote.
        pinned: String,
        /// The version this cage ships.
        shipped: String,
    },
    /// A parameter the package does not know, or a known parameter carrying a
    /// value of the wrong type.
    InvalidParam {
        /// Package name.
        name: String,
        /// The offending parameter key.
        key: String,
        /// What went wrong.
        detail: String,
    },
}

impl fmt::Display for RulePackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RulePackageError::IncompatibleVersion {
                name,
                pinned,
                shipped,
            } => write!(
                f,
                "rule package '{name}' is pinned to version '{pinned}' but this cage \
                 ships {shipped} — a different major version is a breaking change, \
                 migrate the project's pin or select the shipped major"
            ),
            RulePackageError::InvalidParam { name, key, detail } => write!(
                f,
                "invalid parameter '{key}' for rule package '{name}': {detail}"
            ),
        }
    }
}

impl std::error::Error for RulePackageError {}

/// The major version segment of a semver string; `None` when it does not
/// parse as a number.
fn major_of(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

/// A worker plugin (an independent-process validator) registered with the
/// unified L7 executor, together with the project context it needs to run.
struct WorkerRule {
    /// Deterministic run position, from the shared registration order.
    order: u32,
    /// The project root the worker process is spawned in.
    root: PathBuf,
    /// The plugin directory (`plugins/<id>`) — kept even when the manifest
    /// failed to load, so the content lock and the audit record still name
    /// what was attempted.
    dir: PathBuf,
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

    /// Registry preloaded with the explicitly-selected built-in rule packages
    /// (review P3-1). Game-specific sample rules are opt-in: selecting
    /// nothing (the default) registers none, so a generic project never
    /// assumes another game's numeric curve. Uses the default version (no
    /// pin) and default parameters for every package; the parameterized,
    /// version-pinned form is [`Self::with_package_selections`].
    pub fn with_packages(packages: &[RulePackage]) -> Self {
        let selections: Vec<RulePackageSelection> = packages
            .iter()
            .map(|p| RulePackageSelection::new(*p))
            .collect();
        // A default selection carries no version pin and no parameters, which
        // every package accepts — the fallible constructor cannot fail here.
        Self::with_package_selections(&selections)
            .expect("a default rule package selection is always valid")
    }

    /// Registry preloaded with explicitly-selected rule packages carrying a
    /// version pin and parameter overrides (review P3-2). Each selection's
    /// version is checked for upgrade compatibility and its parameters are
    /// validated by the package; a refusal is a hard error, never a silent
    /// skip. Game-specific sample rules stay opt-in: selecting nothing (the
    /// default) registers none.
    pub fn with_package_selections(
        selections: &[RulePackageSelection],
    ) -> Result<Self, RulePackageError> {
        let mut reg = Self::new();
        for selection in selections {
            check_version_compatibility(selection.package, selection.version.as_deref())?;
            let validator = package_validator(selection)?;
            reg.register(validator);
        }
        Ok(reg)
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
        let dir = manifest.dir.clone();
        self.push_worker(root.into(), dir, Ok(manifest), options);
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
        self.push_worker(
            root.clone(),
            dir.to_path_buf(),
            PluginManifest::load(dir),
            options,
        );
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

    /// The ids of every registered worker: the loaded manifest id, or the
    /// `plugins/<id>` directory name when the manifest failed to load. Used
    /// by the project policy's required-plugin check.
    pub fn worker_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                RuleEntry::Worker(worker) => Some(match &worker.manifest {
                    Ok(manifest) => manifest.id.clone(),
                    Err(_) => worker
                        .dir
                        .file_name()
                        .map_or_else(|| "plugin".to_string(), |n| n.to_string_lossy().to_string()),
                }),
                _ => None,
            })
            .collect()
    }

    /// Enforce the project policy's required-plugin list (review P2-5): every
    /// required id that is not a registered worker is pushed as an
    /// unloadable worker whose run fails with E1602 — a required plugin that
    /// is absent fails validation instead of passing silently (review §六.1
    /// 「CI 构建通过但实际缺失必需插件」). Ids already registered (even with a
    /// broken manifest) are not duplicated; their own load failure is the
    /// diagnostic.
    pub fn require_plugins(&mut self, root: impl AsRef<Path>, required: &[String]) -> &mut Self {
        let root = root.as_ref();
        let have = self.worker_ids();
        for id in required {
            if have.contains(id) {
                continue;
            }
            let detail = format!(
                "required validator plugin '{id}' is not present in plugins/ \
                 (declared in [validation] required_plugins)"
            );
            let mut failure = Diagnostics::new();
            failure.add(Diagnostic::error(gamerule::E1602, detail));
            self.push_worker(
                root.to_path_buf(),
                root.join("plugins").join(id),
                Err(failure),
                serde_json::json!({}),
            );
        }
        self
    }

    fn push_worker(
        &mut self,
        root: PathBuf,
        dir: PathBuf,
        manifest: Result<PluginManifest, Diagnostics>,
        options: serde_json::Value,
    ) {
        let order = self.next_order;
        self.next_order += 1;
        self.entries.push(RuleEntry::Worker(WorkerRule {
            order,
            root,
            dir,
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
        self.run_attested(schema, document).0
    }

    /// [`Self::run`] plus the [`ValidationAttestation`] for this pass: plugin
    /// ids/versions, option and content digests, the environment fingerprint
    /// and per-plugin outcomes — the auditable record of review P2-4.
    pub fn run_attested(
        &self,
        schema: &Schema,
        document: &Document,
    ) -> (Vec<Diagnostic>, ValidationAttestation) {
        let mut order: Vec<&RuleEntry> = self.entries.iter().collect();
        order.sort_by_key(|entry| match entry {
            RuleEntry::Plugin { order, .. } | RuleEntry::Worker(WorkerRule { order, .. }) => *order,
            RuleEntry::Declarative(config) => config.order,
        });

        let mut out = Vec::new();
        let mut plugin_records = Vec::new();
        let mut worker_root: Option<&Path> = None;
        for entry in order {
            match entry {
                RuleEntry::Plugin { validator, .. } => {
                    out.extend(validator.validate(schema, document));
                }
                RuleEntry::Declarative(config) if config.enabled => {
                    out.extend(run_rule(config, schema, document));
                }
                RuleEntry::Declarative(_) => {}
                RuleEntry::Worker(worker) => {
                    worker_root.get_or_insert(&worker.root);
                    plugin_records.push(run_worker(worker, schema, document, &mut out));
                }
            }
        }
        // The environment is probed once per pass, only when a worker plugin
        // actually participated (the interpreter probe is a subprocess).
        let environment = match worker_root {
            Some(root) => ValidationEnvironment::probe(root),
            None => ValidationEnvironment::without_python(),
        };
        let attestation = ValidationAttestation::new(
            schema_digest(schema),
            document_digest(document),
            environment,
            plugin_records,
        );
        (out, attestation)
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

// The entry list holds trait objects (`Box<dyn GameRuleValidator>`), so a
// derive is impossible; the registry-level summary is what debugging needs.
impl std::fmt::Debug for GameRuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameRuleRegistry")
            .field("entries", &self.entries.len())
            .field("next_order", &self.next_order)
            .finish()
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

/// Execute one worker plugin, append its diagnostics to `out`, and return its
/// attestation record. A worker that fails is reported as its failure
/// diagnostic (E1602/E1603) — never a silent skip — so the build gate treats
/// plugin failures as validation failures on the same surface.
fn run_worker(
    worker: &WorkerRule,
    schema: &Schema,
    document: &Document,
    out: &mut Vec<Diagnostic>,
) -> PluginAttestation {
    let options_digest = value_digest(&worker.options);
    let content_digest = dir_digest(&worker.dir);
    let Ok(manifest) = &worker.manifest else {
        let detail = worker
            .manifest
            .as_ref()
            .err()
            .and_then(|errors| errors.errors().first().map(|d| d.message.clone()))
            .unwrap_or_else(|| "plugin manifest could not be loaded".to_string());
        out.push(Diagnostic::error(gamerule::E1602, detail.clone()));
        return PluginAttestation {
            id: worker
                .dir
                .file_name()
                .map_or_else(|| "plugin".to_string(), |n| n.to_string_lossy().to_string()),
            version: String::new(),
            options_digest,
            content_digest,
            outcome: AttestationOutcome::Failed {
                class: "missing".to_string(),
                detail,
            },
        };
    };
    let run = WorkerRun {
        request_id: format!("l7-{}", manifest.id),
        schema,
        document,
        options: worker.options.clone(),
    };
    let outcome = match run_plugin(&worker.root, manifest, &run) {
        WorkerOutcome::Ok(findings) => {
            out.extend(findings.iter().cloned());
            AttestationOutcome::Ran {
                errors: findings.errors().len(),
                warnings: findings.warnings().len(),
                infos: findings.infos().len(),
            }
        }
        WorkerOutcome::Failed(failure) => {
            let diag = failure.to_diagnostic();
            let detail = diag.message.clone();
            out.push(diag);
            AttestationOutcome::Failed {
                class: failure.class().to_string(),
                detail,
            }
        }
    };
    PluginAttestation {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        options_digest,
        content_digest,
        outcome,
    }
}

/// blake3 over the canonical schema JSON.
fn schema_digest(schema: &Schema) -> String {
    let value = serde_json::to_value(schema).unwrap_or(serde_json::Value::Null);
    value_digest(&value)
}

/// blake3 over the canonical document (data) JSON.
fn document_digest(document: &Document) -> String {
    let value = serde_json::to_value(document).unwrap_or(serde_json::Value::Null);
    value_digest(&value)
}

/// Check a selection's version pin against the shipped package version
/// (review P3-2, upgrade-compatibility check). A pin on the same major is
/// accepted (semver: same major stays behavior-compatible); a pin on a
/// different major, or a pin that does not parse, is refused. `"*"` accepts
/// any version.
fn check_version_compatibility(
    package: RulePackage,
    pin: Option<&str>,
) -> Result<(), RulePackageError> {
    let Some(pin) = pin else { return Ok(()) };
    if pin == "*" {
        return Ok(());
    }
    let shipped = package.version();
    match (major_of(pin), major_of(shipped)) {
        (Some(pinned_major), Some(shipped_major)) if pinned_major == shipped_major => Ok(()),
        _ => Err(RulePackageError::IncompatibleVersion {
            name: package.name().to_string(),
            pinned: pin.to_string(),
            shipped: shipped.to_string(),
        }),
    }
}

/// Build the validator for one selected package, validating its parameters
/// (review P3-2). An unknown parameter key or a non-numeric value is refused
/// as a usage error — never silently ignored.
fn package_validator(
    selection: &RulePackageSelection,
) -> Result<Box<dyn GameRuleValidator>, RulePackageError> {
    let name = selection.package.name();
    match selection.package {
        RulePackage::SamplePowerCurve => {
            let (slope, intercept) = power_curve_params(name, &selection.params)?;
            Ok(Box::new(PowerCurveValidator { slope, intercept }))
        }
    }
}

/// The `power_curve` sample's parameter overrides: `slope` and `intercept`
/// for the cap `attack <= level * slope + intercept`. Only those two keys are
/// accepted; a value must be a JSON number.
fn power_curve_params(
    name: &str,
    params: &BTreeMap<String, serde_json::Value>,
) -> Result<(f64, f64), RulePackageError> {
    let mut slope = 100.0;
    let mut intercept = 50.0;
    for (key, value) in params {
        let number = value
            .as_f64()
            .ok_or_else(|| RulePackageError::InvalidParam {
                name: name.to_string(),
                key: key.clone(),
                detail: "expected a number (JSON integer or float)".to_string(),
            })?;
        match key.as_str() {
            "slope" => slope = number,
            "intercept" => intercept = number,
            _ => {
                return Err(RulePackageError::InvalidParam {
                    name: name.to_string(),
                    key: key.clone(),
                    detail: "unknown parameter (known: slope, intercept)".to_string(),
                })
            }
        }
    }
    Ok((slope, intercept))
}

/// The `power_curve` sample: `attack <= level * slope + intercept`. Its
/// `slope`/`intercept` come from the package's parameter overrides (default
/// 100/50) and it declares the `level`/`attack` capability it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerCurveValidator {
    /// Multiplier applied to the level.
    pub slope: f64,
    /// Constant added to the product.
    pub intercept: f64,
}

impl Default for PowerCurveValidator {
    fn default() -> Self {
        Self {
            slope: 100.0,
            intercept: 50.0,
        }
    }
}

impl PowerCurveValidator {
    /// Cap the attack stat may reach at a given level: level * slope + intercept.
    fn cap(&self, level: f64) -> f64 {
        level * self.slope + self.intercept
    }

    /// The curve as a readable formula, e.g. `level * 100 + 50`.
    fn formula(&self) -> String {
        let slope = format_number(self.slope);
        let intercept = format_number(self.intercept);
        format!("level * {slope} + {intercept}")
    }
}

/// Format a parameter for the hint text without needless decimals.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
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

    fn validate(&self, schema: &Schema, document: &Document) -> Vec<Diagnostic> {
        // Capability declaration (review P3-2): a selected package whose
        // required fields the schema never declares reports E1609 instead of
        // silently running nothing — the project asked for a rule the schema
        // cannot support, and a vacuous pass must be visible.
        let capable = schema
            .tables
            .values()
            .any(|table| table.fields.contains_key("level") && table.fields.contains_key("attack"));
        if !capable {
            return vec![DiagnosticBuilder::error(
                gamerule::E1609,
                "Selected rule package cannot run on this schema",
            )
            .source("schema")
            .hint(
                "power_curve requires fields 'level' and 'attack' in some table; \
                     none of the schema's tables declares both (the package runs \
                     nothing when its capability is unmet)"
                    .to_string(),
            )
            .build()];
        }
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
                let cap = self.cap(level);
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
                            "power_curve: attack {attack} exceeds the level {level} cap {cap} ({})",
                            self.formula()
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
