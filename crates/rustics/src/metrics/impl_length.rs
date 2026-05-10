//! `impl-length` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

/// impl-length calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplLength;

impl MetricCalculator for ImplLength {
    fn id(&self) -> &'static str {
        "impl-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "impl-block Length (lines, informational)",
            category: MetricCategory::ImplShape,
            // Informational — no "better direction". The CK-defined
            // wmc lens is the gating one for impl-block weight; this
            // value travels along as raw context for the AI / human
            // reading the report.
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
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
Raw line count of an `impl` block — surfaced as informational \
context. The gating signal for impl-block weight is `wmc` (CK 1994, \
Σ CC over methods). This metric exists to give the AI / human \
reader the raw size figure without double-counting the wmc gate.";

const REFACTOR_HINTS: &[&str] = &[
    "If the impl block is long because of many small methods, see \
`wmc` for the complexity-weighted view.",
    "If the length comes from one or two huge methods, those are the \
function-level lenses' problem (CC, SLOC).",
];

const REFERENCES: &[&str] = &[];
