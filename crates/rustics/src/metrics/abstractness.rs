//! `abstractness` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

/// abstractness calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Abstractness;

impl MetricCalculator for Abstractness {
    fn id(&self) -> &'static str {
        "abstractness"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Abstractness (A)",
            category: MetricCategory::Coupling,
            // Informational — Distance from Main Sequence (D = |A + I − 1|)
            // is the actionable derived metric, and it needs cross-file Ca to
            // compute I. We ship A standalone now so the input is recorded.
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
Abstractness names the proportion of type definitions that are *traits* \
(abstract contracts) versus concrete types (struct/enum/union). A library \
module typically sits high (lots of trait-driven design); a leaf \
implementation module sits low. The number is one of two inputs to \
Distance from Main Sequence (D = |A + I − 1|); it is reported \
informationally because Instability needs cross-file aggregation \
that lands.";

const REFACTOR_HINTS: &[&str] = &[
    "If a module mixes many traits with many concrete types, splitting it \
into a `*_traits` module and a `*_impl` module makes the role of each \
file obvious.",
    "Sealed traits (`pub trait Foo: sealed::Sealed {}`) often live alongside \
their implementations; that pattern legitimately lowers a module's \
Abstractness without changing its design.",
];

const REFERENCES: &[&str] = &[
    "Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of Dependencies.",
];
