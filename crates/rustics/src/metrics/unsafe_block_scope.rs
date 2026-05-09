//! Unsafe Block Scope — total lines of `unsafe { … }` blocks inside a
//! function body.
//!
//! Rust-specific safety lens. The signal is "how much
//! of this function lives behind the unsafe contract", measured in raw
//! source lines. Multiple unsafe blocks in one function add up.
//!
//! caveats:
//!
//! * Self-only — we never crawl dependencies or rustc build artefacts;
//!   that is `cargo-geiger`'s job. M3 may revisit a `--with-geiger` flag.
//! * `unsafe fn` *bodies* are not measured — only the syntactic
//!   `unsafe { ... }` blocks. The plan calls this out explicitly.
//! * FFI call counting (a second axis the mentions) is M2; the
//!   M1 lens reports lines only.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::ExprUnsafe;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Unsafe Block Scope calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsafeBlockScope;

impl MetricCalculator for UnsafeBlockScope {
    fn id(&self) -> &'static str {
        "unsafe-block-scope"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Unsafe Block Scope (lines)",
            category: MetricCategory::RustSafety,
            polarity: MetricPolarity::LowerIsBetter,
            // 20 lines of unsafe in one function is already a lot; past
            // 50 the soundness review burden is heavy.
            default_warning: Some(Threshold::new(20.0)),
            default_error: Some(Threshold::new(50.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = UnsafeVisitor { lines: 0 };
                v.visit_block(body);
                f64::from(v.lines)
            })
        })
    }
}

const RATIONALE: &str = "\
Every line inside an `unsafe { … }` block is a soundness obligation: the \
author has promised the compiler that the operation upholds invariants the \
type system cannot check. Long unsafe blocks scale that promise — five \
lines you can audit; fifty lines you cannot. The metric pushes you to \
keep unsafe locally tight so the surrounding safe code can be trusted on \
its types alone.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull the unsafe block down to the smallest possible expression. The \
surrounding safe code does not need the contract.",
    "Wrap the unsafe operation in a safe abstraction (a small helper that \
returns a checked result) and call it from safe code.",
    "If the same unsafe operation appears in multiple places, extract a \
single safe wrapper; the audit surface stays in one file.",
];

const REFERENCES: &[&str] = &[
];

/// Walks a body and accumulates the line span of every `unsafe { … }` block.
struct UnsafeVisitor {
    lines: u32,
}

impl<'ast> Visit<'ast> for UnsafeVisitor {
    fn visit_expr_unsafe(&mut self, node: &'ast ExprUnsafe) {
        self.lines += block_line_span(&node.block);
        // Recurse — nested closures or expressions inside the unsafe
        // block can themselves contain further structures the lens does
        // not care about, but the visitor walk is harmless.
        visit::visit_expr_unsafe(self, node);
    }
}

/// Returns the inclusive line span (`end - start + 1`) of a brace-delimited
/// block. Single-line blocks return 1; an empty body would return 0.
fn block_line_span(block: &syn::Block) -> u32 {
    let span = block.brace_token.span.span();
    let start = span.start().line;
    let end = span.end().line;
    if start == 0 || end < start {
        return 0;
    }
    (end - start + 1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        UnsafeBlockScope.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_unsafe_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn one_line_unsafe_block_is_one() {
        let src = "fn f() { unsafe { let _x = 1; } }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn multiline_unsafe_counts_inclusive() {
        let src = "fn f() {\n    unsafe {\n        let _x = 1;\n    }\n}\n";
        // unsafe block spans 3 lines: `unsafe {`, `    let _x = 1;`, `    }`.
        assert_eq!(n_of(src, "f"), 3);
    }

    #[test]
    fn two_unsafe_blocks_sum() {
        let src = r#"
            fn f() {
                unsafe { let _a = 1; }
                let _b = 2;
                unsafe { let _c = 3; }
            }
        "#;
        // Each one-line unsafe block is 1 line each.
        assert_eq!(n_of(src, "f"), 2);
    }

    #[test]
    fn unsafe_fn_body_is_not_counted_implicitly() {
        // `unsafe fn` bodies don't count — only syntactic `unsafe { ... }`.
        let src = "unsafe fn f() { let _x = 1; }";
        assert_eq!(n_of(src, "f"), 0);
    }
}
