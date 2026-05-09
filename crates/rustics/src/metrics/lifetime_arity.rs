//! Lifetime Arity — number of lifetime parameters in a function signature.
//!
//! Rust-specific ergonomics lens. Lifetimes are the cognitive
//! tax Rust extracts in exchange for compile-time memory safety; the more
//! lifetimes a signature carries, the harder it is for a reader (or an AI
//! agent) to reason about which references are tied to which.
//!
//! What we count: every `'a`-style parameter declared on the function (its
//! `Generics::params` list). Implicit elision is *not* counted — that's
//! the whole point of elision. Lifetime bounds in `where` clauses and on
//! types referenced within the signature do not contribute on their own.

use syn::{GenericParam, Signature};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Lifetime Arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct LifetimeArity;

impl MetricCalculator for LifetimeArity {
    fn id(&self) -> &'static str {
        "lifetime-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Lifetime Arity",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Two-and-fewer is comfortable; three is the smell threshold;
            // five and we are at "rewrite this signature" territory.
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(5.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            Some(f64::from(count_lifetime_params(frame.signature)))
        })
    }
}

const RATIONALE: &str = "\
Each lifetime parameter is one referential constraint a reader has to track. \
Two is normal; past three the signature is asking the reader to mentally \
solve a small constraint puzzle before they can call the function. Lifetime \
elision exists precisely so simple functions don't have to spell them out — \
when elision can't apply, the signature becomes a contract that requires \
study.";

const REFACTOR_HINTS: &[&str] = &[
    "Push the lifetimes into a struct: `struct Borrow<'a> { ... }`. The \
function becomes `fn f(b: Borrow<'_>) -> ...`, with one named-lifetime \
binding instead of N.",
    "Take ownership where possible — `String` instead of `&'a str`, `Vec<T>` \
instead of `&'a [T]`. The borrow-checker bookkeeping disappears.",
    "If the lifetimes only relate one input to one output, elision usually \
applies — try removing the explicit lifetime first and see if rustc \
infers it.",
];

const REFERENCES: &[&str] = &[];

fn count_lifetime_params(sig: &Signature) -> u32 {
    sig.generics
        .params
        .iter()
        .filter(|p| matches!(p, GenericParam::Lifetime(_)))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        LifetimeArity.measure(&input)
    }

    fn arity_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_explicit_lifetimes_is_zero() {
        assert_eq!(arity_of("fn f(x: &str) -> &str { x }", "f"), 0);
    }

    #[test]
    fn one_explicit_lifetime() {
        assert_eq!(arity_of("fn f<'a>(x: &'a str) -> &'a str { x }", "f"), 1);
    }

    #[test]
    fn three_explicit_lifetimes() {
        let src = "fn f<'a, 'b, 'c>(x: &'a str, y: &'b str, z: &'c str) {}";
        assert_eq!(arity_of(src, "f"), 3);
    }

    #[test]
    fn type_params_do_not_count() {
        let src = "fn f<'a, T, 'b>(x: &'a T) -> &'b T { todo!() }";
        assert_eq!(arity_of(src, "f"), 2);
    }
}
