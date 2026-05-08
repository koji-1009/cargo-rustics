//! Cognitive Complexity — SonarSource 2018.
//!
//! Plan §6.1. Each control-flow break adds `+1`; structures that *nest*
//! their bodies add an additional bonus equal to the current nesting
//! level. Sequential structures (`else if`, `else`) get the `+1` only.
//!
//! # Increments at M1
//!
//! * `if` (initial)              → `+1 + nesting`; body at nesting+1
//! * `else if`                   → `+1` (sequential — no nesting bonus)
//! * `else`                      → `+1` (sequential — no nesting bonus)
//! * `while` / `for` / `loop`    → `+1 + nesting`; body at nesting+1
//! * `match`                     → `+1 + nesting`; arms at nesting+1
//! * `&&` / `||`                 → `+1` per operator (simplified — see below)
//! * labelled `break` / `continue` → `+1`
//! * closures (`|...| { ... }`)  → `+0`; body at nesting+1
//!
//! # Boolean operator simplification
//!
//! SonarSource's exact rule increments only on *transitions* between
//! `&&` and `||` within one boolean expression. M1 counts every `&&` /
//! `||` as `+1`, which is a Rust-compatible signal but slightly noisier.
//! The transition rule lands in M2 alongside the parser refactor.
//!
//! # Recursion
//!
//! SonarSource also charges `+1` for direct recursion. Detecting that
//! requires knowing the enclosing function's name and cross-referencing
//! call expressions; the M1 lens omits this. Plan-aligned: the omission
//! is captured as a caveat below.

use syn::visit::{self, Visit};
use syn::{
    BinOp, ExprBinary, ExprBreak, ExprClosure, ExprContinue, ExprForLoop, ExprIf, ExprLoop,
    ExprMatch, ExprWhile,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Cognitive Complexity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CognitiveComplexity;

impl MetricCalculator for CognitiveComplexity {
    fn id(&self) -> &'static str {
        "cognitive-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Cognitive Complexity (SonarSource)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // SonarSource recommends 15 warning, 50 error — we use the
            // same warning, tighten error to 25 because Rust functions
            // tend to be smaller than the Java functions Sonar shipped on.
            default_warning: Some(Threshold::new(15.0)),
            default_error: Some(Threshold::new(25.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = CogVisitor::default();
                v.visit_block(body);
                f64::from(v.total)
            })
        })
    }
}

const RATIONALE: &str = "\
Cognitive Complexity is the cost of *understanding* the code, not the cost \
of testing it. Where Cyclomatic Complexity counts independent paths, \
Cognitive Complexity penalises shapes a human reader has to mentally \
unwind: nested control flow, long boolean expressions, labelled breaks \
that jump several scopes. Past 15, even small functions become hard to \
internalise.";

const REFACTOR_HINTS: &[&str] = &[
    "Each level of nesting compounds — extract the inner-most block into a \
helper. The metric drops disproportionately fast.",
    "Replace nested `if`/`else` with a flat `match` on a small enum.",
    "Use `?` and `let-else` to lift error paths to the top of the function — \
the body that follows reads linearly.",
    "Long boolean expressions split well into named locals (`let valid = a \
&& b; let allowed = c || d; if valid && allowed { … }`).",
];

const REFERENCES: &[&str] = &[
    "Campbell, G. A. (2018). Cognitive Complexity. SonarSource white paper.",
    "plan §6.1 — Cognitive Complexity, default on.",
];

/// Sonar-style cognitive-complexity counter.
#[derive(Default)]
struct CogVisitor {
    nesting: u32,
    total: u32,
    /// `true` when this `Expr::If` is being visited as an `else if` of an
    /// outer `if` chain — it gets the sequential `+1`, not `+1 + nesting`.
    is_else_if: bool,
}

impl CogVisitor {
    fn add_with_nesting(&mut self) {
        self.total += 1 + self.nesting;
    }

    fn add_sequential(&mut self) {
        self.total += 1;
    }

    fn deepen<F: FnOnce(&mut Self)>(&mut self, walk: F) {
        self.nesting += 1;
        walk(self);
        self.nesting -= 1;
    }
}

impl<'ast> Visit<'ast> for CogVisitor {
    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        if self.is_else_if {
            self.add_sequential();
            // Reset so any *nested* `if` inside our then-branch starts fresh.
            self.is_else_if = false;
        } else {
            self.add_with_nesting();
        }
        self.deepen(|v| v.visit_block(&node.then_branch));
        if let Some((_, else_expr)) = &node.else_branch {
            walk_else(self, else_expr);
        }
    }

    fn visit_expr_while(&mut self, node: &'ast ExprWhile) {
        self.add_with_nesting();
        self.deepen(|v| v.visit_block(&node.body));
    }

    fn visit_expr_for_loop(&mut self, node: &'ast ExprForLoop) {
        self.add_with_nesting();
        self.deepen(|v| v.visit_block(&node.body));
    }

    fn visit_expr_loop(&mut self, node: &'ast ExprLoop) {
        self.add_with_nesting();
        self.deepen(|v| v.visit_block(&node.body));
    }

    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        self.add_with_nesting();
        // Arms inherit one extra level so nested control flow inside an
        // arm body is penalised correctly.
        self.deepen(|v| {
            for arm in &node.arms {
                v.visit_expr(&arm.body);
            }
        });
    }

    fn visit_expr_binary(&mut self, node: &'ast ExprBinary) {
        if matches!(node.op, BinOp::And(_) | BinOp::Or(_)) {
            self.total += 1;
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_break(&mut self, node: &'ast ExprBreak) {
        if node.label.is_some() {
            self.total += 1;
        }
        visit::visit_expr_break(self, node);
    }

    fn visit_expr_continue(&mut self, node: &'ast ExprContinue) {
        if node.label.is_some() {
            self.total += 1;
        }
        visit::visit_expr_continue(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast ExprClosure) {
        // Closures don't add their own +1 (Sonar treats them as scopes,
        // not branches), but their body contributes at one level deeper.
        self.deepen(|v| visit::visit_expr_closure(v, node));
    }
}

fn walk_else(v: &mut CogVisitor, else_expr: &syn::Expr) {
    use syn::Expr;
    match else_expr {
        // `else if` — the recursion through visit_expr_if uses the
        // is_else_if flag to apply the sequential rule.
        Expr::If(_) => {
            v.is_else_if = true;
            visit::visit_expr(v, else_expr);
            v.is_else_if = false;
        }
        // `else { … }` — sequential +1, body at nesting+1.
        _ => {
            v.add_sequential();
            v.deepen(|inner| visit::visit_expr(inner, else_expr));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        CognitiveComplexity.measure(&input)
    }

    fn cc_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_function_is_zero() {
        assert_eq!(cc_of("fn f() {}", "f"), 0);
    }

    #[test]
    fn single_if_is_one() {
        assert_eq!(cc_of("fn f(x: bool) { if x {} }", "f"), 1);
    }

    #[test]
    fn nested_if_charges_nesting_bonus() {
        let src = "fn f(x: bool, y: bool) { if x { if y {} } }";
        // outer if: +1 (nesting 0) -> 1
        // inner if: +1 + 1 (nesting 1) -> 2
        // total: 3
        assert_eq!(cc_of(src, "f"), 3);
    }

    #[test]
    fn else_if_chain_is_sequential() {
        let src = "fn f(x: i32) { if x == 0 {} else if x == 1 {} else if x == 2 {} else {} }";
        // initial if: +1
        // each else-if: +1 (no nesting bonus)
        // else: +1
        // total: 1 + 1 + 1 + 1 = 4.
        assert_eq!(cc_of(src, "f"), 4);
    }

    #[test]
    fn while_at_top_level_is_one() {
        assert_eq!(cc_of("fn f() { while true {} }", "f"), 1);
    }

    #[test]
    fn while_inside_if_charges_nesting() {
        let src = "fn f() { if true { while true {} } }";
        // if: 1; inner while: 1 + 1 = 2; total 3.
        assert_eq!(cc_of(src, "f"), 3);
    }

    #[test]
    fn match_at_top_level_is_one() {
        let src = "fn f(x: i32) -> i32 { match x { 0 => 0, _ => 1 } }";
        assert_eq!(cc_of(src, "f"), 1);
    }

    #[test]
    fn boolean_operators_each_count() {
        let src = "fn f(a: bool, b: bool, c: bool) -> bool { a && b || c }";
        // Two operators, each +1 -> 2.
        assert_eq!(cc_of(src, "f"), 2);
    }

    #[test]
    fn labelled_break_counts() {
        let src = r#"
            fn f() {
                'outer: loop {
                    loop {
                        break 'outer;
                    }
                }
            }
        "#;
        // outer loop: 1
        // inner loop: 1 + 1 = 2
        // labelled break: +1
        // total: 1 + 2 + 1 = 4
        assert_eq!(cc_of(src, "f"), 4);
    }

    #[test]
    fn closure_body_inherits_nesting() {
        let src = r#"
            fn f() {
                let g = || {
                    if true {}
                };
                g();
            }
        "#;
        // closure: +0; nesting++ for body
        // if inside closure: +1 + 1 = 2
        assert_eq!(cc_of(src, "f"), 2);
    }
}
