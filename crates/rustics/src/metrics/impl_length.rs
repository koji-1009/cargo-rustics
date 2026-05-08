//! `impl-length` — total physical lines of an `impl` block.
//!
//! Plan §6.2. Pairs with [`crate::ImplMethodCount`]: long impl blocks
//! either have many methods (caught by the method count) or a few very
//! large methods (caught here).

use syn::spanned::Spanned;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// `impl-length` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplLength;

impl MetricCalculator for ImplLength {
    fn id(&self) -> &'static str {
        "impl-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "impl-block Length (lines)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(300.0)),
            default_error: Some(Threshold::new(600.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.ast, |frame| {
            let span = frame.item.brace_token.span.span();
            let start = span.start().line;
            let end = span.end().line;
            let len = if end >= start { end - start + 1 } else { 0 };
            Some(len as f64)
        })
    }
}

const RATIONALE: &str = "\
A long impl block is hard to navigate. The line count ignores how the \
length is distributed (10 long methods vs. 50 short ones) — pair it with \
`impl-method-count` to triage.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the block by role (one impl per concern). The same total length \
becomes legible because each chunk has a name.",
    "If the length comes from one or two huge methods, those are the \
function-level lenses' problem (CC, SLOC, method-length).",
];

const REFERENCES: &[&str] = &["plan §6.2 — impl/trait/struct shape lenses."];
