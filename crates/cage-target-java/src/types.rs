//! Java type mapping: `FieldType` to Java type text, wrapper widening and the imports a type pulls in (split out of lib.rs, LAY-008).

use cage_core::schema::{FieldType, MapKeyType};

/// Java type for a field type. `optional` is the optionality group rule
/// (¬required ∧ no renderable default): primitives widen to their wrapper
/// type, while references (String, arrays, maps, byte[], enums, Object) are
/// implicitly nullable and stay as-is. `enum_text` is the already-resolved
/// type text for an enum-typed field (`None` falls back to `String`).
pub(crate) fn java_type(ft: &FieldType, optional: bool, enum_text: Option<&str>) -> String {
    match ft {
        FieldType::Null | FieldType::Any => "Object".to_string(),
        FieldType::Bool => present_or(optional, "boolean", "Boolean"),
        // Unsigned integers widen one width (Java is signed-only) so the
        // full range of the schema kind stays representable:
        // UInt8→short, UInt16→int, UInt32→long, UInt64→long.
        FieldType::Int8 => present_or(optional, "byte", "Byte"),
        FieldType::UInt8 | FieldType::Int16 => present_or(optional, "short", "Short"),
        FieldType::UInt16 | FieldType::Int32 => present_or(optional, "int", "Integer"),
        FieldType::UInt32 | FieldType::Int64 | FieldType::UInt64 => {
            present_or(optional, "long", "Long")
        }
        FieldType::Float32 => present_or(optional, "float", "Float"),
        FieldType::Float64 => present_or(optional, "double", "Double"),
        FieldType::String => "String".to_string(),
        FieldType::Bytes => "byte[]".to_string(),
        FieldType::Array(inner) => {
            format!(
                "List<{}>",
                box_primitive(&java_type(inner, false, enum_text))
            )
        }
        // map<K, V> → HashMap<K, V>: string keys stay `String`, int keys
        // (i64 semantics) become `Long`; the value type recurses with its
        // primitives boxed, exactly like an array element position.
        FieldType::Map(map) => format!(
            "HashMap<{}, {}>",
            java_map_key(map.key_type),
            box_primitive(&java_type(&map.value_type, false, enum_text))
        ),
        // Tagged union renders as its runtime shape — a flat map with the
        // discriminant key (L2 validates the branch; a Java sealed-hierarchy
        // is a follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => "Map<String, Object>".to_string(),
        FieldType::Enum(_) => enum_text.unwrap_or("String").to_string(),
    }
}

/// Java spelling of a map key type: string keys stay `String`; int keys
/// carry i64 semantics, and generic contexts need the boxed `Long`.
pub(crate) fn java_map_key(key: MapKeyType) -> &'static str {
    match key {
        MapKeyType::String => "String",
        MapKeyType::Int => "Long",
    }
}

/// Present (group A) vs wrapper (group B) spelling of a primitive type.
pub(crate) fn present_or(optional: bool, primitive: &str, wrapper: &str) -> String {
    if optional {
        wrapper.to_string()
    } else {
        primitive.to_string()
    }
}

/// Box a primitive type name for generic contexts (`List<int>` is illegal
/// Java — array element types are always boxed).
pub(crate) fn box_primitive(ty: &str) -> String {
    match ty {
        "boolean" => "Boolean",
        "byte" => "Byte",
        "short" => "Short",
        "int" => "Integer",
        "long" => "Long",
        "float" => "Float",
        "double" => "Double",
        other => other,
    }
    .to_string()
}

/// Whether the rendered Java type mentions `List` at any depth — the import
/// scan walks the full nested type because a map value may be an array.
pub(crate) fn uses_list(ft: &FieldType) -> bool {
    match ft {
        FieldType::Array(_) => true,
        FieldType::Map(map) => uses_list(&map.value_type),
        _ => false,
    }
}

/// Whether the rendered Java type mentions the `Map` interface at any depth
/// (object-typed fields, including object values nested in arrays/maps).
pub(crate) fn uses_map_interface(ft: &FieldType) -> bool {
    match ft {
        FieldType::Object(_) => true,
        FieldType::Array(inner) => uses_map_interface(inner),
        FieldType::Map(map) => uses_map_interface(&map.value_type),
        _ => false,
    }
}

/// Whether the rendered Java type mentions `HashMap` at any depth (a map
/// field, or a map nested inside arrays/other maps).
pub(crate) fn uses_hash_map(ft: &FieldType) -> bool {
    match ft {
        FieldType::Map(_) => true,
        FieldType::Array(inner) => uses_hash_map(inner),
        _ => false,
    }
}

/// The single enum a field type references (top level, through arrays, or
/// through map value positions); `None` for every other kind. Object
/// properties are erased to `Object`, so only array element and map value
/// positions count as an enum use.
pub(crate) fn field_enum(ft: &FieldType) -> Option<&str> {
    match ft {
        FieldType::Enum(name) => Some(name),
        FieldType::Array(inner) => field_enum(inner),
        FieldType::Map(map) => field_enum(&map.value_type),
        _ => None,
    }
}
