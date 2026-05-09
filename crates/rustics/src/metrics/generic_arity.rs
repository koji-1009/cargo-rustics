//! Generic Arity — type-parameter count + where-bound count.
//!
//! Rust-specific ergonomics lens. Generic parameters and
//! where-bounds are independent dimensions of "the signature is doing a
//! lot of work"; we sum them so the metric is one number per signature
//! and tracks the total bound complexity.
//!
//! What counts:
//!
//! * Each `T`-style type parameter on the function — `<T>`, `<T: Trait>`,
//!   `<T = Default>` — is `+1`.
//! * Each predicate in the function's `where` clause is `+1`.
//!
//! Lifetime parameters are *not* counted here — they have their own lens
//! [`crate::LifetimeArity`].

use syn::{GenericParam, Signature};

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
        measure_functions(input.ast, |frame| {
            Some(f64::from(count_generics(frame.signature)))
        })
    }
}

const RATIONALE: &str = "\
A function whose signature has many type parameters and where-bounds is \
asking the reader to mentally solve a small trait-resolution puzzle. The \
number summarises how much of the bound surface the call site has to \
satisfy. Past 4 the signature is best read with rustdoc rendering.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace some generic parameters with `impl Trait` arguments — the bound \
moves out of the visible signature.",
    "Group co-occurring bounds into a single trait alias (`trait My: A + B + C \
{}` then `T: My`). The `where` clause shrinks.",
    "If a parameter is always instantiated with the same type, just use that \
type directly.",
];

const REFERENCES: &[&str] = &[];

fn count_generics(sig: &Signature) -> u32 {
    let type_params = sig
        .generics
        .params
        .iter()
        .filter(|p| matches!(p, GenericParam::Type(_) | GenericParam::Const(_)))
        .count() as u32;
    let where_predicates = sig
        .generics
        .where_clause
        .as_ref()
        .map(|w| w.predicates.len() as u32)
        .unwrap_or(0);
    type_params + where_predicates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        GenericArity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_generics_is_zero() {
        assert_eq!(n_of("fn f() {}", "f"), 0);
    }

    #[test]
    fn one_type_param() {
        assert_eq!(n_of("fn f<T>(_: T) {}", "f"), 1);
    }

    #[test]
    fn two_type_params() {
        assert_eq!(n_of("fn f<T, U>(_: T, _: U) {}", "f"), 2);
    }

    #[test]
    fn lifetime_params_excluded() {
        assert_eq!(n_of("fn f<'a, T>(_: &'a T) {}", "f"), 1);
    }

    #[test]
    fn const_param_counted() {
        assert_eq!(n_of("fn f<const N: usize>(_: [u8; N]) {}", "f"), 1);
    }

    #[test]
    fn where_bounds_add() {
        let src = "fn f<T>(_: T) where T: Clone, T: Send, T: Sync {}";
        // 1 type param + 3 where predicates = 4.
        assert_eq!(n_of(src, "f"), 4);
    }
}
