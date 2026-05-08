//! Number Of Parameters — positional parameter count, excluding `self`.
//!
//! Rust does not have keyword arguments at the language level (named
//! parameters at call sites are a builder pattern), so positional-arity is
//! the relevant signal. `self` is always excluded — it is a receiver, not
//! a parameter the caller chooses.
//!
//! Per plan §6.1, named arguments are out of scope: "positional のみ
//! (named は呼び出し側で名前が見えるためカウント外)".

use syn::{FnArg, Signature};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Number Of Parameters calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct NumberOfParameters;

impl MetricCalculator for NumberOfParameters {
    fn id(&self) -> &'static str {
        "number-of-parameters"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Number Of Parameters",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // 5 is the standard "long parameter list" smell threshold.
            default_warning: Some(Threshold::new(5.0)),
            default_error: Some(Threshold::new(8.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // Signature-only — works on every function, including required
        // trait methods (no body needed).
        measure_functions(input.ast, |frame| {
            Some(f64::from(count_params(frame.signature)))
        })
    }
}

const RATIONALE: &str = "\
Each positional parameter is one fact a caller has to remember and one \
position they can mis-order. Past four or five, callers start passing in \
the wrong cell, and tooling (LSP completion, IDE inlay hints) can't \
recover them all. Rust does not have keyword arguments at the call site, \
so positional-arity is the only contract the user reads.";

const REFACTOR_HINTS: &[&str] = &[
    "Group co-occurring parameters into a struct. The struct's name turns the \
parameter list into self-documenting fields.",
    "If most calls pass the same value for one parameter, it is configuration \
— hoist it to the receiver type or into a builder.",
    "Replace a positional `bool` parameter with an enum so the call site reads \
`Mode::Strict` instead of `true`.",
];

const REFERENCES: &[&str] = &["plan §6.1 — Number Of Parameters; positional のみ。"];

/// Counts positional parameters, excluding any `self` receiver.
fn count_params(sig: &Signature) -> u32 {
    sig.inputs
        .iter()
        .filter(|arg| matches!(arg, FnArg::Typed(_)))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        NumberOfParameters.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_args_is_zero() {
        assert_eq!(n_of("fn f() {}", "f"), 0);
    }

    #[test]
    fn three_args_is_three() {
        assert_eq!(n_of("fn f(a: i32, b: i32, c: i32) {}", "f"), 3);
    }

    #[test]
    fn self_does_not_count() {
        let src = "struct Foo; impl Foo { fn m(&self, x: i32, y: i32) {} }";
        assert_eq!(n_of(src, "Foo::m"), 2);
    }

    #[test]
    fn mut_self_does_not_count() {
        let src = "struct Foo; impl Foo { fn m(&mut self, x: i32) {} }";
        assert_eq!(n_of(src, "Foo::m"), 1);
    }

    #[test]
    fn owned_self_does_not_count() {
        let src = "struct Foo; impl Foo { fn m(self, x: i32) {} }";
        assert_eq!(n_of(src, "Foo::m"), 1);
    }

    #[test]
    fn trait_required_method_is_measured() {
        // Signature-only: required trait methods produce a measurement
        // even though they have no body.
        let src = "trait T { fn f(a: i32, b: i32); }";
        assert_eq!(n_of(src, "T::f"), 2);
    }
}
