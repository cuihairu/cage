//! Cage CLI - command-line interface for the Cage configuration compiler.
//!
//! Skeleton stage: subcommand skeleton in place (T5.2 wires up the full implementation of validation/build/contrast/inspection).

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
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Game configuration compilation and validation framework
#[derive(Parser)]
#[command(
    name = "cage",
    version,
    about = "Game configuration compilation and validation framework",
    long_about = "Cage compiles heterogeneous configuration sources (Excel/CSV/JSON/YAML) into validated, deterministically built runtime configuration assets."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate only, do not generate runtime artifacts
    Check {
        /// Configuration root directory
        path: PathBuf,
    },
    /// Validate and generate target artifacts
    Build {
        /// Configuration root directory
        path: PathBuf,
    },
    /// View Schema and configuration structure
    Inspect {
        /// Table name
        table: String,
    },
    /// Compare artifacts of two configuration builds
    Diff {
        /// Baseline build directory
        baseline: PathBuf,
        /// Target build directory
        target: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Commands::Check { path } => {
            eprintln!(
                "cage check {}: Not yet implemented (planned for T5.2)",
                path.display()
            );
            2
        }
        Commands::Build { path } => {
            eprintln!(
                "cage build {}: Not yet implemented (planned for T5.2)",
                path.display()
            );
            2
        }
        Commands::Inspect { table } => {
            eprintln!("cage inspect {table}: Not yet implemented (planned for T5.2)");
            2
        }
        Commands::Diff { baseline, target } => {
            eprintln!(
                "cage diff {} {}: Not yet implemented (planned for T5.2)",
                baseline.display(),
                target.display()
            );
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_check_subcommand() {
        let cli = Cli::try_parse_from(["cage", "check", "config/"]).expect("parse check");
        assert!(matches!(cli.command, Commands::Check { .. }));
    }

    #[test]
    fn parse_build_subcommand() {
        let cli = Cli::try_parse_from(["cage", "build", "config/"]).expect("parse build");
        assert!(matches!(cli.command, Commands::Build { .. }));
    }

    #[test]
    fn parse_diff_subcommand() {
        let cli = Cli::try_parse_from(["cage", "diff", "build/a", "build/b"]).expect("parse diff");
        assert!(matches!(cli.command, Commands::Diff { .. }));
    }

    #[test]
    fn parse_inspect_subcommand() {
        let cli = Cli::try_parse_from(["cage", "inspect", "Item"]).expect("parse inspect");
        assert!(matches!(cli.command, Commands::Inspect { .. }));
    }
}
