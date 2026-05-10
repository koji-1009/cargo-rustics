//! Impl length — line span of an `impl` block.

use ra_ap_syntax::ast::AstNode;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_impls;

/// Impl length calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplLength;

impl MetricCalculator for ImplLength {
    fn id(&self) -> &'static str {
        "impl-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Impl Length (lines)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.tree, |frame| {
            let range = frame.item.syntax().text_range();
            let start: usize = range.start().into();
            let end: usize = range.end().into();
            let slice = input.source.get(start..end).unwrap_or("");
            let n = slice.bytes().filter(|b| *b == b'\n').count() + 1;
            Some(n as f64)
        })
    }
}

const RATIONALE: &str = "\
Impl length reports total lines an impl block spans. Informational only \
— very high values often correlate with a class doing too much, but the \
class-shape lenses (LCOM4 / WMC / RFC) are the actionable signal.";

const REFACTOR_HINTS: &[&str] = &[
    "If an impl block has many methods that touch disjoint state, see LCOM4 — splitting along those lines is the canonical fix.",
];

const REFERENCES: &[&str] = &[];
