//! Await Depth — longest contiguous chain of `.await` inside a single
//! expression tree.
//!
//! + §6.1 — Rust-specific ergonomics lens. The signal counts
//! `.await` operators that feed into each other within one expression
//! (`a().await.b().await` is depth 2). Sequential awaits across separate
//! statements (`let x = a().await; let y = b().await;`) each contribute
//! depth 1 — the metric is *nested* awaits, not the total.

use syn::visit::{self, Visit};
use syn::Expr;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Await Depth calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct AwaitDepth;

impl MetricCalculator for AwaitDepth {
    fn id(&self) -> &'static str {
        "await-depth"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Await Depth (nested only)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(5.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = AwaitVisitor { max: 0 };
                v.visit_block(body);
                f64::from(v.max)
            })
        })
    }
}

const RATIONALE: &str = "\
Each `.await` is a suspension point; nested awaits within one expression \
mean the function is composing several async operations into a single \
sequenced computation. The metric does not penalise sequential awaits — \
those are a flat list, no harder to read than a flat list of statements. \
Past three links the chain becomes hard to reason about for cancellation \
and error propagation.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull each `.await` into its own `let` binding. Each step gets a name \
and the chain flattens.",
    "If the awaits compose a pipeline, consider an explicit combinator \
(`futures::join!`, `tokio::try_join!`) so the parallel structure is visible.",
    "When the chain mixes `Result` and `Future`, the `await?` form is \
shorthand for two operations — splitting them often makes the error \
handling clearer.",
];

const REFERENCES: &[&str] = &[
];

/// Walks expressions, recording the deepest `.await` chain found.
struct AwaitVisitor {
    max: u32,
}

impl<'ast> Visit<'ast> for AwaitVisitor {
    fn visit_expr(&mut self, node: &'ast Expr) {
        let depth = chain_depth_at(node);
        if depth > self.max {
            self.max = depth;
        }
        visit::visit_expr(self, node);
    }
}

/// `chain_depth_at(e)` is the number of `.await` operators stacked at
/// the top of the chain rooted at `e`. The chain follows `.await`,
/// method calls, field access, `?`, and bracket wrappers.
fn chain_depth_at(expr: &Expr) -> u32 {
    match expr {
        Expr::Await(a) => 1 + chain_depth_at(&a.base),
        Expr::MethodCall(m) => chain_depth_at(&m.receiver),
        Expr::Field(f) => chain_depth_at(&f.base),
        Expr::Try(t) => chain_depth_at(&t.expr),
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
        AwaitDepth.measure(&input)
    }

    fn d_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_await_is_zero() {
        assert_eq!(d_of("async fn f() {}", "f"), 0);
    }

    #[test]
    fn single_await_is_one() {
        let src = "async fn f<F: std::future::Future<Output = i32>>(g: F) -> i32 { g.await }";
        assert_eq!(d_of(src, "f"), 1);
    }

    #[test]
    fn sequential_awaits_each_one() {
        let src = r#"
            async fn f() {
                let _a = std::future::ready(1).await;
                let _b = std::future::ready(2).await;
                let _c = std::future::ready(3).await;
            }
        "#;
        assert_eq!(d_of(src, "f"), 1);
    }

    #[test]
    fn nested_awaits_via_method_chain() {
        // `a().await.b().await` — two awaits, the outer one's receiver
        // chain contains the inner.
        let src = r#"
            async fn f() {
                let _x = bar().await.baz().await;
            }
            async fn bar() -> Quux { todo!() }
            struct Quux;
            impl Quux { async fn baz(self) -> i32 { 0 } }
        "#;
        assert_eq!(d_of(src, "f"), 2);
    }

    #[test]
    fn await_inside_call_arg_does_not_chain_via_args() {
        // `outer(inner.await).await` — the inner await is in an argument,
        // not in the receiver chain. The chain at the outer await is 1.
        // The inner await is at depth 1 too.
        let src = r#"
            async fn f() {
                let _x = identity(bar().await).await;
            }
            async fn bar() -> i32 { 0 }
            async fn identity(x: i32) -> i32 { x }
        "#;
        assert_eq!(d_of(src, "f"), 1);
    }
}
