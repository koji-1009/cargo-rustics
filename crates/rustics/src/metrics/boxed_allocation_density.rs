//! `boxed-allocation-density` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// boxed-allocation-density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct BoxedAllocationDensity;

impl MetricCalculator for BoxedAllocationDensity {
    fn id(&self) -> &'static str {
        "boxed-allocation-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Boxed Allocation Density",
            category: MetricCategory::RustPerformance,
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
`Box::new` heap-allocates its argument. Most uses are deliberate (trait \
objects, recursive types, large stack moves), but a high count in a \
single function suggests the allocations could batch into one larger \
arena, or that the data wants a different lifetime story.";

const REFACTOR_HINTS: &[&str] = &[
    "Several `Box::new` calls on the same trait often want to be a \
`Vec<Box<dyn T>>` or, for size-bounded variants, an enum dispatch.",
    "If the boxed values share a lifetime, an arena (e.g. `bumpalo`) can \
collapse them into one allocation.",
];

const REFERENCES: &[&str] = &[];
