//! Source Lines Of Code (SLOC) — non-blank, non-comment-only lines
//! in a function body.

use ra_ap_syntax::{ast::AstNode, TextRange};

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
            default_warning: Some(Threshold::new(60.0)),
            default_error: Some(Threshold::new(120.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            frame.item.body().map(|body| {
                let range = body.syntax().text_range();
                f64::from(count_sloc(input.source, range))
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

const REFERENCES: &[&str] = &[];

fn count_sloc(source: &str, range: TextRange) -> u32 {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    let slice = source.get(start..end).unwrap_or("");
    let mut count = 0u32;
    for line in slice.lines() {
        if is_loc_line(line) {
            count += 1;
        }
    }
    count
}

fn is_loc_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with("//")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        SourceLinesOfCode.measure(&input)
    }

    #[test]
    fn empty_body_has_two_loc() {
        let m = measure("fn f() {\n}");
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn blank_lines_are_ignored() {
        let src = "fn f() {\n    let x = 1;\n\n\n    let y = 2;\n}";
        assert_eq!(measure(src)[0].value, 4.0);
    }

    #[test]
    fn comment_only_lines_are_ignored() {
        let src = "fn f() {\n    // header\n    let x = 1;\n    // tail\n}";
        assert_eq!(measure(src)[0].value, 3.0);
    }

    #[test]
    fn trailing_comment_after_code_still_counts() {
        let src = "fn f() {\n    let x = 1; // hi\n}";
        assert_eq!(measure(src)[0].value, 3.0);
    }
}
