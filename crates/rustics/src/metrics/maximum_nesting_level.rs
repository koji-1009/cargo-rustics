//! Maximum Nesting Level — early-return-aware (plan §2.5).
//!
//! Counts the deepest nesting reached inside a function body. Each entry
//! into an `if` / `while` / `for` / `loop` / `match` body adds `+1`. Two
//! Rust-aware refinements:
//!
//! * **`else if` is flat.** A chain `if a {} else if b {} else if c {}`
//!   reads as a 3-way switch, not 3-deep nesting. We treat the else-if
//!   tail as a sibling of the original `if`, not as a nested block.
//! * **Early-return `if let` doesn't deepen.** When the `else` branch
//!   diverges (`return`, `break`, `panic!`, `bail!`, etc.) the then-branch
//!   is the linear continuation of the function — it would have been a
//!   `let-else` if the language allowed pattern binding there. We skip
//!   the depth penalty for the then-branch in that case (plan §2.5).
//!
//! `let-else` (`let Some(x) = expr else { … };`) is *never* a depth
//! contributor on its own — it is a statement, not a wrapping block.
//! The visitor recurses through it without entering any new depth.

use syn::visit::{self, Visit};
use syn::{
    Block, Expr, ExprBlock, ExprForLoop, ExprIf, ExprLoop, ExprMatch, ExprWhile, Macro, Stmt,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Maximum Nesting Level calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MaximumNestingLevel;

impl MetricCalculator for MaximumNestingLevel {
    fn id(&self) -> &'static str {
        "maximum-nesting-level"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Maximum Nesting Level (early-return-aware)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // 4 is the standard "deeply nested" threshold; past 6 is hard
            // to hold in working memory at all.
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(6.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = NestingVisitor { depth: 0, max: 0 };
                v.visit_block(body);
                f64::from(v.max)
            })
        })
    }
}

const RATIONALE: &str = "\
Deep nesting forces a reader to keep more context in working memory. The \
top-level `if` is one fact, the inner `for` is another, the conditional \
inside the loop is a third — past 4 levels, unwinding the meaning back to \
the function's intent costs real attention. The Rust adjustment makes \
`if let X else { return }` (and `else if` chains) read as flat switches, \
which is what they semantically are.";

const REFACTOR_HINTS: &[&str] = &[
    "Lift `if let X else { return }` style guards to the top of the function. \
The body that follows stays linear and the metric drops.",
    "Extract the inner-most loop or block into a helper. The deepest level \
becomes the helper's depth-1 body; the call site flattens.",
    "Replace nested `match` with `if let` early-return guards followed by a \
flat `match` at the function's top level.",
    "Use `?` on `Result` / `Option` instead of `match` + `return Err(...)`.",
];

const REFERENCES: &[&str] = &["plan §2.5 — early-return-aware adjustment."];

/// Walks a function body tracking current and maximum depth.
struct NestingVisitor {
    depth: u32,
    max: u32,
}

impl NestingVisitor {
    fn enter(&mut self) {
        self.depth += 1;
        if self.depth > self.max {
            self.max = self.depth;
        }
    }

    fn exit(&mut self) {
        self.depth -= 1;
    }

    fn deepen(&mut self, walk: impl FnOnce(&mut Self)) {
        self.enter();
        walk(self);
        self.exit();
    }
}

impl<'ast> Visit<'ast> for NestingVisitor {
    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        if has_diverging_else(node) {
            // Early-return makes the whole `if let X else { return }` a
            // pass-through — the same shape `let-else` would have if Rust
            // allowed pattern binding here (plan §2.5). Walk both arms at
            // the current depth.
            self.visit_block(&node.then_branch);
            if let Some((_, else_expr)) = &node.else_branch {
                visit::visit_expr(self, else_expr);
            }
            return;
        }
        self.deepen(|v| v.visit_block(&node.then_branch));
        walk_else(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast ExprWhile) {
        self.deepen(|v| v.visit_block(&node.body));
    }

    fn visit_expr_for_loop(&mut self, node: &'ast ExprForLoop) {
        self.deepen(|v| v.visit_block(&node.body));
    }

    fn visit_expr_loop(&mut self, node: &'ast ExprLoop) {
        self.deepen(|v| v.visit_block(&node.body));
    }

    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        // The match itself is +1; arms are at this same depth, so we
        // walk arm bodies directly without `enter`-ing again.
        self.deepen(|v| {
            for arm in &node.arms {
                v.visit_expr(&arm.body);
            }
        });
    }
}

/// Walks the `else` of an `if` expression, treating `else if` as a flat
/// sibling and `else { … }` as a +1-depth block.
fn walk_else(v: &mut NestingVisitor, node: &ExprIf) {
    let Some((_, else_expr)) = &node.else_branch else {
        return;
    };
    match else_expr.as_ref() {
        // `else if` — flat sibling, recurse without adding depth here.
        Expr::If(_) => visit::visit_expr(v, else_expr),
        // `else { … }` — explicit block, +1 depth.
        Expr::Block(_) => v.deepen(|inner| visit::visit_expr(inner, else_expr)),
        _ => visit::visit_expr(v, else_expr),
    }
}

/// True iff the `else` branch of `node` is an explicit block whose tail
/// expression diverges (return / break / continue / `panic!`-class macro).
fn has_diverging_else(node: &ExprIf) -> bool {
    let Some((_, else_expr)) = &node.else_branch else {
        return false;
    };
    match else_expr.as_ref() {
        Expr::Block(ExprBlock { block, .. }) => block_diverges(block),
        _ => false,
    }
}

fn block_diverges(block: &Block) -> bool {
    block.stmts.last().is_some_and(stmt_diverges)
}

fn stmt_diverges(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(expr, _) => expr_diverges(expr),
        Stmt::Macro(m) => is_divergent_macro(&m.mac),
        _ => false,
    }
}

fn expr_diverges(expr: &Expr) -> bool {
    match expr {
        Expr::Return(_) | Expr::Break(_) | Expr::Continue(_) => true,
        Expr::Macro(m) => is_divergent_macro(&m.mac),
        Expr::Block(b) => block_diverges(&b.block),
        Expr::Unsafe(u) => block_diverges(&u.block),
        _ => false,
    }
}

/// Recognises the small, well-known set of "this expression diverges"
/// macros from `std` and `anyhow`-style error crates.
fn is_divergent_macro(m: &Macro) -> bool {
    let last_seg = m.path.segments.last().map(|s| s.ident.to_string());
    matches!(
        last_seg.as_deref(),
        Some("panic")
            | Some("unreachable")
            | Some("todo")
            | Some("unimplemented")
            | Some("bail")
            | Some("ensure")
            | Some("abort"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        MaximumNestingLevel.measure(&input)
    }

    fn depth_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn flat_function_is_zero() {
        assert_eq!(depth_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn single_if_is_one() {
        assert_eq!(depth_of("fn f(x: bool) { if x {} }", "f"), 1);
    }

    #[test]
    fn nested_if_is_two() {
        let src = "fn f(x: bool, y: bool) { if x { if y {} } }";
        assert_eq!(depth_of(src, "f"), 2);
    }

    #[test]
    fn for_inside_if_inside_for_is_three() {
        let src = "fn f() { for _ in 0..1 { if true { for _ in 0..1 {} } } }";
        assert_eq!(depth_of(src, "f"), 3);
    }

    #[test]
    fn else_if_chain_is_flat() {
        let src = "fn f(x: i32) { if x == 0 {} else if x == 1 {} else if x == 2 {} else {} }";
        assert_eq!(depth_of(src, "f"), 1);
    }

    #[test]
    fn else_block_is_sibling_of_then() {
        let src = "fn f(x: bool) { if x { let _y = 1; } else { let _y = 2; } }";
        assert_eq!(depth_of(src, "f"), 1);
    }

    #[test]
    fn match_arms_are_at_same_depth_as_match() {
        let src = "fn f(x: i32) -> i32 { match x { 0 => 0, 1 => 1, _ => 2 } }";
        assert_eq!(depth_of(src, "f"), 1);
    }

    #[test]
    fn if_inside_match_arm_adds_one() {
        let src = "fn f(x: i32) -> i32 { match x { 0 => if x > 0 { 1 } else { 0 }, _ => 2 } }";
        // match=+1; if inside arm body=+1 -> 2.
        assert_eq!(depth_of(src, "f"), 2);
    }

    #[test]
    fn early_return_else_absorbs_then_depth() {
        let src = r#"
            fn f(x: Option<i32>) -> i32 {
                if let Some(_v) = x {
                    let _y = 1;
                } else {
                    return 0;
                }
                42
            }
        "#;
        // The else returns -> then-branch is linear continuation, no depth.
        assert_eq!(depth_of(src, "f"), 0);
    }

    #[test]
    fn panic_else_absorbs_then_depth() {
        let src = r#"
            fn f(x: Option<i32>) -> i32 {
                if let Some(_v) = x {
                    let _y = 1;
                } else {
                    panic!("never");
                }
                42
            }
        "#;
        assert_eq!(depth_of(src, "f"), 0);
    }

    #[test]
    fn ordinary_else_does_not_absorb() {
        let src = r#"
            fn f(x: Option<i32>) -> i32 {
                if let Some(_v) = x {
                    let _y = 1;
                } else {
                    let _y = 2;
                }
                42
            }
        "#;
        assert_eq!(depth_of(src, "f"), 1);
    }

    #[test]
    fn let_else_is_transparent() {
        // let-else doesn't wrap subsequent code, so it doesn't add depth
        // even though it has an else block.
        let src = r#"
            fn f(x: Option<i32>) -> i32 {
                let Some(_v) = x else { return 0 };
                42
            }
        "#;
        assert_eq!(depth_of(src, "f"), 0);
    }
}
