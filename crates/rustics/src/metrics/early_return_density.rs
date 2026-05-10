//! `early-return-density` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// early-return-density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EarlyReturnDensity;

impl MetricCalculator for EarlyReturnDensity {
    fn id(&self) -> &'static str {
        "early-return-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Early-return Density",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(5.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, _input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // TODO: port to ra_ap_syntax.
        Vec::new()
    }
}

const RATIONALE: &str = "\
A function with many `return` statements is often a switch that did not \
commit to its shape. Two or three early returns guard preconditions; \
past five, the function is usually hiding control flow that wants to \
live in an explicit `match` or be split across helpers.";

const REFACTOR_HINTS: &[&str] = &[
    "Convert a chain of `if cond { return x; }` guards into an explicit \
`match` whose arms compute the result.",
    "If returns split into two clusters (precondition rejection vs. \
business-logic shortcut), the second cluster is often a helper function \
in disguise.",
    "Returns inside a `loop` / `for` are different — they are flow \
control, not guards. Refactoring those tends to make the code worse.",
];

const REFERENCES: &[&str] = &[];
