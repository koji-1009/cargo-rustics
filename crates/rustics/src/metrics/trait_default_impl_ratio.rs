//! Trait default-impl ratio — fraction of trait methods that have
//! a default body.

use ra_ap_syntax::ast;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_traits;

/// Trait default-impl ratio calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct TraitDefaultImplRatio;

impl MetricCalculator for TraitDefaultImplRatio {
    fn id(&self) -> &'static str {
        "trait-default-impl-ratio"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Trait Default-Impl Ratio",
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
        measure_traits(input.tree, |frame| {
            let al = frame.item.assoc_item_list()?;
            let mut total = 0u32;
            let mut with_body = 0u32;
            for item in al.assoc_items() {
                if let ast::AssocItem::Fn(f) = item {
                    total += 1;
                    if f.body().is_some() {
                        with_body += 1;
                    }
                }
            }
            if total == 0 { None } else { Some(f64::from(with_body) / f64::from(total)) }
        })
    }
}

const RATIONALE: &str = "\
Trait default-impl ratio reports the fraction of methods that ship a \
default body. Higher values mean implementors only have to provide the \
core methods; lower values force every implementor to write all of \
them.";

const REFACTOR_HINTS: &[&str] = &[
    "Add default impls for derivable methods so implementors don't repeat boilerplate.",
];

const REFERENCES: &[&str] = &[];
