//! Result Chain Depth — longest contiguous chain of `?` operators inside a
//! single expression tree, *plus* hand-rolled `match Result` ladder
//! depth weighted to's calibration.
//!
//! + §2.5 + §6.1 — Rust-specific ergonomics lens. The signal
//! is "how many error paths is this expression flowing through in one
//! go". `a()?.b()?.c()?` is depth 3 — three places this expression
//! can early-return on `Err`. The metric does *not* sum sequential `?`
//! across statements (`let x = a()?; let y = b()?;` is two depth-1
//! chains, not depth 2).
//!
//! # Calibration
//!
//! says `?` chains carry warning at 6 while hand-rolled
//! `match Result { Ok => …, Err => … }` ladders warn at 3. Both
//! axes share one metric value — to keep the threshold single, the
//! match-ladder depth is doubled before being compared with the
//! `?` chain. So a match-Result ladder of depth 3 maps to value 6,
//! crossing the warning threshold on its own.
//!
//! "Match on Result" detection is structural at Layer 1 (no type
//! info): we recognise the `Ok(...)` / `Err(...)` two-arm shape via
//! pattern names. False positives on user-defined enums named `Ok` /
//! `Err` are a known caveat; a richer test lands when
//! Layer 2's rust-analyzer integration arrives.

use syn::visit::{self, Visit};
use syn::{Expr, ExprMatch, Pat};

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
                let mut chain_v = ChainVisitor { max: 0 };
                chain_v.visit_block(body);
                let mut match_v = MatchResultVisitor { current: 0, max: 0 };
                match_v.visit_block(body);
                //: match-Result ladder warns at 3, `?` chain
                // at 6. Doubling the ladder depth normalises both axes
                // to the `?`-chain threshold.
                let combined = chain_v.max.max(match_v.max.saturating_mul(2));
                f64::from(combined)
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

/// Tracks the deepest *nested* `match Result` ladder.
struct MatchResultVisitor {
    current: u32,
    max: u32,
}

impl<'ast> Visit<'ast> for MatchResultVisitor {
    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        if is_result_match(node) {
            self.current += 1;
            if self.current > self.max {
                self.max = self.current;
            }
            visit::visit_expr_match(self, node);
            self.current -= 1;
        } else {
            visit::visit_expr_match(self, node);
        }
    }
}

/// True iff the match looks like `match e { Ok(_) => …, Err(_) => … }`
/// (or vice versa). — two-arm shape with the `Ok` / `Err`
/// constructor names.
fn is_result_match(m: &ExprMatch) -> bool {
    if m.arms.len() != 2 {
        return false;
    }
    let names: Vec<String> = m
        .arms
        .iter()
        .filter_map(|arm| pattern_constructor(&arm.pat))
        .collect();
    let has = |needle: &str| names.iter().any(|n| n == needle);
    has("Ok") && has("Err")
}

fn pattern_constructor(pat: &Pat) -> Option<String> {
    match pat {
        Pat::TupleStruct(ts) => last_segment(&ts.path),
        Pat::Path(p) => last_segment(&p.path),
        Pat::Struct(s) => last_segment(&s.path),
        _ => None,
    }
}

fn last_segment(p: &syn::Path) -> Option<String> {
    p.segments.last().map(|s| s.ident.to_string())
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

    #[test]
    fn single_match_result_counts_two() {
        let src = r#"
            fn f() -> Result<i32, ()> {
                match a() {
                    Ok(x) => Ok(x),
                    Err(e) => Err(e),
                }
            }
        "#;
        //: ladder depth 1 doubles to value 2.
        assert_eq!(d_of(src, "f"), 2);
    }

    #[test]
    fn nested_match_result_three_deep() {
        let src = r#"
            fn f() -> Result<i32, ()> {
                match a() {
                    Ok(x) => match b(x) {
                        Ok(y) => match c(y) {
                            Ok(z) => Ok(z),
                            Err(e) => Err(e),
                        },
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                }
            }
        "#;
        // 3 nested match-Result ladders → value 6.
        assert_eq!(d_of(src, "f"), 6);
    }

    #[test]
    fn match_without_ok_err_does_not_count() {
        let src = r#"
            fn f(x: i32) -> i32 {
                match x { 0 => 0, _ => 1 }
            }
        "#;
        assert_eq!(d_of(src, "f"), 0);
    }

    #[test]
    fn chain_or_match_takes_max() {
        // Single match-Result (value 2) beats a single `?` (value 1).
        let src = r#"
            fn f() -> Result<i32, ()> {
                let _ = g()?;
                match h() { Ok(x) => Ok(x), Err(e) => Err(e) }
            }
        "#;
        assert_eq!(d_of(src, "f"), 2);
    }
}
