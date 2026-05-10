//! Generic Arity — type-parameter count + where-bound count.

use ra_ap_syntax::ast::{self, HasGenericParams};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Generic Arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct GenericArity;

impl MetricCalculator for GenericArity {
    fn id(&self) -> &'static str {
        "generic-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Generic Arity (type params + where bounds)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(7.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            Some(f64::from(count_generics(&frame.item)))
        })
    }
}

const RATIONALE: &str = "\
A function whose signature has many type parameters and where-bounds is \
asking the reader to mentally solve a small trait-resolution puzzle.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace some generic parameters with `impl Trait` arguments — the bound moves out of the visible signature.",
    "Group co-occurring bounds into a single trait alias (`trait My: A + B + C {}` then `T: My`).",
    "If a parameter is always instantiated with the same type, just use that type directly.",
];

const REFERENCES: &[&str] = &["Drysdale, D. (2024). Effective Rust, 2nd ed., Item 12: Understand trade-offs between generics and trait objects. O'Reilly."];

fn count_generics(fn_: &ast::Fn) -> u32 {
    let type_params = fn_
        .generic_param_list()
        .map(|gp| {
            gp.generic_params()
                .filter(|p| {
                    matches!(
                        p,
                        ast::GenericParam::TypeParam(_) | ast::GenericParam::ConstParam(_)
                    )
                })
                .count() as u32
        })
        .unwrap_or(0);
    let where_predicates = fn_
        .where_clause()
        .map(|wc| wc.predicates().count() as u32)
        .unwrap_or(0);
    type_params + where_predicates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        GenericArity.measure(&input)
    }

    #[test]
    fn no_generics_zero() {
        assert_eq!(measure("fn f() {}")[0].value, 0.0);
    }

    #[test]
    fn type_params_count() {
        assert_eq!(measure("fn f<T, U>() {}")[0].value, 2.0);
    }

    #[test]
    fn where_predicates_count() {
        let src = "fn f<T>() where T: Clone, T: Send {}";
        assert_eq!(measure(src)[0].value, 3.0);
    }

    #[test]
    fn lifetimes_do_not_count() {
        assert_eq!(measure("fn f<'a, T>() {}")[0].value, 1.0);
    }
}
