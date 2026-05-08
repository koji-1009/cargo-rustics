//! `closure-arity` — count of inline closure expressions inside a
//! function body.
//!
//! Plan §M4 (continuous lens proliferation). Closure-heavy bodies are
//! a Rust idiom (iterator combinators, `Result::map_err`, callback
//! handlers, …) but past a threshold the local-bindings story gets
//! hard to follow: each closure introduces a fresh scope with its own
//! captures, and reading the function means simulating the closure's
//! body for every call site.
//!
//! Counts every `|...| { ... }` and `move |...| ...` literal in the
//! body, regardless of how short. Closures inside an outer closure
//! count once each (the inner closure adds to the outer function's
//! score; we do not currently emit a per-closure measurement).

use syn::visit::{self, Visit};
use syn::ExprClosure;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `closure-arity` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClosureArity;

impl MetricCalculator for ClosureArity {
    fn id(&self) -> &'static str {
        "closure-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Closure Arity",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Iterator chains naturally hit 3-5; past that the body
            // is more closures than statements.
            default_warning: Some(Threshold::new(6.0)),
            default_error: Some(Threshold::new(12.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = ClosureVisitor { count: 0 };
                v.visit_block(body);
                f64::from(v.count)
            })
        })
    }
}

const RATIONALE: &str = "\
Each inline closure introduces a fresh scope with its own captures and \
return-type story. Iterator pipelines often have 3-5 closures; past six, \
the function reads more like a chain of small lambdas than a sequence of \
statements, and the captures + early-return interactions become hard to \
trace.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract a closure that captures more than one local into a named \
local function. The captures become arguments and the body reads like a \
linear sequence.",
    "Long iterator chains often split at the first stateful operation \
(`fold`, `try_fold`, `scan`); the post-split portion can become a plain \
`for` loop without losing brevity.",
    "Closures whose bodies are themselves multi-statement blocks usually \
want to be functions — `|x| { let y = …; let z = …; …  }` is a function \
in disguise.",
];

const REFERENCES: &[&str] = &["plan §M4 — continuous lens proliferation."];

/// Walks a body counting [`syn::ExprClosure`] occurrences.
struct ClosureVisitor {
    count: u32,
}

impl<'ast> Visit<'ast> for ClosureVisitor {
    fn visit_expr_closure(&mut self, node: &'ast ExprClosure) {
        self.count += 1;
        visit::visit_expr_closure(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        ClosureArity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_closures_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn single_closure_is_one() {
        let src = "fn f() { let _g = |x: i32| x + 1; }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn nested_closures_each_count() {
        let src = "fn f() { let _g = |x: i32| (|y| y + x)(0); }";
        assert_eq!(n_of(src, "f"), 2);
    }

    #[test]
    fn iterator_chain_counts_closures() {
        let src = "fn f(v: Vec<i32>) -> i32 { v.iter().filter(|x| **x > 0).map(|x| *x * 2).sum() }";
        assert_eq!(n_of(src, "f"), 2);
    }

    #[test]
    fn move_closures_count() {
        let src = "fn f() { let _g = move |x: i32| x; }";
        assert_eq!(n_of(src, "f"), 1);
    }
}
