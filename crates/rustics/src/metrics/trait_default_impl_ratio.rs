//! `trait-default-impl-ratio` — informational ratio of default-impl methods
//! over total methods in a `trait` definition.
//!
//! informational. The number ranges 0.0 (no defaults)
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
fixed and you only fill in the few that are required. Informational \
— the value flows into the `rustContext` block.";

const REFACTOR_HINTS: &[&str] = &[
    "Methods with defaults that callers should not override can move out \
into a `*Ext` trait blanket-implemented on the parent.",
    "Methods marked `default fn` mostly for ergonomics often want to be \
free functions on a helper module instead.",
];

const REFERENCES: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        TraitDefaultImplRatio.measure(&input)
    }

    fn ratio_of(src: &str, scope: &str) -> f64 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_trait_is_zero() {
        assert_eq!(ratio_of("trait T {}", "T"), 0.0);
    }

    #[test]
    fn all_provided_is_one() {
        let src = "trait T { fn a(&self) {} fn b(&self) {} }";
        assert_eq!(ratio_of(src, "T"), 1.0);
    }

    #[test]
    fn all_required_is_zero() {
        let src = "trait T { fn a(&self); fn b(&self); }";
        assert_eq!(ratio_of(src, "T"), 0.0);
    }

    #[test]
    fn half_default_is_half() {
        let src = "trait T { fn a(&self); fn b(&self) {} }";
        assert_eq!(ratio_of(src, "T"), 0.5);
    }
}
