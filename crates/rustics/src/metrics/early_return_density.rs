//! Early return density — count of `return` expressions plus `?`
//! operators per fn body.

use ra_ap_syntax::{ast::AstNode, SyntaxKind};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Early return density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EarlyReturnDensity;

impl MetricCalculator for EarlyReturnDensity {
    fn id(&self) -> &'static str {
        "early-return-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Early Return Density",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(6.0)),
            default_error: Some(Threshold::new(12.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            let mut n = 0u32;
            for desc in body.syntax().descendants() {
                if matches!(desc.kind(), SyntaxKind::RETURN_EXPR | SyntaxKind::TRY_EXPR) {
                    n += 1;
                }
            }
            Some(f64::from(n))
        })
    }
}

const RATIONALE: &str = "\
Early-return density counts `return ...;` statements plus `?` operators. \
A few early returns flatten happy-path code; many of them suggest the \
function is dispatching to several outcomes and would split into smaller \
units.";

const REFACTOR_HINTS: &[&str] = &[
    "Group adjacent guards into a single `if let Some(early) = check() { return early; }` block.",
    "Replace a long sequence of early returns with a `match` over a small enum that names each case.",
];

const REFERENCES: &[&str] = &[];
