//! `impl-length` — total physical lines of an `impl` block.
//!
//! Demoted to **informational** (no thresholds, polarity ignored)
//! because dogfood showed `r=0.866` between this and `wmc` (CK 1994).
//! The two metrics measure overlapping things — wmc is the citation-
//! backed "weight" of an impl, impl-length is the raw line count.
//! Keeping both as gates would double-count the same signal in any
//! AI report consumer; demoting to informational means the value is
//! still surfaced but the threshold gate doesn't fire.

use syn::spanned::Spanned;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_impls;

/// `impl-length` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplLength;

impl MetricCalculator for ImplLength {
    fn id(&self) -> &'static str {
        "impl-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "impl-block Length (lines, informational)",
            category: MetricCategory::ImplShape,
            // Informational — no "better direction". The CK-defined
            // wmc lens is the gating one for impl-block weight; this
            // value travels along as raw context for the AI / human
            // reading the report.
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.ast, |frame| {
            let span = frame.item.brace_token.span.span();
            let start = span.start().line;
            let end = span.end().line;
            let len = if end >= start { end - start + 1 } else { 0 };
            Some(len as f64)
        })
    }
}

const RATIONALE: &str = "\
Raw line count of an `impl` block — surfaced as informational \
context. The gating signal for impl-block weight is `wmc` (CK 1994, \
Σ CC over methods). This metric exists to give the AI / human \
reader the raw size figure without double-counting the wmc gate.";

const REFACTOR_HINTS: &[&str] = &[
    "If the impl block is long because of many small methods, see \
`wmc` for the complexity-weighted view.",
    "If the length comes from one or two huge methods, those are the \
function-level lenses' problem (CC, SLOC).",
];

const REFERENCES: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        ImplLength.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn one_line_impl_is_one() {
        assert_eq!(n_of("struct Foo; impl Foo {}", "Foo"), 1);
    }

    #[test]
    fn multiline_impl_counts_inclusive() {
        let src = "struct Foo;\nimpl Foo {\n    fn a(&self) {}\n}\n";
        // impl block spans line 2..line 4 — 3 lines.
        assert_eq!(n_of(src, "Foo"), 3);
    }

    #[test]
    fn metadata_is_informational() {
        // Demoted from gated lens; both thresholds must be None and
        // the polarity must be Informational so no violation fires.
        let md = ImplLength.metadata();
        assert_eq!(md.id, "impl-length");
        assert!(md.default_warning.is_none());
        assert!(md.default_error.is_none());
        assert_eq!(md.polarity, MetricPolarity::Informational);
    }
}
