//! Clone density — count of `.clone()` method calls per function.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Clone density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CloneDensity;

impl MetricCalculator for CloneDensity {
    fn id(&self) -> &'static str {
        "clone-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Clone Density",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(5.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            if frame.is_test() {
                return None;
            }
            frame
                .item
                .body()
                .map(|body| f64::from(count_method_calls_named(body.syntax(), "clone")))
        })
    }
}

pub(crate) fn count_method_calls_named(node: &SyntaxNode, name: &str) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        let Some(call) = ast::MethodCallExpr::cast(desc) else {
            continue;
        };
        if call.name_ref().is_some_and(|nr| nr.text() == name) {
            n += 1;
        }
    }
    n
}

const RATIONALE: &str = "\
Clone density flags functions that lean on `.clone()` to side-step the \
borrow checker. Each clone is a `T::clone` call that allocates / copies; \
dense use suggests a borrow shape that would read better as `&T`.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace `value.clone()` with `&value` where the callee just needs a read.",
    "If the same value is cloned multiple times in a body, bind it once with `let value_ref = &value;`.",
    "For `Arc<T>` / `Rc<T>` cycles, prefer `Arc::clone(&x)` style — the count surfaces, but the cost is just a refcount bump.",
];

const REFERENCES: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        CloneDensity.measure(&input)
    }

    #[test]
    fn no_clone_is_zero() {
        assert_eq!(measure("fn f() {}")[0].value, 0.0);
    }

    #[test]
    fn each_clone_call_counts() {
        let src = "fn f(x: String) { let _ = x.clone(); let _ = x.clone(); }";
        assert_eq!(measure(src)[0].value, 2.0);
    }
}
