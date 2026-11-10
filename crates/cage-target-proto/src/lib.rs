//! Protobuf Target Generator - generates .proto (proto3) definition files from the Cage schema
//!
//! The Cage design doc lists Protobuf under Data Targets (§23) but pins no
//! wire-encoding contract, so the landed form is the schema-driven shape
//! shared with the other code bindings: each table becomes a `proto3`
//! message in a per-table `.proto` file, shared integer-backed enums live in
//! one `cage_enums.proto` imported on demand. Data itself is not embedded —
//! consumers run `protoc` against these definitions with their own runtime.

// Lint gate: default set + pedantic, with scoped allows.
// (nursery/cargo stay at built-in defaults — see crate docs.)
#![warn(clippy::all, clippy::pedantic)]
// Domain: enum values are numeric literals — casts are the mapping's job.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
// Stage: crate-prefixed type names (ProtoTargetGenerator, ...) are idiomatic
// across a multi-crate workspace.
#![allow(clippy::module_name_repetitions)]
// Stage: API still churning at 0.1.0 — revisit #[must_use] before 1.0.
#![allow(clippy::must_use_candidate, clippy::return_self_not_must_use)]
// Design: the generator renders into an owned String buffer.
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

use std::path::PathBuf;

/// Protobuf Target Generator
///
/// Determinism contract: tables and fields iterate in name order, field
/// numbers follow that order from 1, identifiers resolve through a shared
/// name-order allocator, and all decisions are pure functions of the schema
/// — the same schema therefore yields byte-identical `.proto` files.
pub struct ProtoTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.proto")
    pub file_template: String,
    /// File name of the shared enums file
    pub enums_file: String,
    /// proto package clause value (e.g., "cage.generated")
    pub package: String,
}

impl Default for ProtoTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/proto"),
            file_template: "{table}.proto".to_string(),
            enums_file: "cage_enums.proto".to_string(),
            package: "cage.generated".to_string(),
        }
    }
}

mod render;
#[cfg(test)]
mod tests;
