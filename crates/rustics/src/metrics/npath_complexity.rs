//! `npath-complexity` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// npath-complexity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpathComplexity;

impl MetricCalculator for NpathComplexity {
    fn id(&self) -> &'static str {
        "npath-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "NPath Complexity (Nejmeh 1988)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // Nejmeh proposes 200 as the practical threshold; values
            // past that "exceed reasonable testability". Saturation
            // beyond ~1000 means the path space is essentially
            // unbounded.
            default_warning: Some(Threshold::new(200.0)),
            default_error: Some(Threshold::new(1000.0)),
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
NPath Complexity (Nejmeh 1988) counts the acyclic execution paths \
through a function body. Cyclomatic Complexity adds 1 per decision; \
NPath *multiplies* sequential branches, exposing the combinatorial \
test cost that CC misses. Two back-to-back `if-else` blocks score \
CC=3 but NPath=4; ten compose to CC=11 but NPath=1024. Past 200 \
the function exceeds practical exhaustive-testability.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull a sequence of independent decisions into a helper — the \
helper's NPath grows in isolation, the caller's drops to NP(helper) + 1.",
    "Collapse parallel `if-else` chains into a single `match` on a \
small enum: a 4-arm match is NPath=4, while four independent if-else \
blocks compose to 2^4=16.",
    "A loop with internal branching often factors cleanly: lift the \
branching out of the loop body into a helper that decides once, then \
loop over the resulting plan.",
];

const REFERENCES: &[&str] = &[
    "Nejmeh, B. A. (1988). NPATH: a measure of execution path \
complexity and its applications. Commun. ACM 31(2): 188-200.",
];
