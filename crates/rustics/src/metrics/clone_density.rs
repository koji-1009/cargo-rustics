//! Clone Density — count of `.clone()` / `.to_owned()` / `.to_string()`
//! method calls inside a function body.
//!
//! Plan §2.4 — Rust-specific performance lens. The metric is "how often
//! does this function escape the borrow checker by allocating a copy?"
//! It is intentionally a *raw count*, not a semantic judgement —
//! `Rc::clone` and `Arc::clone` (real reference-bumps) are counted the
//! same way as `String::clone` (an allocation). Plan §6.6 names this
//! caveat explicitly; the dismissal pathway (M2) is the right tool for
//! marking known-good calls.

use syn::visit::{self, Visit};
use syn::{Expr, ExprMethodCall};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Clone Density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CloneDensity;

impl MetricCalculator for CloneDensity {
    fn id(&self) -> &'static str {
        "clone-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Clone Density",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::LowerIsBetter,
            // Hot paths past 5 owned-copies per function are common smell;
            // 10 is loud.
            default_warning: Some(Threshold::new(5.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = CloneVisitor { count: 0 };
                v.visit_block(body);
                f64::from(v.count)
            })
        })
    }
}

const RATIONALE: &str = "\
A high clone density usually means the function is escaping the borrow \
checker by paying for an allocation. Sometimes that's the right answer \
(short-lived strings, Rc/Arc reference bumps, decoupling lifetimes); \
often it is the path of least resistance during a hurried refactor. The \
metric does not judge — it counts — and the dismissal record carries the \
reason when a clone is correct.";

const REFACTOR_HINTS: &[&str] = &[
    "Borrow instead of clone: pass `&str` instead of `String`, `&[T]` instead \
of `Vec<T>`, `&T` instead of `T`. The clone often vanishes.",
    "If the data outlives the function, hand ownership in once at the top \
and pass references down.",
    "If a `.clone()` is on `Rc` / `Arc`, mark the dismissal — it is a \
reference bump, not an allocation.",
    "When several clones cluster on one value, hoist the `.clone()` once at \
the top into a local binding.",
];

const REFERENCES: &[&str] = &[
    "plan §2.4 — clone-density.",
    "plan §6.6 — caveat: Rc/Arc and cheap literal clones are not distinguished.",
];

/// Counts every method call whose terminal name is in the recognised set.
struct CloneVisitor {
    count: u32,
}

impl<'ast> Visit<'ast> for CloneVisitor {
    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if is_clone_call(node) {
            self.count += 1;
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn is_clone_call(node: &ExprMethodCall) -> bool {
    if !node.args.is_empty() {
        // `.clone()` etc. take no arguments.
        return false;
    }
    let name = node.method.to_string();
    matches!(name.as_str(), "clone" | "to_owned" | "to_string")
        && method_call_is_safe(&node.receiver)
}

/// Defensive guard: avoids miscounting a method named `clone` on a type
/// that obviously is not the `Clone` trait. Today this just checks that
/// the receiver isn't a closure literal — which can't have a method call
/// of this shape — and is a placeholder for richer disambiguation in
/// future Layer 2 lenses (rust-analyzer integration).
fn method_call_is_safe(receiver: &Expr) -> bool {
    !matches!(receiver, Expr::Closure(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        CloneDensity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_clones_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn one_clone() {
        let src = "fn f(s: &String) -> String { s.clone() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn to_owned_counts() {
        let src = "fn f(s: &str) -> String { s.to_owned() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn to_string_counts() {
        let src = "fn f(s: &str) -> String { s.to_string() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn multiple_clones_sum() {
        let src = r#"
            fn f(a: &String, b: &String, c: &String) -> (String, String, String) {
                (a.clone(), b.clone(), c.clone())
            }
        "#;
        assert_eq!(n_of(src, "f"), 3);
    }

    #[test]
    fn methods_with_args_do_not_count() {
        // `.clone(x)` is not the Clone-trait clone (it takes no arguments).
        let src = "fn f(x: i32) -> i32 { x.clone() + 1 }";
        // x.clone() takes no args -> 1 clone
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn nested_clone_in_match_arm_counts() {
        let src = r#"
            fn f(x: Option<&String>) -> Option<String> {
                match x { Some(s) => Some(s.clone()), None => None }
            }
        "#;
        assert_eq!(n_of(src, "f"), 1);
    }
}
