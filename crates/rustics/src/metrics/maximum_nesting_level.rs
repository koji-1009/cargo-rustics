//! `maximum-nesting-level` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// maximum-nesting-level calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MaximumNestingLevel;

impl MetricCalculator for MaximumNestingLevel {
    fn id(&self) -> &'static str {
        "maximum-nesting-level"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Maximum Nesting Level (early-return-aware)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // 4 is the standard "deeply nested" threshold; past 6 is hard
            // to hold in working memory at all.
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(6.0)),
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
Deep nesting forces a reader to keep more context in working memory. The \
top-level `if` is one fact, the inner `for` is another, the conditional \
inside the loop is a third — past 4 levels, unwinding the meaning back to \
the function's intent costs real attention. The Rust adjustment makes \
`if let X else { return }` (and `else if` chains) read as flat switches, \
which is what they semantically are.";

const REFACTOR_HINTS: &[&str] = &[
    "Lift `if let X else { return }` style guards to the top of the function. \
The body that follows stays linear and the metric drops.",
    "Extract the inner-most loop or block into a helper. The deepest level \
becomes the helper's depth-1 body; the call site flattens.",
    "Replace nested `match` with `if let` early-return guards followed by a \
flat `match` at the function's top level.",
    "Use `?` on `Result` / `Option` instead of `match` + `return Err(...)`.",
];

const REFERENCES: &[&str] = &[];
