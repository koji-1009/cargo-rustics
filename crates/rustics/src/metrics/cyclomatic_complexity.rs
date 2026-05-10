//! Cyclomatic Complexity (McCabe 1976) — `1 + d` where `d` counts
//! decision points in a function body.
//!
//! Decision points (each contributes +1):
//!
//! * `if` / `else if` / `if let`
//! * `while` / `while let`
//! * `for`
//! * `loop`
//! * `match` arms beyond the first **only when a wildcard arm
//!   exists** (sealed-aware: an exhaustive match without `_` does
//!   not count, because the compiler enforces the exhaustiveness
//!   that CC was designed to flag).
//! * `&&` / `||`
//! * `?` (binary `Ok` / `Err` branch)
//!
//! References:
//! * McCabe, T. J. (1976). A Complexity Measure. IEEE TSE.

use ra_ap_syntax::{
    ast::{self, AstNode, BinaryOp, LogicOp, Pat},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// CC calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CyclomaticComplexity;

impl MetricCalculator for CyclomaticComplexity {
    fn id(&self) -> &'static str {
        "cyclomatic-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Cyclomatic Complexity (McCabe 1976)",
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
        measure_functions(input.tree, |frame| {
            frame.item.body().map(|body| f64::from(count_cc(body.syntax())))
        })
    }
}

fn count_cc(node: &SyntaxNode) -> u32 {
    let mut cc = 1; // baseline
    for desc in node.descendants() {
        cc += node_contribution(desc);
    }
    cc
}

fn node_contribution(node: SyntaxNode) -> u32 {
    if matches!(
        node.kind(),
        SyntaxKind::IF_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::FOR_EXPR
            | SyntaxKind::LOOP_EXPR
            | SyntaxKind::TRY_EXPR
    ) {
        return 1;
    }
    if node.kind() == SyntaxKind::MATCH_EXPR {
        return ast::MatchExpr::cast(node)
            .map(|m| match_arms_contribution(&m))
            .unwrap_or(0);
    }
    if node.kind() == SyntaxKind::BIN_EXPR {
        return ast::BinExpr::cast(node).map(bin_logic).unwrap_or(0);
    }
    0
}

fn bin_logic(b: ast::BinExpr) -> u32 {
    matches!(
        b.op_kind(),
        Some(BinaryOp::LogicOp(LogicOp::And)) | Some(BinaryOp::LogicOp(LogicOp::Or))
    ) as u32
}

/// Sealed-aware: contributes (arm_count - 1) only when a `_`
/// wildcard arm is present; otherwise 0.
fn match_arms_contribution(m: &ast::MatchExpr) -> u32 {
    let Some(arm_list) = m.match_arm_list() else {
        return 0;
    };
    let arms: Vec<_> = arm_list.arms().collect();
    if arms.is_empty() {
        return 0;
    }
    let has_wildcard = arms
        .iter()
        .any(|a| a.pat().is_some_and(|p| matches!(p, Pat::WildcardPat(_))));
    if has_wildcard {
        (arms.len() as u32).saturating_sub(1)
    } else {
        0
    }
}

const RATIONALE: &str = "\
Cyclomatic Complexity counts independent execution paths in a function. \
McCabe established 10 as the empirical break-even where defect rates start \
climbing. We extend with a Rust-specific adjustment: a `match` whose subject \
is an exhaustive enum (no `_` arm) is *not* charged for arms — the compiler \
enforces exhaustiveness, so the cognitive risk McCabe targeted (a missed \
case) does not exist.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract independent branches into named helpers — each is a unit the \
reader can scan separately.",
    "Replace nested `if`/`else` chains with a `match` on a small enum. \
Sealed-aware CC charges 0 for the new match.",
    "Lift early-exit checks (`return Err(...)` / `return None`) so the \
function reads as a happy-path sequence with guards on top.",
];

const REFERENCES: &[&str] = &[
    "McCabe, T. J. (1976). A Complexity Measure. IEEE Transactions on Software Engineering, 2(4), 308-320.",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        CyclomaticComplexity.measure(&input)
    }

    #[test]
    fn simple_fn_is_cc_one() {
        let m = measure("fn f() {}");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].value, 1.0);
    }

    #[test]
    fn if_adds_one() {
        let m = measure("fn f(x: i32) -> i32 { if x > 0 { 1 } else { 0 } }");
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn else_if_chain_adds_per_branch() {
        let src = "fn f(x: i32) -> i32 { \
                   if x > 0 { 1 } else if x < 0 { -1 } else { 0 } \
                   }";
        assert_eq!(measure(src)[0].value, 3.0);
    }

    #[test]
    fn while_for_loop_each_add_one() {
        let src = "fn f(xs: &[i32]) { \
                   while xs.is_empty() {} \
                   for _ in xs {} \
                   loop {} \
                   }";
        assert_eq!(measure(src)[0].value, 4.0);
    }

    #[test]
    fn try_op_adds_one() {
        let src = "fn f() -> Result<i32, ()> { \"1\".parse::<i32>().map_err(|_| ())?; Ok(0) }";
        assert_eq!(measure(src)[0].value, 2.0);
    }

    #[test]
    fn sealed_match_contributes_zero() {
        let src = "enum E { A, B } \
                   fn f(e: E) -> i32 { match e { E::A => 1, E::B => 2 } }";
        let m = measure(src);
        let f = m.iter().find(|x| x.scope.path == "f").unwrap();
        assert_eq!(f.value, 1.0);
    }

    #[test]
    fn match_with_wildcard_charges_arms_minus_one() {
        let src = "fn f(x: i32) -> i32 { match x { 1 => 1, 2 => 2, _ => 0 } }";
        assert_eq!(measure(src)[0].value, 3.0);
    }

    #[test]
    fn logic_ops_each_add_one() {
        let src = "fn f(a: bool, b: bool, c: bool) -> bool { a && b || c }";
        assert_eq!(measure(src)[0].value, 3.0);
    }

    #[test]
    fn impl_methods_get_their_own_measurement() {
        let src = "struct Foo; impl Foo { \
                   fn a(&self) {} \
                   fn b(&self, x: i32) -> i32 { if x > 0 { 1 } else { 0 } } \
                   }";
        let m = measure(src);
        let a = m.iter().find(|x| x.scope.path == "Foo::a").unwrap();
        let b = m.iter().find(|x| x.scope.path == "Foo::b").unwrap();
        assert_eq!(a.value, 1.0);
        assert_eq!(b.value, 2.0);
    }

    #[test]
    fn module_prefixes_scope_path() {
        let src = "mod inner { pub fn deep() { if true {} } }";
        let m = measure(src);
        let f = m.iter().find(|x| x.scope.path == "inner::deep").unwrap();
        assert_eq!(f.value, 2.0);
    }
}
