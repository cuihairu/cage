use super::declarative::reference_edge_location;
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

fn plain_table_schema(name: &str, fields: indexmap::IndexMap<String, FieldSchema>) -> TableSchema {
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
        separator: None,
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

#[test]
fn skill_item_category_skips_missing_tables_and_fields() {
    let mut doc = Document::new();
    // No Item table at all → rule skips.
    let skills = declarative_table(
        "Skill",
        vec![declarative_row(
            vec![declarative_field("item", Value::UInt(200), None)],
            None,
            0,
        )],
    );
    doc.add_table(skills);
    let registry = GameRuleRegistry::with_rules(vec![item_category_rule()]);
    assert!(registry.run(&Schema::new(), &doc).is_empty());

    // Item table present but the skill table is missing → skip.
    let mut doc2 = Document::new();
    doc2.add_table(declarative_table(
        "Item",
        vec![declarative_row(
            vec![
                declarative_field("id", Value::UInt(200), None),
                declarative_field("category", Value::String("pet".into()), None),
            ],
            None,
            0,
        )],
    ));
    assert!(registry.run(&Schema::new(), &doc2).is_empty());

    // Skill row has no item field → skip; item row lacks id → skip;
    // item row lacks category → skip.
    let mut doc3 = Document::new();
    doc3.add_table(declarative_table(
        "Skill",
        vec![declarative_row(
            vec![declarative_field("other", Value::UInt(1), None)],
            None,
            0,
        )],
    ));
    doc3.add_table(declarative_table(
        "Item",
        vec![
            declarative_row(
                vec![declarative_field(
                    "category",
                    Value::String("pet".into()),
                    None,
                )],
                None,
                0,
            ),
            declarative_row(
                vec![declarative_field("id", Value::UInt(200), None)],
                None,
                1,
            ),
        ],
    ));
    assert!(registry.run(&Schema::new(), &doc3).is_empty());
}

#[test]
fn skill_item_category_first_row_wins_for_duplicate_ids() {
    // The same item id appears twice; the first row's category is the
    // one consulted (the second duplicate does not overwrite).
    let items = declarative_table(
        "Item",
        vec![
            declarative_row(
                vec![
                    declarative_field("id", Value::UInt(200), None),
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
    let registry = GameRuleRegistry::with_rules(vec![item_category_rule()]);
    assert!(
        registry.run(&Schema::new(), &doc).is_empty(),
        "first category equipment is allowed"
    );
}

#[test]
fn reference_edge_location_falls_back_to_table_source() {
    // No row carries the field → the fallback uses the table source map
    // (and its sheet), with the field name attached.
    let mut table_source = std::collections::HashMap::new();
    table_source.insert(
        "Item".to_string(),
        ("config/Item.json".to_string(), Some("Items".to_string())),
    );
    let loc = reference_edge_location(&Document::new(), "Item", "required_skill", &table_source);
    assert_eq!(loc.file, "config/Item.json");
    assert_eq!(loc.sheet.as_deref(), Some("Items"));
    assert_eq!(loc.field.as_deref(), Some("required_skill"));
}
