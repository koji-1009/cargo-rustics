//! `cyclomatic-complexity` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// cyclomatic-complexity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CyclomaticComplexity;

impl MetricCalculator for CyclomaticComplexity {
    fn id(&self) -> &'static str {
        "cyclomatic-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Cyclomatic Complexity (sealed-aware)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(10.0)),
            default_error: Some(Threshold::new(20.0)),
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
Cyclomatic Complexity counts the linearly independent paths through a function body. \
Higher values correlate with branching density, which raises the cognitive load of \
reading the function and the test combinatorics needed to cover it. The Rust \
adjustment (sealed-aware) keeps `match` on enums from being penalised when the \
compiler is already checking exhaustiveness — the cognitive risk that CC was \
designed to flag (a missed case) does not exist there.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract one branch arm into a helper function so the surrounding control \
flow stays readable.",
    "Replace nested `if`/`else` chains with a single `match` on a small enum \
when possible — the sealed-aware rule then absorbs the branches.",
    "Lift early-return guard clauses to the top with `let ... else { return ... }` \
so the happy path stays on the function's main spine.",
    "Split a god-function into a state machine: each state becomes its own \
small function with a low CC.",
];

const REFERENCES: &[&str] = &[
    "McCabe, T. J. (1976). A Complexity Measure. IEEE Trans. Softw. Eng. SE-2(4).",
];
