//! `trait-default-impl-ratio` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

/// trait-default-impl-ratio calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct TraitDefaultImplRatio;

impl MetricCalculator for TraitDefaultImplRatio {
    fn id(&self) -> &'static str {
        "trait-default-impl-ratio"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Trait Default-impl Ratio",
            category: MetricCategory::ImplShape,
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
The ratio of methods that ship with a default body. Useful when reading a \
trait you have to implement: a high ratio means most of the methods are \
fixed and you only fill in the few that are required. Informational \
— the value flows into the `rustContext` block.";

const REFACTOR_HINTS: &[&str] = &[
    "Methods with defaults that callers should not override can move out \
into a `*Ext` trait blanket-implemented on the parent.",
    "Methods marked `default fn` mostly for ergonomics often want to be \
free functions on a helper module instead.",
];

const REFERENCES: &[&str] = &[];
