//! Source Lines Of Code (SLOC) — non-blank, non-comment-only lines in a
//! function body.
//!
//! SLOC is the conservative size measure paired with `method-length` (plan
//! §2.3). The two often correlate, but in Rust `where` clauses and
//! multi-line `impl Trait` returns can make method-length over-count
//! relative to the body's true logical size. We ship both so the M1
//! correlation study (plan §9, "復活候補") has data.
//!
//! # Algorithm (Layer 1, syn-only)
//!
//! For each function with a body:
//!
//! 1. Resolve the body's first and last source lines via
//!    `proc_macro2::Span::start` / `Span::end` on the brace tokens.
//! 2. Slice that line range out of the full source string.
//! 3. Drop blank lines and lines that are *only* a `//` comment.
//!
//! Block comments and trailing comments are not specially handled —
//! `// xyz` after code still counts the line. The point is conservative
//! signal, not perfect attribution.

use proc_macro2::Span;
use syn::spanned::Spanned;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// SLOC calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct SourceLinesOfCode;

impl MetricCalculator for SourceLinesOfCode {
    fn id(&self) -> &'static str {
        "source-lines-of-code"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Source Lines Of Code (body)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // Threshold defaults are conservative — function bodies past 60
            // logical lines start to test working memory.
            default_warning: Some(Threshold::new(60.0)),
            default_error: Some(Threshold::new(120.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let span = body.brace_token.span.span();
                f64::from(count_sloc(input.source, span))
            })
        })
    }
}

const RATIONALE: &str = "\
SLOC counts non-blank, non-comment-only lines in a function body. It is the \
conservative size measure: a function with low SLOC may still be complex (CC) \
or deep (max-nesting-level), but very high SLOC is on its own a reliability \
signal — long bodies hide what they do.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract a contiguous block of code into a named helper. The helper's \
name is documentation; SLOC drops automatically.",
    "Lift `let` chains and conversions to the top so the function body shows \
its shape at a glance.",
    "Replace a long sequence of `if`/`else` branches with a `match` on a small \
enum (the sealed-aware CC adjustment makes this free).",
];

const REFERENCES: &[&str] =
    &["plan §2.3 — Method Length と SLOC. Rust では where 句や impl Trait で …"];

/// Counts non-blank, non-comment-only lines in `source` between `span.start`
/// and `span.end` (inclusive on both ends).
fn count_sloc(source: &str, span: Span) -> u32 {
    let start = span.start().line;
    let end = span.end().line;
    if start == 0 || end == 0 || end < start {
        return 0;
    }
    let mut count = 0u32;
    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        if line_no < start || line_no > end {
            continue;
        }
        if is_loc_line(line) {
            count += 1;
        }
    }
    count
}

/// True iff `line` carries source content (not blank, not a pure-comment
/// line). `// xyz` after code still passes — we judge per-line, not
/// per-token.
fn is_loc_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("//") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        SourceLinesOfCode.measure(&input)
    }

    fn sloc_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_body_is_two_braces_only() {
        // `fn f() {}` — one line, the body is on a single line containing
        // `{}`. Our line-based counter sees one LOC.
        assert_eq!(sloc_of("fn f() {}", "f"), 1);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let src = "fn f() {\n\n\n    let _x = 1;\n\n}\n";
        // lines: "{", "", "", "    let _x = 1;", "", "}" -> LOC = "{", "let _x", "}".
        assert_eq!(sloc_of(src, "f"), 3);
    }

    #[test]
    fn comment_only_lines_are_skipped() {
        let src = "fn f() {\n    // doc\n    let _x = 1;\n    // doc\n}\n";
        // LOC = "{", "let _x", "}".
        assert_eq!(sloc_of(src, "f"), 3);
    }

    #[test]
    fn trailing_comments_after_code_still_count() {
        let src = "fn f() {\n    let _x = 1; // tail\n}\n";
        assert_eq!(sloc_of(src, "f"), 3);
    }

    #[test]
    fn trait_required_method_has_no_measurement() {
        let src = "trait T { fn f(&self); }";
        let ms = measure(src);
        assert!(ms.iter().all(|m| m.scope.path != "T::f"));
    }
}
