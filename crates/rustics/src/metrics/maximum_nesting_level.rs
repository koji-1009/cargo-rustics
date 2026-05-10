//! Maximum Nesting Level — deepest nesting of control-flow blocks
//! reachable from a function's entry.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Max-nesting-level calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MaximumNestingLevel;

impl MetricCalculator for MaximumNestingLevel {
    fn id(&self) -> &'static str {
        "maximum-nesting-level"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Maximum Nesting Level",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(7.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            frame
                .item
                .body()
                .map(|body| f64::from(max_depth(body.syntax(), 0)))
        })
    }
}

const RATIONALE: &str = "\
Max-nesting-level reports the depth of the deepest control-flow block in \
the function. Each step inward is a context the reader must hold on the \
stack; once depth gets past 4, comprehension drops sharply.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace deepest `if x { if y { ... } }` chains with `if !x { return; } if !y { return; }` early-return guards.",
    "Lift the body of the deepest `match` arm into a named helper.",
    "Use combinators (`Option::and_then`, `Result::and_then`) to flatten chained Option/Result handling.",
];

const REFERENCES: &[&str] = &[
    "NIST SP 500-235: Structured Testing — A Testing Methodology Using the Cyclomatic Complexity Metric.",
];

fn max_depth(node: &SyntaxNode, current: u32) -> u32 {
    let mut max = current;
    for child in node.children() {
        let next = if is_control_block(child.kind())
            || ast::ClosureExpr::cast(child.clone()).is_some()
        {
            current + 1
        } else {
            current
        };
        max = max.max(max_depth(&child, next));
    }
    max
}

fn is_control_block(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::IF_EXPR
            | SyntaxKind::MATCH_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::FOR_EXPR
            | SyntaxKind::LOOP_EXPR
            | SyntaxKind::TRY_EXPR
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        MaximumNestingLevel.measure(&input)
    }

    #[test]
    fn straight_line_is_zero() {
        let m = measure("fn f() { let _ = 1; }");
        assert_eq!(m[0].value, 0.0);
    }

    #[test]
    fn single_if_is_one() {
        assert_eq!(measure("fn f(x: i32) { if x > 0 {} }")[0].value, 1.0);
    }

    #[test]
    fn nested_if_match_for_is_three() {
        let src = "fn f(x: i32) { \
                   if x > 0 { \
                       match x { _ => { for _ in 0..1 {} } } \
                   } \
                   }";
        assert_eq!(measure(src)[0].value, 3.0);
    }

    #[test]
    fn closure_opens_new_scope() {
        let src = "fn f() { (|| { if true {} })(); }";
        assert_eq!(measure(src)[0].value, 2.0);
    }
}
