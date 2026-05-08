//! Output formatters.
//!
//! Each reporter takes the `Report` shape and writes to a `Write` sink.
//! Reporters are stateless — the `Report` is already sorted (plan §4) and
//! contains every field the AI agent needs to act.

pub mod ai;
pub mod console;
pub mod json;
pub mod md;
pub mod sarif;

#[cfg(test)]
mod golden_tests;

use std::collections::HashSet;
use std::io::Write;

use anyhow::Result;

use crate::cli::Reporter;
use crate::report::Report;

/// Knobs the CLI threads into each reporter.
///
/// dartrics splits the rationale-rendering decision into two flags:
/// the AI reporter inlines explanations by default but `--no-auto-explain`
/// suppresses them (token budget), and `--explain <metric-id>` (repeatable)
/// inlines a specific lens's rationale into *any* reporter — useful when
/// an agent runs the markdown reporter but still wants `cyclomatic-complexity`'s
/// rationale visible inline. We mirror the same shape.
#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    /// AI reporter: when `false`, skip the per-violation `explain:` block.
    /// Other reporters: ignored (they don't auto-explain in the first place).
    pub auto_explain: bool,
    /// Lens IDs (kebab-case) whose rationale + refactor hints should be
    /// inlined into every reporter, regardless of `auto_explain`.
    pub explain_metrics: HashSet<String>,
}

impl ReportOptions {
    /// Default for non-AI reporters: no auto-explain, no overrides.
    pub fn lean() -> Self {
        Self {
            auto_explain: false,
            explain_metrics: HashSet::new(),
        }
    }

    /// Default for the AI reporter: inline every rationale.
    pub fn ai_default() -> Self {
        Self {
            auto_explain: true,
            explain_metrics: HashSet::new(),
        }
    }

    /// Returns `true` when the rationale + hints should be rendered for
    /// this violation under this reporter.
    pub fn should_explain(&self, metric: &str) -> bool {
        self.auto_explain || self.explain_metrics.contains(metric)
    }
}

/// Writes `report` in the chosen format to `out` with default options
/// (AI reporter auto-explains, others do not).
///
/// Kept as a stable convenience alongside [`write_with`]: the embedding
/// host (or any future caller that doesn't want to construct
/// [`ReportOptions`] explicitly) gets the right defaults for free.
#[allow(dead_code)] // public convenience API; the CLI uses `write_with`.
pub fn write(reporter: Reporter, report: &Report, out: &mut dyn Write) -> Result<()> {
    let opts = match reporter {
        Reporter::Ai => ReportOptions::ai_default(),
        _ => ReportOptions::lean(),
    };
    write_with(reporter, report, &opts, out)
}

/// Like [`write`] but with explicit options, so the CLI can honour
/// `--no-auto-explain` and `--explain <metric-id>` flags.
pub fn write_with(
    reporter: Reporter,
    report: &Report,
    opts: &ReportOptions,
    out: &mut dyn Write,
) -> Result<()> {
    match reporter {
        Reporter::Console => console::write_with(report, opts, out),
        Reporter::Json => json::write(report, out),
        Reporter::Ai => ai::write_with(report, opts, out),
        Reporter::Md => md::write_with(report, opts, out),
        Reporter::Sarif => sarif::write(report, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lean_options_skip_explain() {
        let opts = ReportOptions::lean();
        assert!(!opts.auto_explain);
        assert!(opts.explain_metrics.is_empty());
        assert!(!opts.should_explain("cyclomatic-complexity"));
    }

    #[test]
    fn ai_default_options_explain_everything() {
        let opts = ReportOptions::ai_default();
        assert!(opts.auto_explain);
        assert!(opts.should_explain("any-metric"));
    }

    #[test]
    fn explain_metrics_overrides_lean() {
        let mut opts = ReportOptions::lean();
        opts.explain_metrics.insert("clone-density".to_string());
        assert!(opts.should_explain("clone-density"));
        assert!(!opts.should_explain("cyclomatic-complexity"));
    }
}
