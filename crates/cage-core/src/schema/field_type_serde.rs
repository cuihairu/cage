//! Hand-written `Deserialize` for [`FieldType`]: the [`FieldTypeDe`]
//! mirror plus the bounded buffered-content machinery that replaces
//! serde's unbounded private `Content` buffer (split out of mod.rs,
//! LAY-006).

use super::field_type::{FieldType, MapKeyType, MAX_FIELD_TYPE_DEPTH};
use indexmap::IndexMap;
use serde::de;
use serde::Deserialize;

/// Mirror of [`FieldType`] used only to drive the `Deserialize` parse.
///
/// `FieldType` cannot derive `Deserialize` directly because a type nested
/// past [`MAX_FIELD_TYPE_DEPTH`] must be refused, but serde's derive has no
/// depth knob. This mirror keeps the exact adjacently tagged parsing
/// semantics (including YAML `kind: Null` matching the `Null` variant) while
/// the conversion in [`FieldType::deserialize`] enforces the cap. The
/// `Deserialize` impl is hand-written ([`FieldTypeDeVisitor`]) so that when
/// the `value` content key precedes `kind`, the content is buffered with a
/// hard depth bound instead of serde's unbounded private `Content` buffer —
/// serde's derived adjacently tagged parser recurses the whole subtree into
/// `Content` before any [`BoundedFieldTypeDe`] check runs, which a deeply
/// nested `serde_json::Value` exploits to overflow the stack.
pub(crate) enum FieldTypeDe {
    Null,
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    String,
    Bytes,
    Array(Box<BoundedFieldTypeDe>),
    Object(IndexMap<String, BoundedFieldTypeDe>),
    TaggedUnion(Box<TaggedUnionDe>),
    Map(MapFieldDe),
    Enum(String),
    Any,
}

/// Wire variant names of [`FieldTypeDe`] in declaration order — the order
/// serde's derive uses for `unknown variant` diagnostics.
const FIELD_TYPE_VARIANTS: &[&str] = &[
    "Null",
    "Bool",
    "Int8",
    "Int16",
    "Int32",
    "Int64",
    "UInt8",
    "UInt16",
    "UInt32",
    "UInt64",
    "Float32",
    "Float64",
    "String",
    "Bytes",
    "Array",
    "Object",
    "TaggedUnion",
    "Map",
    "Enum",
    "Any",
];

/// Depth-bounded stand-in for serde's internal `Content`, used when the
/// `value` content key arrives before `kind` in the adjacently tagged wire
/// format. serde's derived `Deserialize` buffers such content into its
/// private `Content` enum with no depth limit — a 10,000-level borrowed
/// `serde_json::Value` overflows the stack before any [`BoundedFieldTypeDe`]
/// depth check runs. This buffer enforces a hard cap during buffering so an
/// adversarial input is refused instead of overflowing, then replays through
/// the same payload types, so parse semantics are identical to the tag-first
/// path.
enum BufContent {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Unit,
    Seq(Vec<BufContent>),
    Map(Vec<(BufContent, BufContent)>),
}

/// Hard cap on `BufContent` nesting. Every `FieldType` level costs at most a
/// handful of JSON wrapper levels (worst case 4 for `TaggedUnion`: the value
/// object, the branches map, the branch map and the field map), so a legal
/// type at [`MAX_FIELD_TYPE_DEPTH`] stays under ~257 levels and this cap
/// leaves ~2× headroom. The cap only trips on adversarial input and keeps
/// the buffering recursion well below a 1 MiB thread stack at the measured
/// ~0.9 KiB per level — deep enough for any legal schema yet far enough from
/// the guardrail that a 10,000-level input is refused instead of overflowing.
const BUF_CONTENT_DEPTH_CAP: usize = MAX_FIELD_TYPE_DEPTH * 8;

impl BufContent {
    fn unexpected(&self) -> de::Unexpected<'_> {
        match self {
            BufContent::Bool(v) => de::Unexpected::Bool(*v),
            BufContent::I64(v) => de::Unexpected::Signed(*v),
            BufContent::U64(v) => de::Unexpected::Unsigned(*v),
            BufContent::F64(v) => de::Unexpected::Float(*v),
            BufContent::Str(v) => de::Unexpected::Str(v),
            BufContent::Unit => de::Unexpected::Unit,
            BufContent::Seq(_) => de::Unexpected::Seq,
            BufContent::Map(_) => de::Unexpected::Map,
        }
    }
}

/// Seed that buffers one content value with the given depth, refusing to
/// descend past [`BUF_CONTENT_DEPTH_CAP`].
struct BufContentSeed {
    depth: usize,
}

impl<'de> de::DeserializeSeed<'de> for BufContentSeed {
    type Value = BufContent;
    fn deserialize<D>(self, deserializer: D) -> Result<BufContent, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(BufContentVisitor { depth: self.depth })
    }
}

struct BufContentVisitor {
    depth: usize,
}

impl<'de> de::Visitor<'de> for BufContentVisitor {
    type Value = BufContent;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any value")
    }
    fn visit_bool<E>(self, v: bool) -> Result<BufContent, E> {
        Ok(BufContent::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<BufContent, E> {
        Ok(BufContent::I64(v))
    }
    fn visit_u64<E>(self, v: u64) -> Result<BufContent, E> {
        Ok(BufContent::U64(v))
    }
    fn visit_f64<E>(self, v: f64) -> Result<BufContent, E> {
        Ok(BufContent::F64(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<BufContent, E> {
        Ok(BufContent::Str(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<BufContent, E> {
        Ok(BufContent::Str(v))
    }
    fn visit_unit<E>(self) -> Result<BufContent, E> {
        Ok(BufContent::Unit)
    }
    fn visit_none<E>(self) -> Result<BufContent, E> {
        Ok(BufContent::Unit)
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<BufContent, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        if self.depth >= BUF_CONTENT_DEPTH_CAP {
            return Err(de::Error::custom(
                "schema value exceeds the maximum nesting depth",
            ));
        }
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(BufContentSeed {
            depth: self.depth + 1,
        })? {
            items.push(item);
        }
        Ok(BufContent::Seq(items))
    }
    fn visit_map<A>(self, mut map: A) -> Result<BufContent, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        if self.depth >= BUF_CONTENT_DEPTH_CAP {
            return Err(de::Error::custom(
                "schema value exceeds the maximum nesting depth",
            ));
        }
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry_seed(
            BufContentSeed {
                depth: self.depth + 1,
            },
            BufContentSeed {
                depth: self.depth + 1,
            },
        )? {
            entries.push((key, value));
        }
        Ok(BufContent::Map(entries))
    }
}

/// Replays a buffered [`BufContent`] through the same payload types the
/// tag-first path uses, so both key orders share identical parse semantics.
struct BufContentDeserializer<'de, E> {
    content: BufContent,
    marker: std::marker::PhantomData<&'de E>,
}

impl<'de, E> BufContentDeserializer<'de, E>
where
    E: de::Error,
{
    fn new(content: BufContent) -> Self {
        BufContentDeserializer {
            content,
            marker: std::marker::PhantomData,
        }
    }
    #[cold]
    fn invalid_type(&self, exp: &dyn de::Expected) -> E {
        de::Error::invalid_type(self.content.unexpected(), exp)
    }
    fn deserialize_integer<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::I64(v) => visitor.visit_i64(v),
            BufContent::U64(v) => visitor.visit_u64(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_float<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::F64(v) => visitor.visit_f64(v),
            BufContent::I64(v) => visitor.visit_i64(v),
            BufContent::U64(v) => visitor.visit_u64(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
}

impl<'de, E> de::Deserializer<'de> for BufContentDeserializer<'de, E>
where
    E: de::Error,
{
    type Error = E;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Bool(v) => visitor.visit_bool(v),
            BufContent::I64(v) => visitor.visit_i64(v),
            BufContent::U64(v) => visitor.visit_u64(v),
            BufContent::F64(v) => visitor.visit_f64(v),
            BufContent::Str(v) => visitor.visit_string(v),
            BufContent::Unit => visitor.visit_unit(),
            BufContent::Seq(items) => visitor.visit_seq(BufSeqAccess {
                iter: items.into_iter(),
                marker: std::marker::PhantomData,
            }),
            BufContent::Map(entries) => visitor.visit_map(BufMapAccess {
                iter: entries.into_iter(),
                pending: None,
                marker: std::marker::PhantomData,
            }),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Bool(v) => visitor.visit_bool(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_integer(visitor)
    }
    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_float(visitor)
    }
    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_float(visitor)
    }
    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Str(v) => visitor.visit_string(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Str(v) => visitor.visit_string(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Str(v) => visitor.visit_string(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Str(v) => visitor.visit_byte_buf(v.into_bytes()),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Str(v) => visitor.visit_byte_buf(v.into_bytes()),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Unit => visitor.visit_unit(),
            _ => visitor.visit_some(self),
        }
    }
    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Unit => visitor.visit_unit(),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }
    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }
    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Seq(items) => visitor.visit_seq(BufSeqAccess {
                iter: items.into_iter(),
                marker: std::marker::PhantomData,
            }),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }
    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }
    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Map(entries) => visitor.visit_map(BufMapAccess {
                iter: entries.into_iter(),
                pending: None,
                marker: std::marker::PhantomData,
            }),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Map(entries) => visitor.visit_map(BufMapAccess {
                iter: entries.into_iter(),
                pending: None,
                marker: std::marker::PhantomData,
            }),
            BufContent::Seq(items) => visitor.visit_seq(BufSeqAccess {
                iter: items.into_iter(),
                marker: std::marker::PhantomData,
            }),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_enum<V>(
        self,
        _name: &str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        let (variant, value) = match self.content {
            BufContent::Map(entries) => {
                let mut iter = entries.into_iter();
                let Some((variant, value)) = iter.next() else {
                    return Err(de::Error::invalid_value(
                        de::Unexpected::Map,
                        &"map with a single key",
                    ));
                };
                if iter.next().is_some() {
                    return Err(de::Error::invalid_value(
                        de::Unexpected::Map,
                        &"map with a single key",
                    ));
                }
                (variant, Some(value))
            }
            s @ BufContent::Str(_) => (s, None),
            other => {
                return Err(de::Error::invalid_type(
                    other.unexpected(),
                    &"string or map",
                ));
            }
        };
        visitor.visit_enum(BufEnumAccess {
            variant,
            value,
            marker: std::marker::PhantomData,
        })
    }
    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.content {
            BufContent::Str(v) => visitor.visit_string(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        drop(self);
        visitor.visit_unit()
    }
}

struct BufSeqAccess<'de, E> {
    iter: std::vec::IntoIter<BufContent>,
    marker: std::marker::PhantomData<&'de E>,
}

impl<'de, E> de::SeqAccess<'de> for BufSeqAccess<'de, E>
where
    E: de::Error,
{
    type Error = E;
    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, E>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some(content) => Ok(Some(
                seed.deserialize(BufContentDeserializer::new(content))?,
            )),
            None => Ok(None),
        }
    }
}

struct BufMapAccess<'de, E> {
    iter: std::vec::IntoIter<(BufContent, BufContent)>,
    pending: Option<BufContent>,
    marker: std::marker::PhantomData<&'de E>,
}

impl<'de, E> de::MapAccess<'de> for BufMapAccess<'de, E>
where
    E: de::Error,
{
    type Error = E;
    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, E>
    where
        K: de::DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some((key, value)) => {
                let key = seed.deserialize(BufContentDeserializer::new(key))?;
                self.pending = Some(value);
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, E>
    where
        V: de::DeserializeSeed<'de>,
    {
        match self.pending.take() {
            Some(value) => seed.deserialize(BufContentDeserializer::new(value)),
            None => Err(de::Error::custom("map value missing after key")),
        }
    }
}

struct BufEnumAccess<'de, E> {
    variant: BufContent,
    value: Option<BufContent>,
    marker: std::marker::PhantomData<&'de E>,
}

impl<'de, E> de::EnumAccess<'de> for BufEnumAccess<'de, E>
where
    E: de::Error,
{
    type Error = E;
    type Variant = BufVariantAccess<'de, E>;
    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), E>
    where
        V: de::DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(BufContentDeserializer::new(self.variant))?;
        Ok((
            variant,
            BufVariantAccess {
                value: self.value,
                marker: std::marker::PhantomData,
            },
        ))
    }
}

struct BufVariantAccess<'de, E> {
    value: Option<BufContent>,
    marker: std::marker::PhantomData<&'de E>,
}

impl<'de, E> de::VariantAccess<'de> for BufVariantAccess<'de, E>
where
    E: de::Error,
{
    type Error = E;
    fn unit_variant(self) -> Result<(), E> {
        match self.value {
            Some(value) => <() as de::Deserialize>::deserialize(BufContentDeserializer::new(value)),
            None => Ok(()),
        }
    }
    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, E>
    where
        T: de::DeserializeSeed<'de>,
    {
        match self.value {
            Some(value) => seed.deserialize(BufContentDeserializer::new(value)),
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"newtype variant",
            )),
        }
    }
    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Some(value) => {
                de::Deserializer::deserialize_any(BufContentDeserializer::new(value), visitor)
            }
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"tuple variant",
            )),
        }
    }
    fn struct_variant<V>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, E>
    where
        V: de::Visitor<'de>,
    {
        match self.value {
            Some(value) => {
                de::Deserializer::deserialize_any(BufContentDeserializer::new(value), visitor)
            }
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"struct variant",
            )),
        }
    }
}

/// Accepts only null/unit content for a unit variant, mirroring serde's
/// `UntaggedUnitVisitor` used by the adjacently tagged seed path.
struct UnitContentVisitor;

impl de::Visitor<'_> for UnitContentVisitor {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("unit")
    }
    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }
}

fn is_unit_variant(name: &str) -> bool {
    matches!(
        name,
        "Null"
            | "Bool"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "Float32"
            | "Float64"
            | "String"
            | "Bytes"
            | "Any"
    )
}

fn unit_construct(name: &str) -> FieldTypeDe {
    match name {
        "Null" => FieldTypeDe::Null,
        "Bool" => FieldTypeDe::Bool,
        "Int8" => FieldTypeDe::Int8,
        "Int16" => FieldTypeDe::Int16,
        "Int32" => FieldTypeDe::Int32,
        "Int64" => FieldTypeDe::Int64,
        "UInt8" => FieldTypeDe::UInt8,
        "UInt16" => FieldTypeDe::UInt16,
        "UInt32" => FieldTypeDe::UInt32,
        "UInt64" => FieldTypeDe::UInt64,
        "Float32" => FieldTypeDe::Float32,
        "Float64" => FieldTypeDe::Float64,
        "String" => FieldTypeDe::String,
        "Bytes" => FieldTypeDe::Bytes,
        "Any" => FieldTypeDe::Any,
        _ => unreachable!("unit variant name checked by is_unit_variant"),
    }
}

/// Dispatches a buffered or direct deserializer to the payload type of the
/// named variant — the same payload types the derived adjacently tagged path
/// uses, so both key orders parse identically.
fn dispatch_variant<'de, D>(name: &str, deserializer: D) -> Result<FieldTypeDe, D::Error>
where
    D: de::Deserializer<'de>,
{
    match name {
        "Null" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Null)
        }
        "Bool" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Bool)
        }
        "Int8" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Int8)
        }
        "Int16" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Int16)
        }
        "Int32" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Int32)
        }
        "Int64" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Int64)
        }
        "UInt8" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::UInt8)
        }
        "UInt16" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::UInt16)
        }
        "UInt32" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::UInt32)
        }
        "UInt64" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::UInt64)
        }
        "Float32" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Float32)
        }
        "Float64" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Float64)
        }
        "String" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::String)
        }
        "Bytes" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Bytes)
        }
        "Array" => Ok(FieldTypeDe::Array(Box::new(
            BoundedFieldTypeDe::deserialize(deserializer)?,
        ))),
        "Object" => Ok(FieldTypeDe::Object(
            IndexMap::<String, BoundedFieldTypeDe>::deserialize(deserializer)?,
        )),
        "TaggedUnion" => Ok(FieldTypeDe::TaggedUnion(Box::new(
            TaggedUnionDe::deserialize(deserializer)?,
        ))),
        "Map" => Ok(FieldTypeDe::Map(MapFieldDe::deserialize(deserializer)?)),
        "Enum" => Ok(FieldTypeDe::Enum(String::deserialize(deserializer)?)),
        "Any" => {
            deserializer.deserialize_any(UnitContentVisitor)?;
            Ok(FieldTypeDe::Any)
        }
        other => Err(de::Error::unknown_variant(other, FIELD_TYPE_VARIANTS)),
    }
}

/// The seed used to map a `kind`/`value` map key, mirroring serde's
/// `TagContentOtherFieldVisitor` (unknown keys become `Other` and are
/// skipped).
struct FieldTypeVariantSeed {
    variant: String,
}

/// Reads the `kind` value (the variant name) and rejects an unknown variant
/// while the value is still being deserialized. serde's derive builds the
/// `unknown_variant` error inside `next_value_seed` for the tag, so
/// `serde_path_to_error` attributes it to the `kind` key; validating here
/// reproduces that path rather than pointing at the enclosing node.
struct VariantNameSeed;

impl<'de> de::DeserializeSeed<'de> for VariantNameSeed {
    type Value = String;
    fn deserialize<D>(self, deserializer: D) -> Result<String, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        if FIELD_TYPE_VARIANTS.contains(&name.as_str()) {
            Ok(name)
        } else {
            Err(de::Error::unknown_variant(&name, FIELD_TYPE_VARIANTS))
        }
    }
}

impl<'de> de::DeserializeSeed<'de> for FieldTypeVariantSeed {
    type Value = FieldTypeDe;
    fn deserialize<D>(self, deserializer: D) -> Result<FieldTypeDe, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        dispatch_variant(&self.variant, deserializer)
    }
}

enum TagKey {
    Kind,
    Value,
    Other,
}

struct TagKeySeed;

impl<'de> de::DeserializeSeed<'de> for TagKeySeed {
    type Value = TagKey;
    fn deserialize<D>(self, deserializer: D) -> Result<TagKey, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_identifier(TagKeyVisitor)
    }
}

struct TagKeyVisitor;

impl de::Visitor<'_> for TagKeyVisitor {
    type Value = TagKey;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("`kind` or `value`")
    }
    fn visit_u64<E>(self, value: u64) -> Result<TagKey, E>
    where
        E: de::Error,
    {
        match value {
            0 => Ok(TagKey::Kind),
            1 => Ok(TagKey::Value),
            _ => Ok(TagKey::Other),
        }
    }
    fn visit_str<E>(self, value: &str) -> Result<TagKey, E>
    where
        E: de::Error,
    {
        self.visit_bytes(value.as_bytes())
    }
    fn visit_bytes<E>(self, value: &[u8]) -> Result<TagKey, E>
    where
        E: de::Error,
    {
        match value {
            b"kind" => Ok(TagKey::Kind),
            b"value" => Ok(TagKey::Value),
            _ => Ok(TagKey::Other),
        }
    }
}

/// Hand-written adjacently tagged `Deserialize` for [`FieldTypeDe`]: accepts
/// `kind` and `value` in either order. When `value` precedes `kind` the
/// content is buffered with a hard depth bound ([`BufContent`]) instead of
/// serde's unbounded private buffer, then replayed through the same payload
/// types so both orders share identical semantics; when `kind` precedes
/// `value` the content is deserialized directly, so the
/// [`BoundedFieldTypeDe`] depth check fires on the live deserializer.
struct FieldTypeDeVisitor;

impl<'de> de::Visitor<'de> for FieldTypeDeVisitor {
    type Value = FieldTypeDe;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("adjacently tagged enum FieldTypeDe")
    }
    fn visit_map<A>(self, mut map: A) -> Result<FieldTypeDe, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut kind: Option<String> = None;
        let mut buffered: Option<BufContent> = None;
        let mut result: Option<FieldTypeDe> = None;
        while let Some(key) = map.next_key_seed(TagKeySeed)? {
            match key {
                TagKey::Other => {
                    map.next_value::<de::IgnoredAny>()?;
                }
                TagKey::Kind => {
                    if result.is_some() || kind.is_some() {
                        return Err(de::Error::duplicate_field("kind"));
                    }
                    let name: String = map.next_value_seed(VariantNameSeed)?;
                    if let Some(content) = buffered.take() {
                        result = Some(dispatch_variant(
                            &name,
                            BufContentDeserializer::<A::Error>::new(content),
                        )?);
                    }
                    kind = Some(name);
                }
                TagKey::Value => {
                    if result.is_some() || buffered.is_some() {
                        return Err(de::Error::duplicate_field("value"));
                    }
                    match &kind {
                        Some(name) => {
                            result = Some(map.next_value_seed(FieldTypeVariantSeed {
                                variant: name.clone(),
                            })?);
                        }
                        None => {
                            buffered = Some(map.next_value_seed(BufContentSeed { depth: 0 })?);
                        }
                    }
                }
            }
        }
        match (result, kind, buffered) {
            (Some(parsed), _, _) => Ok(parsed),
            (None, Some(name), Some(content)) => {
                dispatch_variant(&name, BufContentDeserializer::<A::Error>::new(content))
            }
            (None, Some(name), None) => {
                // `VariantNameSeed` already rejected unknown variants while
                // reading the tag, so `name` is a known variant here.
                if is_unit_variant(&name) {
                    Ok(unit_construct(&name))
                } else {
                    Err(de::Error::missing_field("value"))
                }
            }
            (None, None, _) => Err(de::Error::missing_field("kind")),
        }
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<FieldTypeDe, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        // Seq form mirrors the derive exactly: the tag is required
        // (`invalid_length(0)`), the content element is required even for
        // unit variants (`invalid_length(1)`), and unit content must be
        // unit-shaped (null) — `["Int32",true]` is refused, not ignored.
        // Extra elements are left to the format layer, which reports them
        // as trailing characters just like the derive does.
        let name: String = match seq.next_element_seed(VariantNameSeed)? {
            Some(name) => name,
            None => {
                return Err(de::Error::invalid_length(
                    0,
                    &"adjacently tagged enum FieldTypeDe",
                ))
            }
        };
        if is_unit_variant(&name) {
            match seq.next_element::<()>()? {
                Some(()) => Ok(unit_construct(&name)),
                None => Err(de::Error::invalid_length(
                    1,
                    &"adjacently tagged enum FieldTypeDe",
                )),
            }
        } else {
            match seq.next_element_seed(FieldTypeVariantSeed {
                variant: name.clone(),
            })? {
                Some(parsed) => Ok(parsed),
                None => Err(de::Error::invalid_length(
                    1,
                    &"adjacently tagged enum FieldTypeDe",
                )),
            }
        }
    }
}

impl<'de> de::Deserialize<'de> for FieldTypeDe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(FieldTypeDeVisitor)
    }
}

/// Deserializes one `FieldTypeDe` layer at a time, refusing descent once the
/// schema cap is reached. The derived mirror cannot express a depth-aware
/// seed, so the thread-local depth is bounded by `MAX_FIELD_TYPE_DEPTH` and
/// is always restored by `DepthGuard`, including error paths.
pub(crate) struct BoundedFieldTypeDe(pub(crate) FieldTypeDe);

thread_local! {
    static FIELD_TYPE_DESERIALIZE_DEPTH: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

struct DepthGuard;

impl Drop for DepthGuard {
    fn drop(&mut self) {
        FIELD_TYPE_DESERIALIZE_DEPTH.with(|depth| {
            depth.set(depth.get().saturating_sub(1));
        });
    }
}

impl<'de> serde::Deserialize<'de> for BoundedFieldTypeDe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        FIELD_TYPE_DESERIALIZE_DEPTH.with(|depth| {
            let current = depth.get();
            if current >= MAX_FIELD_TYPE_DEPTH {
                return Err(D::Error::custom(format!(
                    "field type exceeds the maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                )));
            }
            depth.set(current + 1);
            let _guard = DepthGuard;
            FieldTypeDe::deserialize(deserializer).map(Self)
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct TaggedUnionDe {
    pub(crate) tag: String,
    pub(crate) branches: IndexMap<String, IndexMap<String, BoundedFieldTypeDe>>,
}

#[derive(Deserialize)]
pub(crate) struct MapFieldDe {
    pub(crate) key_type: MapKeyType,
    pub(crate) value_type: Box<BoundedFieldTypeDe>,
}

impl<'de> serde::Deserialize<'de> for FieldType {
    /// Bounded `Deserialize`.
    ///
    /// The derived `Deserialize` recursively descends `Array` / `Map` /
    /// `Object` / `TaggedUnion` with no depth limit beyond serde's own ~128
    /// recursion guard, which is asymmetric with the iterative `Serialize`
    /// that refuses a type deeper than [`MAX_FIELD_TYPE_DEPTH`]. Parsing goes
    /// through `FieldTypeDe` (exact derived semantics) whose recursive
    /// children are wrapped in `BoundedFieldTypeDe`, refusing descent past
    /// [`MAX_FIELD_TYPE_DEPTH`] during the parse itself; the conversion
    /// enforces the same cap the serializer reports — so a value either
    /// round-trips or is refused on both sides, and a pathologically deep
    /// input is refused instead of overflowing.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        FIELD_TYPE_DESERIALIZE_DEPTH.with(|depth| depth.set(0));
        let de = FieldTypeDe::deserialize(deserializer)?;
        FieldType::from_de(de, 0).map_err(D::Error::custom)
    }
}

/// White-box coverage of the buffered-content replay layer. `FieldType`
/// payloads only ever ask the replay deserializer for `any` / `map` /
/// `struct` / `string`, but the `Deserializer` impl must provide the full
/// trait surface and keep serde's `ContentDeserializer` semantics; every
/// method below is driven against both a matching and a mismatched buffered
/// value so the replay cannot drift from serde's behavior.
#[cfg(test)]
mod buf_content_replay_tests {
    use super::*;
    use serde::de::{DeserializeSeed as _, MapAccess as _, Visitor as _};
    use serde::Deserialize;

    type Replay = BufContentDeserializer<'static, serde_json::Error>;

    fn replay(content: BufContent) -> Replay {
        BufContentDeserializer::new(content)
    }

    /// Accepts exactly one string-shaped call (str / string / char /
    /// identifier), whatever the replay hands it.
    struct TakeString;
    impl de::Visitor<'_> for TakeString {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_owned())
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
    }

    /// Accepts byte-shaped calls (`bytes` / `byte_buf`).
    struct TakeBytes;
    impl de::Visitor<'_> for TakeBytes {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bytes")
        }
        fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
            Ok(v)
        }
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct NewtypeCarrier(i64);

    #[derive(Deserialize, Debug, PartialEq)]
    struct TupleCarrier(i64, String);

    #[derive(Deserialize, Debug, PartialEq)]
    enum ReplayEnum {
        Unit,
        Wrapped(i64),
        Pair(i64, String),
        Record { x: i64 },
    }

    /// `deserialize_any` replays each buffered kind to the matching visit.
    #[test]
    fn any_replays_every_buffered_kind() {
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::Bool(true))).unwrap(),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::I64(-5))).unwrap(),
            serde_json::json!(-5)
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::U64(5))).unwrap(),
            serde_json::json!(5u64)
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::F64(1.5))).unwrap(),
            serde_json::json!(1.5)
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::Str("s".into()))).unwrap(),
            serde_json::Value::String("s".into())
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::Unit)).unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::Seq(vec![BufContent::I64(1)])))
                .unwrap(),
            serde_json::json!([1])
        );
        assert_eq!(
            serde_json::Value::deserialize(replay(BufContent::Map(vec![(
                BufContent::Str("k".into()),
                BufContent::I64(1),
            )])))
            .unwrap(),
            serde_json::json!({"k": 1})
        );
    }

    /// Scalars replay with serde `ContentDeserializer` type semantics.
    #[test]
    fn scalars_replay_like_serde_content() {
        assert!(bool::deserialize(replay(BufContent::Bool(true))).unwrap());
        let err = bool::deserialize::<Replay>(replay(BufContent::Str("s".into()))).unwrap_err();
        assert!(err.to_string().contains("invalid type: string"), "{err}");

        assert_eq!(i32::deserialize(replay(BufContent::I64(-7))).unwrap(), -7);
        assert_eq!(u32::deserialize(replay(BufContent::U64(7))).unwrap(), 7);
        assert_eq!(i64::deserialize(replay(BufContent::U64(7))).unwrap(), 7);
        let err = i32::deserialize::<Replay>(replay(BufContent::Bool(true))).unwrap_err();
        assert!(
            err.to_string().contains("invalid type: boolean `true`"),
            "{err}"
        );

        assert_eq!(f32::deserialize(replay(BufContent::F64(1.5))).unwrap(), 1.5);
        assert_eq!(f32::deserialize(replay(BufContent::I64(2))).unwrap(), 2.0);
        assert_eq!(f32::deserialize(replay(BufContent::U64(2))).unwrap(), 2.0);
        let err = f32::deserialize::<Replay>(replay(BufContent::Str("s".into()))).unwrap_err();
        assert!(err.to_string().contains("invalid type: string"), "{err}");

        assert_eq!(
            char::deserialize(replay(BufContent::Str("x".into()))).unwrap(),
            'x'
        );
        let err = char::deserialize::<Replay>(replay(BufContent::Unit)).unwrap_err();
        assert!(err.to_string().contains("invalid type: null"), "{err}");

        assert_eq!(
            String::deserialize(replay(BufContent::Str("s".into()))).unwrap(),
            "s"
        );
        let err = String::deserialize::<Replay>(replay(BufContent::I64(1))).unwrap_err();
        assert!(
            err.to_string().contains("invalid type: integer `1`"),
            "{err}"
        );

        assert_eq!(
            de::Deserializer::deserialize_str(replay(BufContent::Str("s".into())), TakeString)
                .unwrap(),
            "s"
        );
        assert!(de::Deserializer::deserialize_str::<TakeString>(
            replay(BufContent::Unit),
            TakeString
        )
        .is_err());
        assert_eq!(
            de::Deserializer::deserialize_char(replay(BufContent::Str("x".into())), TakeString)
                .unwrap(),
            "x"
        );
        assert_eq!(
            de::Deserializer::deserialize_identifier(
                replay(BufContent::Str("k".into())),
                TakeString
            )
            .unwrap(),
            "k"
        );
        assert!(de::Deserializer::deserialize_identifier::<TakeString>(
            replay(BufContent::I64(1)),
            TakeString
        )
        .is_err());

        assert_eq!(
            de::Deserializer::deserialize_bytes(replay(BufContent::Str("ab".into())), TakeBytes)
                .unwrap(),
            b"ab".to_vec()
        );
        assert_eq!(
            de::Deserializer::deserialize_byte_buf(replay(BufContent::Str("ab".into())), TakeBytes)
                .unwrap(),
            b"ab".to_vec()
        );
        assert!(de::Deserializer::deserialize_bytes::<TakeBytes>(
            replay(BufContent::I64(1)),
            TakeBytes
        )
        .is_err());
    }

    /// Option and unit replay like serde's content deserializer.
    #[test]
    fn option_and_unit_replay_like_serde_content() {
        assert_eq!(
            Option::<i32>::deserialize(replay(BufContent::Unit)).unwrap(),
            None
        );
        assert_eq!(
            Option::<String>::deserialize(replay(BufContent::Str("s".into()))).unwrap(),
            Some("s".into())
        );
        let err =
            Option::<i32>::deserialize::<Replay>(replay(BufContent::Str("s".into()))).unwrap_err();
        assert!(err.to_string().contains("invalid type: string"), "{err}");

        assert_eq!(<()>::deserialize(replay(BufContent::Unit)).unwrap(), ());
        assert!(<()>::deserialize::<Replay>(replay(BufContent::F64(1.0))).is_err());

        de::Deserializer::deserialize_unit_struct(
            replay(BufContent::Unit),
            "UnitCarrier",
            UnitContentVisitor,
        )
        .unwrap();
        assert!(
            de::Deserializer::deserialize_unit_struct::<UnitContentVisitor>(
                replay(BufContent::Bool(true)),
                "UnitCarrier",
                UnitContentVisitor,
            )
            .is_err()
        );
    }

    /// Containers replay like serde's content deserializer, including the
    /// struct-from-positional-seq fallback and the mismatch refusals.
    #[test]
    fn containers_replay_like_serde_content() {
        assert_eq!(
            NewtypeCarrier::deserialize(replay(BufContent::I64(5)))
                .unwrap()
                .0,
            5
        );
        assert_eq!(
            Vec::<i64>::deserialize(replay(BufContent::Seq(vec![
                BufContent::I64(1),
                BufContent::I64(2),
            ])))
            .unwrap(),
            vec![1, 2]
        );
        assert!(Vec::<i64>::deserialize::<Replay>(replay(BufContent::I64(1))).is_err());
        assert_eq!(
            <(i64, String)>::deserialize(replay(BufContent::Seq(vec![
                BufContent::I64(1),
                BufContent::Str("x".into()),
            ])))
            .unwrap(),
            (1, "x".to_string())
        );
        assert_eq!(
            TupleCarrier::deserialize(replay(BufContent::Seq(vec![
                BufContent::I64(1),
                BufContent::Str("x".into()),
            ])))
            .unwrap()
            .0,
            1
        );

        let map: std::collections::HashMap<String, i64> = Deserialize::deserialize(replay(
            BufContent::Map(vec![(BufContent::Str("k".into()), BufContent::I64(1))]),
        ))
        .unwrap();
        assert_eq!(map["k"], 1);
        assert!(
            std::collections::HashMap::<String, i64>::deserialize::<Replay>(replay(
                BufContent::Seq(vec![BufContent::I64(1)])
            ),)
            .is_err()
        );

        let value_leaf = || {
            BufContent::Map(vec![(
                BufContent::Str("kind".into()),
                BufContent::Str("Int32".into()),
            )])
        };
        let from_map = MapFieldDe::deserialize(replay(BufContent::Map(vec![
            (
                BufContent::Str("key_type".into()),
                BufContent::Str("string".into()),
            ),
            (BufContent::Str("value_type".into()), value_leaf()),
        ])))
        .unwrap();
        assert_eq!(from_map.key_type, MapKeyType::String);
        let from_seq = MapFieldDe::deserialize(replay(BufContent::Seq(vec![
            BufContent::Str("string".into()),
            value_leaf(),
        ])))
        .unwrap();
        assert_eq!(from_seq.key_type, MapKeyType::String);
        assert!(MapFieldDe::deserialize::<Replay>(replay(BufContent::I64(1))).is_err());
    }

    /// Enum replay mirrors serde's content enum: bare strings for unit
    /// variants, single-key maps for payloads, everything else refused.
    #[test]
    fn enum_replay_mirrors_serde_content_enum() {
        use self::ReplayEnum as E;
        assert_eq!(
            E::deserialize(replay(BufContent::Str("Unit".into()))).unwrap(),
            E::Unit
        );
        assert_eq!(
            E::deserialize(replay(BufContent::Map(vec![(
                BufContent::Str("Unit".into()),
                BufContent::Unit,
            )])))
            .unwrap(),
            E::Unit
        );
        assert_eq!(
            E::deserialize(replay(BufContent::Map(vec![(
                BufContent::Str("Wrapped".into()),
                BufContent::I64(9),
            )])))
            .unwrap(),
            E::Wrapped(9)
        );
        let err = E::deserialize::<Replay>(replay(BufContent::Str("Wrapped".into()))).unwrap_err();
        assert!(
            err.to_string().contains("invalid type: unit variant"),
            "{err}"
        );

        assert_eq!(
            E::deserialize(replay(BufContent::Map(vec![(
                BufContent::Str("Pair".into()),
                BufContent::Seq(vec![BufContent::I64(1), BufContent::Str("x".into())]),
            )])))
            .unwrap(),
            E::Pair(1, "x".into())
        );
        assert!(E::deserialize::<Replay>(replay(BufContent::Str("Pair".into()))).is_err());

        assert_eq!(
            E::deserialize(replay(BufContent::Map(vec![(
                BufContent::Str("Record".into()),
                BufContent::Map(vec![(BufContent::Str("x".into()), BufContent::I64(1))]),
            )])))
            .unwrap(),
            E::Record { x: 1 }
        );
        assert!(E::deserialize::<Replay>(replay(BufContent::Str("Record".into()))).is_err());

        let err = E::deserialize::<Replay>(replay(BufContent::U64(5))).unwrap_err();
        assert!(
            err.to_string().contains("invalid type: integer `5`"),
            "{err}"
        );
        assert!(E::deserialize::<Replay>(replay(BufContent::Map(vec![]))).is_err());
        assert!(E::deserialize::<Replay>(replay(BufContent::Map(vec![
            (BufContent::Str("a".into()), BufContent::Unit),
            (BufContent::Str("b".into()), BufContent::Unit),
        ])))
        .is_err());
        assert!(E::deserialize::<Replay>(replay(BufContent::Map(vec![(
            BufContent::I64(1),
            BufContent::Unit,
        )])))
        .is_err());
        assert!(de::IgnoredAny::deserialize::<Replay>(replay(BufContent::F64(1.0))).is_ok());
    }

    /// The buffering visitor itself: owned-string / none arms, and the seq
    /// depth cap (`serde_json`'s own recursion limit is disabled so this test
    /// targets the guardrail, not `serde_json`'s).
    #[test]
    fn buffering_visitor_accepts_none_and_refuses_over_cap_seqs() {
        assert!(matches!(
            BufContentVisitor { depth: 0 }
                .visit_none::<serde_json::Error>()
                .unwrap(),
            BufContent::Unit
        ));
        assert!(matches!(
            BufContentVisitor { depth: 0 }
                .visit_string::<serde_json::Error>("s".to_owned())
                .unwrap(),
            BufContent::Str(_)
        ));

        let mut value = serde_json::Value::Null;
        for _ in 0..(BUF_CONTENT_DEPTH_CAP + 8) {
            value = serde_json::Value::Array(vec![value]);
        }
        let err = BufContentSeed { depth: 0 }
            .deserialize(&value)
            .err()
            .expect("over-cap seq must be refused");
        assert!(err.to_string().contains("nesting depth"), "{err}");
    }

    /// Replay-side plumbing: unknown payload names, the tag-key wire forms,
    /// and the map value without a key.
    #[test]
    fn replay_plumbing_reports_unknown_names_and_missing_values() {
        let err = dispatch_variant("Quadruple", &serde_json::Value::Null)
            .err()
            .expect("unknown payload name must fail");
        assert!(
            err.to_string().contains("unknown variant `Quadruple`"),
            "{err}"
        );

        assert!(matches!(
            TagKeyVisitor.visit_u64::<serde_json::Error>(0).unwrap(),
            TagKey::Kind
        ));
        assert!(matches!(
            TagKeyVisitor.visit_u64::<serde_json::Error>(1).unwrap(),
            TagKey::Value
        ));
        assert!(matches!(
            TagKeyVisitor.visit_u64::<serde_json::Error>(7).unwrap(),
            TagKey::Other
        ));
        assert!(matches!(
            TagKeyVisitor
                .visit_bytes::<serde_json::Error>(b"kind")
                .unwrap(),
            TagKey::Kind
        ));
        assert!(matches!(
            TagKeyVisitor
                .visit_bytes::<serde_json::Error>(b"other")
                .unwrap(),
            TagKey::Other
        ));

        let mut access = BufMapAccess::<serde_json::Error> {
            iter: Vec::new().into_iter(),
            pending: None,
            marker: std::marker::PhantomData,
        };
        let err = access
            .next_value_seed(BufContentSeed { depth: 0 })
            .err()
            .expect("no pending value must fail");
        assert!(
            err.to_string().contains("map value missing after key"),
            "{err}"
        );
    }

    /// The thin integer / float / char wrappers delegate to the same shared
    /// replay arms serde's `ContentDeserializer` uses, so a buffered scalar
    /// replays through any of them; each wrapper is driven once with matching
    /// content, and a buffered map refused by a scalar wrapper renders the
    /// same "map" unexpected shape as serde's error.
    #[test]
    fn scalar_wrappers_replay_like_serde_content_deserializers() {
        let neg = || replay(BufContent::I64(-1));
        assert_eq!(i8::deserialize(neg()).unwrap(), -1);
        assert_eq!(i16::deserialize(neg()).unwrap(), -1);
        assert_eq!(i32::deserialize(neg()).unwrap(), -1);
        assert_eq!(i64::deserialize(neg()).unwrap(), -1);
        let pos = || replay(BufContent::U64(7));
        assert_eq!(u8::deserialize(pos()).unwrap(), 7);
        assert_eq!(u16::deserialize(pos()).unwrap(), 7);
        assert_eq!(u32::deserialize(pos()).unwrap(), 7);
        assert_eq!(u64::deserialize(pos()).unwrap(), 7);
        let frac = || replay(BufContent::F64(0.5));
        assert_eq!(f32::deserialize(frac()).unwrap(), 0.5);
        assert_eq!(f64::deserialize(frac()).unwrap(), 0.5);
        assert_eq!(
            char::deserialize(replay(BufContent::Str("x".into()))).unwrap(),
            'x'
        );

        let err = u8::deserialize(replay(BufContent::Map(vec![(
            BufContent::Str("k".into()),
            BufContent::Bool(true),
        )])))
        .expect_err("a buffered map must refuse as an integer");
        assert!(err.to_string().contains("map"), "{err}");
    }

    /// Arms only reachable through direct calls: the buffering visitor's
    /// `expecting` (used when an `invalid_type` message is rendered), its
    /// `visit_i64` (`serde_json` produces `visit_u64` for positive integers),
    /// the unit visitor's `visit_none`, the string visitor's borrowed arm,
    /// and the tag-key visitor's `expecting`.
    #[test]
    fn direct_visitor_arms_behave_like_their_serde_counterparts() {
        let msg = <serde_json::Error as de::Error>::invalid_type(
            de::Unexpected::Bool(true),
            &BufContentVisitor { depth: 0 },
        )
        .to_string();
        assert!(msg.contains("any value"), "{msg}");
        assert!(matches!(
            BufContentVisitor { depth: 0 }
                .visit_i64::<serde_json::Error>(-5)
                .unwrap(),
            BufContent::I64(-5)
        ));
        assert!(UnitContentVisitor.visit_none::<serde_json::Error>().is_ok());
        assert_eq!(
            TakeString
                .visit_str::<serde_json::Error>("borrowed")
                .unwrap(),
            "borrowed"
        );
        let msg = <serde_json::Error as de::Error>::invalid_type(
            de::Unexpected::Bool(true),
            &TagKeyVisitor,
        )
        .to_string();
        assert!(msg.contains("`kind` or `value`"), "{msg}");
    }

    /// The constructor is only ever called with names that passed
    /// `is_unit_variant`; any other name is a programming error.
    #[test]
    #[should_panic(expected = "unit variant name checked by is_unit_variant")]
    fn unit_construct_refuses_names_that_is_unit_variant_guards() {
        unit_construct("Map");
    }
}
