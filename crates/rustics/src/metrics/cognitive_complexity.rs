//! `cognitive-complexity` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// cognitive-complexity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CognitiveComplexity;

impl MetricCalculator for CognitiveComplexity {
    fn id(&self) -> &'static str {
        "cognitive-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Cognitive Complexity (SonarSource)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // SonarSource recommends 15 warning, 50 error — we use the
            // same warning, tighten error to 25 because Rust functions
            // tend to be smaller than the Java functions Sonar shipped on.
            default_warning: Some(Threshold::new(15.0)),
            default_error: Some(Threshold::new(25.0)),
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
Cognitive Complexity is the cost of *understanding* the code, not the cost \
of testing it. Where Cyclomatic Complexity counts independent paths, \
Cognitive Complexity penalises shapes a human reader has to mentally \
unwind: nested control flow, long boolean expressions, labelled breaks \
that jump several scopes. Past 15, even small functions become hard to \
internalise.";

const REFACTOR_HINTS: &[&str] = &[
    "Each level of nesting compounds — extract the inner-most block into a \
helper. The metric drops disproportionately fast.",
    "Replace nested `if`/`else` with a flat `match` on a small enum.",
    "Use `?` and `let-else` to lift error paths to the top of the function — \
the body that follows reads linearly.",
    "Long boolean expressions split well into named locals (`let valid = a \
&& b; let allowed = c || d; if valid && allowed { … }`).",
];

const REFERENCES: &[&str] = &[
    "Campbell, G. A. (2018). Cognitive Complexity. SonarSource white paper.",
];
