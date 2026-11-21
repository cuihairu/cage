//! Rule 人话层的正反例试算（评审 P1-4）。
//!
//! 一条规则的机器逻辑（`assert`）之外，策划还应能填中文标题、业务说明与
//! **正反例**——正例（这条规则应满足的样例行）与反例（该被拦的样例行）。
//! 本模块把正反例变成可执行的试算：用同一套表达式引擎对样例行求值，回
//! 答每个例「期望 vs 实际」，策划在编辑器一键就能确认规则写得对不对。
//!
//! 试算是**提示性**的（不进校验主链、不影响 canonical 形状）：样例行只是
//! 人写的例子，不是生产数据。校验主链的语义（L6）仍只看真实数据行。
//!
//! [`check_examples`] 不会修改任何东西；`examples` 字段本身只作为
//! `ExpressionRule` 的可选人话层随契约原样承载（`schema::RuleExample`）。

use crate::schema::{ExpressionRule, RuleExample};
use crate::validation::expr::{self, Outcome};
use crate::value::{Row, SourceLocation, TypedValue, Value};
use indexmap::IndexMap;
use serde::Serialize;

/// 期望结果：正例
pub const EXPECT_PASS: &str = "pass";
/// 期望结果：反例
pub const EXPECT_VIOLATE: &str = "violate";

/// 一条正/反例的试算结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExampleCheck {
    /// 该例在 `rule.examples` 中的下标（0-based）
    pub index: usize,
    /// 策划写的期望结果（`"pass"` / `"violate"`）
    pub expected: String,
    /// 引擎算出的结果：`"pass"` / `"violate"` / `"invalid"`（例本身写错）/
    /// `"unparseable"`（assert 解析不了）
    pub actual: String,
    /// 是否符合期望
    pub ok: bool,
    /// 人话备注（例写错或 assert 解析失败时是原因）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 对一条规则的全部正反例试算，按 `examples` 顺序返回。
///
/// 结果映射沿用 L6 的语义：样例行中缺/为 null 的字段视为「规则说不上」
/// （通过），类型不可比视为违反。
pub fn check_examples(rule: &ExpressionRule) -> Vec<ExampleCheck> {
    let Some(examples) = &rule.examples else {
        return Vec::new();
    };
    examples
        .iter()
        .enumerate()
        .map(|(index, ex)| check_one(index, ex, &rule.assert))
        .collect()
}

fn check_one(index: usize, ex: &RuleExample, assert_text: &str) -> ExampleCheck {
    let expected = ex.expected.trim();
    if expected != EXPECT_PASS && expected != EXPECT_VIOLATE {
        return ExampleCheck {
            index,
            expected: ex.expected.clone(),
            actual: "invalid".to_string(),
            ok: false,
            note: Some(format!("期望结果必须是 {EXPECT_PASS} 或 {EXPECT_VIOLATE}")),
        };
    }

    let Some(row) = example_row(&ex.row, index) else {
        return ExampleCheck {
            index,
            expected: ex.expected.clone(),
            actual: "invalid".to_string(),
            ok: false,
            note: Some("样例必须是字段名 → 值的 JSON 对象".to_string()),
        };
    };

    let cmp = match expr::parse_assert(assert_text) {
        Ok(cmp) => cmp,
        Err(err) => {
            return ExampleCheck {
                index,
                expected: ex.expected.clone(),
                actual: "unparseable".to_string(),
                ok: false,
                note: Some(err),
            };
        }
    };
    let outcome = expr::evaluate(&cmp, &row);
    let actual = match outcome {
        Outcome::Holds(true) | Outcome::MissingField(_) => EXPECT_PASS,
        Outcome::Holds(false) | Outcome::Incomparable { .. } => EXPECT_VIOLATE,
    };
    let ok = actual == expected;
    let note = if ok {
        None
    } else {
        Some(match actual {
            EXPECT_PASS => "样例满足规则（期望违反）".to_string(),
            _ => "样例违反规则（期望满足）".to_string(),
        })
    };
    ExampleCheck {
        index,
        expected: ex.expected.clone(),
        actual: actual.to_string(),
        ok,
        note,
    }
}

/// 样例行 JSON 对象 → 求值用的 Row（仅值、无源位置信息）。
/// 不是 JSON 对象时返回 `None`（例本身写错，不是规则问题）。
fn example_row(json: &serde_json::Value, index: usize) -> Option<Row> {
    let obj = json.as_object()?;
    let mut fields: IndexMap<String, TypedValue> = IndexMap::new();
    for (key, value) in obj {
        fields.insert(
            key.clone(),
            TypedValue::new(
                Value::from_json(value.clone()),
                SourceLocation::new("<rule-example>"),
            ),
        );
    }
    Some(Row {
        primary_key: Vec::new(),
        fields,
        location: SourceLocation::new("<rule-example>"),
        index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ExpressionRule;
    use serde_json::json;

    fn rule(assert: &str, examples: serde_json::Value) -> ExpressionRule {
        ExpressionRule {
            name: "r".to_string(),
            assert: assert.to_string(),
            message: None,
            warning_only: false,
            title: None,
            description: None,
            examples: Some(
                examples
                    .as_array()
                    .cloned()
                    .unwrap()
                    .into_iter()
                    .map(|v| serde_json::from_value(v).expect("example shape"))
                    .collect(),
            ),
        }
    }

    fn check(assert: &str, examples: serde_json::Value) -> Vec<ExampleCheck> {
        check_examples(&rule(assert, examples))
    }

    fn actuals(checks: &[ExampleCheck]) -> Vec<&str> {
        checks.iter().map(|c| c.actual.as_str()).collect()
    }

    #[test]
    fn positive_and_negative_examples_report_what_the_engine_answers() {
        let checks = check(
            "price <= cap",
            json!([
                { "row": { "price": 10, "cap": 20 }, "expected": "pass" },
                { "row": { "price": 30, "cap": 20 }, "expected": "violate" }
            ]),
        );
        assert_eq!(actuals(&checks), ["pass", "violate"]);
        assert!(
            checks.iter().all(|c| c.ok),
            "all examples match: {checks:?}"
        );
    }

    #[test]
    fn a_mismatched_expectation_is_flagged_with_a_note() {
        let checks = check(
            "price <= cap",
            json!([
                { "row": { "price": 30, "cap": 20 }, "expected": "pass" }
            ]),
        );
        assert_eq!(checks[0].ok, false);
        assert_eq!(checks[0].actual, "violate");
        assert!(checks[0].note.as_deref().unwrap().contains("期望满足"));
    }

    #[test]
    fn a_missing_optional_field_counts_as_pass_like_l6() {
        let checks = check(
            "level >= 10",
            json!([
                { "row": {}, "expected": "pass" }
            ]),
        );
        assert_eq!(checks[0].actual, "pass");
        assert!(checks[0].ok);
    }

    #[test]
    fn a_bad_example_shape_is_invalid_not_a_pass() {
        let checks = check(
            "price <= cap",
            json!([
                { "row": [1, 2], "expected": "pass" },
                { "row": { "price": 1 }, "expected": "maybe" }
            ]),
        );
        assert_eq!(checks[0].actual, "invalid");
        assert_eq!(checks[1].actual, "invalid");
        assert!(!checks[0].ok && !checks[1].ok);
    }

    #[test]
    fn an_unparseable_assert_is_unparseable() {
        let checks = check(
            "price << cap",
            json!([
                { "row": { "price": 1 }, "expected": "pass" }
            ]),
        );
        assert_eq!(checks[0].actual, "unparseable");
        assert!(!checks[0].ok);
    }

    #[test]
    fn no_examples_means_no_checks() {
        let rule = ExpressionRule {
            name: "r".to_string(),
            assert: "a <= b".to_string(),
            message: None,
            warning_only: false,
            title: None,
            description: None,
            examples: None,
        };
        assert!(check_examples(&rule).is_empty());
    }

    #[test]
    fn the_human_layer_round_trips_through_json() {
        let rule = ExpressionRule {
            name: "r".to_string(),
            assert: "a <= b".to_string(),
            message: Some("超了".to_string()),
            warning_only: false,
            title: Some("a 不得高于 b".to_string()),
            description: Some("同表两字段大小关系".to_string()),
            examples: Some(vec![RuleExample {
                row: json!({ "a": 1, "b": 2 }),
                expected: "pass".to_string(),
                note: Some("正常".to_string()),
            }]),
        };
        let v = serde_json::to_value(&rule).unwrap();
        assert_eq!(v["title"], "a 不得高于 b");
        assert_eq!(v["description"], "同表两字段大小关系");
        assert_eq!(v["examples"][0]["expected"], "pass");
        assert_eq!(v["examples"][0]["row"]["a"], 1);
        let back: ExpressionRule = serde_json::from_value(v).unwrap();
        assert_eq!(back, rule);
        assert_eq!(check_examples(&back).len(), 1);
    }

    #[test]
    fn a_rule_without_the_human_layer_omits_the_fields() {
        let rule = ExpressionRule {
            name: "r".to_string(),
            assert: "a <= b".to_string(),
            message: None,
            warning_only: false,
            title: None,
            description: None,
            examples: None,
        };
        let v = serde_json::to_value(&rule).unwrap();
        // 契约向后兼容：没有就整个省掉，老 Schema 一字节不变。
        assert_eq!(v, json!({ "name": "r", "assert": "a <= b" }));
    }
}
