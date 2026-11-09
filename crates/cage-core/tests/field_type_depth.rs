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

fn wrap_value_first(value: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("value".into(), value);
    map.insert("kind".into(), Value::String("Array".into()));
    Value::Object(map)
}

fn build_value_first(n: usize) -> Value {
    let mut value = inner_int32();
    for _ in 0..n {
        value = wrap_value_first(value);
    }
    value
}

/// The `value` content key may legally precede `kind`. serde's derived
/// adjacently tagged parser buffers such content into its private, unbounded
/// `Content` enum before any `BoundedFieldTypeDe` depth check runs — a deep
/// borrowed `Value` with this key order overflows the stack and the guard is
/// bypassed. The hand-written parser must refuse it instead.
///
/// The parse runs on a controlled large stack: libtest test-thread stacks vary
/// by platform and the per-level frame in this binary is large enough that a
/// 2 MiB harness thread is marginal, so the guardrail (not the harness) must be
/// what decides the outcome here.
#[test]
fn value_first_borrowed_way_over_cap_is_safely_refused() {
    let value = build_value_first(10_000);
    let err = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let r = FieldType::deserialize(&value);
            drop_iteratively(value);
            r
        })
        .unwrap()
        .join()
        .unwrap()
        .expect_err("value-first way-over-cap must be refused");
    assert!(err.to_string().contains("nesting depth"), "{err}");
}

#[test]
fn value_first_from_str_deep_nested_is_safely_refused() {
    let mut wire = r#"{"kind":"Int32"}"#.to_string();
    for _ in 0..10_000 {
        wire = format!(r#"{{"value":{wire},"kind":"Array"}}"#);
    }
    let err = serde_json::from_str::<FieldType>(&wire)
        .expect_err("value-first deep wire must be refused");
    // serde_json's own token recursion limit (128) fires while the content is
    // still being buffered, so either layer refusing counts; the guardrail is
    // the sole protection on the `from_value`/borrowed path, which is covered
    // by `value_first_borrowed_way_over_cap_is_safely_refused`.
    let msg = err.to_string();
    assert!(
        msg.contains("nesting depth") || msg.contains("recursion limit"),
        "{msg}"
    );
}

#[test]
fn value_first_just_over_cap_is_refused() {
    let err = serde_json::from_value::<FieldType>(build_value_first(MAX_FIELD_TYPE_DEPTH + 1))
        .expect_err("value-first over-cap must be refused");
    assert!(err.to_string().contains("nesting depth limit"), "{err}");
}

/// A legal `value`-before-`kind` type at the cap must parse to the same
/// `FieldType` as the canonical kind-first wire, and round-trip.
#[test]
fn value_first_at_cap_matches_kind_first() {
    let kind_first = serde_json::from_value::<FieldType>(build(MAX_FIELD_TYPE_DEPTH))
        .expect("kind-first at-cap parses");
    let value_first = serde_json::from_value::<FieldType>(build_value_first(MAX_FIELD_TYPE_DEPTH))
        .expect("value-first at-cap parses");
    assert_eq!(value_first, kind_first);
    let wire = serde_json::to_string(&value_first).expect("at-cap value-first type serializes");
    let back: FieldType = serde_json::from_str(&wire).expect("at-cap wire re-parses");
    assert_eq!(back, value_first);
}

/// Every container family parses identically when the content key precedes
/// the tag (including a unit variant with `value: null`).
#[test]
fn value_first_container_families_parse_like_kind_first() {
    let value_first_cases = [
        r#"{"value":{"key_type":"string","value_type":{"kind":"Int32"}},"kind":"Map"}"#,
        r#"{"value":{"name":{"kind":"String"},"count":{"kind":"Int32"}},"kind":"Object"}"#,
        r#"{"value":{"tag":"kind","branches":{"Damage":{"amount":{"kind":"Int32"},"element":{"kind":"String"}}}},"kind":"TaggedUnion"}"#,
        r#"{"value":null,"kind":"Int32"}"#,
    ];
    let kind_first_cases = [
        r#"{"kind":"Map","value":{"key_type":"string","value_type":{"kind":"Int32"}}}"#,
        r#"{"kind":"Object","value":{"name":{"kind":"String"},"count":{"kind":"Int32"}}}"#,
        r#"{"kind":"TaggedUnion","value":{"tag":"kind","branches":{"Damage":{"amount":{"kind":"Int32"},"element":{"kind":"String"}}}}}"#,
        r#"{"kind":"Int32"}"#,
    ];
    for (value_first, kind_first) in value_first_cases.iter().zip(kind_first_cases.iter()) {
        let parsed_value_first: FieldType =
            serde_json::from_str(value_first).expect("value-first parses");
        let parsed_kind_first: FieldType =
            serde_json::from_str(kind_first).expect("kind-first parses");
        assert_eq!(
            parsed_value_first, parsed_kind_first,
            "wire mismatch:\n{value_first}"
        );
    }
}
