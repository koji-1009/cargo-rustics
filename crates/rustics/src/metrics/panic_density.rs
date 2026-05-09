//! Panic Density — count of `panic!`, `.unwrap()`, `.expect(...)`, and the
//! divergent siblings (`unreachable!`, `todo!`, `unimplemented!`,
//! `assert!`-class).
//!
//! + §2.5 — Rust-specific safety lens. The signal is "how many
//! places in this function will abort the program at runtime if the wrong
//! input arrives". The §2.5 calibration excludes `unwrap_or_*` family
//! members because those *cannot* panic (the name is `unwrap_or_default`,
//! `unwrap_or_else`, …, but the path is panic-impossible).
//!
//! caveat:
//!
//! * production-vs-test distinction is M2 (`test: true` mode skips
//!   `#[cfg(test)]` and `#[test]` bodies). Today the count is global.

use syn::visit::{self, Visit};
use syn::{ExprMethodCall, Macro};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Panic Density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanicDensity;

impl MetricCalculator for PanicDensity {
    fn id(&self) -> &'static str {
        "panic-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Panic Density (unwrap_or-aware)",
            category: MetricCategory::RustSafety,
            polarity: MetricPolarity::LowerIsBetter,
            // Three panicking paths in one function is the "stop and
            // think" threshold; ten is "this needs an error type".
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            // caveat: production-vs-test split. Test bodies are
            // expected to assert and unwrap — that is the test framework's
            // language — so we don't measure them.
            if frame.is_test() {
                return None;
            }
            frame.body.map(|body| {
                let mut v = PanicVisitor { count: 0 };
                v.visit_block(body);
                f64::from(v.count)
            })
        })
    }
}

const RATIONALE: &str = "\
Each panicking site is a runtime crash waiting for the wrong input. \
`.unwrap()` and `.expect()` look small but they are the same as `panic!()` \
under bad data. A high count says the function is not modelling its error \
cases — it is hoping. The unwrap_or-aware adjustment keeps the lens from \
flagging defensible APIs (`.unwrap_or_default()`, `.unwrap_or_else(...)`) \
that cannot panic by construction.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace `.unwrap()` on `Option` with `.unwrap_or(default)` or \
`.ok_or(error)?`. The `?` keeps the linear shape.",
    "Replace `.expect(\"…\")` on `Result` with `?` and let the caller see the \
real error.",
    "If the panic represents an internal invariant the function genuinely \
guarantees, leave it but document the invariant in a `// SAFETY:` comment \
and consider a `debug_assert!` instead.",
    "Wrap repeated panics into one early-return guard (`let-else`) at the \
top of the function.",
];

const REFERENCES: &[&str] = &[
];

/// Walks a body counting panicking call/macro sites.
struct PanicVisitor {
    count: u32,
}

impl<'ast> Visit<'ast> for PanicVisitor {
    /// `visit_macro` fires for both `Stmt::Macro` (statement-position) and
    /// `Expr::Macro` (expression-position); we don't need separate handlers
    /// for each.
    fn visit_macro(&mut self, node: &'ast Macro) {
        if is_panicking_macro(node) {
            self.count += 1;
        }
        visit::visit_macro(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if is_panicking_method(node) {
            self.count += 1;
        }
        visit::visit_expr_method_call(self, node);
    }
}

/// True iff this method call is `.unwrap()` or `.expect("…")` on any
/// receiver. Other unwrap_or-family members are *not* counted.
fn is_panicking_method(node: &ExprMethodCall) -> bool {
    let name = node.method.to_string();
    match name.as_str() {
        "unwrap" => node.args.is_empty(),
        "expect" => node.args.len() == 1,
        _ => false,
    }
}

/// True iff `m`'s last path segment is a panicking macro from std or
/// the well-known error crates (`anyhow!`, `bail!`, `ensure!`,
/// `assert!`/`assert_eq!`/`assert_ne!`, …).
fn is_panicking_macro(m: &Macro) -> bool {
    let last_seg = m.path.segments.last().map(|s| s.ident.to_string());
    matches!(
        last_seg.as_deref(),
        Some("panic")
            | Some("unreachable")
            | Some("todo")
            | Some("unimplemented")
            | Some("assert")
            | Some("assert_eq")
            | Some("assert_ne")
            | Some("debug_assert")
            | Some("debug_assert_eq")
            | Some("debug_assert_ne")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        PanicDensity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_panic_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn unwrap_counts() {
        let src = "fn f(o: Option<i32>) -> i32 { o.unwrap() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn expect_counts() {
        let src = "fn f(o: Option<i32>) -> i32 { o.expect(\"present\") }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn unwrap_or_does_not_count() {
        let src = "fn f(o: Option<i32>) -> i32 { o.unwrap_or(0) }";
        assert_eq!(n_of(src, "f"), 0);
    }

    #[test]
    fn unwrap_or_default_does_not_count() {
        let src = "fn f(o: Option<i32>) -> i32 { o.unwrap_or_default() }";
        assert_eq!(n_of(src, "f"), 0);
    }

    #[test]
    fn unwrap_or_else_does_not_count() {
        let src = "fn f(o: Option<i32>) -> i32 { o.unwrap_or_else(|| 0) }";
        assert_eq!(n_of(src, "f"), 0);
    }

    #[test]
    fn panic_macro_counts() {
        let src = "fn f() -> i32 { panic!(\"oh no\"); }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn unreachable_counts() {
        let src = "fn f() -> i32 { unreachable!(); }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn todo_counts() {
        let src = "fn f() -> i32 { todo!() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn assert_counts() {
        let src = "fn f(x: i32) { assert!(x > 0); }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn multiple_sources_sum() {
        let src = r#"
            fn f(a: Option<i32>, b: Result<i32, ()>) -> i32 {
                let _x = a.unwrap();
                let _y = b.expect("ok");
                if false { panic!("uh") } else { 0 }
            }
        "#;
        assert_eq!(n_of(src, "f"), 3);
    }
}
