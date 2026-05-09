//! `trait-method-count` — number of method items in a `trait` definition.
//!
//! Counts both required and provided methods (default impls).

use syn::TraitItem;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_traits;

/// `trait-method-count` calculator.
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
        measure_traits(input.ast, |frame| {
            let n = frame
                .item
                .items
                .iter()
                .filter(|i| matches!(i, TraitItem::Fn(_)))
                .count();
            Some(n as f64)
        })
    }
}

const RATIONALE: &str = "\
A trait with many methods imposes a heavy contract on every implementor. \
Past 15 methods, splitting into smaller traits (with the original as a \
super-trait that combines them) usually makes implementations easier to \
write and read.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the trait into a hierarchy: `trait Read`, `trait Write`, then \
`trait ReadWrite: Read + Write {}` for the combined contract.",
    "Move methods that have natural defaults into a separate `*Ext` trait \
implemented blanket on the original.",
];

const REFERENCES: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        TraitMethodCount.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_trait_is_zero() {
        assert_eq!(n_of("trait T {}", "T"), 0);
    }

    #[test]
    fn required_and_provided_methods_both_count() {
        let src = "trait T { fn a(&self); fn b(&self); fn c(&self) {} }";
        assert_eq!(n_of(src, "T"), 3);
    }

    #[test]
    fn associated_type_does_not_count() {
        let src = "trait T { type Item; fn a(&self); }";
        assert_eq!(n_of(src, "T"), 1);
    }
}
