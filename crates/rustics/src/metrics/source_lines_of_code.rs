//! `source-lines-of-code` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// source-lines-of-code calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct SourceLinesOfCode;

impl MetricCalculator for SourceLinesOfCode {
    fn id(&self) -> &'static str {
        "source-lines-of-code"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Source Lines Of Code (body)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // Threshold defaults are conservative — function bodies past 60
            // logical lines start to test working memory.
            default_warning: Some(Threshold::new(60.0)),
            default_error: Some(Threshold::new(120.0)),
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
SLOC counts non-blank, non-comment-only lines in a function body. It is the \
conservative size measure: a function with low SLOC may still be complex (CC) \
or deep (max-nesting-level), but very high SLOC is on its own a reliability \
signal — long bodies hide what they do.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract a contiguous block of code into a named helper. The helper's \
name is documentation; SLOC drops automatically.",
    "Lift `let` chains and conversions to the top so the function body shows \
its shape at a glance.",
    "Replace a long sequence of `if`/`else` branches with a `match` on a small \
enum (the sealed-aware CC adjustment makes this free).",
];

const REFERENCES: &[&str] =
    &[];
