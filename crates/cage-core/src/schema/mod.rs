//! Schema definition - decoupled from Source, defines structure and constraints

use crate::value::SourceLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Root schema document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Table schemas by name
    pub tables: IndexMap<String, TableSchema>,
    /// Shared enum definitions
    pub enums: IndexMap<String, EnumSchema>,
    /// Schema-level metadata
    pub metadata: Option<SchemaMetadata>,
}

/// Metadata about the schema itself
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaMetadata {
    /// Schema version string
    pub version: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Author name
    pub author: Option<String>,
}

impl Schema {
    /// Every environment name declared by any table's `env_overrides`,
    /// sorted — the valid `--env` inputs for this schema.
    pub fn declared_envs(&self) -> std::collections::BTreeSet<String> {
        self.tables
            .values()
            .flat_map(|table| table.env_overrides.keys().cloned())
            .collect()
    }

    /// Structural check of every table's `env_overrides`: each override
    /// must name a field that exists in its table and set at least one
    /// constraint key. Errors carry the table/env/field path — the CLI
    /// renders them as plain load errors, before any `--env` is chosen.
    pub fn validate_env_overrides(&self) -> Result<(), String> {
        match self.lint_env_overrides().into_iter().next() {
            Some(problem) => Err(problem),
            None => Ok(()),
        }
    }

    /// Every `env_overrides` structural problem, all of them, not just the
    /// first — the base lane's lint. A schema with a broken override in an
    /// environment nobody selected would otherwise validate clean until
    /// someone runs `--env` on it; the CLI prints these as warnings on the
    /// base path (no `--env`) and hard-errors on the `--env` path.
    pub fn lint_env_overrides(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (table_name, table) in &self.tables {
            for (env, overrides) in &table.env_overrides {
                for (field_name, patch) in overrides {
                    if !table.fields.contains_key(field_name) {
                        problems.push(format!(
                            "env_overrides[{env}] names unknown field '{field_name}' in table \
                             '{table_name}'"
                        ));
                    }
                    if *patch == FieldOverride::default() {
                        problems.push(format!(
                            "env_overrides[{env}].{field_name} in table '{table_name}' sets no \
                             constraint"
                        ));
                    }
                }
            }
        }
        problems
    }

    /// The schema as it validates under `env`: every `env_overrides[env]`
    /// patch applied onto the base fields (later keys in one patch win —
    /// an override is a whole-constraint replacement, not a merge). The
    /// base schema is never mutated; an environment that declares no
    /// overrides for a table leaves that table untouched. The result is
    /// what every downstream stage — validation, codegen, hashing —
    /// consumes, so `--env` changes exactly the rules those stages see.
    pub fn resolve_env(&self, env: &str) -> Result<Schema, String> {
        let mut resolved = self.clone();
        for (table_name, table) in &mut resolved.tables {
            let Some(overrides) = table.env_overrides.get(env) else {
                continue;
            };
            for (field_name, patch) in overrides {
                let Some(field) = table.fields.get_mut(field_name) else {
                    return Err(format!(
                        "env_overrides[{env}] names unknown field '{field_name}' in table \
                         '{table_name}'"
                    ));
                };
                if let Some(v) = patch.required {
                    field.required = v;
                }
                if let Some(v) = patch.min {
                    field.min = Some(v);
                }
                if let Some(v) = patch.max {
                    field.max = Some(v);
                }
                if let Some(v) = patch.min_length {
                    field.min_length = Some(v);
                }
                if let Some(v) = patch.max_length {
                    field.max_length = Some(v);
                }
                if let Some(v) = patch.pattern.clone() {
                    field.pattern = Some(v);
                }
                if let Some(v) = patch.enum_values.clone() {
                    field.enum_values = Some(v);
                }
                if let Some(v) = patch.min_items {
                    field.min_items = Some(v);
                }
                if let Some(v) = patch.max_items {
                    field.max_items = Some(v);
                }
            }
        }
        Ok(resolved)
    }
}

/// Table schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    /// Table name
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Primary key field(s)
    pub primary_key: Vec<String>,
    /// Field definitions
    pub fields: IndexMap<String, FieldSchema>,
    /// Composite unique constraints
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub unique_constraints: Vec<UniqueConstraint>,
    /// Row ordering requirement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<Vec<String>>,
    /// Target profiles this table applies to
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub targets: Vec<String>,
    /// Per-environment constraint overrides (`--env` validation, design
    /// §48): environment name → field name → constraint deltas. Applied
    /// by [`Schema::resolve_env`] before validation; the base fields
    /// stay untouched.
    #[serde(skip_serializing_if = "IndexMap::is_empty", default)]
    pub env_overrides: IndexMap<String, IndexMap<String, FieldOverride>>,
}

/// One environment's constraint deltas for one field (design §48): a
/// partial field patch — only the constraint keys an environment may
/// tighten or loosen. Keys absent here keep the base value. Types stay
/// out on purpose: an environment never re-shapes a field, it re-tunes
/// how strictly the same shape is checked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldOverride {
    /// Override `required`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Override `min`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Override `max`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Override `min_length`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    /// Override `max_length`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// Override `pattern`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Override `enum_values`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// Override `min_items`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,
    /// Override `max_items`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
}

/// Unique constraint on multiple fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniqueConstraint {
    /// Constraint name
    pub name: String,
    /// Field names forming the unique key
    pub fields: Vec<String>,
}

/// Field schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Field name
    pub name: String,
    /// Field type
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Human-readable description
    pub description: Option<String>,
    /// Whether the field must be present
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// Whether a present Null (e.g. an Excel empty cell) is a valid value
    /// (回流-2): the nullable equivalent of Luban's `string?` / `int?` — the
    /// type check accepts Null without weakening the field to `Any`. A
    /// nullable reference field should also use `cardinality: optional`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub nullable: bool,
    /// Default value applied when absent
    pub default: Option<serde_json::Value>,
    // Numeric constraints
    /// Minimum numeric value (inclusive)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum numeric value (inclusive)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    // String constraints
    /// Minimum string length
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    /// Maximum string length
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// Regular expression the string must match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    // Enum constraint
    /// Allowed values (inline enum)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    // Array constraints
    /// Minimum number of items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,
    /// Maximum number of items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
    /// Element schema for arrays
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<FieldSchema>>,
    /// Split a scalar table cell (Excel/CSV cannot hold arrays) into the
    /// field's array elements on this separator (回流-3; the equivalent of
    /// Luban's `(list#sep=,)`). Each token is trimmed and coerced to the
    /// element type; a cell that fails to split cleanly stays as-is and the
    /// type check reports it. Only meaningful on `Array` fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
    // Object constraints
    /// Property schemas for objects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<IndexMap<String, FieldSchema>>,
    /// Whether keys outside `properties` are allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_properties: Option<bool>,
    // Reference constraint
    /// Cross-table reference definition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceSchema>,
    /// Target profiles this field appears in
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub targets: Vec<String>,
    /// Semantic validation rules (expressions)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rules: Vec<ExpressionRule>,
    /// Custom metadata
    #[serde(flatten)]
    pub metadata: IndexMap<String, serde_json::Value>,
}

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

/// Mirror of [`FieldType`] used only to drive the derived `Deserialize`.
///
/// `FieldType` cannot derive `Deserialize` directly because a type nested
/// past [`MAX_FIELD_TYPE_DEPTH`] must be refused, but serde's derive has no
/// depth knob. This mirror keeps the exact derived parsing semantics
/// (including YAML `kind: Null` matching the `Null` variant) while the
/// conversion in [`FieldType::deserialize`] enforces the cap.
#[derive(Deserialize)]
#[serde(tag = "kind", content = "value")]
enum FieldTypeDe {
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

/// Deserializes one `FieldTypeDe` layer at a time, refusing descent once the
/// schema cap is reached. The derived mirror cannot express a depth-aware
/// seed, so the thread-local depth is bounded by `MAX_FIELD_TYPE_DEPTH` and
/// is always restored by `DepthGuard`, including error paths.
struct BoundedFieldTypeDe(FieldTypeDe);

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
struct TaggedUnionDe {
    tag: String,
    branches: IndexMap<String, IndexMap<String, BoundedFieldTypeDe>>,
}

#[derive(Deserialize)]
struct MapFieldDe {
    key_type: MapKeyType,
    value_type: Box<BoundedFieldTypeDe>,
}

impl<'de> serde::Deserialize<'de> for FieldType {
    /// Bounded `Deserialize`.
    ///
    /// The derived `Deserialize` recursively descends `Array` / `Map` /
    /// `Object` / `TaggedUnion` with no depth limit beyond serde's own ~128
    /// recursion guard, which is asymmetric with the iterative `Serialize`
    /// that refuses a type deeper than [`MAX_FIELD_TYPE_DEPTH`]. Parsing goes
    /// through [`FieldTypeDe`] (exact derived semantics) whose recursive
    /// children are wrapped in [`BoundedFieldTypeDe`], refusing descent past
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

impl FieldType {
    /// Convert a parsed [`FieldTypeDe`] into a [`FieldType`], enforcing the
    /// nesting cap. The recursion is bounded by [`MAX_FIELD_TYPE_DEPTH`]: a
    /// node past the cap returns an error before descending further.
    fn from_de(value: FieldTypeDe, depth: usize) -> Result<Self, String> {
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

fn scalar_wire(t: &FieldType) -> serde_json::Value {
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

/// Reference to another table/field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceSchema {
    /// Referenced table name
    pub table: String,
    /// Referenced field name
    pub field: String,
    /// Additional predicate: the referenced object must satisfy this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ExpressionRule>,
    /// Cardinality: "one" | "many" | "optional"
    #[serde(default = "default_cardinality", skip_serializing_if = "is_one")]
    pub cardinality: String,
    /// Compatibility check (for E1411)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_with: Option<Vec<String>>,
}

fn default_cardinality() -> String {
    "one".to_string()
}

fn is_one(s: &str) -> bool {
    s == "one"
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Expression rule for semantic validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionRule {
    /// Rule name
    pub name: String,
    /// Expression like "`min_level` <= `max_level`"
    pub assert: String,
    /// Custom diagnostic message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Whether a violation is reported as a warning
    #[serde(default, skip_serializing_if = "is_false")]
    pub warning_only: bool,
}

/// Enum schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumSchema {
    /// Enum name
    pub name: String,
    /// Allowed values
    pub values: Vec<EnumValue>,
    /// Human-readable description
    pub description: Option<String>,
}

/// One value of an enum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumValue {
    /// Value name
    pub name: String,
    /// Optional literal backing value
    pub value: Option<serde_json::Value>,
    /// Human-readable description
    pub description: Option<String>,
}

/// Validation context for error reporting
#[derive(Debug, Clone)]
pub struct ValidationContext<'a> {
    /// Schema being validated against
    pub schema: &'a Schema,
    /// Table currently being validated
    pub current_table: Option<&'a str>,
    /// Row currently being validated (0-based)
    pub current_row: Option<usize>,
    /// Field currently being validated
    pub current_field: Option<&'a str>,
    /// Source location of the value under validation
    pub source_location: Option<SourceLocation>,
}

/// Validated schema with resolved references
#[derive(Debug, Clone)]
pub struct ValidatedSchema {
    /// The validated schema
    pub schema: Schema,
    /// Table dependency graph (for build order)
    pub dependency_graph: crate::reference::DependencyGraph,
}

impl Schema {
    /// Create an empty schema
    pub fn new() -> Self {
        Self {
            tables: IndexMap::new(),
            enums: IndexMap::new(),
            metadata: None,
        }
    }

    /// Add (or replace) a table schema
    pub fn add_table(&mut self, table: TableSchema) {
        self.tables.insert(table.name.clone(), table);
    }

    /// Add (or replace) a shared enum definition
    pub fn add_enum(&mut self, enum_schema: EnumSchema) {
        self.enums.insert(enum_schema.name.clone(), enum_schema);
    }

    /// Look up a table schema by name
    pub fn get_table(&self, name: &str) -> Option<&TableSchema> {
        self.tables.get(name)
    }

    /// Look up a field schema by table and field name
    pub fn get_field(&self, table: &str, field: &str) -> Option<&FieldSchema> {
        self.tables.get(table)?.fields.get(field)
    }

    /// Validate schema internal consistency
    pub fn validate(&self) -> crate::diagnostics::Diagnostics {
        let mut diags = crate::diagnostics::Diagnostics::new();

        // FieldType nesting depth guard: recursion through Array / Map /
        // Object / TaggedUnion is bounded by MAX_FIELD_TYPE_DEPTH.
        for (table_name, table) in &self.tables {
            for (field_name, field) in &table.fields {
                if let Some(measure) = field_type_depth(&field.field_type) {
                    if measure.depth > MAX_FIELD_TYPE_DEPTH {
                        // A saturated probe means the real depth is far past
                        // the cap (e.g. a hand-built 200k-deep type): name
                        // the cap without pretending to know the depth. A
                        // non-saturated depth of exactly MAX+1 is a real,
                        // one-over-the-cap authoring mistake — report it so
                        // the operator can tell "one level over" from a
                        // pathological type.
                        let msg = if measure.saturated {
                            format!(
                                "Field '{field_name}' in table '{table_name}' exceeds the \
                                 maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
                            )
                        } else {
                            format!(
                                "Field '{field_name}' in table '{table_name}' has nesting \
                                 depth {} which exceeds the maximum nesting depth limit of \
                                 {MAX_FIELD_TYPE_DEPTH}",
                                measure.depth
                            )
                        };
                        diags.add(
                            crate::diagnostics::Diagnostic::error("E1004", msg)
                                .with_source("schema"),
                        );
                    }
                }
            }
        }

        // Check primary keys exist
        for (table_name, table) in &self.tables {
            for pk in &table.primary_key {
                if !table.fields.contains_key(pk) {
                    diags.add(
                        crate::diagnostics::Diagnostic::error(
                            "E1004",
                            format!("Primary key '{pk}' not found in table '{table_name}' fields"),
                        )
                        .with_source("schema"),
                    );
                }
            }
            // Check unique constraints reference existing fields
            for uc in &table.unique_constraints {
                for f in &uc.fields {
                    if !table.fields.contains_key(f) {
                        diags.add(
                            crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!(
                                    "Unique constraint '{}' references unknown field '{}'",
                                    uc.name, f
                                ),
                            )
                            .with_source("schema"),
                        );
                    }
                }
            }
        }

        // Check enum references
        for (table_name, table) in &self.tables {
            for (field_name, field) in &table.fields {
                if let FieldType::Enum(enum_name) = &field.field_type {
                    if !self.enums.contains_key(enum_name) {
                        diags.add(
                            crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!(
                                    "Field '{field_name}' in table '{table_name}' references unknown enum '{enum_name}'"
                                ),
                            )
                            .with_source("schema"),
                        );
                    }
                }
            }
        }

        // Check reference targets exist
        for (table_name, table) in &self.tables {
            for (field_name, field) in &table.fields {
                if let Some(ref_schema) = &field.reference {
                    if !self.tables.contains_key(&ref_schema.table) {
                        diags.add(
                            crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!(
                                    "Field '{}' in table '{}' references unknown table '{}'",
                                    field_name, table_name, ref_schema.table
                                ),
                            )
                            .with_source("schema"),
                        );
                    } else if let Some(ref_table) = self.tables.get(&ref_schema.table) {
                        if !ref_table.fields.contains_key(&ref_schema.field) {
                            diags.add(crate::diagnostics::Diagnostic::error(
                                "E1004",
                                format!("Field '{}' in table '{}' references unknown field '{}' in table '{}'", field_name, table_name, ref_schema.field, ref_schema.table),
                            ).with_source("schema"));
                        }
                    }
                }
            }
        }

        diags
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
struct DepthMeasure {
    /// The nesting depth, or `MAX_FIELD_TYPE_DEPTH + 1` when `saturated`.
    depth: usize,
    /// Whether the walk short-circuited at the cap. When set, `depth` is a
    /// sentinel rather than the real depth, so callers must not report it as
    /// if it were measured.
    saturated: bool,
}

/// Measure the nesting depth of a `FieldType`. Returns `None` for a
/// scalar/leaf; otherwise the measure, saturated past the cap.
fn field_type_depth(t: &FieldType) -> Option<DepthMeasure> {
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

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// `map<key_type, value_type>` — shared by the tests below.
    fn map_type(key_type: MapKeyType, value_type: FieldType) -> FieldType {
        FieldType::Map(MapField {
            key_type,
            value_type: Box::new(value_type),
        })
    }

    #[test]
    fn test_schema_validation() {
        let mut schema = Schema::new();
        let mut table = TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        table.fields.insert(
            "id".to_string(),
            FieldSchema {
                name: "id".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
                nullable: false,
                default: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
                enum_values: None,
                min_items: None,
                max_items: None,
                items: None,
                separator: None,
                properties: None,
                additional_properties: None,
                reference: None,
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        schema.add_table(table);

        let diags = schema.validate();
        assert!(!diags.has_errors());
    }

    #[test]
    fn test_dependency_graph() {
        let mut schema = Schema::new();
        let mut item = TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        item.fields.insert(
            "id".to_string(),
            FieldSchema {
                name: "id".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
                nullable: false,
                default: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
                enum_values: None,
                min_items: None,
                max_items: None,
                items: None,
                separator: None,
                properties: None,
                additional_properties: None,
                reference: None,
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        schema.add_table(item);

        let mut monster = TableSchema {
            name: "Monster".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        monster.fields.insert(
            "id".to_string(),
            FieldSchema {
                name: "id".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
                nullable: false,
                default: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
                enum_values: None,
                min_items: None,
                max_items: None,
                items: None,
                separator: None,
                properties: None,
                additional_properties: None,
                reference: None,
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        monster.fields.insert(
            "drop_item_id".to_string(),
            FieldSchema {
                name: "drop_item_id".to_string(),
                field_type: FieldType::UInt32,
                description: None,
                required: true,
                nullable: false,
                default: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
                enum_values: None,
                min_items: None,
                max_items: None,
                items: None,
                separator: None,
                properties: None,
                additional_properties: None,
                reference: Some(ReferenceSchema {
                    table: "Item".to_string(),
                    field: "id".to_string(),
                    predicate: None,
                    cardinality: "many".to_string(),
                    compatible_with: None,
                }),
                targets: vec![],
                rules: vec![],
                metadata: IndexMap::new(),
            },
        );
        schema.add_table(monster);

        let graph = crate::reference::DependencyGraph::from_schema(&schema);
        let deps = graph.dependencies("Monster");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "Item");

        let order = graph.topological_sort().unwrap();
        assert_eq!(order[0], "Item");
        assert_eq!(order[1], "Monster");
    }

    #[test]
    fn field_type_classification_helpers() {
        for t in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
            FieldType::Float32,
            FieldType::Float64,
        ] {
            assert!(t.is_numeric());
        }
        for t in [
            FieldType::Int8,
            FieldType::Int16,
            FieldType::Int32,
            FieldType::Int64,
            FieldType::UInt8,
            FieldType::UInt16,
            FieldType::UInt32,
            FieldType::UInt64,
        ] {
            assert!(t.is_integer());
            assert!(!t.is_float());
        }
        assert!(FieldType::Float32.is_float());
        assert!(FieldType::Float64.is_float());
        assert!(FieldType::Float32.is_numeric());
        assert!(FieldType::Float64.is_numeric());
        assert!(!FieldType::Float32.is_integer());
        assert!(!FieldType::Float64.is_integer());
        for t in [
            FieldType::Null,
            FieldType::Bool,
            FieldType::String,
            FieldType::Bytes,
            FieldType::Any,
            FieldType::Enum("E".to_string()),
            FieldType::Array(Box::new(FieldType::Int32)),
            FieldType::Object(IndexMap::new()),
            map_type(MapKeyType::String, FieldType::Int32),
            map_type(MapKeyType::Int, FieldType::Int32),
        ] {
            assert!(!t.is_numeric());
            assert!(!t.is_integer());
            assert!(!t.is_float());
        }
    }

    #[test]
    fn rust_type_names_for_every_variant() {
        assert_eq!(FieldType::Null.rust_type(), "()");
        assert_eq!(FieldType::Bool.rust_type(), "bool");
        assert_eq!(FieldType::Int8.rust_type(), "i8");
        assert_eq!(FieldType::Int16.rust_type(), "i16");
        assert_eq!(FieldType::Int32.rust_type(), "i32");
        assert_eq!(FieldType::Int64.rust_type(), "i64");
        assert_eq!(FieldType::UInt8.rust_type(), "u8");
        assert_eq!(FieldType::UInt16.rust_type(), "u16");
        assert_eq!(FieldType::UInt32.rust_type(), "u32");
        assert_eq!(FieldType::UInt64.rust_type(), "u64");
        assert_eq!(FieldType::Float32.rust_type(), "f32");
        assert_eq!(FieldType::Float64.rust_type(), "f64");
        assert_eq!(FieldType::String.rust_type(), "String");
        assert_eq!(FieldType::Bytes.rust_type(), "Vec<u8>");
        assert_eq!(
            FieldType::Array(Box::new(FieldType::Int32)).rust_type(),
            "Vec<_>"
        );
        assert_eq!(
            FieldType::Object(IndexMap::new()).rust_type(),
            "IndexMap<String, _>"
        );
        assert_eq!(
            map_type(MapKeyType::String, FieldType::Int32).rust_type(),
            "HashMap<String, _>"
        );
        assert_eq!(
            map_type(MapKeyType::Int, FieldType::Int32).rust_type(),
            "HashMap<i64, _>"
        );
        assert_eq!(FieldType::Enum("Rarity".to_string()).rust_type(), "Enum");
        assert_eq!(FieldType::Any.rust_type(), "serde_json::Value");
    }

    #[test]
    fn field_type_yaml_parsing_all_kinds_and_edges() {
        // Every one of the 19 FieldType kinds must round-trip through YAML.
        let cases = [
            ("{ kind: Null }", FieldType::Null),
            ("{ kind: Bool }", FieldType::Bool),
            ("{ kind: Int8 }", FieldType::Int8),
            ("{ kind: Int16 }", FieldType::Int16),
            ("{ kind: Int32 }", FieldType::Int32),
            ("{ kind: Int64 }", FieldType::Int64),
            ("{ kind: UInt8 }", FieldType::UInt8),
            ("{ kind: UInt16 }", FieldType::UInt16),
            ("{ kind: UInt32 }", FieldType::UInt32),
            ("{ kind: UInt64 }", FieldType::UInt64),
            ("{ kind: Float32 }", FieldType::Float32),
            ("{ kind: Float64 }", FieldType::Float64),
            ("{ kind: String }", FieldType::String),
            ("{ kind: Bytes }", FieldType::Bytes),
            ("{ kind: Any }", FieldType::Any),
            (
                "{ kind: Array, value: { kind: Int32 } }",
                FieldType::Array(Box::new(FieldType::Int32)),
            ),
            (
                "{ kind: Object, value: { hp: { kind: Int32 }, tag: { kind: String } } }",
                FieldType::Object(IndexMap::from([
                    ("hp".to_string(), FieldType::Int32),
                    ("tag".to_string(), FieldType::String),
                ])),
            ),
            (
                "{ kind: Enum, value: Rarity }",
                FieldType::Enum("Rarity".to_string()),
            ),
            // The 19th kind: map<K, V> — string keys ...
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Int32 } } }",
                map_type(MapKeyType::String, FieldType::Int32),
            ),
            // ... signed-integer keys ...
            (
                "{ kind: Map, value: { key_type: int, value_type: { kind: String } } }",
                map_type(MapKeyType::Int, FieldType::String),
            ),
            // ... and arbitrary nesting in both directions.
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Map, value: { key_type: string, value_type: { kind: Int32 } } } } }",
                map_type(
                    MapKeyType::String,
                    map_type(MapKeyType::String, FieldType::Int32),
                ),
            ),
            (
                "{ kind: Map, value: { key_type: string, value_type: { kind: Array, value: { kind: Int32 } } } }",
                map_type(MapKeyType::String, FieldType::Array(Box::new(FieldType::Int32))),
            ),
        ];
        let mut kinds = HashSet::new();
        for (yaml, expected) in cases {
            let parsed: FieldType =
                serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("parse {yaml}: {e}"));
            assert_eq!(parsed, expected, "yaml: {yaml}");
            // serializing and re-parsing keeps the same shape
            let emitted = serde_yaml::to_string(&parsed).expect("serialize kind");
            let back: FieldType = serde_yaml::from_str(&emitted).expect("re-parse kind");
            assert_eq!(back, expected, "roundtrip: {emitted}");
            // Debug starts with the variant name (e.g. `Map(MapField { ... })`)
            let debug = format!("{expected:?}");
            kinds.insert(
                debug
                    .split('(')
                    .next()
                    .expect("Debug rendering starts with the variant name")
                    .to_string(),
            );
        }
        assert_eq!(kinds.len(), 19, "kinds covered: {kinds:?}");

        // unknown kind, missing content for data variants, missing tag, not a map
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Quadruple }").is_err());
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Array }").is_err());
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Enum }").is_err());
        assert!(serde_yaml::from_str::<FieldType>("{ kind: Map }").is_err());
        assert!(
            serde_yaml::from_str::<FieldType>(
                "{ kind: Map, value: { key_type: bool, value_type: { kind: Int32 } } }"
            )
            .is_err(),
            "unknown key_type must fail"
        );
        assert!(
            serde_yaml::from_str::<FieldType>("{ kind: Map, value: { key_type: string } }")
                .is_err(),
            "missing value_type must fail"
        );
        assert!(serde_yaml::from_str::<FieldType>("{}").is_err());
        assert!(serde_yaml::from_str::<FieldType>("just-a-string").is_err());
    }

    #[test]
    fn map_key_type_wire_format_is_lowercase() {
        // Wire format is the lowercase rename — "string" / "int", nothing else.
        assert_eq!(
            serde_yaml::to_string(&MapKeyType::String).expect("yaml string"),
            "string\n"
        );
        assert_eq!(
            serde_yaml::to_string(&MapKeyType::Int).expect("yaml int"),
            "int\n"
        );
        assert_eq!(
            serde_json::to_string(&MapKeyType::String).expect("json string"),
            "\"string\""
        );
        assert_eq!(
            serde_json::to_string(&MapKeyType::Int).expect("json int"),
            "\"int\""
        );
        for (wire, expected) in [
            ("string", MapKeyType::String),
            ("int", MapKeyType::Int),
            ("\"string\"", MapKeyType::String),
            ("\"int\"", MapKeyType::Int),
        ] {
            let key: MapKeyType = if wire.starts_with('"') {
                serde_json::from_str(wire).expect("json key type")
            } else {
                serde_yaml::from_str(wire).expect("yaml key type")
            };
            assert_eq!(key, expected, "wire: {wire}");
        }
        assert!(serde_yaml::from_str::<MapKeyType>("u32").is_err());
        assert!(serde_yaml::from_str::<MapKeyType>("String").is_err());
        assert!(serde_json::from_str::<MapKeyType>("\"u32\"").is_err());
    }

    #[test]
    fn field_schema_yaml_parses_map_field() {
        let yaml = r"
name: drops
type:
  kind: Map
  value:
    key_type: string
    value_type:
      kind: Array
      value: { kind: Int32 }
description: drop table keyed by rarity
";
        let field: FieldSchema = serde_yaml::from_str(yaml).expect("map field parses");
        assert_eq!(field.name, "drops");
        assert_eq!(
            field.field_type,
            map_type(
                MapKeyType::String,
                FieldType::Array(Box::new(FieldType::Int32))
            )
        );
        assert_eq!(
            field.description.as_deref(),
            Some("drop table keyed by rarity")
        );
        // JSON carries the same shape
        let json = serde_json::to_value(&field.field_type).expect("map type to json");
        assert_eq!(json["kind"], "Map");
        assert_eq!(json["value"]["key_type"], "string");
        assert_eq!(json["value"]["value_type"]["kind"], "Array");
        let back: FieldType = serde_json::from_value(json).expect("map type from json");
        assert_eq!(back, field.field_type);
    }

    #[test]
    fn field_schema_yaml_parsing_rules_constraints_and_metadata() {
        let yaml = r#"
name: level
type: { kind: Int32 }
description: skill level
required: true
default: 1
min: 1
max: 100
targets: [server]
rules:
  - name: level_bounds
    assert: "`level` >= 1"
    message: level must be positive
    warning_only: true
  - name: no_message
    assert: "`level` <= 100"
custom_note: keep
"#;
        let field: FieldSchema = serde_yaml::from_str(yaml).expect("field parses");
        assert_eq!(field.name, "level");
        assert_eq!(field.field_type, FieldType::Int32);
        assert_eq!(field.description.as_deref(), Some("skill level"));
        assert!(field.required);
        assert_eq!(field.default, Some(serde_json::json!(1)));
        assert_eq!(field.min, Some(1.0));
        assert_eq!(field.max, Some(100.0));
        assert_eq!(field.targets, ["server"]);
        assert_eq!(field.rules.len(), 2);
        assert_eq!(field.rules[0].name, "level_bounds");
        assert_eq!(field.rules[0].assert, "`level` >= 1");
        assert_eq!(
            field.rules[0].message.as_deref(),
            Some("level must be positive")
        );
        assert!(field.rules[0].warning_only);
        // message absent and warning_only defaults to false
        assert_eq!(field.rules[1].message, None);
        assert!(!field.rules[1].warning_only);
        // unknown keys flatten into the metadata bag
        assert_eq!(
            field.metadata.get("custom_note"),
            Some(&serde_json::json!("keep"))
        );
        assert!(field.min_length.is_none());
        assert!(field.max_length.is_none());
        assert!(field.pattern.is_none());

        // required keys: `name` and `type` must both be present
        assert!(serde_yaml::from_str::<FieldSchema>("type: { kind: String }\n").is_err());
        assert!(serde_yaml::from_str::<FieldSchema>("name: x\n").is_err());
        // an expression rule requires its `assert`
        assert!(serde_yaml::from_str::<ExpressionRule>("name: r\n").is_err());
        let bare: ExpressionRule =
            serde_yaml::from_str("name: r\nassert: 'true'\n").expect("minimal rule parses");
        assert!(!bare.warning_only);
        assert!(bare.message.is_none());
    }

    #[test]
    fn table_schema_yaml_parsing_primary_key_unique_and_order() {
        let yaml = r"
name: Item
description: an item
primary_key: [id, version]
fields:
  id: { name: id, type: { kind: UInt32 } }
  version: { name: version, type: { kind: UInt16 } }
unique_constraints:
  - { name: uniq_pair, fields: [id, version] }
order_by: [id]
targets: [server, client]
";
        let table: TableSchema = serde_yaml::from_str(yaml).expect("table parses");
        assert_eq!(table.name, "Item");
        assert_eq!(table.description.as_deref(), Some("an item"));
        assert_eq!(table.primary_key, ["id", "version"]);
        assert_eq!(table.fields.len(), 2);
        assert_eq!(table.unique_constraints.len(), 1);
        assert_eq!(table.unique_constraints[0].name, "uniq_pair");
        assert_eq!(table.unique_constraints[0].fields, ["id", "version"]);
        assert_eq!(table.order_by, Some(vec!["id".to_string()]));
        assert_eq!(table.targets, ["server", "client"]);

        // primary_key has no default → required
        assert!(
            serde_yaml::from_str::<TableSchema>("name: T\nfields: {}\n").is_err(),
            "missing primary_key must fail"
        );
        // a unique constraint must name its fields
        assert!(serde_yaml::from_str::<TableSchema>(
            "name: T\nprimary_key: [a]\nfields: {}\nunique_constraints: [{ name: c }]\n"
        )
        .is_err());
    }

    #[test]
    fn reference_schema_cardinality_serde_edges() {
        let bare: ReferenceSchema =
            serde_yaml::from_str("table: Item\nfield: id\n").expect("minimal reference");
        assert_eq!(bare.cardinality, "one");
        assert!(bare.predicate.is_none());
        assert!(bare.compatible_with.is_none());
        // cardinality "one" is the default and is skipped when serializing
        let emitted = serde_yaml::to_string(&bare).expect("serialize reference");
        assert!(!emitted.contains("cardinality"));

        let many: ReferenceSchema = serde_yaml::from_str(
            "table: Item\nfield: id\ncardinality: many\ncompatible_with: [server]\n",
        )
        .expect("many reference");
        assert_eq!(many.cardinality, "many");
        assert_eq!(many.compatible_with, Some(vec!["server".to_string()]));
        let emitted = serde_yaml::to_string(&many).expect("serialize many");
        assert!(emitted.contains("cardinality"));
        let back: ReferenceSchema = serde_yaml::from_str(&emitted).expect("re-parse reference");
        assert_eq!(back.cardinality, "many");

        // required keys: table and field
        assert!(serde_yaml::from_str::<ReferenceSchema>("table: Item\n").is_err());
        assert!(serde_yaml::from_str::<ReferenceSchema>("field: id\n").is_err());
    }

    #[test]
    fn schema_default_and_enum_registration() {
        let mut schema = Schema::default();
        assert!(schema.tables.is_empty());
        assert!(schema.enums.is_empty());
        assert!(schema.metadata.is_none());
        assert!(schema.get_table("Item").is_none());

        schema.add_enum(EnumSchema {
            name: "Rarity".to_string(),
            values: vec![EnumValue {
                name: "Common".to_string(),
                value: None,
                description: None,
            }],
            description: None,
        });
        assert!(schema.enums.contains_key("Rarity"));
        // adding the same name replaces the previous definition
        schema.add_enum(EnumSchema {
            name: "Rarity".to_string(),
            values: vec![],
            description: None,
        });
        assert!(schema.enums["Rarity"].values.is_empty());
    }

    #[test]
    fn schema_lookup_helpers_hit_and_miss() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 } }
enums:
  Rarity:
    name: Rarity
    values:
      - { name: Common }
",
        )
        .expect("schema parses");

        let item = schema.get_table("Item").expect("Item exists");
        assert_eq!(item.primary_key, ["id"]);
        assert!(schema.get_table("Ghost").is_none());

        let id = schema.get_field("Item", "id").expect("id exists");
        assert_eq!(id.field_type, FieldType::UInt32);
        assert!(schema.get_field("Item", "ghost").is_none());
        assert!(schema.get_field("Ghost", "id").is_none());
    }

    /// A one-field table whose sole field carries the given type.
    fn schema_with_one_field(t: FieldType) -> Schema {
        let field = FieldSchema {
            name: "deep".to_string(),
            field_type: t,
            description: None,
            required: false,
            nullable: false,
            default: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            enum_values: None,
            min_items: None,
            max_items: None,
            items: None,
            separator: None,
            properties: None,
            additional_properties: None,
            reference: None,
            targets: Vec::new(),
            rules: Vec::new(),
            metadata: IndexMap::new(),
        };
        let mut fields = IndexMap::new();
        fields.insert("deep".to_string(), field);
        let mut tables = IndexMap::new();
        tables.insert(
            "T".to_string(),
            TableSchema {
                name: "T".to_string(),
                description: None,
                primary_key: Vec::new(),
                fields,
                unique_constraints: Vec::new(),
                order_by: None,
                targets: Vec::new(),
                env_overrides: IndexMap::new(),
            },
        );
        Schema {
            tables,
            enums: IndexMap::new(),
            metadata: None,
        }
    }

    /// A `FieldType` nested deeper than `MAX_FIELD_TYPE_DEPTH` is refused by
    /// `validate` with E1004 — the recursive schema/generator walks never
    /// see an unbounded nesting depth.
    #[test]
    fn schema_rejects_field_type_deeper_than_the_max() {
        // Build Array<Array<...Array<Int32>...>> with MAX+2 levels.
        let mut t = FieldType::Int32;
        for _ in 0..(MAX_FIELD_TYPE_DEPTH + 2) {
            t = FieldType::Array(Box::new(t));
        }
        let schema = schema_with_one_field(t);
        let diags = schema.validate();
        let msg = diags
            .errors()
            .iter()
            .find(|d| d.code == "E1004" && d.message.contains("nesting depth"))
            .map(|d| d.message.clone())
            .expect("a too-deep type is E1004");
        // A depth of MAX+2 is still within one probe of the cap (the walk
        // only saturates past MAX+1), so the real depth is reported.
        assert!(
            msg.contains(&format!("depth {}", MAX_FIELD_TYPE_DEPTH + 2)),
            "the real marginal depth is named: {msg}"
        );
        assert!(
            msg.contains(&format!(
                "maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
            )),
            "E1004 names the cap: {msg}"
        );

        // Exactly at the limit is accepted.
        let mut t = FieldType::Int32;
        for _ in 0..MAX_FIELD_TYPE_DEPTH {
            t = FieldType::Array(Box::new(t));
        }
        let schema = schema_with_one_field(t);
        assert!(
            !schema
                .validate()
                .errors()
                .iter()
                .any(|d| d.message.contains("nesting depth")),
            "at the limit is accepted"
        );

        // One level over the cap is a real, non-saturated measurement: the
        // walk reports MAX+1 as the measured depth (distinguishable from a
        // pathologically deep type that saturates).
        let mut t = FieldType::Int32;
        for _ in 0..(MAX_FIELD_TYPE_DEPTH + 1) {
            t = FieldType::Array(Box::new(t));
        }
        let schema = schema_with_one_field(t);
        let diags = schema.validate();
        let msg = diags
            .errors()
            .iter()
            .find(|d| d.code == "E1004" && d.message.contains("nesting depth"))
            .map(|d| d.message.clone())
            .expect("one-over-the-cap is E1004");
        assert!(
            msg.contains(&format!("depth {}", MAX_FIELD_TYPE_DEPTH + 1)),
            "one level over the cap reports its real depth: {msg}"
        );
    }

    /// The depth probe itself must not overflow the stack on a
    /// maliciously deep type: the walk short-circuits at
    /// `MAX_FIELD_TYPE_DEPTH` instead of recursing with the input depth,
    /// so even a type nested thousands of times beyond the cap is refused
    /// with E1004 rather than crashing the process. (An unbounded probe
    /// would recurse ~200k frames and overflow the test stack before
    /// `validate` could report anything.)
    #[test]
    fn schema_depth_probe_short_circuits_on_a_huge_type() {
        let mut t = FieldType::Int32;
        for _ in 0..200_000 {
            t = FieldType::Array(Box::new(t));
        }
        let schema = schema_with_one_field(t);
        let diags = schema.validate();
        let msg = diags
            .errors()
            .iter()
            .find(|d| d.code == "E1004" && d.message.contains("nesting depth"))
            .expect("a huge type is refused, not a crash")
            .message
            .clone();
        // The diagnostic must not carry the short-circuit sentinel (65) as if
        // it were the real depth — that would mislead an operator debugging a
        // 200k-deep type into thinking the value is only one level over. The
        // saturated probe omits any depth figure entirely.
        assert!(
            !msg.contains("has nesting depth"),
            "a saturated probe must not name a (sentinel) depth: {msg}"
        );
        assert!(
            msg.contains(&format!(
                "maximum nesting depth limit of {MAX_FIELD_TYPE_DEPTH}"
            )),
            "E1004 names the cap: {msg}"
        );
        // Drop the (adversarial) schema for real: the iterative `Drop` must
        // release the 200k-deep spine without overflowing the stack. This is
        // the regression the old `mem::forget` masked — a test that leaks the
        // value never proves the lifecycle is safe end to end.
        drop(schema);
    }

    /// The derived `Clone` / `PartialEq` / `Debug` / `Serialize` recursed
    /// through `Array` / `Map` / `Object` / `TaggedUnion` and overflowed on a
    /// type nested past the depth cap. `Clone` / `PartialEq` / `Debug` now walk
    /// the spine with an explicit worklist, and `Serialize` refuses an
    /// over-cap type rather than recursing — so a hand-built 200k-deep type
    /// survives a full round of clone, compare, debug, and a *refused*
    /// serialize (drop is covered by the probe test above).
    #[test]
    fn huge_field_type_survives_clone_eq_debug_and_serialize() {
        let mut t = FieldType::Int32;
        for _ in 0..200_000 {
            t = FieldType::Array(Box::new(t));
        }
        let cloned = t.clone();
        assert!(t == cloned, "iterative PartialEq handles a huge type");
        let _ = format!("{t:?}");
        let err = serde_json::to_string(&t)
            .expect_err("Serialize refuses an over-cap type instead of recursing");
        assert!(
            err.to_string().contains("nesting depth limit"),
            "the refusal names the cap: {err}"
        );
        drop(cloned);
        drop(t);
    }

    /// The custom iterative `Drop` means Rust refuses by-value moves out of
    /// the container variants (E0509). `FieldType::take` is the documented
    /// migration path for code that needs to *move the whole value out* of a
    /// `&mut` slot (e.g. evicting a field from an `IndexMap`) — it swaps the
    /// default `Null` in, returning the owned value without a by-value match.
    #[test]
    fn field_type_take_is_the_migration_path_for_e0509() {
        let mut map = IndexMap::new();
        map.insert(
            "a".to_string(),
            FieldType::Array(Box::new(FieldType::Int32)),
        );
        let owned = map.get_mut("a").expect("present").take();
        assert_eq!(
            map["a"],
            FieldType::Null,
            "the slot is reset to the default"
        );
        assert_eq!(owned, FieldType::Array(Box::new(FieldType::Int32)));
    }

    /// The iterative `Debug` must reproduce the derived `Debug` byte-for-byte
    /// for an at-or-under-cap type — including the compact `{ "a": ... }`
    /// spacing (no leading space after `{`), the empty-map `{}` shape, and
    /// `{:#?}` pretty alternation. Downstream snapshots and diagnostics that
    /// pin `{:?}` / `{:#?}` strings must not drift from the derive.
    #[test]
    fn debug_matches_the_derived_shape_for_compact_and_pretty() {
        let mut fields = IndexMap::new();
        fields.insert("a".to_string(), FieldType::Int32);
        fields.insert(
            "b".to_string(),
            FieldType::Array(Box::new(FieldType::String)),
        );
        let object = FieldType::Object(fields);
        assert_eq!(
            format!("{object:?}"),
            "Object({\"a\": Int32, \"b\": Array(String)})"
        );
        assert_eq!(
            format!("{object:#?}"),
            "Object(\n    {\n        \"a\": Int32,\n        \"b\": Array(\n            String,\n        ),\n    },\n)"
        );
        assert_eq!(
            format!("{:?}", FieldType::Object(IndexMap::new())),
            "Object({})"
        );
        assert_eq!(
            format!("{:#?}", FieldType::Object(IndexMap::new())),
            "Object(\n    {},\n)"
        );

        let mut branches = IndexMap::new();
        let mut dmg = IndexMap::new();
        dmg.insert("amount".to_string(), FieldType::Int32);
        branches.insert("Damage".to_string(), dmg);
        let tu = FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "$type".to_string(),
            branches,
        }));
        assert_eq!(
            format!("{tu:?}"),
            "TaggedUnion(TaggedUnion { tag: \"$type\", branches: {\"Damage\": {\"amount\": Int32}} })"
        );
        // A type nested past the cap still renders (bounded stack) even if it
        // is an over-cap value the validator would refuse; it must not crash.
        let mut t = FieldType::Int32;
        for _ in 0..(MAX_FIELD_TYPE_DEPTH + 3) {
            t = FieldType::Array(Box::new(t));
        }
        let _ = format!("{t:?}");
    }

    /// The iterative `Serialize` must emit `Object` / `TaggedUnion` keys in
    /// declaration order and without a `preserve_order` dependency: the crate
    /// enables `serde_json/preserve_order` itself, so the wire format is
    /// stable even when compiled in isolation (or if a consumer does not
    /// re-enable the feature). Also the child values are moved out of the
    /// work stack, not deep-cloned (no O(n) copies per node).
    #[test]
    fn serialize_object_and_union_keys_stay_in_declaration_order() {
        let mut fields = IndexMap::new();
        fields.insert("zz".to_string(), FieldType::Int32);
        fields.insert("aa".to_string(), FieldType::String);
        fields.insert(
            "mm".to_string(),
            FieldType::Array(Box::new(FieldType::Int32)),
        );
        let object = FieldType::Object(fields);
        let wire = serde_json::to_string(&object).expect("serialize object");
        assert_eq!(
            wire,
            r#"{"kind":"Object","value":{"zz":{"kind":"Int32"},"aa":{"kind":"String"},"mm":{"kind":"Array","value":{"kind":"Int32"}}}}"#
        );

        let mut branches = IndexMap::new();
        let mut heal = IndexMap::new();
        heal.insert("duration".to_string(), FieldType::Float32);
        branches.insert("Heal".to_string(), heal);
        let mut dmg = IndexMap::new();
        dmg.insert("amount".to_string(), FieldType::Int32);
        branches.insert("Damage".to_string(), dmg);
        let tu = FieldType::TaggedUnion(Box::new(TaggedUnion {
            tag: "$type".to_string(),
            branches,
        }));
        let wire = serde_json::to_string(&tu).expect("serialize union");
        assert_eq!(
            wire,
            r#"{"kind":"TaggedUnion","value":{"tag":"$type","branches":{"Heal":{"duration":{"kind":"Float32"}},"Damage":{"amount":{"kind":"Int32"}}}}}"#
        );
    }

    /// A type nested past [`MAX_FIELD_TYPE_DEPTH`] must be refused by both
    /// `Serialize` and `Deserialize` — a value either round-trips or is
    /// rejected on both sides. Before the bounded `Deserialize`, a depth
    /// 65..~127 type could be `from_str`'d (serde's own ~128 recursion guard)
    /// yet rejected by `to_string`, an asymmetry.
    #[test]
    fn deserialize_refuses_a_type_deeper_than_the_cap() {
        // At the cap: accepted on both sides.
        let mut at = FieldType::Int32;
        for _ in 0..MAX_FIELD_TYPE_DEPTH {
            at = FieldType::Array(Box::new(at));
        }
        let wire = serde_yaml::to_string(&at).expect("at-cap serializes");
        let back: FieldType = serde_yaml::from_str(&wire).expect("at-cap parses");
        assert_eq!(back, at);

        // One over the cap: refused by `Serialize` (E1004-style error) and now
        // refused by `Deserialize` too.
        let mut over = FieldType::Int32;
        for _ in 0..(MAX_FIELD_TYPE_DEPTH + 1) {
            over = FieldType::Array(Box::new(over));
        }
        assert!(
            serde_yaml::to_string(&over).is_err(),
            "over-cap must not serialize"
        );
        // Build the over-cap YAML shape directly (serialize refuses it): an
        // `Array` spine of `MAX + 1` levels wrapping `Int32`.
        let mut v = serde_yaml::to_value(FieldType::Int32).unwrap();
        for _ in 0..(MAX_FIELD_TYPE_DEPTH + 1) {
            v = serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([
                (
                    serde_yaml::Value::String("kind".into()),
                    serde_yaml::Value::String("Array".into()),
                ),
                (serde_yaml::Value::String("value".into()), v),
            ]));
        }
        let over_wire = serde_yaml::to_string(&v).unwrap();
        let err = serde_yaml::from_str::<FieldType>(&over_wire).expect_err("over-cap refuses");
        assert!(
            err.to_string().contains("nesting depth limit"),
            "the refusal names the cap: {err}"
        );
    }

    #[test]
    fn schema_validate_reports_every_error_kind() {
        let schema: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [missing_pk]
    fields:
      id: { name: id, type: { kind: UInt32 } }
      rarity: { name: rarity, type: { kind: Enum, value: Rarity } }
      element: { name: element, type: { kind: Enum, value: Present } }
      drop: { name: drop, type: { kind: UInt32 }, reference: { table: Ghost, field: id } }
      sub: { name: sub, type: { kind: UInt32 }, reference: { table: Item, field: nope } }
    unique_constraints:
      - { name: uc_missing, fields: [ghost_field] }
enums:
  Present:
    name: Present
    values:
      - { name: A }
",
        )
        .expect("schema parses");

        let diags = schema.validate();
        assert!(diags.has_errors());
        let messages: Vec<String> = diags.errors().iter().map(|d| d.message.clone()).collect();
        assert_eq!(messages.len(), 5, "messages: {messages:?}");
        let expect = |needle: &str| {
            assert!(
                messages.iter().any(|m| m.contains(needle)),
                "no message contains {needle:?} in {messages:?}"
            );
        };
        expect("Primary key 'missing_pk' not found in table 'Item' fields");
        expect("Unique constraint 'uc_missing' references unknown field 'ghost_field'");
        expect("Field 'rarity' in table 'Item' references unknown enum 'Rarity'");
        expect("Field 'drop' in table 'Item' references unknown table 'Ghost'");
        expect("Field 'sub' in table 'Item' references unknown field 'nope' in table 'Item'");
        for d in &diags {
            assert_eq!(d.code, "E1004");
            assert_eq!(d.source, "schema");
        }
        // a schema with only resolvable references validates cleanly
        let clean: Schema = serde_yaml::from_str(
            r"
tables:
  Item:
    name: Item
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: UInt32 } }
enums: {}
",
        )
        .expect("clean schema parses");
        assert!(!clean.validate().has_errors());
    }

    #[test]
    fn dependency_graph_sort_handles_revisits_and_cycles() {
        // A → B → C: C is pulled in as a dependency before the loop reaches B,
        // so the second visit of B skips the recursive call entirely.
        let mut graph = crate::reference::DependencyGraph::new();
        graph.add_edge("A", "B");
        graph.add_edge("B", "C");
        assert_eq!(graph.dependencies("A"), ["B"]);
        assert_eq!(graph.dependencies("B"), ["C"]);
        assert_eq!(graph.dependencies("C"), [] as [&String; 0]);
        let order = graph.topological_sort().expect("acyclic");
        assert_eq!(order, ["C", "B", "A"]);

        // circular dependencies are reported instead of looping forever
        let mut cyclic = crate::reference::DependencyGraph::new();
        cyclic.add_edge("X", "Y");
        cyclic.add_edge("Y", "X");
        let err = cyclic.topological_sort().expect_err("cycle detected");
        assert!(
            err.contains("Circular dependency detected involving:"),
            "unexpected error: {err}"
        );
    }

    /// A two-table fixture: `Item.hp` is required with `min: 1`,
    /// `Monster.drop_table` is optional; only `Item` carries env
    /// overrides.
    fn env_schema() -> Schema {
        let mut schema = Schema::new();
        let field = |name: &str, required: bool| FieldSchema {
            name: name.to_string(),
            field_type: FieldType::Int32,
            nullable: false,
            description: None,
            required,
            default: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            enum_values: None,
            min_items: None,
            max_items: None,
            items: None,
            separator: None,
            properties: None,
            additional_properties: None,
            reference: None,
            targets: vec![],
            rules: vec![],
            metadata: IndexMap::new(),
        };
        let mut item = TableSchema {
            name: "Item".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        item.fields.insert("id".to_string(), field("id", true));
        item.fields.insert("hp".to_string(), field("hp", true));
        let mut prod = IndexMap::new();
        let hp = FieldOverride {
            required: Some(false),
            min: Some(100.0),
            ..FieldOverride::default()
        };
        prod.insert("hp".to_string(), hp);
        item.env_overrides.insert("prod".to_string(), prod);
        schema.add_table(item);

        let mut monster = TableSchema {
            name: "Monster".to_string(),
            description: None,
            primary_key: vec!["id".to_string()],
            fields: IndexMap::new(),
            unique_constraints: vec![],
            order_by: None,
            targets: vec![],
            env_overrides: IndexMap::new(),
        };
        monster.fields.insert("id".to_string(), field("id", true));
        monster
            .fields
            .insert("drop_table".to_string(), field("drop_table", false));
        schema.add_table(monster);
        schema
    }

    #[test]
    fn declared_envs_unions_table_override_keys() {
        let schema = env_schema();
        assert_eq!(
            schema.declared_envs(),
            ["prod".to_string()]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );

        let plain = Schema::new();
        assert!(plain.declared_envs().is_empty());
    }

    #[test]
    fn validate_env_overrides_rejects_unknown_fields_and_empty_patches() {
        let mut schema = env_schema();

        // unknown field → named in the error
        let mut prod = IndexMap::new();
        prod.insert("ghost".to_string(), FieldOverride::default());
        schema
            .tables
            .get_mut("Item")
            .unwrap()
            .env_overrides
            .insert("prod".to_string(), prod);
        let err = schema.validate_env_overrides().unwrap_err();
        assert!(
            err.contains("unknown field 'ghost' in table 'Item'"),
            "{err}"
        );

        // empty patch → also rejected
        let mut schema = env_schema();
        let mut prod = IndexMap::new();
        prod.insert(
            "hp".to_string(),
            FieldOverride {
                min: Some(1.0),
                ..FieldOverride::default()
            },
        );
        prod.insert("id".to_string(), FieldOverride::default());
        schema
            .tables
            .get_mut("Item")
            .unwrap()
            .env_overrides
            .insert("prod".to_string(), prod);
        let err = schema.validate_env_overrides().unwrap_err();
        assert!(err.contains("sets no constraint"), "{err}");

        // well-formed overrides pass
        assert!(env_schema().validate_env_overrides().is_ok());
    }

    /// The lint reports every broken override at once — the base lane can
    /// see all unselected-environment breakage, not just the first.
    #[test]
    fn lint_env_overrides_collects_all_problems() {
        let mut schema = env_schema();
        let mut prod = IndexMap::new();
        prod.insert("ghost".to_string(), FieldOverride::default());
        schema
            .tables
            .get_mut("Item")
            .unwrap()
            .env_overrides
            .insert("prod".to_string(), prod);
        // A second break in another env, on another table.
        let mut dev = IndexMap::new();
        dev.insert("hp".to_string(), FieldOverride::default());
        schema
            .tables
            .get_mut("Item")
            .unwrap()
            .env_overrides
            .insert("dev".to_string(), dev);

        let problems = schema.lint_env_overrides();
        // `ghost` reports twice (unknown field AND empty patch — an empty
        // patch is also a break in its own right), `dev.hp` once.
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(
            problems.iter().any(|p| p.contains("unknown field 'ghost'")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("env_overrides[dev]")
                && p.contains(
                    "sets no \
             constraint"
                )),
            "{problems:?}"
        );
        // validate still fails on the first one (the hard path).
        assert!(schema.validate_env_overrides().is_err());
    }

    #[test]
    fn resolve_env_patches_fields_and_keeps_base_untouched() {
        let schema = env_schema();

        // base: hp required, no min
        let base = schema.tables["Item"].fields["hp"].clone();
        assert!(base.required);
        assert_eq!(base.min, None);

        // prod: hp optional with min 100 — same shape, tighter/looser rules
        let prod = schema.resolve_env("prod").unwrap();
        let hp = &prod.tables["Item"].fields["hp"];
        assert!(!hp.required, "prod loosens required");
        assert_eq!(hp.min, Some(100.0), "prod tightens min");
        assert!(
            prod.tables["Item"].fields["id"].required,
            "untouched field stays"
        );

        // Monster has no overrides: identical in both
        assert_eq!(
            schema.tables["Monster"].fields["drop_table"].required,
            prod.tables["Monster"].fields["drop_table"].required
        );

        // the base schema is never mutated
        assert!(schema.tables["Item"].fields["hp"].required);
        assert_eq!(schema.tables["Item"].fields["hp"].min, None);

        // an env with no overrides anywhere resolves to the base
        let dev = schema.resolve_env("dev").unwrap();
        assert_eq!(
            serde_yaml::to_string(&dev).unwrap(),
            serde_yaml::to_string(&schema).unwrap()
        );
    }
}
