//! Result Chain Depth — longest contiguous chain of `?` operators inside a
//! single expression tree.
//!
//! Plan §2.4 + §2.5 + §6.1 — Rust-specific ergonomics lens. The signal
//! is "how many error paths is this expression flowing through in one go".
//! `a()?.b()?.c()?` is depth 3 — three places this expression can early-
//! return on `Err`. The metric does *not* sum sequential `?` across
//! statements (`let x = a()?; let y = b()?;` is two depth-1 chains, not
//! depth 2).
//!
//! # Calibration (plan §2.5)
//!
//! `?` chains are decision-cheap because Rust's type inference makes the
//! error path mechanical — each `?` does the same thing. We set a
//! generous threshold accordingly. Hand-rolled `match Result { Ok => …,
//! Err => … }` nesting carries higher cognitive weight, but at Layer 1
//! (no type info) we can't tell whether a `match` is on `Result`. That
//! refinement is M2; the M1 lens reports `?`-chain depth only.

use syn::visit::{self, Visit};
use syn::Expr;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Result Chain Depth calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResultChainDepth;

impl MetricCalculator for ResultChainDepth {
    fn id(&self) -> &'static str {
        "result-chain-depth"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Result Chain Depth (?)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Calibrated for Rust idioms: `?` is cheap, the chain has to
            // be *long* before reading suffers.
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
Each `?` is a place this expression can early-return on `Err`. Inference \
makes the error path mechanical, so a long chain is *legible* — but past \
six links a reader still has to track which `?` corresponds to which \
fallible step in the chain. The threshold is generous compared to the \
hand-rolled `match Result { … }` ladder it replaces.";

const REFACTOR_HINTS: &[&str] = &[
    "Break a long chain into named local bindings: `let x = a()?; let y = \
x.b()?; …`. Each step now has a name and the chain depth resets.",
    "If the chain is mostly `.method()?`, consider whether `.method()` should \
return the type already (some failure points may collapse).",
    "Use combinators (`map`, `and_then`, `?` as a single tail) sparingly when \
the steps are heterogeneous — the named-binding form usually reads clearer.",
];

const REFERENCES: &[&str] = &[
    "plan §2.4 — result-chain-depth.",
    "plan §2.5 — calibration: `?` chain warning 6, hand-rolled `match Result` \
ladder warning 3 (M2).",
];

/// Walks expressions, recording the deepest `?` chain found.
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

/// `chain_depth_at(e)` is the number of `?` operators stacked at the top
/// of the chain rooted at `e`. The chain follows `?`, method calls, field
/// access, and bracket wrappers — anything else stops the walk.
fn chain_depth_at(expr: &Expr) -> u32 {
    match expr {
        Expr::Try(t) => 1 + chain_depth_at(&t.expr),
        Expr::MethodCall(m) => chain_depth_at(&m.receiver),
        Expr::Field(f) => chain_depth_at(&f.base),
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
        ResultChainDepth.measure(&input)
    }

    fn d_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_question_marks_is_zero() {
        assert_eq!(d_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn single_question_mark_is_one() {
        let src = "fn f() -> Option<i32> { let v = Some(1)?; Some(v) }";
        assert_eq!(d_of(src, "f"), 1);
    }

    #[test]
    fn chained_question_marks_count_per_link() {
        let src = "fn f() -> Result<i32, ()> { Ok(Ok::<i32, ()>(1)?.checked_add(1).ok_or(())?) }";
        assert_eq!(d_of(src, "f"), 2);
    }

    #[test]
    fn sequential_question_marks_in_separate_statements_each_count_one() {
        let src = r#"
            fn f() -> Option<i32> {
                let _a = Some(1)?;
                let _b = Some(2)?;
                let _c = Some(3)?;
                Some(0)
            }
        "#;
        assert_eq!(d_of(src, "f"), 1);
    }

    #[test]
    fn three_link_method_chain() {
        // We don't need this to type-check — `syn::parse_file` just parses.
        // The chain is `a()?.b()?.c()?` — three `?` along the same chain.
        let src = "fn f() -> Option<i32> { a()?.b()?.c()? }";
        assert_eq!(d_of(src, "f"), 3);
    }
}
