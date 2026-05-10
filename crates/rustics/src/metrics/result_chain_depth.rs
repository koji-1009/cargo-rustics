//! Result chain depth — deepest nesting of `?` operators inside a
//! single expression chain.

use ra_ap_syntax::{ast::AstNode, SyntaxKind, SyntaxNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Result chain depth calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResultChainDepth;

impl MetricCalculator for ResultChainDepth {
    fn id(&self) -> &'static str {
        "result-chain-depth"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Result Chain Depth (nested ?)",
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
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            Some(f64::from(max_try_depth(body.syntax(), 0)))
        })
    }
}

fn max_try_depth(node: &SyntaxNode, current: u32) -> u32 {
    let mut max = current;
    for child in node.children() {
        let next = if child.kind() == SyntaxKind::TRY_EXPR {
            current + 1
        } else {
            current
        };
        max = max.max(max_try_depth(&child, next));
    }
    max
}

const RATIONALE: &str = "\
Result chain depth flags `?` operators nested inside other `?`s. \
Sequential `let x = a()?; let y = b()?;` is depth-1 twice; \
`a()?.b()?.c()?` is depth-3 — and that's the shape that hides which \
step actually failed.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull each `?` into its own `let` binding so the failure site is named.",
    "Use `Result::and_then` for genuinely sequential operations and let the closure show the data flow.",
];

const REFERENCES: &[&str] = &[];
