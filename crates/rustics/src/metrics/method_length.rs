//! Method Length — total physical lines from `fn` keyword to the body's
//! closing brace, inclusive.
//!
//! Paired with [`crate::SourceLinesOfCode`] for the M1 correlation study
//! (plan §2.3): in Rust the two diverge when `where` clauses or
//! multi-line `impl Trait` return types pad a signature without adding to
//! the body. We measure both and read them together.

use syn::spanned::Spanned;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Method Length calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MethodLength;

impl MetricCalculator for MethodLength {
    fn id(&self) -> &'static str {
        "method-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Method Length (signature + body)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(80.0)),
            default_error: Some(Threshold::new(160.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let start = frame.signature.fn_token.span.start().line;
                let end = body.brace_token.span.span().end().line;
                let len = if end >= start { end - start + 1 } else { 0 };
                len as f64
            })
        })
    }
}

const RATIONALE: &str = "\
Method Length is the total physical line count of a function — signature \
plus body. It is a coarser size signal than SLOC, but it captures what a \
reader actually scrolls past. In Rust the gap between SLOC and method-length \
is informative: a wide gap means the signature is doing a lot of work \
(`where` clauses, `impl Trait`, multi-line generics).";

const REFACTOR_HINTS: &[&str] = &[
    "If the gap with SLOC is large, the signature is heavy. Consider a type \
alias or a builder so the signature reads in one line.",
    "If the gap is small but method-length is high, the body is long — extract \
named helpers as for SLOC.",
];

const REFERENCES: &[&str] =
    &["plan §2.3 — Method Length と SLOC; Rust 特性で相関が他言語より低い可能性."];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        MethodLength.measure(&input)
    }

    fn ml_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn one_liner_is_one() {
        assert_eq!(ml_of("fn f() {}", "f"), 1);
    }

    #[test]
    fn multiline_body_counts_signature_and_body() {
        let src = "fn f() {\n    let _x = 1;\n}\n";
        // line 1: fn f() {
        // line 2: ...
        // line 3: }
        assert_eq!(ml_of(src, "f"), 3);
    }

    #[test]
    fn trait_required_method_is_skipped() {
        let src = "trait T { fn f(&self); }";
        let ms = measure(src);
        assert!(ms.iter().all(|m| m.scope.path != "T::f"));
    }
}
