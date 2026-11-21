//! Structured rule AST + Rust-Core conversion to the Schema's rule
//! representation (review P1-2).
//!
//! The visual condition builder (editor 规则模式) produces a [`RuleAst`];
//! this module compiles it **down** to the Schema's one rule representation
//! ([`ExpressionRule`] arrays on `field.rules`). The AST is an authoring
//! intermediate only — the Schema never stores it, so P1-1's "no third rule
//! format" guarantee holds: the Schema only ever carries `ExpressionRule`.
//!
//! The current expression engine (`validation::expr`) evaluates a single
//! comparison `operand OP operand`. The AST therefore compiles exactly the
//! conditions the engine can express: a [`RuleAst::Compare`] becomes one
//! `ExpressionRule`; a [`RuleAst::And`] becomes one rule per item (the
//! Schema ANDs a field's rules). Anything the engine cannot express (a
//! literal it cannot render — an array/object/null, a string holding both
//! quote kinds, a `UInt` beyond `i64`) is refused with a [`CompileError`] the UI
//! surfaces as an expressiveness boundary notice (P1-5 makes it prominent),
//! never silently faked.
//!
//! [`decompile`] is the inverse for the builder to load an existing rule
//! (an assert the engine cannot parse yields [`RuleAst::Unparseable`]).

use crate::schema::ExpressionRule;
use crate::validation::expr::{self, CmpOp};
use serde::{Deserialize, Serialize};

/// Comparison operator, on the wire as the symbol itself (`"<="`, `"=="`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    #[serde(rename = "<")]
    /// Less than.
    Lt,
    #[serde(rename = "<=")]
    /// Less than or equal.
    Le,
    #[serde(rename = ">")]
    /// Greater than.
    Gt,
    #[serde(rename = ">=")]
    /// Greater than or equal.
    Ge,
    #[serde(rename = "==")]
    /// Equal.
    Eq,
    #[serde(rename = "!=")]
    /// Not equal.
    Ne,
}

impl From<Op> for CmpOp {
    fn from(op: Op) -> Self {
        match op {
            Op::Lt => CmpOp::Lt,
            Op::Le => CmpOp::Le,
            Op::Gt => CmpOp::Gt,
            Op::Ge => CmpOp::Ge,
            Op::Eq => CmpOp::Eq,
            Op::Ne => CmpOp::Ne,
        }
    }
}

impl From<CmpOp> for Op {
    fn from(op: CmpOp) -> Self {
        match op {
            CmpOp::Lt => Op::Lt,
            CmpOp::Le => Op::Le,
            CmpOp::Gt => Op::Gt,
            CmpOp::Ge => Op::Ge,
            CmpOp::Eq => Op::Eq,
            CmpOp::Ne => Op::Ne,
        }
    }
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Eq => "==",
            Op::Ne => "!=",
        }
    }
}

/// The right-hand side of a comparison: an inline literal or another field
/// of the same row (cross-field comparison).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Rhs {
    /// An inline literal (number / string / bool).
    Literal {
        /// The value compared against.
        value: serde_json::Value,
    },
    /// A field of the same row.
    Field {
        /// The name of the field read from the same row.
        name: String,
    },
}

/// A structured condition node the visual builder produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RuleAst {
    /// `field OP rhs` — the engine's single-comparison primitive.
    Compare {
        /// The field read from the row.
        field: String,
        /// The comparison operator.
        op: Op,
        /// The compared value or field.
        rhs: Rhs,
    },
    /// All items must hold: one rule per item (the Schema ANDs a field's
    /// rules). An empty group refuses to compile.
    And {
        /// The conditions that must all hold.
        items: Vec<RuleAst>,
    },
    /// An assert the engine cannot parse back into a structured condition.
    /// The builder shows it read-only and refuses to compile it.
    Unparseable {
        /// The original assert text, kept visible.
        assert: String,
    },
}

/// The human-facing part carried onto every compiled rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleMeta {
    /// Base rule name (the first compiled rule keeps it; later ones in an
    /// `And` get a `_2`, `_3`, … suffix).
    pub name: String,
    /// Custom diagnostic message.
    pub message: Option<String>,
    /// Whether a violation reports as a warning.
    pub warning_only: bool,
}

/// Why an AST could not compile to the Schema's rule representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason")]
pub enum CompileError {
    /// The engine cannot express this condition; the reason is human-readable
    /// and the UI shows it as a boundary notice (P1-5).
    Unsupported {
        /// Human-readable reason the condition has no expressible form.
        reason: String,
    },
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Render a field reference: bare when it is a plain identifier, backticked
/// otherwise (names with spaces/punctuation, per the engine grammar).
fn render_field(name: &str) -> String {
    if is_ident(name) {
        name.to_string()
    } else {
        format!("`{name}`")
    }
}

/// Render a literal value as the engine's token, or `Err(reason)` when the
/// value has no token form (array/object/null, a string holding both quote
/// kinds, a `UInt` beyond `i64` — the lexer reads numbers as `i64`).
fn render_literal(value: &serde_json::Value) -> Result<String, String> {
    match value {
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(u) = n.as_u64() {
                let i: i64 = u.try_into().map_err(|_| {
                    format!("unsigned literal {u} exceeds i64 — the engine reads numbers as i64")
                })?;
                Ok(i.to_string())
            } else if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.is_finite() {
                    // Keep the float token a float (re-parses as Int otherwise).
                    Ok(format!("{f:.1}"))
                } else {
                    Ok(f.to_string())
                }
            } else {
                Err("number literal has no engine token form".to_string())
            }
        }
        serde_json::Value::String(s) => {
            // No escapes in the lexer: choose a quote kind the value lacks.
            if !s.contains('"') {
                Ok(format!("\"{s}\""))
            } else if !s.contains('\'') {
                Ok(format!("'{s}'"))
            } else {
                Err("string literal holds both quote kinds — the engine has no escape".to_string())
            }
        }
        serde_json::Value::Null => Err("null has no comparison literal form".to_string()),
        serde_json::Value::Array(_) => Err("array literal is not a comparison operand".to_string()),
        serde_json::Value::Object(_) => {
            Err("object literal is not a comparison operand".to_string())
        }
    }
}

/// Compile the AST down to `ExpressionRule`s — the Schema's one rule
/// representation. `And` yields one rule per item; the k-th item's rule name
/// is the base name for the first and `{base}_{k+1}` for the rest.
pub fn compile(ast: &RuleAst, meta: &RuleMeta) -> Result<Vec<ExpressionRule>, CompileError> {
    let mut rules = Vec::new();
    compile_into(ast, meta, &mut rules)?;
    Ok(rules)
}

fn compile_into(
    ast: &RuleAst,
    meta: &RuleMeta,
    out: &mut Vec<ExpressionRule>,
) -> Result<(), CompileError> {
    match ast {
        RuleAst::Compare { field, op, rhs } => {
            let assert = render_compare(field, *op, rhs)?;
            out.push(ExpressionRule {
                name: meta.name.clone(),
                assert,
                message: meta.message.clone(),
                warning_only: meta.warning_only,
            });
            Ok(())
        }
        RuleAst::And { items } => {
            if items.is_empty() {
                return Err(CompileError::Unsupported {
                    reason: "AND 组为空".to_string(),
                });
            }
            for (i, item) in items.iter().enumerate() {
                let sub = if i == 0 {
                    meta.name.clone()
                } else {
                    format!("{}_{}", meta.name, i + 1)
                };
                let sub_meta = RuleMeta {
                    name: sub,
                    message: meta.message.clone(),
                    warning_only: meta.warning_only,
                };
                compile_into(item, &sub_meta, out)?;
            }
            Ok(())
        }
        RuleAst::Unparseable { assert } => Err(CompileError::Unsupported {
            reason: format!("无法结构化编译表达式：{assert}"),
        }),
    }
}

fn render_compare(field: &str, op: Op, rhs: &Rhs) -> Result<String, CompileError> {
    let lhs = render_field(field);
    let rhs = match rhs {
        Rhs::Literal { value } => {
            render_literal(value).map_err(|reason| CompileError::Unsupported { reason })?
        }
        Rhs::Field { name } => render_field(name),
    };
    Ok(format!("{lhs} {} {rhs}", op.as_str()))
}

/// The inverse: parse an `ExpressionRule` back into the AST so the builder
/// can load and edit it. An assert the engine cannot parse (or that is not
/// a plain single comparison) yields [`RuleAst::Unparseable`] — shown
/// read-only rather than lost.
pub fn decompile(rule: &ExpressionRule) -> RuleAst {
    match expr::parse_assert(&rule.assert) {
        Ok(cmp) => {
            let op = Op::from(cmp.op);
            let field = match cmp.lhs {
                expr::Operand::Field(name) => name,
                expr::Operand::Lit(_) => {
                    // A literal on the left is a comparison the builder does
                    // not structure (the editor models `field OP rhs`).
                    return RuleAst::Unparseable {
                        assert: rule.assert.clone(),
                    };
                }
            };
            let rhs = match cmp.rhs {
                expr::Operand::Field(name) => Rhs::Field { name },
                expr::Operand::Lit(v) => Rhs::Literal { value: v.to_json() },
            };
            RuleAst::Compare { field, op, rhs }
        }
        Err(_) => RuleAst::Unparseable {
            assert: rule.assert.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str) -> RuleMeta {
        RuleMeta {
            name: name.to_string(),
            message: Some("hint".to_string()),
            warning_only: false,
        }
    }

    fn compare(field: &str, op: Op, rhs: Rhs) -> RuleAst {
        RuleAst::Compare {
            field: field.to_string(),
            op,
            rhs,
        }
    }

    #[test]
    fn compiles_a_single_literal_comparison_to_one_rule() {
        let ast = compare(
            "price",
            Op::Le,
            Rhs::Literal {
                value: serde_json::json!(200),
            },
        );
        let rules = compile(&ast, &meta("price_cap")).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "price_cap");
        assert_eq!(rules[0].assert, "price <= 200");
        assert_eq!(rules[0].message.as_deref(), Some("hint"));
        assert!(!rules[0].warning_only);
        // The compiled assert is a valid engine assert — the Schema's one
        // rule representation, no third format.
        assert!(expr::parse_assert(&rules[0].assert).is_ok());
    }

    #[test]
    fn compiles_cross_field_comparison_to_field_operands() {
        let ast = compare(
            "min_level",
            Op::Le,
            Rhs::Field {
                name: "max_level".into(),
            },
        );
        let rules = compile(&ast, &meta("levels")).unwrap();
        assert_eq!(rules[0].assert, "min_level <= max_level");
        assert!(expr::parse_assert(&rules[0].assert).is_ok());
    }

    #[test]
    fn an_and_group_compiles_one_rule_per_item_with_suffixes() {
        let ast = RuleAst::And {
            items: vec![
                compare(
                    "price",
                    Op::Ge,
                    Rhs::Literal {
                        value: serde_json::json!(10),
                    },
                ),
                compare(
                    "price",
                    Op::Le,
                    Rhs::Literal {
                        value: serde_json::json!(300),
                    },
                ),
                compare(
                    "price",
                    Op::Ne,
                    Rhs::Literal {
                        value: serde_json::json!(250),
                    },
                ),
            ],
        };
        let rules = compile(&ast, &meta("price_band")).unwrap();
        let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["price_band", "price_band_2", "price_band_3"]);
        assert_eq!(rules[0].assert, "price >= 10");
        assert_eq!(rules[1].assert, "price <= 300");
        assert_eq!(rules[2].assert, "price != 250");
        for r in &rules {
            assert_eq!(r.message.as_deref(), Some("hint"));
            assert!(expr::parse_assert(&r.assert).is_ok());
        }
    }

    #[test]
    fn refuses_expressions_the_engine_cannot_render_as_asserts() {
        let cases = [
            compare(
                "a",
                Op::Eq,
                Rhs::Literal {
                    value: serde_json::json!(null),
                },
            ),
            compare(
                "a",
                Op::Eq,
                Rhs::Literal {
                    value: serde_json::json!([1]),
                },
            ),
            compare(
                "a",
                Op::Eq,
                Rhs::Literal {
                    value: serde_json::json!({"k": 1}),
                },
            ),
            compare(
                "a",
                Op::Eq,
                Rhs::Literal {
                    value: serde_json::json!("say \"hi\" and 'bye'"),
                },
            ),
            compare(
                "a",
                Op::Eq,
                Rhs::Literal {
                    value: serde_json::json!(18_446_744_073_709_551_615u64),
                },
            ),
        ];
        for ast in cases {
            let err = compile(&ast, &meta("r")).unwrap_err();
            assert!(
                matches!(err, CompileError::Unsupported { .. }),
                "expected a boundary notice, got {err:?}"
            );
        }
        // An empty AND group is also a boundary.
        let err = compile(&RuleAst::And { items: vec![] }, &meta("r")).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported { .. }));
        // An unparseable assert the builder refused to structure never
        // silently becomes a rule.
        let err = compile(
            &RuleAst::Unparseable {
                assert: "not an assert".into(),
            },
            &meta("r"),
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::Unsupported { .. }));
    }

    #[test]
    fn renders_strings_and_backticked_fields_so_they_reparse() {
        // A string with a double quote uses the single-quote form.
        let ast = compare(
            "name",
            Op::Eq,
            Rhs::Literal {
                value: serde_json::json!("he said \"hi\""),
            },
        );
        let rules = compile(&ast, &meta("n")).unwrap();
        assert_eq!(rules[0].assert, "name == 'he said \"hi\"'");
        assert!(expr::parse_assert(&rules[0].assert).is_ok());
        // A field name that is not a plain identifier is backticked.
        let ast = compare(
            "my field",
            Op::Ge,
            Rhs::Literal {
                value: serde_json::json!(3),
            },
        );
        let rules = compile(&ast, &meta("r")).unwrap();
        assert_eq!(rules[0].assert, "`my field` >= 3");
        // An empty string literal round-trips.
        let ast = compare(
            "x",
            Op::Ne,
            Rhs::Literal {
                value: serde_json::json!(""),
            },
        );
        let rules = compile(&ast, &meta("r")).unwrap();
        assert!(expr::parse_assert(&rules[0].assert).is_ok());
    }

    #[test]
    fn decompile_recovers_the_ast_from_compiled_rules() {
        let ast = compare(
            "hp",
            Op::Ge,
            Rhs::Literal {
                value: serde_json::json!(5),
            },
        );
        let rules = compile(&ast, &meta("hp_min")).unwrap();
        assert_eq!(decompile(&rules[0]), ast);

        let ast = compare(
            "min_level",
            Op::Le,
            Rhs::Field {
                name: "max_level".into(),
            },
        );
        let rules = compile(&ast, &meta("lvl")).unwrap();
        assert_eq!(decompile(&rules[0]), ast);

        // String literals round-trip through the quoted form.
        let ast = compare(
            "kind",
            Op::Eq,
            Rhs::Literal {
                value: serde_json::json!("sword"),
            },
        );
        let rules = compile(&ast, &meta("k")).unwrap();
        assert_eq!(decompile(&rules[0]), ast);

        // Floats keep their float token (integral floats would else become Int).
        let ast = compare(
            "ratio",
            Op::Le,
            Rhs::Literal {
                value: serde_json::json!(1.0),
            },
        );
        let rules = compile(&ast, &meta("r")).unwrap();
        assert_eq!(decompile(&rules[0]), ast);
    }

    #[test]
    fn decompile_marks_unparseable_and_literal_lhs_asserts() {
        let rule = ExpressionRule {
            name: "bad".into(),
            assert: "not an assert".into(),
            message: None,
            warning_only: false,
        };
        assert_eq!(
            decompile(&rule),
            RuleAst::Unparseable {
                assert: "not an assert".into()
            }
        );
        // A literal on the left has no `field OP rhs` structure to show.
        let rule = ExpressionRule {
            name: "bad".into(),
            assert: "5 > price".into(),
            message: None,
            warning_only: false,
        };
        assert_eq!(
            decompile(&rule),
            RuleAst::Unparseable {
                assert: "5 > price".into()
            }
        );
    }

    #[test]
    fn ast_round_trips_through_serde_in_the_wire_shape() {
        // The frontend sends the AST as tagged JSON; it must survive.
        let ast = RuleAst::And {
            items: vec![
                compare(
                    "price",
                    Op::Le,
                    Rhs::Literal {
                        value: serde_json::json!(200),
                    },
                ),
                compare(
                    "price",
                    Op::Ge,
                    Rhs::Literal {
                        value: serde_json::json!(10),
                    },
                ),
            ],
        };
        let json = serde_json::to_value(&ast).unwrap();
        let back: RuleAst = serde_json::from_value(json).unwrap();
        assert_eq!(back, ast);
        // Op on the wire is the bare operator symbol.
        assert_eq!(
            serde_json::to_value(Op::Le).unwrap(),
            serde_json::json!("<=")
        );
        assert_eq!(
            serde_json::from_value::<Op>(serde_json::json!(">=")).unwrap(),
            Op::Ge
        );
    }
}
