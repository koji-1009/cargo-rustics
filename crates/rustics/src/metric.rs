//! [`MetricCalculator`] trait — the seam every lens implements.
//!
//! The trait is the single extension point for metric authors. Anything that
//! satisfies it can be:
//!
//! * driven from the CLI's `analyze` command,
//! * listed by `cargo rustics rules`,
//! * embedded as part of a 30-line consumer (plan §11.6).
//!
//! The trait is intentionally small — `id`, `metadata`, `measure` — so every
//! M1 metric is implementable from the syn AST alone. Adjustments that need
//! type information (Layer 2, plan §3.5 / §6.5) will fit by widening the
//! input type, not the trait.

use serde::{Deserialize, Serialize};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;

/// A *metric* — one lens through the codebase.
///
/// Every implementation must be `Send + Sync`: the CLI runs metrics in
/// parallel across files, then in parallel across metrics within a file. The
/// independence principle (plan §3.2) is enforced by `&self`: a metric
/// cannot accumulate state from another metric's run.
pub trait MetricCalculator: Send + Sync {
    /// Stable kebab-case id used in reports, config, and dismissals.
    ///
    /// Examples: `cyclomatic-complexity`, `clone-density`. Must not change
    /// across a 0.x release — the AI-report contract pins it (plan §4.1).
    fn id(&self) -> &'static str;

    /// Static description used by `cargo rustics rules` and the AI report's
    /// `explain:` block (plan §4.2).
    fn metadata(&self) -> MetricMetadata;

    /// Walks the AST and produces zero or more measurements.
    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement>;
}

/// Static description of a metric.
///
/// `'static` strings are used everywhere because metric metadata is baked
/// into the binary — nothing here is computed at run time. The CLI's
/// `manual` command relies on the same property (plan §5.4).
#[derive(Debug, Clone)]
pub struct MetricMetadata {
    /// Same as [`MetricCalculator::id`]; duplicated for ergonomics.
    pub id: &'static str,
    /// Human-readable name for reports and docs.
    pub display_name: &'static str,
    /// Coarse category for grouping in `rules` and the AI report.
    pub category: MetricCategory,
    /// Whether high values are bad, good, or neutral.
    pub polarity: MetricPolarity,
    /// Default warning threshold; `None` if the metric is informational.
    pub default_warning: Option<Threshold>,
    /// Default error threshold; `None` to disable error escalation.
    pub default_error: Option<Threshold>,
    /// One-paragraph "why does this metric matter" string for `--explain`.
    pub rationale: &'static str,
    /// Concrete refactor hints; the AI report ships these inline.
    pub refactor_hints: &'static [&'static str],
    /// Original-source citations; e.g. `["McCabe 1976"]`.
    pub references: &'static [&'static str],
}

/// Categories used to group metrics in `cargo rustics rules` output.
///
/// The categories follow plan §2.4 — Performance / Safety / Ergonomics /
/// Macro — plus the structural categories from §6.1–§6.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricCategory {
    /// Function/method-level structural metrics (CC, Cognitive, SLOC, …).
    Function,
    /// `impl` / `trait` / `struct` shape (plan §6.2).
    ImplShape,
    /// Module / crate coupling (plan §6.3).
    Coupling,
    /// Macro-related signals (plan §6.4).
    Macro,
    /// Rust-specific performance lenses (`clone-density`, `dyn-density`…).
    RustPerformance,
    /// Rust-specific safety lenses (`unsafe-block-scope`, `panic-density`).
    RustSafety,
    /// Rust-specific ergonomics lenses (`lifetime-arity`, `await-depth`…).
    RustErgonomics,
}

/// Polarity — does a higher value indicate worse code?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricPolarity {
    /// Smaller values are better — the typical case for complexity-style
    /// metrics.
    LowerIsBetter,
    /// Larger values are better. Rare; included for completeness.
    HigherIsBetter,
    /// No direction; the value is shown but never crosses a threshold.
    Informational,
}

/// Severity of a single violation. Mirrors Clippy / SARIF for parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricSeverity {
    /// Below warning threshold — included only when `--verbose` is on.
    Info,
    /// At or above warning threshold; default-fatal under `--fatal-warnings`.
    Warning,
    /// At or above error threshold; always fatal.
    Error,
}

/// A single threshold value.
///
/// A wrapper rather than a bare `f64` so we can later carry coverage gating
/// metadata (`complexityJustifiedThreshold`, plan §4.3) without breaking
/// existing `MetricMetadata` consumers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threshold {
    /// The numeric threshold.
    pub value: f64,
}

impl Threshold {
    /// Returns a new threshold of `value`.
    pub const fn new(value: f64) -> Self {
        Self { value }
    }

    /// True if `measurement` violates this threshold given the metric's polarity.
    pub fn is_violated_by(&self, measurement: f64, polarity: MetricPolarity) -> bool {
        match polarity {
            MetricPolarity::LowerIsBetter => measurement > self.value,
            MetricPolarity::HigherIsBetter => measurement < self.value,
            MetricPolarity::Informational => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_lower_is_better() {
        let t = Threshold::new(10.0);
        assert!(t.is_violated_by(11.0, MetricPolarity::LowerIsBetter));
        assert!(!t.is_violated_by(10.0, MetricPolarity::LowerIsBetter));
        assert!(!t.is_violated_by(9.0, MetricPolarity::LowerIsBetter));
    }

    #[test]
    fn threshold_higher_is_better() {
        let t = Threshold::new(0.8);
        assert!(t.is_violated_by(0.5, MetricPolarity::HigherIsBetter));
        assert!(!t.is_violated_by(0.9, MetricPolarity::HigherIsBetter));
    }

    #[test]
    fn threshold_informational_never_violates() {
        let t = Threshold::new(0.0);
        assert!(!t.is_violated_by(9999.0, MetricPolarity::Informational));
    }
}
