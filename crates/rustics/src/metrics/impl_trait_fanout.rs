//! `impl-trait-fanout` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

/// impl-trait-fanout calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplTraitFanout;

impl MetricCalculator for ImplTraitFanout {
    fn id(&self) -> &'static str {
        "impl-trait-fanout"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "impl-Trait Fanout",
            category: MetricCategory::RustErgonomics,
            // Informational — never crosses a threshold.
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
`impl Trait` erases the concrete type at the boundary; every occurrence \
in the signature is one more place where the caller cannot name the \
return value's type without `<…>` annotations or a separate alias. \
Informational — the value feeds the `rustContext` block (plan \
§4.3) that lands with the regression command.";

const REFACTOR_HINTS: &[&str] = &[
    "If callers need to name the type (store it in a struct, return it from \
their own `fn`), give the function a concrete return type or a type alias.",
    "When `impl Trait` is used because the type is *truly* hidden (RPIT for \
async or iterators), keep it — the count is informational, not a smell.",
];

const REFERENCES: &[&str] = &[
];
