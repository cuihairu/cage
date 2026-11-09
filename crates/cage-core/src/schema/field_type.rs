//! `FieldType`: the recursive field-type enum, its inherent helpers,
//! wire (de)serialization support types, and the nesting-depth measure
//! (split out of mod.rs, LAY-006).

use super::field_type_serde::FieldTypeDe;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Field type system
#[derive(Default)]
pub enum FieldType {
    /// Null type
    #[default]
    Null,
    /// Boolean type
    Bool,
    /// 8-bit signed integer
    Int8,
    /// 16-bit signed integer
    Int16,
    /// 32-bit signed integer
    Int32,
    /// 64-bit signed integer
    Int64,
    /// 8-bit unsigned integer
    UInt8,
    /// 16-bit unsigned integer
    UInt16,
    /// 32-bit unsigned integer
    UInt32,
    /// 64-bit unsigned integer
    UInt64,
    /// 32-bit floating point
    Float32,
    /// 64-bit floating point
    Float64,
    /// UTF-8 string
    String,
    /// Raw bytes
    Bytes,
    /// Array with element type
    Array(Box<FieldType>),
    /// Object with typed properties
    Object(IndexMap<String, FieldType>),
    /// Tagged union: a flat object whose `tag` key selects one branch, and
    /// only the selected branch's payload fields may appear alongside it
    /// (回流-6, the Luban abstract-bean `$type` polymorphism).
    ///
    /// Canonical YAML/JSON schema syntax (adjacently tagged like every other
    /// kind):
    ///
    /// ```yaml
    /// type:
    ///   kind: TaggedUnion
    ///   value:
    ///     tag: $type
    ///     branches:
    ///       Damage: { amount: { kind: Int32 }, element: { kind: String } }
    ///       Heal:   { amount: { kind: Int32 }, duration: { kind: Float32 } }
    /// ```
    ///
    /// A value `{"$type":"Damage","amount":13,"element":"wind"}` selects the
    /// `Damage` branch; `buff_id` (a Heal-only key) on a `$type: Damage`
    /// object is rejected. Sources carrying native objects (JSON/YAML)
    /// express this directly; flat table sources stay on the enum + optional
    /// payload-column workaround until structure building (回流-5).
    TaggedUnion(Box<TaggedUnion>),
    /// Map with typed keys and values: `map<K, V>`.
    ///
    /// Canonical YAML/JSON schema syntax (adjacently tagged like every other
    /// kind):
    ///
    /// ```yaml
    /// type:
    ///   kind: Map
    ///   value:
    ///     key_type: string              # string | int
    ///     value_type:
    ///       kind: Array
    ///       value: { kind: Int32 }
    /// ```
    ///
    /// So `map<string, Array<Int32>>` =
    /// `{ kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }`.
    /// The payload is a struct rather than a bare `Box<FieldType>` because the
    /// key type declaration has to live inside the type — nested maps carry
    /// their own `key_type` that way.
    Map(MapField),
    /// Reference to a named enum
    Enum(String),
    /// Any value
    Any,
}

impl FieldType {
    /// Convert a parsed [`FieldTypeDe`] into a [`FieldType`], enforcing the
    /// nesting cap. The recursion is bounded by [`MAX_FIELD_TYPE_DEPTH`]: a
    /// node past the cap returns an error before descending further.
    pub(crate) fn from_de(value: FieldTypeDe, depth: usize) -> Result<Self, String> {
        match value {
            FieldTypeDe::Null => Ok(FieldType::Null),
            FieldTypeDe::Bool => Ok(FieldType::Bool),
            FieldTypeDe::Int8 => Ok(FieldType::Int8),
            FieldTypeDe::Int16 => Ok(FieldType::Int16),
            FieldTypeDe::Int32 => Ok(FieldType::Int32),
            FieldTypeDe::Int64 => Ok(FieldType::Int64),
            FieldTypeDe::UInt8 => Ok(FieldType::UInt8),
            FieldTypeDe::UInt16 => Ok(FieldType::UInt16),
            FieldTypeDe::UInt32 => Ok(FieldType::UInt32),
            FieldTypeDe::UInt64 => Ok(FieldType::UInt64),
            FieldTypeDe::Float32 => Ok(FieldType::Float32),
            FieldTypeDe::Float64 => Ok(FieldType::Float64),
            FieldTypeDe::String => Ok(FieldType::String),
            FieldTypeDe::Bytes => Ok(FieldType::Bytes),
            FieldTypeDe::Any => Ok(FieldType::Any),
            FieldTypeDe::Enum(name) => Ok(FieldType::Enum(name)),
            FieldTypeDe::Array(inner) => {
                if depth + 1 > MAX_FIELD_TYPE_DEPTH {
                    return Err(format!(
                        "field type exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                    ));
                }
                Ok(FieldType::Array(Box::new(FieldType::from_de(
                    inner.0,
                    depth + 1,
                )?)))
            }
            FieldTypeDe::Map(map) => {
                if depth + 1 > MAX_FIELD_TYPE_DEPTH {
                    return Err(format!(
                        "field type exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                    ));
                }
                Ok(FieldType::Map(MapField {
                    key_type: map.key_type,
                    value_type: Box::new(FieldType::from_de(map.value_type.0, depth + 1)?),
                }))
            }
            FieldTypeDe::Object(fields) => {
                if depth + 1 > MAX_FIELD_TYPE_DEPTH {
                    return Err(format!(
                        "field type exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                    ));
                }
                let mut out = IndexMap::new();
                for (name, field) in fields {
                    out.insert(name, FieldType::from_de(field.0, depth + 1)?);
                }
                Ok(FieldType::Object(out))
            }
            FieldTypeDe::TaggedUnion(tu) => {
                if depth + 1 > MAX_FIELD_TYPE_DEPTH {
                    return Err(format!(
                        "field type exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                    ));
                }
                let mut branches = IndexMap::new();
                for (name, branch) in tu.branches {
                    let mut fields = IndexMap::new();
                    for (fname, field) in branch {
                        fields.insert(fname, FieldType::from_de(field.0, depth + 1)?);
                    }
                    branches.insert(name, fields);
                }
                Ok(FieldType::TaggedUnion(Box::new(TaggedUnion {
                    tag: tu.tag,
                    branches,
                })))
            }
        }
    }
}

/// Tagged-union payload: the discriminant key plus one field-type map per
/// branch — the L2 type-safety face of the Luban `$type` polymorphism (回流-6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaggedUnion {
    /// Object key carrying the discriminant (Luban `$type`, five-case `kind`).
    /// Must not collide with a branch payload key.
    pub tag: String,
    /// Branch key → the branch's payload field types. The tag key itself is
    /// the discriminant, not a payload field; only the selected branch's
    /// payload keys may appear next to it.
    pub branches: IndexMap<String, IndexMap<String, FieldType>>,
}

/// Map payload: key type plus (possibly nested) value type — `map<K, V>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapField {
    /// Key type: string keys or signed-integer keys.
    pub key_type: MapKeyType,
    /// Value type — arbitrary, may nest (Array / Map / Object / ...).
    pub value_type: Box<FieldType>,
}

/// Maximum nesting depth for a `FieldType` (`Array` / `Map` / `Object` /
/// `TaggedUnion` recursively nest). A deeper type is refused by
/// [`Schema::validate`] rather than recursing the schema and the
/// normalize/generator passes without bound — a maliciously deep schema
/// must not overflow the stack. 64 is far beyond any realistic data
/// shape and keeps the recursive walks safely bounded.
pub const MAX_FIELD_TYPE_DEPTH: usize = 64;

/// Key kinds accepted by [`MapField::key_type`] (wire format: `string` | `int`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapKeyType {
    /// String keys — every `Value::Object` key is a string, so all pass.
    String,
    /// Signed-integer keys, stored as numeric strings in the data model
    /// (`"42"`, `"-7"`); non-numeric keys are rejected at L2.
    Int,
}

/// Iterative `Drop` for [`FieldType`].
///
/// The derived `Drop` recursively frees `Array(Box<FieldType>)` /
/// `Map(Box<FieldType>)` / `Object(...)` / `TaggedUnion(...)`. A schema
/// type nested past the depth cap (which `validate` and the bounded
/// deserializers refuse) would otherwise overflow the stack *when the value
/// goes out of scope*, even though validation already rejected it. An
/// explicit worklist flattens the container spine so teardown stays bounded.
///
/// Because `FieldType` has a custom `Drop`, an emptied container shell (all
/// children already peeled onto the worklist) would otherwise re-enter this
/// `Drop` on its own implicit drop and recurse forever — so a node with no
/// owned `FieldType` children is released immediately without peeling.
impl Drop for FieldType {
    fn drop(&mut self) {
        // A leaf, or a container whose direct children are all already
        // Null/empty, owns nothing to peel. Returning here is essential:
        // the implicit drop of such a node must not re-enter this worklist.
        let trivial = match self {
            FieldType::Array(inner) => matches!(**inner, FieldType::Null),
            FieldType::Map(map) => matches!(*map.value_type, FieldType::Null),
            FieldType::Object(fields) => fields.is_empty(),
            FieldType::TaggedUnion(tu) => tu.branches.is_empty(),
            _ => true,
        };
        if trivial {
            return;
        }
        // Peel the spine iteratively: each node's owned children are moved
        // onto the worklist, and the emptied shell is dropped on the next
        // loop iteration (where `trivial` is now true and returns at once).
        let mut work: Vec<FieldType> = Vec::new();
        work.push(std::mem::take(self));
        while let Some(mut node) = work.pop() {
            match &mut node {
                FieldType::Array(inner) => {
                    work.push(std::mem::replace(&mut **inner, FieldType::Null));
                }
                FieldType::Map(map) => {
                    work.push(std::mem::replace(&mut *map.value_type, FieldType::Null));
                }
                FieldType::Object(fields) => {
                    work.extend(std::mem::take(fields).into_values());
                }
                FieldType::TaggedUnion(tu) => {
                    let tu = std::mem::replace(
                        &mut **tu,
                        TaggedUnion {
                            tag: String::new(),
                            branches: IndexMap::new(),
                        },
                    );
                    for (_bk, branch) in tu.branches {
                        work.extend(branch.into_values());
                    }
                }
                _ => {}
            }
        }
    }
}

impl Clone for FieldType {
    /// Iterative `Clone`.
    ///
    /// The derived `Clone` recursively copies `Array(Box<FieldType>)` /
    /// `Map(Box<FieldType>)` / `Object(...)` / `TaggedUnion(...)`. A type
    /// nested past the depth cap would otherwise overflow the stack during
    /// the copy, mirroring the `Drop` problem this type already solved. An
    /// explicit task stack performs the same copy without unbounded
    /// recursion: leaves are pushed onto a result stack, and container
    /// assemblers pop their children back off in declaration order.
    fn clone(&self) -> Self {
        enum Task<'a> {
            Visit(&'a FieldType),
            AssembleArray,
            AssembleMap(&'a MapField),
            AssembleObject(&'a IndexMap<String, FieldType>),
            AssembleTaggedUnion(&'a TaggedUnion),
        }
        let mut tasks: Vec<Task> = vec![Task::Visit(self)];
        let mut results: Vec<FieldType> = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(t) => match t {
                    FieldType::Array(inner) => {
                        tasks.push(Task::AssembleArray);
                        tasks.push(Task::Visit(inner));
                    }
                    FieldType::Map(map) => {
                        tasks.push(Task::AssembleMap(map));
                        tasks.push(Task::Visit(&map.value_type));
                    }
                    FieldType::Object(fields) => {
                        tasks.push(Task::AssembleObject(fields));
                        for f in fields.values() {
                            tasks.push(Task::Visit(f));
                        }
                    }
                    FieldType::TaggedUnion(tu) => {
                        tasks.push(Task::AssembleTaggedUnion(tu));
                        for branch in tu.branches.values() {
                            for f in branch.values() {
                                tasks.push(Task::Visit(f));
                            }
                        }
                    }
                    FieldType::Null => results.push(FieldType::Null),
                    FieldType::Bool => results.push(FieldType::Bool),
                    FieldType::Int8 => results.push(FieldType::Int8),
                    FieldType::Int16 => results.push(FieldType::Int16),
                    FieldType::Int32 => results.push(FieldType::Int32),
                    FieldType::Int64 => results.push(FieldType::Int64),
                    FieldType::UInt8 => results.push(FieldType::UInt8),
                    FieldType::UInt16 => results.push(FieldType::UInt16),
                    FieldType::UInt32 => results.push(FieldType::UInt32),
                    FieldType::UInt64 => results.push(FieldType::UInt64),
                    FieldType::Float32 => results.push(FieldType::Float32),
                    FieldType::Float64 => results.push(FieldType::Float64),
                    FieldType::String => results.push(FieldType::String),
                    FieldType::Bytes => results.push(FieldType::Bytes),
                    FieldType::Any => results.push(FieldType::Any),
                    FieldType::Enum(name) => {
                        results.push(FieldType::Enum(name.clone()));
                    }
                },
                Task::AssembleArray => {
                    let inner = results.pop().expect("array child is on the stack");
                    results.push(FieldType::Array(Box::new(inner)));
                }
                Task::AssembleMap(map) => {
                    let value = results.pop().expect("map value is on the stack");
                    results.push(FieldType::Map(MapField {
                        key_type: map.key_type,
                        value_type: Box::new(value),
                    }));
                }
                Task::AssembleObject(fields) => {
                    let mut built = IndexMap::with_capacity(fields.len());
                    for (name, _) in fields {
                        let value = results.pop().expect("object child is on the stack");
                        built.insert(name.clone(), value);
                    }
                    results.push(FieldType::Object(built));
                }
                Task::AssembleTaggedUnion(tu) => {
                    let mut built = IndexMap::with_capacity(tu.branches.len());
                    for (branch_name, branch) in &tu.branches {
                        let mut fields = IndexMap::with_capacity(branch.len());
                        for (name, _) in branch {
                            let value = results.pop().expect("tagged-union child is on the stack");
                            fields.insert(name.clone(), value);
                        }
                        built.insert(branch_name.clone(), fields);
                    }
                    results.push(FieldType::TaggedUnion(Box::new(TaggedUnion {
                        tag: tu.tag.clone(),
                        branches: built,
                    })));
                }
            }
        }
        results.pop().expect("a clone yields exactly one root")
    }
}

impl std::fmt::Debug for FieldType {
    /// Iterative `Debug`.
    ///
    /// The derived `Debug` recursively formats `Array(Box<FieldType>)` /
    /// `Map(Box<FieldType>)` / `Object(...)` / `TaggedUnion(...)`, which
    /// overflows on a type nested past the depth cap. This writes the same
    /// shape as the derive (variant names and nested fields) using an explicit
    /// worklist so deep types render in bounded stack.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A type nested past the depth cap is refused by `validate` and only
        // reaches `Debug` in a pathological/hand-built case. Recursing with
        // the standard `debug_*` builders would overflow the stack there, so
        // that case falls back to an iterative compact renderer. A normal
        // (at-or-under-cap) type uses the builders, which reproduces the
        // derived `Debug` byte-for-byte — including `{:#?}` alternation.
        if field_type_depth(self).is_some_and(|m| m.depth > MAX_FIELD_TYPE_DEPTH) {
            return self.fmt_deep(f);
        }
        match self {
            FieldType::Null => f.write_str("Null"),
            FieldType::Bool => f.write_str("Bool"),
            FieldType::Int8 => f.write_str("Int8"),
            FieldType::Int16 => f.write_str("Int16"),
            FieldType::Int32 => f.write_str("Int32"),
            FieldType::Int64 => f.write_str("Int64"),
            FieldType::UInt8 => f.write_str("UInt8"),
            FieldType::UInt16 => f.write_str("UInt16"),
            FieldType::UInt32 => f.write_str("UInt32"),
            FieldType::UInt64 => f.write_str("UInt64"),
            FieldType::Float32 => f.write_str("Float32"),
            FieldType::Float64 => f.write_str("Float64"),
            FieldType::String => f.write_str("String"),
            FieldType::Bytes => f.write_str("Bytes"),
            FieldType::Any => f.write_str("Any"),
            FieldType::Enum(name) => f.debug_tuple("Enum").field(name).finish(),
            FieldType::Array(inner) => f.debug_tuple("Array").field(inner).finish(),
            FieldType::Map(map) => f.debug_tuple("Map").field(map).finish(),
            FieldType::Object(fields) => f.debug_tuple("Object").field(fields).finish(),
            FieldType::TaggedUnion(tu) => f.debug_tuple("TaggedUnion").field(tu).finish(),
        }
    }
}

impl FieldType {
    /// Bounded-stack `Debug` for a type nested past [`MAX_FIELD_TYPE_DEPTH`].
    ///
    /// The standard `debug_*` builders recurse once per container level and
    /// would overflow on a pathologically deep type. This renders the same
    /// compact shape (variant names and nested fields) with an explicit
    /// worklist, sacrificing `{:#?}` alternation for a type that `validate`
    /// refuses anyway.
    fn fmt_deep(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Write as _;

        enum Piece<'a> {
            Type(&'a FieldType),
            Literal(&'static str),
            Quoted(&'a str),
        }
        fn fmt_map<'a>(fields: &'a IndexMap<String, FieldType>, pieces: &mut Vec<Piece<'a>>) {
            // Output `{"name": <type>, ...}`. The worklist pops in reverse, so
            // push the pieces that should print *last* first.
            pieces.push(Piece::Literal("}"));
            for (i, (name, field)) in fields.iter().enumerate().rev() {
                pieces.push(Piece::Type(field));
                pieces.push(Piece::Literal(": "));
                pieces.push(Piece::Quoted(name));
                if i > 0 {
                    pieces.push(Piece::Literal(", "));
                }
            }
            pieces.push(Piece::Literal("{"));
        }
        let mut pieces: Vec<Piece> = vec![Piece::Type(self)];
        let mut out = String::new();
        while let Some(piece) = pieces.pop() {
            match piece {
                Piece::Type(t) => match t {
                    FieldType::Null => out.push_str("Null"),
                    FieldType::Bool => out.push_str("Bool"),
                    FieldType::Int8 => out.push_str("Int8"),
                    FieldType::Int16 => out.push_str("Int16"),
                    FieldType::Int32 => out.push_str("Int32"),
                    FieldType::Int64 => out.push_str("Int64"),
                    FieldType::UInt8 => out.push_str("UInt8"),
                    FieldType::UInt16 => out.push_str("UInt16"),
                    FieldType::UInt32 => out.push_str("UInt32"),
                    FieldType::UInt64 => out.push_str("UInt64"),
                    FieldType::Float32 => out.push_str("Float32"),
                    FieldType::Float64 => out.push_str("Float64"),
                    FieldType::String => out.push_str("String"),
                    FieldType::Bytes => out.push_str("Bytes"),
                    FieldType::Any => out.push_str("Any"),
                    FieldType::Enum(name) => {
                        out.push_str("Enum(");
                        write!(out, "{name:?}").expect("writing to String cannot fail");
                        out.push(')');
                    }
                    FieldType::Array(inner) => {
                        out.push_str("Array(");
                        pieces.push(Piece::Literal(")"));
                        pieces.push(Piece::Type(inner));
                    }
                    FieldType::Map(map) => {
                        out.push_str("Map(MapField { key_type: ");
                        write!(out, "{:?}, value_type: ", map.key_type)
                            .expect("writing to String cannot fail");
                        pieces.push(Piece::Literal(" })"));
                        pieces.push(Piece::Type(&map.value_type));
                    }
                    FieldType::Object(fields) => {
                        out.push_str("Object(");
                        pieces.push(Piece::Literal(")"));
                        fmt_map(fields, &mut pieces);
                    }
                    FieldType::TaggedUnion(tu) => {
                        out.push_str("TaggedUnion(TaggedUnion { tag: ");
                        write!(out, "{:?}, branches: ", tu.tag)
                            .expect("writing to String cannot fail");
                        pieces.push(Piece::Literal(" })"));
                        pieces.push(Piece::Literal("}"));
                        for (i, (branch_name, branch)) in tu.branches.iter().rev().enumerate() {
                            if i > 0 {
                                pieces.push(Piece::Literal(", "));
                            }
                            fmt_map(branch, &mut pieces);
                            pieces.push(Piece::Literal(": "));
                            pieces.push(Piece::Quoted(branch_name));
                        }
                        pieces.push(Piece::Literal("{"));
                    }
                },
                Piece::Literal(s) => out.push_str(s),
                Piece::Quoted(s) => {
                    write!(out, "{s:?}").expect("writing to String cannot fail");
                }
            }
        }
        f.write_str(&out)
    }
}

impl PartialEq for FieldType {
    /// Iterative structural equality.
    ///
    /// The derived `PartialEq` recursively compares `Array(Box<FieldType>)` /
    /// `Map(Box<FieldType>)` / `Object(...)` / `TaggedUnion(...)`, which
    /// overflows on a type nested past the depth cap. This walks both sides
    /// with an explicit worklist so equality of deep types stays bounded.
    fn eq(&self, other: &Self) -> bool {
        enum Frame<'a> {
            Pair(&'a FieldType, &'a FieldType),
            TaggedUnion(&'a TaggedUnion, &'a TaggedUnion),
        }
        let mut stack: Vec<Frame> = vec![Frame::Pair(self, other)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Pair(a, b) => match (a, b) {
                    (FieldType::Null, FieldType::Null)
                    | (FieldType::Bool, FieldType::Bool)
                    | (FieldType::Int8, FieldType::Int8)
                    | (FieldType::Int16, FieldType::Int16)
                    | (FieldType::Int32, FieldType::Int32)
                    | (FieldType::Int64, FieldType::Int64)
                    | (FieldType::UInt8, FieldType::UInt8)
                    | (FieldType::UInt16, FieldType::UInt16)
                    | (FieldType::UInt32, FieldType::UInt32)
                    | (FieldType::UInt64, FieldType::UInt64)
                    | (FieldType::Float32, FieldType::Float32)
                    | (FieldType::Float64, FieldType::Float64)
                    | (FieldType::String, FieldType::String)
                    | (FieldType::Bytes, FieldType::Bytes)
                    | (FieldType::Any, FieldType::Any) => {}
                    (FieldType::Enum(x), FieldType::Enum(y)) => {
                        if x != y {
                            return false;
                        }
                    }
                    (FieldType::Array(x), FieldType::Array(y)) => {
                        stack.push(Frame::Pair(x, y));
                    }
                    (FieldType::Map(x), FieldType::Map(y)) => {
                        if x.key_type != y.key_type {
                            return false;
                        }
                        stack.push(Frame::Pair(&x.value_type, &y.value_type));
                    }
                    (FieldType::Object(x), FieldType::Object(y)) => {
                        if x.len() != y.len() {
                            return false;
                        }
                        for (name, xv) in x {
                            match y.get(name) {
                                Some(yv) => stack.push(Frame::Pair(xv, yv)),
                                None => return false,
                            }
                        }
                    }
                    (FieldType::TaggedUnion(x), FieldType::TaggedUnion(y)) => {
                        if x.tag != y.tag || x.branches.len() != y.branches.len() {
                            return false;
                        }
                        stack.push(Frame::TaggedUnion(x, y));
                    }
                    _ => return false,
                },
                Frame::TaggedUnion(x, y) => {
                    for (branch_name, xb) in &x.branches {
                        let Some(yb) = y.branches.get(branch_name) else {
                            return false;
                        };
                        if xb.len() != yb.len() {
                            return false;
                        }
                        for (name, xv) in xb {
                            match yb.get(name) {
                                Some(yv) => stack.push(Frame::Pair(xv, yv)),
                                None => return false,
                            }
                        }
                    }
                }
            }
        }
        true
    }
}

impl Eq for FieldType {}

fn tagged_wire(kind: &str, value: serde_json::Value) -> serde_json::Value {
    serde_json::Value::Object(
        [
            (
                "kind".to_string(),
                serde_json::Value::String(kind.to_string()),
            ),
            ("value".to_string(), value),
        ]
        .into_iter()
        .collect(),
    )
}

pub(crate) fn scalar_wire(t: &FieldType) -> serde_json::Value {
    let kind = match t {
        FieldType::Null => "Null",
        FieldType::Bool => "Bool",
        FieldType::Int8 => "Int8",
        FieldType::Int16 => "Int16",
        FieldType::Int32 => "Int32",
        FieldType::Int64 => "Int64",
        FieldType::UInt8 => "UInt8",
        FieldType::UInt16 => "UInt16",
        FieldType::UInt32 => "UInt32",
        FieldType::UInt64 => "UInt64",
        FieldType::Float32 => "Float32",
        FieldType::Float64 => "Float64",
        FieldType::String => "String",
        FieldType::Bytes => "Bytes",
        FieldType::Any => "Any",
        FieldType::Enum(name) => {
            let mut wire = serde_json::Map::new();
            wire.insert(
                "kind".to_string(),
                serde_json::Value::String("Enum".to_string()),
            );
            wire.insert("value".to_string(), serde_json::Value::String(name.clone()));
            return serde_json::Value::Object(wire);
        }
        _ => unreachable!("container variants are handled by the serializer"),
    };
    let mut wire = serde_json::Map::new();
    wire.insert(
        "kind".to_string(),
        serde_json::Value::String(kind.to_string()),
    );
    serde_json::Value::Object(wire)
}

enum SerializeFrame<'a> {
    Visit(&'a FieldType),
    /// Finish an `Array` whose child is already on the value stack.
    FinishArray,
    /// Finish a `Map` whose `value_type` is already on the stack.
    FinishMap(&'a MapField),
    /// Finish an `Object` with the given fields (in order).
    FinishObject(&'a IndexMap<String, FieldType>),
    /// Finish a `TaggedUnion` with the given branches (in order).
    FinishUnion(&'a TaggedUnion),
}

impl Serialize for FieldType {
    /// Iterative `Serialize` into the same adjacent-tagged wire format the
    /// derive produced (`{ kind: <variant>, value: <payload> }`).
    ///
    /// The derived `Serialize` recurses through `Array` / `Map` / `Object` /
    /// `TaggedUnion` and would overflow on a type nested past the depth cap.
    /// This flattens the container spine into a `serde_json::Value` tree with
    /// an explicit worklist, then hands the tree to serde — the tree itself is
    /// a plain `Value` whose serialization is also bounded (it contains no
    /// `FieldType` recursion, only JSON scalars/arrays/objects).
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Reject an over-cap type instead of building a nested JSON tree and
        // overflowing deep inside serde. Valid types (<= MAX depth) serialize
        // through the iterative builder below.
        if let Some(measure) = field_type_depth(self) {
            if measure.depth > MAX_FIELD_TYPE_DEPTH {
                return Err(serde::ser::Error::custom(format!(
                    "field type exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                )));
            }
        }
        let mut frames: Vec<SerializeFrame> = vec![SerializeFrame::Visit(self)];
        let mut values: Vec<serde_json::Value> = Vec::new();
        while let Some(frame) = frames.pop() {
            match frame {
                SerializeFrame::Visit(t) => match t {
                    FieldType::Array(inner) => {
                        frames.push(SerializeFrame::FinishArray);
                        frames.push(SerializeFrame::Visit(inner));
                    }
                    FieldType::Map(map) => {
                        frames.push(SerializeFrame::FinishMap(map));
                        frames.push(SerializeFrame::Visit(&map.value_type));
                    }
                    FieldType::Object(fields) => {
                        frames.push(SerializeFrame::FinishObject(fields));
                        for field in fields.values() {
                            frames.push(SerializeFrame::Visit(field));
                        }
                    }
                    FieldType::TaggedUnion(tu) => {
                        frames.push(SerializeFrame::FinishUnion(tu));
                        for branch in tu.branches.values() {
                            for field in branch.values() {
                                frames.push(SerializeFrame::Visit(field));
                            }
                        }
                    }
                    other => values.push(scalar_wire(other)),
                },
                SerializeFrame::FinishArray => {
                    let inner = values.pop().expect("array child on the stack");
                    values.push(tagged_wire("Array", inner));
                }
                SerializeFrame::FinishMap(map) => {
                    let value = values.pop().expect("map value on the stack");
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "key_type".to_string(),
                        serde_json::Value::String(
                            match map.key_type {
                                MapKeyType::String => "string",
                                MapKeyType::Int => "int",
                            }
                            .to_string(),
                        ),
                    );
                    obj.insert("value_type".to_string(), value);
                    values.push(tagged_wire("Map", serde_json::Value::Object(obj)));
                }
                SerializeFrame::FinishObject(fields) => {
                    let mut obj = serde_json::Map::new();
                    // Children were pushed in declaration order, so popped in
                    // reverse. Pop into a Vec then rebuild in order.
                    let count = fields.len();
                    let mut children = Vec::with_capacity(count);
                    for _ in 0..count {
                        children.push(values.pop().expect("object child on the stack"));
                    }
                    for ((name, _), child) in fields.iter().zip(children) {
                        obj.insert(name.clone(), child);
                    }
                    values.push(tagged_wire("Object", serde_json::Value::Object(obj)));
                }
                SerializeFrame::FinishUnion(tu) => {
                    let mut branches = serde_json::Map::new();
                    for (branch_name, branch) in &tu.branches {
                        let count = branch.len();
                        let mut children = Vec::with_capacity(count);
                        for _ in 0..count {
                            children.push(values.pop().expect("union child on the stack"));
                        }
                        let mut branch_obj = serde_json::Map::new();
                        for ((name, _), child) in branch.iter().zip(children) {
                            branch_obj.insert(name.clone(), child);
                        }
                        branches.insert(branch_name.clone(), serde_json::Value::Object(branch_obj));
                    }
                    let mut value = serde_json::Map::new();
                    value.insert("tag".to_string(), serde_json::Value::String(tu.tag.clone()));
                    value.insert("branches".to_string(), serde_json::Value::Object(branches));
                    values.push(tagged_wire("TaggedUnion", serde_json::Value::Object(value)));
                }
            }
        }
        values
            .pop()
            .expect("serialize yields exactly one root")
            .serialize(serializer)
    }
}

impl FieldType {
    /// Take ownership of this value, leaving `FieldType::Null` behind.
    ///
    /// `FieldType` has a custom iterative `Drop`, so Rust's E0509 prevents
    /// matching by value and moving a child out of `Array` / `Map` / `Object` /
    /// `TaggedUnion` / `Enum`. Existing code that needs the owned value should
    /// first call this helper to move the whole value out (the slot is reset
    /// to `Null`). The returned value still implements the custom `Drop`, so a
    /// by-value match that would move a child out of a container variant is
    /// still rejected by E0509 — to extract a child, clone it out (or match on
    /// a borrowed `&mut` slot after this helper has emptied the spine).
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    /// Whether this is any numeric type
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            FieldType::Int8
                | FieldType::Int16
                | FieldType::Int32
                | FieldType::Int64
                | FieldType::UInt8
                | FieldType::UInt16
                | FieldType::UInt32
                | FieldType::UInt64
                | FieldType::Float32
                | FieldType::Float64
        )
    }

    /// Whether this is any integer type
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            FieldType::Int8
                | FieldType::Int16
                | FieldType::Int32
                | FieldType::Int64
                | FieldType::UInt8
                | FieldType::UInt16
                | FieldType::UInt32
                | FieldType::UInt64
        )
    }

    /// Whether this is a floating point type
    pub fn is_float(&self) -> bool {
        matches!(self, FieldType::Float32 | FieldType::Float64)
    }

    /// Corresponding Rust type name (for code generation)
    pub fn rust_type(&self) -> &'static str {
        match self {
            FieldType::Null => "()",
            FieldType::Bool => "bool",
            FieldType::Int8 => "i8",
            FieldType::Int16 => "i16",
            FieldType::Int32 => "i32",
            FieldType::Int64 => "i64",
            FieldType::UInt8 => "u8",
            FieldType::UInt16 => "u16",
            FieldType::UInt32 => "u32",
            FieldType::UInt64 => "u64",
            FieldType::Float32 => "f32",
            FieldType::Float64 => "f64",
            FieldType::String => "String",
            FieldType::Bytes => "Vec<u8>",
            FieldType::Array(_) => "Vec<_>",
            FieldType::Object(_) => "IndexMap<String, _>",
            FieldType::Map(m) => match m.key_type {
                MapKeyType::String => "HashMap<String, _>",
                MapKeyType::Int => "HashMap<i64, _>",
            },
            FieldType::Enum(_) => "Enum",
            FieldType::TaggedUnion(_) => "IndexMap<String, serde_json::Value>",
            FieldType::Any => "serde_json::Value",
        }
    }
}

/// A depth measurement of a [`FieldType`]. `depth` counts the number of
/// container levels (`Int32` is 0, `Array<Int32>` is 1,
/// `Array<Array<Int32>>` is 2, `Object{ x: Array<Int32> }` is 2), but only
/// up to the first level past [`MAX_FIELD_TYPE_DEPTH`]. A node one or two
/// levels over the cap is still measured precisely (so an authoring mistake
/// just past the cap is reported with its real depth); a node three or more
/// levels past the cap short-circuits and reports `MAX_FIELD_TYPE_DEPTH + 1`
/// with `saturated` set, so a call on a pathologically deep type stays
/// stack-bounded instead of recursing thousands of frames.
#[derive(Debug)]
pub(crate) struct DepthMeasure {
    /// The nesting depth, or `MAX_FIELD_TYPE_DEPTH + 1` when `saturated`.
    pub(crate) depth: usize,
    /// Whether the walk short-circuited at the cap. When set, `depth` is a
    /// sentinel rather than the real depth, so callers must not report it as
    /// if it were measured.
    pub(crate) saturated: bool,
}

/// Measure the nesting depth of a `FieldType`. Returns `None` for a
/// scalar/leaf; otherwise the measure, saturated past the cap.
pub(crate) fn field_type_depth(t: &FieldType) -> Option<DepthMeasure> {
    fn walk(t: &FieldType, ancestors: usize) -> Option<DepthMeasure> {
        match t {
            FieldType::Array(inner) => {
                // The current node is the (ancestors + 1)-th container. A
                // node MAX+1 or MAX+2 deep is still measured precisely so a
                // one-or-two-over-the-cap authoring mistake is reported with
                // its real depth; only a node MAX+3 deep and beyond (the
                // pathologically deep case) saturates. The recursion therefore
                // never descends more than a few frames past the cap.
                if ancestors > MAX_FIELD_TYPE_DEPTH + 1 {
                    return Some(DepthMeasure {
                        depth: MAX_FIELD_TYPE_DEPTH + 1,
                        saturated: true,
                    });
                }
                match walk(inner, ancestors + 1) {
                    Some(d) if d.saturated => Some(DepthMeasure {
                        depth: MAX_FIELD_TYPE_DEPTH + 1,
                        saturated: true,
                    }),
                    Some(d) => Some(DepthMeasure {
                        depth: d.depth + 1,
                        saturated: false,
                    }),
                    None => Some(DepthMeasure {
                        depth: 1,
                        saturated: false,
                    }),
                }
            }
            FieldType::Map(map) => {
                if ancestors > MAX_FIELD_TYPE_DEPTH + 1 {
                    return Some(DepthMeasure {
                        depth: MAX_FIELD_TYPE_DEPTH + 1,
                        saturated: true,
                    });
                }
                match walk(&map.value_type, ancestors + 1) {
                    Some(d) if d.saturated => Some(DepthMeasure {
                        depth: MAX_FIELD_TYPE_DEPTH + 1,
                        saturated: true,
                    }),
                    Some(d) => Some(DepthMeasure {
                        depth: d.depth + 1,
                        saturated: false,
                    }),
                    None => Some(DepthMeasure {
                        depth: 1,
                        saturated: false,
                    }),
                }
            }
            FieldType::Object(fields) => {
                if ancestors > MAX_FIELD_TYPE_DEPTH + 1 {
                    return Some(DepthMeasure {
                        depth: MAX_FIELD_TYPE_DEPTH + 1,
                        saturated: true,
                    });
                }
                let mut max = 0usize;
                let mut saturated = false;
                for m in fields.values().filter_map(|f| walk(f, ancestors + 1)) {
                    if m.saturated {
                        saturated = true;
                        break;
                    }
                    max = max.max(m.depth);
                }
                Some(DepthMeasure {
                    depth: if saturated {
                        MAX_FIELD_TYPE_DEPTH + 1
                    } else {
                        max + 1
                    },
                    saturated,
                })
            }
            FieldType::TaggedUnion(union) => {
                if ancestors > MAX_FIELD_TYPE_DEPTH + 1 {
                    return Some(DepthMeasure {
                        depth: MAX_FIELD_TYPE_DEPTH + 1,
                        saturated: true,
                    });
                }
                let mut max = 0usize;
                let mut saturated = false;
                for m in union
                    .branches
                    .values()
                    .flat_map(|branch| branch.values())
                    .filter_map(|f| walk(f, ancestors + 1))
                {
                    if m.saturated {
                        saturated = true;
                        break;
                    }
                    max = max.max(m.depth);
                }
                Some(DepthMeasure {
                    depth: if saturated {
                        MAX_FIELD_TYPE_DEPTH + 1
                    } else {
                        max + 1
                    },
                    saturated,
                })
            }
            _ => None,
        }
    }
    walk(t, 0)
}
