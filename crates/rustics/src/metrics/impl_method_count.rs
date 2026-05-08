//! `impl-method-count` — number of `fn` items in a single `impl` block.
//!
//! Plan §6.2 — impl/trait/struct shape lens. Sized impl blocks are an
//! organisational smell — when one block holds 30+ methods the type's
//! responsibilities have probably outgrown the impl boundary. The metric
//! is per-block, so multiple impls for the same type each get their own
//! measurement.

use syn::ImplItem;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// `impl-method-count` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplMethodCount;

impl MetricCalculator for ImplMethodCount {
    fn id(&self) -> &'static str {
        "impl-method-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "impl-block Method Count",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(20.0)),
            default_error: Some(Threshold::new(40.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.ast, |frame| {
            let n = frame
                .item
                .items
                .iter()
                .filter(|i| matches!(i, ImplItem::Fn(_)))
                .count();
            Some(n as f64)
        })
    }
}

const RATIONALE: &str = "\
A single `impl` block carrying twenty-plus methods is usually a sign that \
the type has accumulated several roles. Splitting the block by role \
(separate impls per trait, separate impls per concern) lets the reader \
locate behaviour by purpose without scanning the whole block.";

const REFACTOR_HINTS: &[&str] = &[
    "Group methods by role into separate `impl` blocks (`impl Foo { /* core */ \
}` + `impl Foo { /* serde adapters */ }`).",
    "Consider whether some of the methods belong on a separate type that \
holds a reference to this one.",
    "If the methods cluster by trait conformance, move them out into trait \
impls.",
];

const REFERENCES: &[&str] = &["plan §6.2 — impl/trait/struct shape lenses."];
