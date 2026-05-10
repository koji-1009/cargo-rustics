//! Trait method count — number of `fn` items declared in each
//! `trait` definition.

use ra_ap_syntax::ast;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_traits;

/// Trait method count calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct TraitMethodCount;

impl MetricCalculator for TraitMethodCount {
    fn id(&self) -> &'static str {
        "trait-method-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Trait Method Count",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(15.0)),
            default_error: Some(Threshold::new(30.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_traits(input.tree, |frame| {
            let n = frame
                .item
                .assoc_item_list()
                .map(|al| {
                    al.assoc_items()
                        .filter(|i| matches!(i, ast::AssocItem::Fn(_)))
                        .count()
                })
                .unwrap_or(0);
            Some(n as f64)
        })
    }
}

const RATIONALE: &str = "\
Trait method count flags traits with broad method surfaces. Past 15 \
methods, every implementor pays a porting cost; consider splitting the \
trait into composable smaller traits.";

const REFACTOR_HINTS: &[&str] = &[
    "Split a wide trait into orthogonal traits that implementors can opt into separately.",
    "Move methods that are derivable from a smaller core to a blanket impl on the smaller trait.",
];

const REFERENCES: &[&str] = &[];
