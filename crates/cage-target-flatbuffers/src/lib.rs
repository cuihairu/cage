//! `FlatBuffers` Target Generator - generates `.fbs` definition files from the Cage schema
//!
//! `FlatBuffers` consumers must have a compiled schema before they can read
//! data, so the landed form is the schema-driven definition generation
//! shared with the `Protobuf` target: each table becomes a `FlatBuffers`
//! `table` in a per-table `.fbs` file, shared integer-backed enums live in
//! one `cage_enums.fbs` included on demand. Data itself is not embedded —
//! consumers run `flatc` against these definitions with their own runtime.
//! Wire encoding is explicitly out of scope (design §49: no flatc in this
//! repo to act as a byte-level oracle; deterministic binary needs are
//! covered by the `MessagePack` target).

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
// Stage: crate-prefixed type names (FlatbuffersTargetGenerator, ...) are
// idiomatic across a multi-crate workspace.
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

/// `FlatBuffers` Target Generator
///
/// Determinism contract: tables and fields iterate in name order, wrapper
/// tables resolve through a shared name-order allocator, and all decisions
/// are pure functions of the schema — the same schema therefore yields
/// byte-identical `.fbs` files.
pub struct FlatbuffersTargetGenerator {
    /// Output directory
    pub output_dir: PathBuf,
    /// File name template (e.g., "{table}.fbs")
    pub file_template: String,
    /// File name of the shared enums file
    pub enums_file: String,
    /// namespace clause value (e.g., "cage.generated")
    pub namespace: String,
}

impl Default for FlatbuffersTargetGenerator {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("build/fbs"),
            file_template: "{table}.fbs".to_string(),
            enums_file: "cage_enums.fbs".to_string(),
            namespace: "cage.generated".to_string(),
        }
    }
}

mod render;
#[cfg(test)]
mod tests;
