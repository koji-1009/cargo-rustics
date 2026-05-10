//! Lifetime Arity — count of explicit lifetime parameters on a fn.

use ra_ap_syntax::ast::{self, HasGenericParams};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Lifetime arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct LifetimeArity;

impl MetricCalculator for LifetimeArity {
    fn id(&self) -> &'static str {
        "lifetime-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Lifetime Arity (explicit `'a` params)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(5.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| Some(f64::from(count_lifetimes(&frame.item))))
    }
}

const RATIONALE: &str = "\
Lifetime arity counts explicit `'a` parameters on a function signature. \
A signature with many lifetimes is asking the reader to track several \
independent borrowings; past 2-3 the function is usually doing two \
unrelated things and would split into smaller bodies.";

const REFACTOR_HINTS: &[&str] = &[
    "If two `&'a T` parameters are always passed the same value, merge them.",
    "Lifetimes that appear once in the signature can usually be elided — `fn f<'a>(x: &'a T) -> &'a U` ⇒ `fn f(x: &T) -> &U`.",
    "Splitting the function so each half handles one borrow shape often eliminates the need for explicit lifetimes.",
];

const REFERENCES: &[&str] = &[];

fn count_lifetimes(fn_: &ast::Fn) -> u32 {
    fn_.generic_param_list()
        .map(|gp| {
            gp.generic_params()
                .filter(|p| matches!(p, ast::GenericParam::LifetimeParam(_)))
                .count() as u32
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        LifetimeArity.measure(&input)
    }

    #[test]
    fn none_zero() {
        assert_eq!(measure("fn f() {}")[0].value, 0.0);
    }

    #[test]
    fn each_lifetime_counts() {
        assert_eq!(measure("fn f<'a, 'b>() {}")[0].value, 2.0);
    }

    #[test]
    fn type_params_do_not_count() {
        assert_eq!(measure("fn f<'a, T>() {}")[0].value, 1.0);
    }
}
