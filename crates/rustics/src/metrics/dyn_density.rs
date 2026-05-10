//! `dyn-density` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

/// dyn-density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct DynDensity;

impl MetricCalculator for DynDensity {
    fn id(&self) -> &'static str {
        "dyn-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "dyn-Trait Density",
            category: MetricCategory::RustPerformance,
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
Each `dyn Trait` in the signature is one virtual-dispatch boundary the \
runtime has to honour. Dynamic dispatch is sometimes the right answer \
(plug-in architectures, heterogeneous collections); sometimes it is the \
path of least resistance for a generic that did not fit. Informational \
— the value feeds the `rustContext` block that travels with each \
violation.";

const REFACTOR_HINTS: &[&str] = &[
    "If only a small set of types implements the trait, prefer a generic \
parameter or an enum to keep the dispatch static.",
    "Inside hot loops, converting `Box<dyn T>` to `T: Trait` (a generic \
parameter) often removes the per-call indirection.",
];

const REFERENCES: &[&str] = &[
];
