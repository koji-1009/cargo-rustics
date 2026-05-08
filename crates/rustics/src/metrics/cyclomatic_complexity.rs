//! Cyclomatic Complexity (McCabe 1976), sealed-aware.
//!
//! # Algorithm
//!
//! Every function starts at `CC = 1` (one straight-line path). Each of the
//! following adds `+1`:
//!
//! * `if` / `else if` (each tail `if` is its own node — `else if` recurses
//!   through the visitor as a fresh `Expr::If`)
//! * `if let` / `while let` (the `let` `cond` lives inside an `Expr::If` /
//!   `Expr::While`, so the same visitor arm picks them up)
//! * `while` / `for` / `loop`
//! * `?` (the `Try` expression branches on `Ok`/`Err`)
//! * short-circuit `&&` / `||`
//!
//! `match` is the sealed-aware case (plan §2.5):
//!
//! * If the match has a wildcard arm (`_ => …`), it cannot be reasoned about
//!   structurally — count `arms - 1` decision points (one per non-default
//!   alternative).
//! * If there is no wildcard, assume the compiler is checking exhaustiveness
//!   for us and contribute `0` to CC. The "missed case" cognitive load that
//!   McCabe was designed to flag does not apply (per plan §2.5).
//!
//! # Scope
//!
//! One measurement is emitted per function body — free `fn`, `impl` method,
//! and `trait` method (provided only — required methods have no body).
//! Nested closures contribute to the enclosing function's score; they are
//! not measured separately at M1 to match common implementations and to
//! keep one number per function.

use syn::visit::{self, Visit};
use syn::{BinOp, ExprBinary, ExprForLoop, ExprIf, ExprLoop, ExprMatch, ExprTry, ExprWhile, Pat};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Cyclomatic Complexity (sealed-aware) calculator.
///
/// Stateless — every call to [`MetricCalculator::measure`] computes from
/// scratch.
#[derive(Debug, Default, Clone, Copy)]
pub struct CyclomaticComplexity;

impl MetricCalculator for CyclomaticComplexity {
    fn id(&self) -> &'static str {
        "cyclomatic-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Cyclomatic Complexity (sealed-aware)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(10.0)),
            default_error: Some(Threshold::new(20.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| f64::from(compute_cc(&body.stmts)))
        })
    }
}

const RATIONALE: &str = "\
Cyclomatic Complexity counts the linearly independent paths through a function body. \
Higher values correlate with branching density, which raises the cognitive load of \
reading the function and the test combinatorics needed to cover it. The Rust \
adjustment (sealed-aware) keeps `match` on enums from being penalised when the \
compiler is already checking exhaustiveness — the cognitive risk that CC was \
designed to flag (a missed case) does not exist there.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract one branch arm into a helper function so the surrounding control \
flow stays readable.",
    "Replace nested `if`/`else` chains with a single `match` on a small enum \
when possible — the sealed-aware rule then absorbs the branches.",
    "Lift early-return guard clauses to the top with `let ... else { return ... }` \
so the happy path stays on the function's main spine.",
    "Split a god-function into a state machine: each state becomes its own \
small function with a low CC.",
];

const REFERENCES: &[&str] = &[
    "McCabe, T. J. (1976). A Complexity Measure. IEEE Trans. Softw. Eng. SE-2(4).",
    "plan §2.5 — Type-system-aware adjustments (sealed-aware match).",
];

/// Computes CC for a function body's statement list.
fn compute_cc(stmts: &[syn::Stmt]) -> u32 {
    let mut visitor = CcVisitor { cc: 1 };
    for stmt in stmts {
        visitor.visit_stmt(stmt);
    }
    visitor.cc
}

/// Counts decision points inside a function body.
struct CcVisitor {
    cc: u32,
}

impl<'ast> Visit<'ast> for CcVisitor {
    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        // `if`, `else if`, `if let` all funnel through here. Each adds +1.
        self.cc += 1;
        visit::visit_expr_if(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast ExprWhile) {
        // `while` and `while let` both count.
        self.cc += 1;
        visit::visit_expr_while(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast ExprForLoop) {
        self.cc += 1;
        visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast ExprLoop) {
        // `loop {}` is unconditional but every reachable exit is a `break` —
        // count one decision point so a `loop`-shaped state machine is not
        // free.
        self.cc += 1;
        visit::visit_expr_loop(self, node);
    }

    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        let arm_count = node.arms.len() as u32;
        if has_wildcard_arm(node) {
            // Non-sealed match — each branch beyond the first is a decision.
            self.cc += arm_count.saturating_sub(1);
        }
        // Sealed-aware case: the compiler is checking exhaustiveness for us;
        // contribute 0. Plan §2.5.
        visit::visit_expr_match(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast ExprBinary) {
        if matches!(node.op, BinOp::And(_) | BinOp::Or(_)) {
            self.cc += 1;
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast ExprTry) {
        // The `?` operator is a binary branch on `Ok`/`Err`.
        self.cc += 1;
        visit::visit_expr_try(self, node);
    }
}

/// True iff the match contains a top-level wildcard arm (`_ => …`).
fn has_wildcard_arm(m: &ExprMatch) -> bool {
    m.arms.iter().any(|arm| pat_is_wildcard(&arm.pat))
}

fn pat_is_wildcard(pat: &Pat) -> bool {
    match pat {
        Pat::Wild(_) => true,
        // `_ | other` should still be treated as having a wildcard alternative.
        Pat::Or(or) => or.cases.iter().any(pat_is_wildcard),
        // `(_,)` etc. — a wildcard alone in a single-element tuple/etc. is
        // structurally equivalent to a wildcard match. Recurse through
        // simple wrappers so we don't false-negative.
        Pat::Paren(p) => pat_is_wildcard(&p.pat),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        CyclomaticComplexity.measure(&input)
    }

    fn cc_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}` in measurements"))
    }

    #[test]
    fn empty_function_is_one() {
        assert_eq!(cc_of("fn f() {}", "f"), 1);
    }

    #[test]
    fn single_if_adds_one() {
        assert_eq!(cc_of("fn f(x: bool) { if x {} }", "f"), 2);
    }

    #[test]
    fn else_if_chain_adds_per_branch() {
        let src = "fn f(x: i32) { if x == 0 {} else if x == 1 {} else if x == 2 {} else {} }";
        // 3 if expressions (the chain produces nested ExprIf nodes) -> base 1 + 3 = 4.
        assert_eq!(cc_of(src, "f"), 4);
    }

    #[test]
    fn while_for_loop_each_count_one() {
        let src = "fn f() { while true {} for _ in 0..1 {} loop { break; } }";
        assert_eq!(cc_of(src, "f"), 1 + 3);
    }

    #[test]
    fn short_circuit_and_or_count() {
        let src = "fn f(a: bool, b: bool) -> bool { a && b || !a }";
        assert_eq!(cc_of(src, "f"), 3);
    }

    #[test]
    fn try_operator_counts() {
        let src = "fn f() -> Option<i32> { let x = Some(1)?; Some(x) }";
        assert_eq!(cc_of(src, "f"), 2);
    }

    #[test]
    fn sealed_match_is_free() {
        let src = r#"
            enum Color { R, G, B }
            fn f(c: Color) -> i32 {
                match c { Color::R => 0, Color::G => 1, Color::B => 2 }
            }
        "#;
        assert_eq!(cc_of(src, "f"), 1);
    }

    #[test]
    fn unsealed_match_charges_per_arm() {
        let src = r#"
            fn f(x: i32) -> i32 {
                match x { 0 => 0, 1 => 1, 2 => 2, _ => 99 }
            }
        "#;
        assert_eq!(cc_of(src, "f"), 4);
    }

    #[test]
    fn impl_method_scope_uses_type_name() {
        let src = "struct Foo; impl Foo { fn bar(&self) {} }";
        assert_eq!(cc_of(src, "Foo::bar"), 1);
    }

    #[test]
    fn trait_for_type_scope_uses_receiver_type() {
        let src = r#"
            struct Foo;
            trait Show { fn show(&self); }
            impl Show for Foo { fn show(&self) { if true {} } }
        "#;
        assert_eq!(cc_of(src, "Foo::show"), 2);
    }

    #[test]
    fn trait_method_with_default_body_is_measured() {
        let src = "trait T { fn f(&self) { if true {} } }";
        assert_eq!(cc_of(src, "T::f"), 2);
    }

    #[test]
    fn trait_method_without_body_is_skipped() {
        let src = "trait T { fn f(&self); }";
        let ms = measure(src);
        assert!(ms.iter().all(|m| m.scope.path != "T::f"));
    }

    #[test]
    fn module_nesting_prefixes_scope() {
        let src = "mod outer { mod inner { pub fn f() {} } }";
        assert_eq!(cc_of(src, "outer::inner::f"), 1);
    }

    #[test]
    fn if_let_counts_as_branch() {
        let src = "fn f(x: Option<i32>) { if let Some(_v) = x {} }";
        assert_eq!(cc_of(src, "f"), 2);
    }

    #[test]
    fn while_let_counts_as_branch() {
        let src = "fn f(x: Option<i32>) { while let Some(_v) = x { break; } }";
        assert_eq!(cc_of(src, "f"), 2);
    }

    #[test]
    fn or_pattern_with_wildcard_alternative_treated_as_unsealed() {
        let src = "fn f(x: i32) -> i32 { match x { 0 | _ => 1, } }";
        assert_eq!(cc_of(src, "f"), 1);
    }

    #[test]
    fn nested_decisions_accumulate() {
        let src = r#"
            fn f(x: i32, y: i32) -> i32 {
                if x > 0 && y > 0 {
                    for _ in 0..x { if y > 1 {} }
                    1
                } else {
                    0
                }
            }
        "#;
        assert_eq!(cc_of(src, "f"), 5);
    }
}
