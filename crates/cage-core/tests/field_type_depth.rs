use cage_core::schema::{FieldType, MAX_FIELD_TYPE_DEPTH};
use serde::Deserialize;
use serde_json::Value;

fn inner_int32() -> Value {
    let mut map = serde_json::Map::new();
    map.insert("kind".into(), Value::String("Int32".into()));
    Value::Object(map)
}

fn wrap_array(value: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("kind".into(), Value::String("Array".into()));
    map.insert("value".into(), value);
    Value::Object(map)
}

fn build(n: usize) -> Value {
    let mut value = inner_int32();
    for _ in 0..n {
        value = wrap_array(value);
    }
    value
}

/// Drops a `serde_json::Value` without recursing, so adversarial test inputs
/// do not overflow the test thread while being reclaimed.
fn drop_iteratively(value: Value) {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(items) => pending.extend(items),
            Value::Object(map) => pending.extend(map.into_iter().map(|(_, value)| value)),
            _ => {}
        }
    }
}

#[test]
fn from_value_just_over_cap_is_refused() {
    let err = serde_json::from_value::<FieldType>(build(MAX_FIELD_TYPE_DEPTH + 1))
        .expect_err("over-cap must be refused");
    assert!(err.to_string().contains("nesting depth limit"), "{err}");
}

#[test]
fn from_str_deep_nested_is_safely_refused() {
    let mut wire = r#"{"kind":"Int32"}"#.to_string();
    for _ in 0..10_000 {
        wire = format!(r#"{{"kind":"Array","value":{wire}}}"#);
    }
    let err = serde_json::from_str::<FieldType>(&wire).expect_err("deep wire must be refused");
    assert!(err.to_string().contains("nesting depth limit"), "{err}");
}

#[test]
fn from_value_at_cap_round_trips() {
    let parsed: FieldType = serde_json::from_value(build(MAX_FIELD_TYPE_DEPTH))
        .expect("a type exactly at the cap parses");
    let wire = serde_json::to_string(&parsed).expect("at-cap type serializes");
    let back: FieldType = serde_json::from_str(&wire).expect("at-cap wire re-parses");
    assert_eq!(back, parsed);
}

/// `serde_json::from_value` consumes the `Value`, and dropping an over-cap
/// `Value` recurses through serde_json's own `Drop`; that residual is tracked
/// separately. The borrowed path must still reject at the cap without
/// recursing through the whole input.
#[test]
fn borrowed_value_way_over_cap_is_safely_refused() {
    let value = build(10_000);
    let err = FieldType::deserialize(&value).expect_err("way-over-cap must be refused");
    assert!(err.to_string().contains("nesting depth limit"), "{err}");
    drop_iteratively(value);
}
