//! `trait-default-impl-ratio` — informational ratio of default-impl methods
//! over total methods in a `trait` definition.
//!
//! Plan §6.2 — informational at M1. The number ranges 0.0 (no defaults)
//! to 1.0 (every method has a default body). It is a *shape* signal, not
//! a quality signal — high ratios are sometimes correct (e.g.
//! `Iterator`'s many adapters), sometimes a hint that the trait should
//! expose less.

use syn::TraitItem;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_traits;

/// `trait-default-impl-ratio` calculator.
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

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_traits(input.ast, |frame| {
            let (defaulted, total) = frame.item.items.iter().fold((0u32, 0u32), |(d, t), it| {
                if let TraitItem::Fn(method) = it {
                    let d = if method.default.is_some() { d + 1 } else { d };
                    (d, t + 1)
                } else {
                    (d, t)
                }
            });
            if total == 0 {
                return Some(0.0);
            }
            Some(f64::from(defaulted) / f64::from(total))
        })
    }
}

const RATIONALE: &str = "\
The ratio of methods that ship with a default body. Useful when reading a \
trait you have to implement: a high ratio means most of the methods are \
fixed and you only fill in the few that are required. Informational at M1 \
— the value flows into the `rustContext` block in M2.";

const REFACTOR_HINTS: &[&str] = &[
    "Methods with defaults that callers should not override can move out \
into a `*Ext` trait blanket-implemented on the parent.",
    "Methods marked `default fn` mostly for ergonomics often want to be \
free functions on a helper module instead.",
];

const REFERENCES: &[&str] = &["plan §6.2 — trait-default-impl-ratio (informational)."];
