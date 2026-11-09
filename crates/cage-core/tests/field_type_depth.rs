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

/// Every unit variant parses with no `value` at all (kind-first) and with
/// explicit `value: null` (value-first), yielding identical types — the
/// derived parser accepts both shapes, and the replay must accept them too.
#[test]
fn unit_variants_parse_without_content_in_both_key_orders() {
    const UNIT_VARIANTS: [&str; 15] = [
        "Null", "Bool", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
        "Float32", "Float64", "String", "Bytes", "Any",
    ];
    for name in UNIT_VARIANTS {
        let kind_first = format!(r#"{{"kind":"{name}"}}"#);
        let value_first = format!(r#"{{"value":null,"kind":"{name}"}}"#);
        let direct: FieldType = serde_json::from_str(&kind_first)
            .unwrap_or_else(|e| panic!("{name} must parse without content: {e}"));
        let replayed: FieldType = serde_json::from_str(&value_first)
            .unwrap_or_else(|e| panic!("{name} must parse with null content: {e}"));
        assert_eq!(direct, replayed, "{name}");
    }
}

/// Buffered non-content (booleans, numbers, floats, sequences) replays into
/// the same "expected unit" refusals the direct path produces.
#[test]
fn primitive_content_is_refused_in_value_first_order() {
    let cases: [(&str, &str); 5] = [
        (r#"{"value":true,"kind":"Int32"}"#, "boolean `true`"),
        (r#"{"value":42,"kind":"Int32"}"#, "integer `42`"),
        (
            r#"{"value":18446744073709551615,"kind":"Int32"}"#,
            "integer `18446744073709551615`",
        ),
        (r#"{"value":1.5,"kind":"Int32"}"#, "floating point `1.5`"),
        (r#"{"value":[1],"kind":"Int32"}"#, "sequence"),
    ];
    for (wire, unexpected) in cases {
        let err = serde_json::from_str::<FieldType>(wire)
            .expect_err("unit variant must refuse non-unit content");
        assert!(err.to_string().contains(unexpected), "{err}");
    }
}

/// The positional `[tag, value]` form parses like the map form for unit,
/// string-payload and container payloads; every malformed shape is refused
/// with the derive's own diagnostics — including non-unit content after a
/// unit tag, a bare tag without content, and extra elements.
#[test]
fn seq_form_matches_the_map_form() {
    let cases: [(&str, &str); 3] = [
        (r#"["Int32",null]"#, r#"{"kind":"Int32"}"#),
        (
            r#"["Enum","Damage"]"#,
            r#"{"kind":"Enum","value":"Damage"}"#,
        ),
        (
            r#"["Array",{"kind":"Int32"}]"#,
            r#"{"kind":"Array","value":{"kind":"Int32"}}"#,
        ),
    ];
    for (seq_wire, map_wire) in cases {
        let from_seq: FieldType =
            serde_json::from_str(seq_wire).unwrap_or_else(|e| panic!("{seq_wire}: {e}"));
        let from_map: FieldType =
            serde_json::from_str(map_wire).unwrap_or_else(|e| panic!("{map_wire}: {e}"));
        assert_eq!(from_seq, from_map, "{seq_wire}");
    }

    let err = serde_json::from_str::<FieldType>(r#"["Quadruple"]"#)
        .expect_err("unknown variant must be refused in seq form");
    assert!(
        err.to_string().contains("unknown variant `Quadruple`"),
        "{err}"
    );

    // The derive requires the content element even for unit variants and
    // only accepts unit-shaped content: a bare tag is an invalid length and
    // illegal content is refused, not ignored.
    for wire in [r#"["Int32"]"#, r#"["Enum"]"#] {
        let err = serde_json::from_str::<FieldType>(wire)
            .expect_err("bare tag without content must be refused");
        assert!(err.to_string().contains("invalid length 1"), "{err}");
    }

    let err = serde_json::from_str::<FieldType>(r#"[]"#).expect_err("empty seq must be refused");
    assert!(err.to_string().contains("invalid length 0"), "{err}");

    for wire in [r#"["Int32",true]"#, r#"["Int32",1]"#] {
        let err = serde_json::from_str::<FieldType>(wire)
            .expect_err("unit variant must refuse non-unit seq content");
        assert!(err.to_string().contains("invalid type"), "{err}");
    }

    let err = serde_json::from_str::<FieldType>(r#"["Int32",null,1]"#)
        .expect_err("extra seq element must be refused");
    assert!(err.to_string().contains("trailing characters"), "{err}");
}

/// Duplicate `kind`/`value` keys and a missing `kind` are refused in both
/// key orders; unrelated keys are ignored like the derive does.
#[test]
fn duplicate_and_missing_keys_are_refused_in_both_orders() {
    let dup_kind: [&str; 2] = [
        r#"{"kind":"Int32","kind":"Bool"}"#,
        r#"{"value":null,"kind":"Int32","kind":"Bool"}"#,
    ];
    for wire in dup_kind {
        let err =
            serde_json::from_str::<FieldType>(wire).expect_err("duplicate kind must be refused");
        assert!(err.to_string().contains("duplicate field `kind`"), "{err}");
    }
    let dup_value: [&str; 2] = [
        r#"{"kind":"Int32","value":null,"value":1}"#,
        r#"{"value":null,"kind":"Int32","value":1}"#,
    ];
    for wire in dup_value {
        let err =
            serde_json::from_str::<FieldType>(wire).expect_err("duplicate value must be refused");
        assert!(err.to_string().contains("duplicate field `value`"), "{err}");
    }

    let err = serde_json::from_str::<FieldType>(r#"{"value":null}"#)
        .expect_err("missing kind must be refused");
    assert!(err.to_string().contains("missing field `kind`"), "{err}");

    let parsed: FieldType = serde_json::from_str(r#"{"kind":"Int32","extra":{"nested":[1,2]}}"#)
        .expect("unrelated keys are ignored");
    assert_eq!(parsed, FieldType::Int32);
}

/// A non-unit variant name with no `value` key is refused exactly like the
/// derived parser (`missing field \`value\``).
#[test]
fn non_unit_variant_without_value_is_refused() {
    let err = serde_json::from_str::<FieldType>(r#"{"kind":"Enum"}"#)
        .expect_err("non-unit variant must demand a value");
    assert!(err.to_string().contains("missing field `value`"), "{err}");
}
