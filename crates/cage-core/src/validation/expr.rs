//! L6 semantic expression engine — single-comparison asserts (design §16).
//!
//! Grammar: `operand OP operand`
//!
//! ```text
//! operand := `field` | bare_ident | number | "string" | 'string' | true | false
//! OP      := == | != | <= | >= | < | >
//! ```
//!
//! Bare identifiers and backtick-quoted names both resolve as field refs of
//! the row under evaluation (design examples are bare; backticks allow names
//! with spaces or punctuation). Numbers parse as Int when integral and Float
//! when they carry a fraction. Anything else is a parse error — the caller
//! surfaces it as a schema defect (E1004), never a silent pass.
//!
//! Evaluation semantics:
//! - A referenced field that is absent (or null) from the row yields
//!   [`Outcome::MissingField`] — optional fields say nothing about the row.
//! - Numeric operands (Int/UInt/Float) compare numerically; Int/UInt go
//!   through i128 (exact), anything mixed with Float goes through f64.
//! - String vs String is lexicographic; Bool vs Bool orders false < true.
//! - Any other pairing yields [`Outcome::Incomparable`] — reported as a
//!   violation rather than silently passing.

use crate::value::{Row, Value};

/// Comparison operator in an assert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

impl CmpOp {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "<" => Self::Lt,
            "<=" => Self::Le,
            ">" => Self::Gt,
            ">=" => Self::Ge,
            "==" => Self::Eq,
            "!=" => Self::Ne,
            _ => return None,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }
}

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// Field reference (bare identifier or backtick-quoted).
    Field(String),
    /// Literal number / string / bool.
    Lit(Value),
}

impl Operand {
    /// Short rendering for diagnostics (fields stay bare, literals inlined).
    pub fn render(&self) -> String {
        match self {
            Self::Field(name) => name.clone(),
            Self::Lit(v) => v
                .coerce_to_string()
                .unwrap_or_else(|| v.type_name().to_string()),
        }
    }
}

/// A parsed assert: `lhs OP rhs`.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// Left operand.
    pub lhs: Operand,
    /// Operator.
    pub op: CmpOp,
    /// Right operand.
    pub rhs: Operand,
}

impl Comparison {
    /// Referenced field names in evaluation order, deduplicated.
    pub fn field_refs(&self) -> Vec<&str> {
        let mut refs = Vec::new();
        if let Operand::Field(name) = &self.lhs {
            refs.push(name.as_str());
        }
        if let Operand::Field(name) = &self.rhs {
            if !refs.contains(&name.as_str()) {
                refs.push(name.as_str());
            }
        }
        refs
    }
}

/// Result of evaluating a parsed assert against a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The comparison held (`true`) or was violated (`false`).
    Holds(bool),
    /// A referenced field is absent or null in this row — an optional field
    /// the rule says nothing about. Callers treat this as a pass.
    MissingField(String),
    /// Both operands are present but their types define no ordering
    /// (e.g. string vs int). Callers treat this as a violation.
    Incomparable {
        /// Type of the left operand.
        lhs: String,
        /// Type of the right operand.
        rhs: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Op(CmpOp),
    Field(String),
    Lit(Value),
}

fn lex(text: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let mut rest = text;
    'scan: while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let head = rest.chars().next().unwrap_or_default();
        // Backtick-quoted field reference.
        if head == '`' {
            let body = &rest[1..];
            match body.find('`') {
                Some(end) => {
                    let name = &body[..end];
                    if name.is_empty() {
                        return Err("empty backtick reference".to_string());
                    }
                    toks.push(Tok::Field(name.to_string()));
                    rest = &body[end + 1..];
                    continue;
                }
                None => return Err("unterminated backtick reference".to_string()),
            }
        }
        // Quoted string literal.
        if head == '"' || head == '\'' {
            let body = &rest[1..];
            match body.find(head) {
                Some(end) => {
                    toks.push(Tok::Lit(Value::String(body[..end].to_string())));
                    rest = &body[end + 1..];
                    continue;
                }
                None => return Err(format!("unterminated {head} string literal")),
            }
        }
        // Operators (two-char forms first so `<=` is not eaten as `<`).
        for sym in ["==", "!=", "<=", ">=", "<", ">"] {
            if rest.starts_with(sym) {
                toks.push(Tok::Op(
                    CmpOp::parse(sym).expect("listed symbols all parse"),
                ));
                rest = &rest[sym.len()..];
                continue 'scan;
            }
        }
        // Number literal (optional sign, integral → Int, fractional → Float).
        if head.is_ascii_digit() || head == '-' {
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            let num: Result<f64, _> = raw.parse();
            let num = num.map_err(|_| format!("bad number literal '{raw}'"))?;
            if raw.contains('.') {
                toks.push(Tok::Lit(Value::Float(num)));
            } else {
                let int: i64 = raw
                    .parse()
                    .map_err(|_| format!("number literal '{raw}' out of i64 range"))?;
                toks.push(Tok::Lit(Value::Int(int)));
            }
            rest = &rest[end..];
            continue;
        }
        // Bare word: bool literal or field reference.
        if head.is_ascii_alphabetic() || head == '_' {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            let word = &rest[..end];
            toks.push(match word {
                "true" => Tok::Lit(Value::Bool(true)),
                "false" => Tok::Lit(Value::Bool(false)),
                _ => Tok::Field(word.to_string()),
            });
            rest = &rest[end..];
            continue;
        }
        return Err(format!("unexpected character '{head}'"));
    }
    Ok(toks)
}

/// Parse an assert string into a single comparison.
///
/// Errors name the position problem; the assert text itself goes into the
/// caller's diagnostic, so errors here stay short.
pub fn parse_assert(text: &str) -> Result<Comparison, String> {
    let toks = lex(text)?;
    let mut it = toks.into_iter();
    let lhs = take_operand(&mut it, text)?;
    let op = match it.next() {
        Some(Tok::Op(op)) => op,
        Some(_) => return Err(format!("expected an operator after '{}'", lhs.render())),
        None => return Err("assert must be `operand OP operand`".to_string()),
    };
    let rhs = take_operand(&mut it, text)?;
    if let Some(extra) = it.next() {
        return Err(match extra {
            Tok::Op(op) => format!("unexpected operator '{}'", op.as_str()),
            Tok::Field(name) => format!("unexpected field '{name}'"),
            Tok::Lit(_) => "unexpected literal".to_string(),
        });
    }
    Ok(Comparison { lhs, op, rhs })
}

fn take_operand(it: &mut std::vec::IntoIter<Tok>, text: &str) -> Result<Operand, String> {
    match it.next() {
        Some(Tok::Field(name)) => Ok(Operand::Field(name)),
        Some(Tok::Lit(v)) => Ok(Operand::Lit(v)),
        Some(Tok::Op(op)) => Err(format!("expected an operand, found '{}'", op.as_str())),
        None => Err(if text.is_empty() {
            "assert is empty".to_string()
        } else {
            format!("assert '{text}' is missing an operand or operator")
        }),
    }
}

/// Evaluate a parsed assert against a row.
pub fn evaluate(cmp: &Comparison, row: &Row) -> Outcome {
    let lhs = resolve(&cmp.lhs, row);
    let rhs = resolve(&cmp.rhs, row);
    let (l, r) = match (lhs, rhs) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(name), _) | (_, Err(name)) => return Outcome::MissingField(name),
    };
    compare(l, cmp.op, r)
}

fn resolve<'a>(op: &'a Operand, row: &'a Row) -> Result<&'a Value, String> {
    let value = match op {
        Operand::Lit(v) => v,
        Operand::Field(name) => row
            .fields
            .get(name)
            .map(|tv| &tv.value)
            .ok_or_else(|| name.clone())?,
    };
    // A null counts as absent: the rule says nothing about a missing value.
    if matches!(value, Value::Null) {
        return Err(match op {
            Operand::Field(name) => name.clone(),
            Operand::Lit(_) => String::new(),
        });
    }
    Ok(value)
}

/// Numeric operand folded into one comparison path: Int/UInt stay exact in
/// i128; anything mixed with Float compares in f64.
#[derive(Debug, Clone, Copy)]
enum Numeric {
    Int(i128),
    Float(f64),
}

impl Numeric {
    fn as_f64(self) -> f64 {
        match self {
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }
}

fn as_numeric(v: &Value) -> Option<Numeric> {
    match v {
        Value::Int(i) => Some(Numeric::Int(i128::from(*i))),
        Value::UInt(u) => Some(Numeric::Int(i128::from(*u))),
        Value::Float(f) => Some(Numeric::Float(*f)),
        _ => None,
    }
}

fn compare(l: &Value, op: CmpOp, r: &Value) -> Outcome {
    use Value::{Bool, String as Str};
    match (as_numeric(l), as_numeric(r)) {
        (Some(a), Some(b)) => Outcome::Holds(match (a, b) {
            (Numeric::Int(a), Numeric::Int(b)) => apply(&a, op, &b),
            (a, b) => apply(&a.as_f64(), op, &b.as_f64()),
        }),
        _ => match (l, r) {
            (Str(a), Str(b)) => Outcome::Holds(apply(&a.as_str(), op, &b.as_str())),
            (Bool(a), Bool(b)) => Outcome::Holds(apply(a, op, b)),
            _ => Outcome::Incomparable {
                lhs: l.type_name().to_string(),
                rhs: r.type_name().to_string(),
            },
        },
    }
}

fn apply<T: PartialOrd + PartialEq>(l: &T, op: CmpOp, r: &T) -> bool {
    match op {
        CmpOp::Lt => l < r,
        CmpOp::Le => l <= r,
        CmpOp::Gt => l > r,
        CmpOp::Ge => l >= r,
        CmpOp::Eq => l == r,
        CmpOp::Ne => l != r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{SourceLocation, TypedValue};
    use indexmap::IndexMap;

    fn row_of(fields: &[(&str, Value)]) -> Row {
        let mut map = IndexMap::new();
        for (name, value) in fields {
            map.insert(
                (*name).to_string(),
                TypedValue {
                    value: value.clone(),
                    location: SourceLocation::new("test"),
                    schema_type: None,
                },
            );
        }
        Row {
            primary_key: vec![],
            fields: map,
            location: SourceLocation::new("test"),
            index: 0,
        }
    }

    fn eval(assert: &str, fields: &[(&str, Value)]) -> Outcome {
        let cmp = parse_assert(assert).expect("assert parses");
        evaluate(&cmp, &row_of(fields))
    }

    #[test]
    fn parses_bare_backtick_and_literal_operands() {
        // Bare names, backticked names, numbers, quoted strings, bools.
        let cmp = parse_assert("min_level <= max_level").expect("bare parses");
        assert_eq!(cmp.op, CmpOp::Le);
        assert_eq!(cmp.field_refs(), vec!["min_level", "max_level"]);

        let cmp = parse_assert("`level` >= 1").expect("backtick parses");
        assert_eq!(cmp.lhs, Operand::Field("level".to_string()));
        assert_eq!(cmp.rhs, Operand::Lit(Value::Int(1)));

        let cmp = parse_assert("tradable == true").expect("bool literal parses");
        assert_eq!(cmp.rhs, Operand::Lit(Value::Bool(true)));

        let cmp = parse_assert("kind != 'weapon'").expect("single-quoted parses");
        assert_eq!(cmp.rhs, Operand::Lit(Value::String("weapon".to_string())));

        let cmp = parse_assert("price>=2.5").expect("spacing optional");
        assert_eq!(cmp.op, CmpOp::Ge);
        assert_eq!(cmp.rhs, Operand::Lit(Value::Float(2.5)));

        let cmp = parse_assert("code == \"a b\"").expect("double-quoted parses");
        assert_eq!(cmp.rhs, Operand::Lit(Value::String("a b".to_string())));
    }

    #[test]
    fn parser_rejects_malformed_asserts() {
        for (bad, why) in [
            ("", "empty"),
            ("price <=", "missing rhs"),
            ("<= 10", "missing lhs"),
            ("price", "no operator"),
            ("a < b < c", "chained comparison"),
            ("price = 10", "single equals"),
            ("price ~ 10", "unknown operator"),
            ("price & 10", "unexpected character"),
            ("`open name", "unterminated backtick"),
            ("1.2.3 == x", "bad number"),
        ] {
            assert!(
                parse_assert(bad).is_err(),
                "expected '{bad}' ({why}) to fail"
            );
        }
    }

    #[test]
    fn evaluates_numeric_string_and_bool_comparisons() {
        assert_eq!(
            eval("price <= 10000", &[("price", Value::UInt(20000))]),
            Outcome::Holds(false)
        );
        assert_eq!(
            eval(
                "hp <= attack",
                &[("hp", Value::Int(120)), ("attack", Value::Int(120))]
            ),
            Outcome::Holds(true)
        );
        // Int vs UInt vs Float all compare numerically.
        assert_eq!(
            eval("price >= 1.5", &[("price", Value::UInt(2))]),
            Outcome::Holds(true)
        );
        assert_eq!(
            eval("a < b", &[("a", Value::Int(-3)), ("b", Value::UInt(2))]),
            Outcome::Holds(true)
        );
        assert_eq!(
            eval(
                "kind != 'weapon'",
                &[("kind", Value::String("armor".into()))]
            ),
            Outcome::Holds(true)
        );
        assert_eq!(
            eval("name < 'b'", &[("name", Value::String("abc".into()))]),
            Outcome::Holds(true)
        );
        assert_eq!(
            eval("flag == true", &[("flag", Value::Bool(false))]),
            Outcome::Holds(false)
        );
        assert_eq!(
            eval(
                "a != b",
                &[("a", Value::Bool(false)), ("b", Value::Bool(true))]
            ),
            Outcome::Holds(true)
        );
    }

    #[test]
    fn missing_fields_pass_and_mixed_types_are_incomparable() {
        // Absent optional field → the rule says nothing.
        assert_eq!(
            eval("bonus >= 1", &[]),
            Outcome::MissingField("bonus".to_string())
        );
        // Explicit null behaves like absent.
        assert_eq!(
            eval("bonus >= 1", &[("bonus", Value::Null)]),
            Outcome::MissingField("bonus".to_string())
        );
        // Cross-family comparisons are violations, not silent passes.
        assert_eq!(
            eval(
                "a <= b",
                &[("a", Value::String("x".into())), ("b", Value::Int(1))]
            ),
            Outcome::Incomparable {
                lhs: "string".to_string(),
                rhs: "int".to_string()
            }
        );
        assert_eq!(
            eval("a == b", &[("a", Value::Bool(true)), ("b", Value::Int(1))]),
            Outcome::Incomparable {
                lhs: "bool".to_string(),
                rhs: "int".to_string()
            }
        );
        // Arrays/objects/bytes define no ordering at all.
        assert_eq!(
            eval(
                "a == b",
                &[("a", Value::Bytes(vec![])), ("b", Value::Bytes(vec![]))]
            ),
            Outcome::Incomparable {
                lhs: "bytes".to_string(),
                rhs: "bytes".to_string()
            }
        );
    }

    #[test]
    fn renders_operands_for_diagnostics() {
        assert_eq!(Operand::Field("hp".into()).render(), "hp");
        assert_eq!(Operand::Lit(Value::Int(10)).render(), "10");
        assert_eq!(Operand::Lit(Value::Float(2.5)).render(), "2.5");
        assert_eq!(Operand::Lit(Value::Bool(true)).render(), "true");
        assert_eq!(Operand::Lit(Value::String("w".into())).render(), "w");
    }
}
