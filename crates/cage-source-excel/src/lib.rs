//! Excel Source Adapter - parses Excel files (.xlsx, .xls, .ods) into Cage's Canonical Model

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
use cage_core::convention::ExcelConvention;
use cage_core::diagnostics::Diagnostics;
use cage_core::value::Document;
use std::path::{Path, PathBuf};

/// Excel Source Adapter
pub struct ExcelSourceAdapter {
    /// Row index (0-based) that contains headers
    pub header_row: usize,
    /// Whether to evaluate formulas (use cached values)
    pub eval_formulas: bool,
    /// Skip empty rows
    pub skip_empty_rows: bool,
    /// Treat first row as header if true, otherwise generate col0, col1...
    pub has_header: bool,
    /// Meta-row convention declaration (`[source.excel]`, design §51).
    /// `None` is the baseline: no convention at all, today's byte-for-byte
    /// data plane.
    pub convention: Option<ExcelConvention>,
    /// Project root, used to resolve a declared script path (§51 Q2).
    pub project_root: Option<PathBuf>,
}

impl Default for ExcelSourceAdapter {
    fn default() -> Self {
        Self {
            header_row: 0,
            eval_formulas: true, // Use cached formula values
            skip_empty_rows: true,
            has_header: true,
            convention: None,
            project_root: None,
        }
    }
}

/// One top-level column group from a data header row: the column name (the
/// header text at the group's anchor) and the absolute sheet columns the
/// group occupies — a single column for a flat field, the merged span for a
/// multi-level parent whose sub-columns share its header (design §51 Q6).
pub(crate) struct HeaderGroup {
    name: String,
    columns: Vec<usize>,
}

/// Trait for source adapters
pub trait SourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics>;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn format_name(&self) -> &'static str;
}

impl SourceAdapter for ExcelSourceAdapter {
    fn parse_file(&self, path: &Path) -> Result<Document, Diagnostics> {
        self.parse_file(path)
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["xlsx", "xlsm", "ods"]
    }

    fn format_name(&self) -> &'static str {
        "Excel"
    }
}

mod cells;
mod meta;
#[cfg(test)]
mod tests;
