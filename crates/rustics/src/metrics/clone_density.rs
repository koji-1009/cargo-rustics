//! `clone-density` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// clone-density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CloneDensity;

impl MetricCalculator for CloneDensity {
    fn id(&self) -> &'static str {
        "clone-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Clone Density",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::LowerIsBetter,
            // Hot paths past 5 owned-copies per function are common smell;
            // 10 is loud.
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
A high clone density usually means the function is escaping the borrow \
checker by paying for an allocation. Sometimes that's the right answer \
(short-lived strings, Rc/Arc reference bumps, decoupling lifetimes); \
often it is the path of least resistance during a hurried refactor. The \
metric does not judge — it counts — and the dismissal record carries the \
reason when a clone is correct.";

const REFACTOR_HINTS: &[&str] = &[
    "Borrow instead of clone: pass `&str` instead of `String`, `&[T]` instead \
of `Vec<T>`, `&T` instead of `T`. The clone often vanishes.",
    "If the data outlives the function, hand ownership in once at the top \
and pass references down.",
    "If a `.clone()` is on `Rc` / `Arc`, mark the dismissal — it is a \
reference bump, not an allocation.",
    "When several clones cluster on one value, hoist the `.clone()` once at \
the top into a local binding.",
];

const REFERENCES: &[&str] = &[
];
