//! Cognitive Complexity — SonarSource 2018.
//!
//! Each control-flow break adds `+1`; structures that *nest*
//! their bodies add an additional bonus equal to the current nesting
//! level. Sequential structures (`else if`, `else`) get the `+1` only.
//!
//! # Increments
//!
//! * `if` (initial)              → `+1 + nesting`; body at nesting+1
//! * `else if`                   → `+1` (sequential — no nesting bonus)
//! * `else`                      → `+1` (sequential — no nesting bonus)
//! * `while` / `for` / `loop`    → `+1 + nesting`; body at nesting+1
//! * `match`                     → `+1 + nesting`; arms at nesting+1
//! * `&&` / `||`                 → `+1` per *run* + `+1` per kind switch
//! * labelled `break` / `continue` → `+1`
//! * closures (`|...| { ... }`)  → `+0`; body at nesting+1
//!
//! # Boolean operator runs (SonarSource transition rule)
//!
//! In one boolean expression, every *run* of same-kind operators counts
//! as one increment, and every *switch* between `&&` and `||` adds
//! another. So `a && b && c` is `+1`, `a && b || c` is `+2`, `a && b ||
//! c && d` is `+3`. The walk recurses only through bool-operator
//! children when collecting the run sequence — non-bool subexpressions
//! (e.g. `f(x && y)` inside `a || b`) are visited normally so a nested
//! bool chain inside a function-call argument is counted as its own
//! chain rather than absorbed into the outer one.
//!
//! # Direct recursion
//!
//! SonarSource charges `+1` per direct recursive call. We detect two
//! shapes — both Layer-1 friendly:
//!
//! * `<name>(...)` — bare path call whose only segment is the
//!   enclosing function's name.
//! * `Self::<name>(...)` and `self.<name>(...)` — receiver-based
//!   recursion on a method.
//!
//! Module-prefixed self-calls (`crate::foo::f()`) need name resolution
//! to disambiguate from same-named functions in other modules; they
//! are not caught at Layer 1.

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
                let fn_name = frame.signature.ident.to_string();
                let mut v = CogVisitor {
                    fn_name,
                    ..CogVisitor::default()
                };
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
];

/// Sonar-style cognitive-complexity counter.
#[derive(Default)]
struct CogVisitor {
    nesting: u32,
    total: u32,
    /// `true` when this `Expr::If` is being visited as an `else if` of an
    /// outer `if` chain — it gets the sequential `+1`, not `+1 + nesting`.
    is_else_if: bool,
    /// Name of the enclosing function. Used by the direct-recursion
    /// detector to charge `+1` when the body calls itself by name.
    /// (SonarSource direct-recursion rule).
    fn_name: String,
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
        if let Some(kind) = bool_op(&node.op) {
            // SonarSource transition rule: walk the entire boolean
            // chain rooted here, count every place the operator
            // *changes*, plus +1 for the chain itself. The chain
            // forms a tree of ExprBinary; we recurse only through
            // bool-op children, leaving non-bool subexpressions to
            // the outer visitor.
            //
            // Suppress double-counting by *not* recursing into bool
            // children with the default visitor — instead, walk the
            // boolean tree once via `walk_bool` and recurse with
            // `visit::visit_expr` on the non-bool leaves.
            self.total += count_bool_switches(node, kind);
            walk_bool_subexpressions(self, node);
            return;
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

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if call_targets_name(&node.func, &self.fn_name) {
            self.total += 1;
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        // `self.<name>(...)` — direct recursion on a method.
        if node.method == self.fn_name && is_self_receiver(&node.receiver) {
            self.total += 1;
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast ExprClosure) {
        // Closures don't add their own +1 (Sonar treats them as scopes,
        // not branches), but their body contributes at one level deeper.
        self.deepen(|v| visit::visit_expr_closure(v, node));
    }
}

/// True iff `func` is a path expression `<name>` or `Self::<name>` —
/// the two shapes a direct-recursive call can take in Rust without
/// type information. Module-prefixed self-calls (`crate::foo::f()`)
/// are *not* covered; that needs name resolution.
fn call_targets_name(func: &syn::Expr, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let syn::Expr::Path(p) = func else {
        return false;
    };
    let segs: Vec<_> = p.path.segments.iter().collect();
    match segs.as_slice() {
        [only] => only.ident == name,
        [first, second] => first.ident == "Self" && second.ident == name,
        _ => false,
    }
}

/// True iff `expr` is the literal `self` receiver.
fn is_self_receiver(expr: &syn::Expr) -> bool {
    let syn::Expr::Path(p) = expr else {
        return false;
    };
    p.path.is_ident("self")
}

/// Identifier for the two boolean operators we care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoolKind {
    And,
    Or,
}

fn bool_op(op: &BinOp) -> Option<BoolKind> {
    match op {
        BinOp::And(_) => Some(BoolKind::And),
        BinOp::Or(_) => Some(BoolKind::Or),
        _ => None,
    }
}

/// Counts the number of distinct *runs* of same-kind operators in the
/// boolean tree rooted at `node`. SonarSource: a run is `+1`,
/// transitions add another `+1` per switch.
///
/// `outer` is the operator that brought us into this chain at the call
/// site; we count it as the "current" run kind at entry.
fn count_bool_switches(node: &ExprBinary, outer: BoolKind) -> u32 {
    let mut ops = Vec::new();
    collect_bool_ops(&syn::Expr::Binary(node.clone()), &mut ops);
    if ops.is_empty() {
        // Shouldn't happen — we entered with a bool op — but be defensive.
        return 1;
    }
    // Force the outer op into the sequence so SonarSource's "first run"
    // rule fires correctly even when the first leaf disagreed.
    let _ = outer;
    let mut runs = 1u32;
    for win in ops.windows(2) {
        if win[0] != win[1] {
            runs += 1;
        }
    }
    runs
}

/// Walks `expr` collecting bool operators in inorder. Non-bool
/// subexpressions stop the walk for collection purposes.
fn collect_bool_ops(expr: &syn::Expr, out: &mut Vec<BoolKind>) {
    if let syn::Expr::Binary(b) = expr {
        if let Some(kind) = bool_op(&b.op) {
            collect_bool_ops(&b.left, out);
            out.push(kind);
            collect_bool_ops(&b.right, out);
        }
    }
}

/// Walks the *non-bool* subexpressions of a boolean tree so the rest
/// of the visitor sees nested control flow (e.g. an `if` inside a
/// short-circuit operand).
fn walk_bool_subexpressions(v: &mut CogVisitor, node: &ExprBinary) {
    walk_bool_subtree(v, &node.left);
    walk_bool_subtree(v, &node.right);
}

fn walk_bool_subtree(v: &mut CogVisitor, expr: &syn::Expr) {
    if let syn::Expr::Binary(b) = expr {
        if bool_op(&b.op).is_some() {
            walk_bool_subexpressions(v, b);
            return;
        }
    }
    visit::visit_expr(v, expr);
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
    fn boolean_runs_collapse() {
        // Same-kind sequence is one run -> +1.
        assert_eq!(
            cc_of(
                "fn f(a: bool, b: bool, c: bool) -> bool { a && b && c }",
                "f"
            ),
            1
        );
    }

    #[test]
    fn boolean_transition_charges_extra_run() {
        // && then || -> 2 runs.
        assert_eq!(
            cc_of(
                "fn f(a: bool, b: bool, c: bool) -> bool { a && b || c }",
                "f"
            ),
            2
        );
    }

    #[test]
    fn boolean_two_transitions() {
        // && || && -> 3 runs.
        assert_eq!(
            cc_of(
                "fn f(a: bool, b: bool, c: bool, d: bool) -> bool { a && b || c && d }",
                "f"
            ),
            3
        );
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
    fn direct_recursion_charges_one() {
        // SonarSource direct-recursion rule. Pure recursion (no
        // surrounding `if`) so we can check the recursion-only delta.
        let src = "fn f(x: i32) -> i32 { f(x - 1) }";
        // No control flow + 1 for the recursive call.
        assert_eq!(cc_of(src, "f"), 1);
    }

    #[test]
    fn self_dot_method_recursion_charges_one() {
        let src = r#"
            struct S;
            impl S { fn f(&self, x: i32) -> i32 { self.f(x - 1) } }
        "#;
        assert_eq!(cc_of(src, "S::f"), 1);
    }

    #[test]
    fn self_capital_path_recursion_charges_one() {
        // `Self::f(...)` is also direct recursion.
        let src = r#"
            struct S;
            impl S { fn f(x: i32) -> i32 { Self::f(x - 1) } }
        "#;
        assert_eq!(cc_of(src, "S::f"), 1);
    }

    #[test]
    fn calls_to_other_functions_do_not_charge() {
        let src = "fn f() -> i32 { other_fn() } fn other_fn() -> i32 { 1 }";
        assert_eq!(cc_of(src, "f"), 0);
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
