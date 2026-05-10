//! Clap definitions for the CLI surface.
//!
//! The set is deliberately small — `analyze`, `manual`, `rules`, … — so
//! the help output stays readable. Subcommand wording mirrors dartrics
//! so the same muscle memory carries over.

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
(Cyclomatic Complexity, Cognitive Complexity, panic-density, …) and emits a \
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
    /// Run the lens catalogue against a workspace and emit a report.
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
    /// Validate the user's `rustics.toml` and report any issues.
    Doctor,
    /// Re-emit an existing JSON snapshot in another reporter format.
    Report(ReportArgs),
    /// List public items whose name is referenced zero times outside
    /// their declaration. With `--apply`, deletes top-level orphan
    /// items in place (methods / variants are reported only).
    Unused(UnusedArgs),
}

/// `cargo rustics unused` arguments.
#[derive(Debug, Parser)]
pub struct UnusedArgs {
    /// Delete every detected unused declaration the kind filter
    /// admits. Refuses to run on a dirty git tree — pass `--force`
    /// to override. Files under `tests/` / `integration_test/` are
    /// skipped unless `--include-tests` is set. Imports left
    /// dangling after deletion can be cleaned up with `cargo fix
    /// --allow-staged`.
    #[arg(long)]
    pub apply: bool,

    /// Allow `--apply` to run on a dirty git tree. Useful in CI on a
    /// fresh check-out, or when the operator has staged unrelated
    /// changes they want to keep untouched.
    #[arg(long)]
    pub force: bool,

    /// Include workspace test files (`tests/**`, `**/integration_test/**`)
    /// in the deletion pass. By default they're skipped — orphan
    /// helpers under `tests/` are usually intentional dead branches
    /// kept around for fixture discoverability.
    #[arg(long)]
    pub include_tests: bool,

    /// Narrow the report (and `--apply`) to specific declaration
    /// kinds. Repeatable or comma-separated; e.g. `--filter
    /// fn,method`. Valid kinds: `fn`, `struct`, `enum`, `trait`,
    /// `type`, `const`, `static`, `union`, `variant`, `method`,
    /// `assoc-const`. Unknown kinds exit with a usage error so a
    /// typo doesn't silently drop the whole report. Defaults to all
    /// kinds when the flag is not passed.
    #[arg(long = "filter", value_name = "KIND", value_delimiter = ',')]
    pub filter: Vec<String>,
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
    /// `<workspace_root>/rustics.toml`.
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
    /// reported in the summary.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Ignore every dismissal in `.rustics-dismissals.toml`. Useful in
    /// CI / final review.
    #[arg(long)]
    pub strict_dismiss: bool,

    /// Path to an lcov.info coverage file. Defaults to
    /// `target/coverage/lcov.info` when present. Pass `none` to
    /// disable.
    #[arg(long, value_name = "PATH")]
    pub coverage: Option<String>,

    /// Restrict output to violations in `.rs` files changed vs the given
    /// git ref. Cross-file analysis stays accurate; only the emitted
    /// records are filtered.
    #[arg(long, value_name = "REF")]
    pub since: Option<String>,

    /// Measure on the macro-expanded AST. Spawns `cargo expand`
    /// (install with `cargo install cargo-expand`); slower than the
    /// default `syn`-only walk because the macro-expansion subprocess
    /// dominates wall time.
    #[arg(long)]
    pub expanded_macros: bool,

    /// Output destination. `-` (default) writes to stdout.
    #[arg(short, long, value_name = "PATH", default_value = "-")]
    pub output: PathBuf,

    /// Suppress the per-violation `explain:` block in the AI reporter
    /// to save tokens. Other reporters are unaffected — they don't
    /// auto-explain in the first place. Mirrors `dartrics`'s flag.
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
    #[arg(long, value_enum, default_value_t = SnapshotModeArg::None)]
    pub snapshot_mode: SnapshotModeArg,

    /// After producing the report, also emit a lens-pair correlation
    /// matrix on stderr. Used to detect redundant lenses (Pearson r >
    /// 0.95 ⇒ pair carries the same signal). Useful before adding
    /// more lenses to keep the catalogue from drifting into a
    /// padding-with-noise state.
    #[arg(long)]
    pub statistics: bool,

    /// Narrow the unused-declaration report to specific declaration
    /// kinds. Repeatable or comma-separated; e.g. `--filter
    /// fn,method`. Valid kinds: `fn`, `struct`, `enum`, `trait`,
    /// `type`, `const`, `static`, `union`, `variant`, `method`,
    /// `assoc-const`. Default: every kind. Mirrors `cargo rustics
    /// unused --filter`; `analyze` is read-only, so `--filter` only
    /// narrows the unused list — metric lenses are filtered via
    /// `--metric` / `--exclude-metric` instead.
    #[arg(long = "filter", value_name = "KIND", value_delimiter = ',')]
    pub filter: Vec<String>,
}

/// `cargo rustics rules` arguments.
#[derive(Debug, Parser)]
pub struct RulesArgs {
    /// Show only the named lens.
    #[arg(long, value_name = "ID")]
    pub metric: Option<String>,
}

/// `cargo rustics report` arguments.
///
/// Re-emits an existing JSON snapshot through a different reporter.
/// Useful when the original `analyze` ran in CI with `--reporter json`
/// and a downstream tool wants `ai` or `console` without re-running
/// the analysis.
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
/// * a path to a JSON snapshot — produced by
///   `cargo rustics analyze --reporter json > snap.json`, or by a
///   previous `--snapshot-mode cache|baseline` run.
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
