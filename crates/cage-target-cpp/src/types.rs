//! C++ type mapping: `FieldType` to type text, include needs and enum allocations (split out of lib.rs, LAY-011).

use cage_core::schema::{FieldType, MapKeyType};
use std::collections::HashMap;

use super::{EnumAlloc, Needs};
pub(crate) fn cpp_type(
    ft: &FieldType,
    enums: &HashMap<&str, EnumAlloc>,
    needs: &mut Needs,
) -> String {
    match ft {
        FieldType::Null | FieldType::Any => {
            needs.angle.insert("any");
            "std::any".to_string()
        }
        FieldType::Bool => "bool".to_string(),
        FieldType::Int8 => {
            needs.angle.insert("cstdint");
            "std::int8_t".to_string()
        }
        FieldType::Int16 => {
            needs.angle.insert("cstdint");
            "std::int16_t".to_string()
        }
        FieldType::Int32 => {
            needs.angle.insert("cstdint");
            "std::int32_t".to_string()
        }
        FieldType::Int64 => {
            needs.angle.insert("cstdint");
            "std::int64_t".to_string()
        }
        FieldType::UInt8 => {
            needs.angle.insert("cstdint");
            "std::uint8_t".to_string()
        }
        FieldType::UInt16 => {
            needs.angle.insert("cstdint");
            "std::uint16_t".to_string()
        }
        FieldType::UInt32 => {
            needs.angle.insert("cstdint");
            "std::uint32_t".to_string()
        }
        FieldType::UInt64 => {
            needs.angle.insert("cstdint");
            "std::uint64_t".to_string()
        }
        FieldType::Float32 => "float".to_string(),
        FieldType::Float64 => "double".to_string(),
        FieldType::String => {
            needs.angle.insert("string");
            "std::string".to_string()
        }
        FieldType::Bytes => {
            needs.angle.insert("vector");
            needs.angle.insert("cstdint");
            "std::vector<std::uint8_t>".to_string()
        }
        FieldType::Array(inner) => {
            needs.angle.insert("vector");
            let item = cpp_type(inner, enums, needs);
            format!("std::vector<{item}>")
        }
        // Tagged union renders as its runtime shape — a flat map with the
        // discriminant key (L2 validates the branch; std::variant is a
        // follow-up).
        FieldType::Object(_) | FieldType::TaggedUnion(_) => {
            needs.angle.insert("map");
            needs.angle.insert("string");
            needs.angle.insert("any");
            "std::map<std::string, std::any>".to_string()
        }
        FieldType::Map(map) => {
            // Homogeneous `map<K, V>` → unordered_map; the key type decides
            // its own include (string keys → <string>, int keys → <cstdint>),
            // the value type recurses through this same mapping.
            needs.angle.insert("unordered_map");
            let key = match map.key_type {
                MapKeyType::String => {
                    needs.angle.insert("string");
                    "std::string".to_string()
                }
                MapKeyType::Int => {
                    needs.angle.insert("cstdint");
                    "std::int64_t".to_string()
                }
            };
            let value = cpp_type(&map.value_type, enums, needs);
            format!("std::unordered_map<{key}, {value}>")
        }
        FieldType::Enum(name) => {
            let Some(alloc) = enums.get(name.as_str()) else {
                // Unresolved (or empty) enum: fall back to plain string.
                needs.angle.insert("string");
                return "std::string".to_string();
            };
            needs.enums_header = true;
            if alloc.integral {
                needs.angle.insert("cstdint");
                alloc.ident.clone()
            } else {
                // A string-bucket enum is a namespace, not a type; its
                // constants are `std::string_view`, so fields hold views.
                needs.angle.insert("string_view");
                "std::string_view".to_string()
            }
        }
    }
}
