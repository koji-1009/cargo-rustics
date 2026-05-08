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
    /// Diff two analyze snapshots; classifies improved / regressed /
    /// unchanged violations and produces a verdict.
    Regression(RegressionArgs),
    /// Print the embedded operator's manual.
    Manual,
    /// Print the embedded AI-loop walkthrough — concrete prompts and
    /// commands for driving cargo-rustics from a coding agent.
    AiLoop,
    /// List built-in lenses with their default thresholds and rationales.
    Rules(RulesArgs),
    /// Reverse-look up a violation id and print the lens metadata that
    /// would explain it.
    Explain(ExplainArgs),
    /// Validate the user's `rustics.toml` and report any issues.
    Doctor,
    /// Re-emit an existing JSON snapshot in another reporter format.
    Report(ReportArgs),
    /// List public items whose name is referenced zero times outside
    /// their declaration. Plan §M3 / §7.1.
    Unused,
}

/// Analysis depth (plan §3.5 / §7.2).
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Depth {
    /// Layer 1 — `syn` AST only. Default; what every M2 lens uses.
    Shallow,
    /// Layer 2 — rust-analyzer-backed lenses (`monomorphization-count`,
    /// `trait-resolution-depth`, `actual-borrow-cost`). M3 follow-up;
    /// the flag is recognised today and prints a stderr note.
    Deep,
}

/// `--snapshot-mode` choices. Mirrors dartrics's `cache` / `baseline`
/// modes: `cache` writes the snapshot under `target/` (gitignored),
/// `baseline` writes it at the workspace root for committing.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum SnapshotModeArg {
    /// Don't persist a snapshot (default).
    None,
    /// `target/.rustics-cache/snapshot.json`. Local, gitignored.
    Cache,
    /// `<workspace>/rustics-snapshot.json`. Commit; CI reads it.
    Baseline,
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
    /// Markdown — designed for posting as a PR comment.
    Md,
    /// SARIF v2.1.0 — for GitHub Code Scanning / Azure DevOps.
    Sarif,
}

/// `cargo rustics analyze` arguments.
#[derive(Debug, Parser)]
pub struct AnalyzeArgs {
    /// Analysis root directory. Defaults to the current working directory;
    /// the workspace root is auto-detected from there.
    #[arg(long, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Path to an explicit `rustics.toml`. Takes precedence over
    /// `<workspace_root>/rustics.toml`. Plan §7.2.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

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

    /// Cap the number of violations the report shows. Truncated count is
    /// reported in the summary. Plan §7.2.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Ignore every dismissal (`.rustics-dismissals.toml` and doc-comment
    /// `rustics:dismiss`). Useful in CI / final review (plan §7.2).
    #[arg(long)]
    pub strict_dismiss: bool,

    /// Path to a `cargo clippy --message-format=json` output file.
    /// Plan §5.7 / §10.1 — every `clippy::<lint>` warning/error is
    /// folded into the report as one more violation.
    #[arg(long, value_name = "PATH")]
    pub from_clippy: Option<PathBuf>,

    /// Path to an lcov.info coverage file. Defaults to
    /// `target/coverage/lcov.info` when present. Pass `none` to
    /// disable. Plan §4.3, §7.2.
    #[arg(long, value_name = "PATH")]
    pub coverage: Option<String>,

    /// Restrict output to violations in `.rs` files changed vs the given
    /// git ref. Cross-file analysis stays accurate; only the emitted
    /// records are filtered. Plan §7.2.
    #[arg(long, value_name = "REF")]
    pub since: Option<String>,

    /// Measure on the macro-expanded AST (slower; requires cargo-expand).
    /// Plan §7.2 / M3 — cargo-expand subprocess integration is the
    /// next slice; the flag is recognised today and prints a stderr
    /// note when set so the surface stays stable.
    #[arg(long)]
    pub expanded_macros: bool,

    /// Analysis depth (plan §3.5 / §7.2). `shallow` (default) uses
    /// the syn AST only; `deep` adds rust-analyzer-backed lenses
    /// (`monomorphization-count`, `trait-resolution-depth`,
    /// `actual-borrow-cost`). M3 wires the rust-analyzer crates in;
    /// the flag is recognised today.
    #[arg(long, value_enum, default_value_t = Depth::Shallow)]
    pub depth: Depth,

    /// Output destination. `-` (default) writes to stdout.
    #[arg(short, long, value_name = "PATH", default_value = "-")]
    pub output: PathBuf,

    /// Suppress the per-violation `explain:` block in the AI reporter
    /// to save tokens. Other reporters are unaffected (they don't
    /// auto-explain in the first place). Plan §5.2 / dartrics parity.
    #[arg(long)]
    pub no_auto_explain: bool,

    /// Inline this lens's rationale + refactor hints into *any*
    /// reporter's output, regardless of `--no-auto-explain`. Repeatable;
    /// useful when running `--reporter md` for a PR comment but still
    /// wanting `cyclomatic-complexity`'s rationale visible inline.
    #[arg(long = "explain", value_name = "METRIC_ID")]
    pub explain_metrics: Vec<String>,

    /// Persist the finished report as a snapshot for `cargo rustics
    /// regression --before <cache|baseline>` to consume. `cache` writes
    /// to `target/.rustics-cache/snapshot.json` (gitignored); `baseline`
    /// writes to `<workspace>/rustics-snapshot.json` (commit + CI).
    /// Plan §M2 / dartrics parity.
    #[arg(long, value_enum, default_value_t = SnapshotModeArg::None)]
    pub snapshot_mode: SnapshotModeArg,
}

/// `cargo rustics rules` arguments.
#[derive(Debug, Parser)]
pub struct RulesArgs {
    /// Show only the named lens.
    #[arg(long, value_name = "ID")]
    pub metric: Option<String>,
}

/// `cargo rustics explain` arguments.
///
/// Plan §5.2 — looks up a violation `id` (16-hex) inside a JSON snapshot
/// and prints the lens metadata that produced it. Used by the AI loop
/// when it wants the rationale + refactor hints for a specific id
/// without re-running `analyze`.
#[derive(Debug, Parser)]
pub struct ExplainArgs {
    /// 16-hex violation id (`sha256(<file>|<scope>|<metric>)[..16]`).
    pub id: String,
    /// Path to a JSON snapshot the violation was reported in.
    /// Defaults to reading stdin (`cargo rustics analyze --reporter json
    /// | cargo rustics explain <id>`).
    #[arg(long, value_name = "PATH")]
    pub snapshot: Option<PathBuf>,
}

/// `cargo rustics report` arguments.
///
/// Plan §5.2 — re-emits an existing JSON snapshot through a different
/// reporter. Useful when the original `analyze` ran in CI with
/// `--reporter json` and a downstream tool wants `ai` or `console`
/// without re-running the analysis.
#[derive(Debug, Parser)]
pub struct ReportArgs {
    /// Path to a JSON snapshot. Use `-` to read from stdin.
    #[arg(value_name = "PATH")]
    pub input: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Reporter::Console)]
    pub reporter: Reporter,
    /// Suppress the AI reporter's per-violation explain block.
    #[arg(long)]
    pub no_auto_explain: bool,
    /// Inline this lens's rationale into every reporter. Repeatable.
    #[arg(long = "explain", value_name = "METRIC_ID")]
    pub explain_metrics: Vec<String>,
}

/// `cargo rustics regression` arguments.
///
/// `--before` accepts either:
/// * a path to a JSON snapshot (`cargo rustics analyze --reporter json
///   > snap.json` or `--snapshot-mode cache|baseline` from a previous
///   run), or
/// * the keyword `cache` (resolves to
///   `<workspace>/target/.rustics-cache/snapshot.json`) or `baseline`
///   (resolves to `<workspace>/rustics-snapshot.json`). Mirrors dartrics.
#[derive(Debug, Parser)]
pub struct RegressionArgs {
    /// Path to the "before" snapshot, or the keyword `cache` /
    /// `baseline`. See the type-level docs for details.
    #[arg(long, value_name = "PATH_OR_KEYWORD")]
    pub before: String,
    /// Path to the "after" JSON snapshot.
    #[arg(long, value_name = "PATH")]
    pub after: PathBuf,
    /// Output format. Same set as `analyze`.
    #[arg(long, value_enum, default_value_t = Reporter::Console)]
    pub reporter: Reporter,
    /// Exit non-zero if any violation regressed.
    #[arg(long)]
    pub fatal_regressions: bool,
}
