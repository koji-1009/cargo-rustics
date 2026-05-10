//! Await depth — deepest nesting of `.await` expressions inside a
//! function body. Sequential awaits are depth-1 each; one await
//! waiting on the result of another contributes depth-2.

use ra_ap_syntax::{ast::AstNode, SyntaxKind, SyntaxNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Await depth calculator.
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
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            Some(f64::from(max_await_depth(body.syntax(), 0)))
        })
    }
}

fn max_await_depth(node: &SyntaxNode, current: u32) -> u32 {
    let mut max = current;
    for child in node.children() {
        let next = if child.kind() == SyntaxKind::AWAIT_EXPR {
            current + 1
        } else {
            current
        };
        max = max.max(max_await_depth(&child, next));
    }
    max
}

const RATIONALE: &str = "\
Await depth flags `.await` expressions stacked through other `.await`s — \
the kind of chain where each suspension is waiting on a value the next \
one will compute. Sequential awaits don't accumulate.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull each `.await` into its own `let` binding. Each step gets a name and the chain flattens.",
    "If the awaits compose a pipeline, consider an explicit combinator (`futures::join!`, `tokio::try_join!`).",
];

const REFERENCES: &[&str] = &[];
