// 常量：19 种字段类型 + 各实体已知键（split out of app.js, LAY-027）。

// ── 常量：19 种字段类型 + 各实体已知键 ──────────────────────────────

const KINDS = [
  "Null", "Bool",
  "Int8", "Int16", "Int32", "Int64",
  "UInt8", "UInt16", "UInt32", "UInt64",
  "Float32", "Float64",
  "String", "Bytes",
  "Array", "Object", "Map", "Enum", "Any",
];

const NUMERIC_KINDS = KINDS.slice(2, 14); // Int8..Float64
const STRING_KIND = "String";
const MAX_DEPTH = 5;

// 实体对象的已知键（用于自定义元数据识别 & 表单渲染的键序）
const TABLE_KEYS = ["name", "description", "primary_key", "fields", "unique_constraints", "order_by", "targets"];
const FIELD_KEYS = [
  "name", "type", "description", "required", "default",
  "min", "max", "min_length", "max_length", "pattern", "enum_values",
  "min_items", "max_items", "reference", "targets", "rules",
  // 遗留（保留不编辑）：items, properties, additional_properties
];

export { KINDS, NUMERIC_KINDS, STRING_KIND, MAX_DEPTH, TABLE_KEYS, FIELD_KEYS };
