//! Closure arity — maximum parameter count across closures inside
//! a function body.

use ra_ap_syntax::ast::{self, AstNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Closure arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClosureArity;

impl MetricCalculator for ClosureArity {
    fn id(&self) -> &'static str {
        "closure-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Closure Arity (max params per closure)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(6.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            let mut max = 0u32;
            for desc in body.syntax().descendants() {
                let Some(c) = ast::ClosureExpr::cast(desc) else {
                    continue;
                };
                let n = c
                    .param_list()
                    .map(|pl| pl.params().count() as u32)
                    .unwrap_or(0);
                if n > max {
                    max = n;
                }
            }
            if max == 0 { None } else { Some(f64::from(max)) }
        })
    }
}

const RATIONALE: &str = "\
Closure arity reports the widest closure in a function body. A closure \
with many parameters is a function in disguise — give it a name and the \
caller-side reads better.";

const REFACTOR_HINTS: &[&str] = &[
    "Promote a wide closure to a named `fn` so its parameters get types and docs.",
    "Bundle co-occurring parameters into a single struct.",
];

const REFERENCES: &[&str] = &[];
