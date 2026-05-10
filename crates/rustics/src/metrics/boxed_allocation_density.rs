//! Boxed allocation density — count of explicit `Box::new` /
//! `Box::pin` / `Rc::new` / `Arc::new` calls per fn.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Boxed allocation density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct BoxedAllocationDensity;

const ALLOC_PATHS: &[&[&str]] = &[
    &["Box", "new"],
    &["Box", "pin"],
    &["Rc", "new"],
    &["Arc", "new"],
];

impl MetricCalculator for BoxedAllocationDensity {
    fn id(&self) -> &'static str {
        "boxed-allocation-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Boxed Allocation Density",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(10.0)),
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
                .map(|body| f64::from(count_alloc_calls(body.syntax())))
        })
    }
}

fn count_alloc_calls(node: &SyntaxNode) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        let Some(call) = ast::CallExpr::cast(desc) else {
            continue;
        };
        let Some(ast::Expr::PathExpr(path_expr)) = call.expr() else {
            continue;
        };
        let Some(path) = path_expr.path() else {
            continue;
        };
        let segments: Vec<String> = path
            .segments()
            .filter_map(|s| s.name_ref().map(|n| n.text().to_string()))
            .collect();
        if matches_alloc_path(&segments) {
            n += 1;
        }
    }
    n
}

fn matches_alloc_path(segments: &[String]) -> bool {
    if segments.len() < 2 {
        return false;
    }
    let last_two = &segments[segments.len() - 2..];
    ALLOC_PATHS
        .iter()
        .any(|p| p.len() == 2 && p[0] == last_two[0] && p[1] == last_two[1])
}

const RATIONALE: &str = "\
Boxed allocation density flags functions that lean on heap allocation \
(`Box::new`, `Rc::new`, `Arc::new`, `Box::pin`). Each call is a heap \
allocation; clusters often indicate a stack-shaped data flow that was \
forced through indirection unnecessarily.";

const REFACTOR_HINTS: &[&str] = &[
    "If the boxed type fits on the stack, replace `Box<T>` with `T` directly.",
    "Reuse a single allocation across loop iterations rather than `Box::new`-ing per iteration.",
    "For trait-object collections, consider `&dyn Trait` instead of `Box<dyn Trait>` when ownership doesn't need to move.",
];

const REFERENCES: &[&str] = &[];
