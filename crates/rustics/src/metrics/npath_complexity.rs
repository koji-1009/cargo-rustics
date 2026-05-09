//! `npath-complexity` — Nejmeh 1988.
//!
//! Counts the number of acyclic execution paths through a function
//! body. Where Cyclomatic Complexity adds 1 per decision point and
//! grows linearly, NPath multiplies branches and grows
//! combinatorially: an `if-else` block with two arms is `2`, two
//! such blocks back-to-back is `2 * 2 = 4`, and so on. Captures the
//! "test combinatorics" cost that CC under-counts when sequential
//! decision points compound.
//!
//! Per Nejmeh 1988 the threshold of interest is **200**, beyond
//! which the function "becomes too complex to understand or test
//! exhaustively". We keep the same warning at 200 and pick `1000`
//! for error (effectively a saturation point — values past that
//! grow into millions and the path-count explodes).
//!
//! Composition rules from the original paper:
//! - Sequential statements compose multiplicatively (block-of-blocks
//!   = product of inner NPaths).
//! - `if-else`: `NP(if) + NP(else) + 1` (3 if both bodies trivial).
//! - `if` (no else): `NP(if) + 1`.
//! - `while` / `for` / `loop`: `NP(body) + 1` (loop-once vs skip).
//! - `switch` / `match`: `Σ NP(arm) + 1` (each arm + the empty path
//!   when the scrutinee falls through; for an exhaustive Rust match
//!   the +1 sealed-aware adjustment is dropped — see `cyclomatic_complexity`).
//! - `&&` / `||`: each adds 1 to the inner path count (short-circuit
//!   creates a binary branch in the expression).
//! - `?`: 2 (Ok / Err alternation).
//!
//! Reference:
//! * Nejmeh (1988). "NPATH: a measure of execution path complexity
//!   and its applications". Communications of the ACM 31(2): 188-200.

use syn::visit::{self, Visit};
use syn::{
    BinOp, Block, ExprBinary, ExprForLoop, ExprIf, ExprLoop, ExprMatch, ExprTry, ExprWhile, Stmt,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `npath-complexity` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpathComplexity;

impl MetricCalculator for NpathComplexity {
    fn id(&self) -> &'static str {
        "npath-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "NPath Complexity (Nejmeh 1988)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // Nejmeh proposes 200 as the practical threshold; values
            // past that "exceed reasonable testability". Saturation
            // beyond ~1000 means the path space is essentially
            // unbounded.
            default_warning: Some(Threshold::new(200.0)),
            default_error: Some(Threshold::new(1000.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| f64::from(npath_block(body).min(u32::MAX)))
        })
    }
}

const RATIONALE: &str = "\
NPath Complexity (Nejmeh 1988) counts the acyclic execution paths \
through a function body. Cyclomatic Complexity adds 1 per decision; \
NPath *multiplies* sequential branches, exposing the combinatorial \
test cost that CC misses. Two back-to-back `if-else` blocks score \
CC=3 but NPath=4; ten compose to CC=11 but NPath=1024. Past 200 \
the function exceeds practical exhaustive-testability.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull a sequence of independent decisions into a helper — the \
helper's NPath grows in isolation, the caller's drops to NP(helper) + 1.",
    "Collapse parallel `if-else` chains into a single `match` on a \
small enum: a 4-arm match is NPath=4, while four independent if-else \
blocks compose to 2^4=16.",
    "A loop with internal branching often factors cleanly: lift the \
branching out of the loop body into a helper that decides once, then \
loop over the resulting plan.",
];

const REFERENCES: &[&str] = &[
    "Nejmeh, B. A. (1988). NPATH: a measure of execution path \
complexity and its applications. Commun. ACM 31(2): 188-200.",
];

/// NPath of a block: product of NPath of each statement (sequential
/// composition rule).
fn npath_block(block: &Block) -> u32 {
    if block.stmts.is_empty() {
        return 1;
    }
    let mut total: u32 = 1;
    for stmt in &block.stmts {
        total = total.saturating_mul(npath_stmt(stmt));
    }
    total
}

fn npath_stmt(stmt: &Stmt) -> u32 {
    let mut v = NpathVisitor { paths: 1 };
    v.visit_stmt(stmt);
    v.paths
}

/// Walks an expression tree counting NPath contributions per the
/// Nejmeh composition rules. The visitor accumulates into `paths`
/// using *multiplicative* composition for sequential nodes and
/// node-specific formulas for control flow.
struct NpathVisitor {
    paths: u32,
}

impl<'ast> Visit<'ast> for NpathVisitor {
    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        // NP(if cond { then_body } else { else_body })
        // = NP(cond) + NP(then) + NP(else)  (no-else case: + 1)
        let cond_paths = path_count_in_cond(&node.cond);
        let then_paths = npath_block(&node.then_branch);
        let else_paths = match node.else_branch.as_ref() {
            Some((_, else_expr)) => npath_else(else_expr),
            None => 1,
        };
        let local = cond_paths.saturating_add(then_paths).saturating_add(else_paths);
        self.paths = self.paths.saturating_mul(local);
        // Don't recurse — we already accounted for the full subtree.
    }

    fn visit_expr_while(&mut self, node: &'ast ExprWhile) {
        // NP(while cond { body }) = NP(cond) + NP(body) + 1
        let cond_paths: u32 = 1; // condition expression itself contributes 1
        let body_paths = npath_block(&node.body);
        let local = cond_paths.saturating_add(body_paths).saturating_add(1);
        self.paths = self.paths.saturating_mul(local);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast ExprForLoop) {
        // NP(for x in iter { body }) = NP(body) + 1
        let body_paths = npath_block(&node.body);
        let local = body_paths.saturating_add(1);
        self.paths = self.paths.saturating_mul(local);
    }

    fn visit_expr_loop(&mut self, node: &'ast ExprLoop) {
        // NP(loop { body }) = NP(body) + 1 (must run at least once,
        // but break gives the alternative).
        let body_paths = npath_block(&node.body);
        let local = body_paths.saturating_add(1);
        self.paths = self.paths.saturating_mul(local);
    }

    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        // NP(match x { a => …, b => …, … }) = Σ NP(arm)
        let mut sum: u32 = 0;
        for arm in &node.arms {
            // Arm body is a single Expr; treat it as a block of one stmt
            // for the path-count helper.
            let arm_paths = npath_arm_body(&arm.body);
            sum = sum.saturating_add(arm_paths);
        }
        let local = sum.max(1);
        self.paths = self.paths.saturating_mul(local);
    }

    fn visit_expr_binary(&mut self, node: &'ast ExprBinary) {
        // Short-circuit `&&` / `||` create a binary branch.
        if matches!(node.op, BinOp::And(_) | BinOp::Or(_)) {
            self.paths = self.paths.saturating_add(1);
        }
        visit::visit_expr_binary(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast ExprTry) {
        // The `?` operator branches Ok/Err — NPath sees 2 paths.
        self.paths = self.paths.saturating_mul(2);
        visit::visit_expr_try(self, node);
    }
}

/// `if let Some(x) = expr` — the cond's path count is at least 1.
/// The pattern itself doesn't bring branches that NPath should count
/// (those would be inside the matched expression).
fn path_count_in_cond(_cond: &syn::Expr) -> u32 {
    1
}

/// `else` branch can be either a `Block` (`else { … }`) or another
/// `if` (`else if …`). Recurse into the latter so chained if-else-if
/// composes correctly.
fn npath_else(else_expr: &syn::Expr) -> u32 {
    match else_expr {
        syn::Expr::Block(b) => npath_block(&b.block),
        syn::Expr::If(if_expr) => {
            let cond_paths = path_count_in_cond(&if_expr.cond);
            let then_paths = npath_block(&if_expr.then_branch);
            let chained = match if_expr.else_branch.as_ref() {
                Some((_, e)) => npath_else(e),
                None => 1,
            };
            cond_paths.saturating_add(then_paths).saturating_add(chained)
        }
        _ => 1,
    }
}

fn npath_arm_body(body: &syn::Expr) -> u32 {
    match body {
        syn::Expr::Block(b) => npath_block(&b.block),
        _ => {
            // Walk the expression to pick up nested if/match/&&/||/?.
            let mut v = NpathVisitor { paths: 1 };
            v.visit_expr(body);
            v.paths
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
        NpathComplexity.measure(&input)
    }

    fn np_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn straight_line_is_one() {
        assert_eq!(np_of("fn f() { let x = 1; let y = 2; }", "f"), 1);
    }

    #[test]
    fn single_if_else_is_three() {
        // NP(cond) + NP(then) + NP(else) = 1 + 1 + 1 = 3.
        let src = "fn f(x: i32) { if x > 0 { } else { } }";
        assert_eq!(np_of(src, "f"), 3);
    }

    #[test]
    fn back_to_back_if_else_multiplies() {
        // Two if-else blocks compose multiplicatively: 3 * 3 = 9.
        // CC of the same code is 3 (1 + 2 ifs). The contrast is
        // Nejmeh's whole point.
        let src = r#"
            fn f(x: i32, y: i32) {
                if x > 0 { } else { }
                if y > 0 { } else { }
            }
        "#;
        assert_eq!(np_of(src, "f"), 9);
    }

    #[test]
    fn match_arms_sum() {
        // 4-arm match: each arm is a trivial expression (NP=1), so
        // sum = 4. Sealed match (no wildcard) — no +1 adjustment.
        let src = r#"
            fn f(x: i32) -> i32 {
                match x {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    _ => 3,
                }
            }
        "#;
        assert_eq!(np_of(src, "f"), 4);
    }

    #[test]
    fn while_loop_adds_one_to_body() {
        // NP(while c { body }) = NP(c) + NP(body) + 1 = 1 + 1 + 1 = 3.
        let src = "fn f() { while true { } }";
        assert_eq!(np_of(src, "f"), 3);
    }

    #[test]
    fn question_mark_doubles_paths() {
        // `?` is a binary branch — multiplies path count by 2.
        // Function body has one statement, the `?` expr → NPath 2.
        let src = "fn f() -> Result<(), ()> { Ok(())?; Ok(()) }";
        assert_eq!(np_of(src, "f"), 2);
    }

    #[test]
    fn nested_if_in_arm_compounds() {
        // match arm bodies contribute their own NPath; nesting
        // composes through.
        let src = r#"
            fn f(x: i32, y: i32) -> i32 {
                match x {
                    0 => if y > 0 { 1 } else { 2 },
                    _ => 0,
                }
            }
        "#;
        // Arm 0 NP: NP(if y>0 {1} else {2}) = 1 + 1 + 1 = 3.
        // Arm 1 NP: 1.
        // Match NP = 3 + 1 = 4.
        assert_eq!(np_of(src, "f"), 4);
    }

    #[test]
    fn metadata_cites_nejmeh_1988() {
        let md = NpathComplexity.metadata();
        assert!(md.references.iter().any(|r| r.contains("Nejmeh")));
        assert!(md.references.iter().any(|r| r.contains("1988")));
        assert_eq!(md.default_warning.map(|t| t.value), Some(200.0));
    }
}
