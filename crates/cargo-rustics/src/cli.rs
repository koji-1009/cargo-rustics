//! Clap definitions for the CLI surface.
//!
//! Subcommand wording mirrors plan §7.1; option wording mirrors plan §7.2.
//! The set is deliberately small at M1 — `analyze`, `manual`, `rules` — so
//! the help output stays readable. M2 adds `regression`, `explain`,
//! `doctor`, `report`. M3 adds `unused`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Top-level CLI.
#[derive(Debug, Parser)]
#[command(
    name = "cargo-rustics",
    bin_name = "cargo rustics",
    version,
    about = "Classical + Rust-specific code metrics for the AI coding loop",
    long_about = "cargo-rustics looks at Rust code through a stack of lenses \
(Cyclomatic Complexity, Cognitive Complexity, clone-density, …) and emits a \
report tuned for AI agent consumption. Each violation carries a stable id, \
the rationale of the lens, and concrete refactor hints. See `cargo rustics \
manual` for the embedded operator's manual."
)]
pub struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands recognised by cargo-rustics.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the M1 lens catalogue against a workspace and emit a report.
    Analyze(AnalyzeArgs),
    /// Print the embedded operator's manual.
    Manual,
    /// List built-in lenses with their default thresholds and rationales.
    Rules(RulesArgs),
}

/// Output-format choices for `analyze`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Reporter {
    /// Human-readable, lined up. Suitable for terminals.
    Console,
    /// Newline-delimited JSON object. Schema in
    /// `schemas/rustics-report.schema.json`.
    Json,
    /// YAML-ish, header-anchored, tuned for LLM consumption.
    Ai,
}

/// `cargo rustics analyze` arguments.
#[derive(Debug, Parser)]
pub struct AnalyzeArgs {
    /// Analysis root directory. Defaults to the current working directory;
    /// the workspace root is auto-detected from there.
    #[arg(long, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Reporter::Console)]
    pub reporter: Reporter,

    /// Run only the named lens (kebab-case id). Repeatable.
    #[arg(long = "metric", value_name = "ID")]
    pub include_metrics: Vec<String>,

    /// Skip the named lens (kebab-case id). Repeatable.
    #[arg(long = "exclude-metric", value_name = "ID")]
    pub exclude_metrics: Vec<String>,

    /// Exit with code 1 if any warning was reported.
    #[arg(long)]
    pub fatal_warnings: bool,

    /// Maximum files analysed in parallel. Defaults to host CPU count
    /// (clamped to 16 — diminishing returns past there).
    #[arg(long, value_name = "N")]
    pub concurrency: Option<usize>,

    /// Verbose logging.
    #[arg(short, long)]
    pub verbose: bool,
}

/// `cargo rustics rules` arguments.
#[derive(Debug, Parser)]
pub struct RulesArgs {
    /// Show only the named lens.
    #[arg(long, value_name = "ID")]
    pub metric: Option<String>,
}
