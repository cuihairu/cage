//! Cage Core - Configuration Compiler & Validation Framework
//!
//! This crate provides the core types, validation pipeline, and infrastructure
//! for the Cage configuration compiler. It is designed to be embedded in
//! applications or used via the `cage-cli` binary.

#![warn(missing_docs)]
// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: the Value/normalize layer is a numeric coercion engine —
// float<->int casts are its job, not a defect.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::approx_constant,
    clippy::checked_conversions
)]
// Stage: crate-prefixed type names (JsonTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: Diagnostics is the deliberate first-class error type,
// returned by value from every pipeline stage.
#![allow(clippy::result_large_err)]
// Restriction lints kept off for a 0.1.0 codebase.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names
)]

pub mod diagnostics;
pub mod edit;
pub mod error;
pub mod manifest;
pub mod migrate;
pub mod normalize;
pub mod reference;
pub mod registry;
pub mod remote;
pub mod schema;
pub mod snapshot;
pub mod validation;
pub mod value;

// Re-exports for convenience
pub use diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics, RelatedDiagnostic, Severity};
pub use error::codes::{
    build, default_severity, error_title, gamerule, internal, parse, reference as ref_codes,
    schema as schema_codes, semantic, table, type_val, value as value_codes,
    Severity as ErrorSeverity,
};
pub use manifest::{
    verify_manifest, ArtifactInfo, BuildManifest, BuildProfile, ManifestGenerator, ProjectConfig,
    ProjectInfo, RegistryConfig, TargetConfig,
};
pub use normalize::{coerce_to_type, normalize_document, normalize_typed_value, normalize_value};
pub use reference::{
    DependencyGraph, IncrementalPlanner, ReferenceResolver, ResolvedReference, UnresolvedReference,
};
pub use schema::{
    EnumSchema, EnumValue, ExpressionRule, FieldSchema, FieldType, MapField, MapKeyType,
    ReferenceSchema, Schema, SchemaMetadata, TableSchema, UniqueConstraint, ValidatedSchema,
    ValidationContext,
};
pub use validation::{validate, ValidationContext as ValidationCtx, ValidationLevel};
pub use value::{Document, DocumentMetadata, Row, SourceLocation, Table, TypedValue, Value};

/// Prelude module for common imports
pub mod prelude {
    pub use crate::diagnostics::{Diagnostic, DiagnosticBuilder, Diagnostics, Severity};
    pub use crate::error::codes::*;
    pub use crate::manifest::{
        BuildManifest, BuildProfile, ManifestGenerator, ProjectConfig, TargetConfig,
    };
    pub use crate::normalize::normalize_value;
    pub use crate::reference::DependencyGraph;
    pub use crate::schema::{FieldSchema, FieldType, Schema, TableSchema};
    pub use crate::validation::{validate, ValidationLevel};
    pub use crate::value::{Document, Row, SourceLocation, Table, TypedValue, Value};
}
