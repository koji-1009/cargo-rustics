//! `iterator-chain-length` — longest contiguous chain of iterator-ish
//! method calls inside a single expression.
//!
//! Plan §M4. Iterator pipelines are an idiomatic Rust shape, but past a
//! certain length they read as a one-liner that hides each step's
//! intent. The metric counts the deepest run of `.method()` calls
//! inside one expression — straight-line `.iter().filter().map().sum()`
//! is depth 4. Sequential statements (`let x = v.iter().filter(...);
//! let y = x.map(...);`) reset the chain.
//!
//! At Layer 1 we cannot tell whether `.method()` is on an iterator or
//! something else (`String::push_str`, `HashMap::insert`, …). The
//! metric measures method-call chain length regardless; it is
//! informational on its own and pairs with `closure-arity` (closures
//! inside the chain) for a fuller picture.

use syn::visit::{self, Visit};
use syn::Expr;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `iterator-chain-length` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct IteratorChainLength;

impl MetricCalculator for IteratorChainLength {
    fn id(&self) -> &'static str {
        "iterator-chain-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Iterator Chain Length",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Three-link iterator chains are everywhere; six is the
            // smell threshold; ten is "name your intermediate values".
            default_warning: Some(Threshold::new(6.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = ChainVisitor { max: 0 };
                v.visit_block(body);
                f64::from(v.max)
            })
        })
    }
}

const RATIONALE: &str = "\
A method-call chain is a one-liner that hides each step's intent. \
Iterator pipelines naturally chain three or four links \
(`.iter().filter().map().sum()`); past six the reader has to mentally \
hold a long pipeline of transformations. Naming an intermediate value \
restores legibility.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the chain at the first stateful step (`fold`, `try_fold`, \
`scan`, `inspect`) — extract the prefix into a named local binding.",
    "Long chains often hide an early-return path that wants to be a \
plain `for` loop. The CC drops slightly and the early-return reads \
explicitly.",
    "If the chain ends with `collect()`, see if a `for` loop with \
`Vec::push` is clearer at the call site.",
];

const REFERENCES: &[&str] = &["plan §M4 — continuous lens proliferation."];

/// Tracks the deepest `.method().method()...` chain found.
struct ChainVisitor {
    max: u32,
}

impl<'ast> Visit<'ast> for ChainVisitor {
    fn visit_expr(&mut self, node: &'ast Expr) {
        let depth = chain_depth_at(node);
        if depth > self.max {
            self.max = depth;
        }
        visit::visit_expr(self, node);
    }
}

/// `chain_depth_at(e)` is the number of method calls stacked at the
/// top of the chain rooted at `e`. Field access / `?` / `.await` /
/// paren wrappers transparently pass through; anything else (calls,
/// literals, operators) stops the walk.
fn chain_depth_at(expr: &Expr) -> u32 {
    match expr {
        Expr::MethodCall(m) => 1 + chain_depth_at(&m.receiver),
        Expr::Field(f) => chain_depth_at(&f.base),
        Expr::Try(t) => chain_depth_at(&t.expr),
        Expr::Await(a) => chain_depth_at(&a.base),
        Expr::Paren(p) => chain_depth_at(&p.expr),
        Expr::Group(g) => chain_depth_at(&g.expr),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        IteratorChainLength.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_chain_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn single_call_is_one() {
        assert_eq!(n_of("fn f(s: &str) -> usize { s.len() }", "f"), 1);
    }

    #[test]
    fn typical_iterator_pipeline_is_four() {
        let src = "fn f(v: Vec<i32>) -> i32 { v.iter().filter(|x| **x > 0).map(|x| *x).sum() }";
        assert_eq!(n_of(src, "f"), 4);
    }

    #[test]
    fn sequential_chains_in_separate_statements_each_count_independently() {
        let src = r#"
            fn f(v: Vec<i32>) -> i32 {
                let a = v.iter().sum::<i32>();
                let b = v.iter().count();
                a + b as i32
            }
        "#;
        // Each chain is depth 2 (.iter().method); max is 2.
        assert_eq!(n_of(src, "f"), 2);
    }

    #[test]
    fn try_passes_through_chain_depth() {
        // `?` is transparent — only the method-call links count.
        // `.next()` (1) on `?` on `Some(...)` (a Call, which stops the
        // walk) -> 1; then `.map()` (1 more) -> 2.
        let src = "fn f() -> Option<usize> { Some([1].iter())?.next().map(|x| *x as usize) }";
        assert_eq!(n_of(src, "f"), 2);
    }
}
