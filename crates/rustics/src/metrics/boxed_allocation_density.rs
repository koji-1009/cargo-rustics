//! `boxed-allocation-density` — count of `Box::new` calls inside a
//! function body.
//!
//! Plan §M4. `Box::new` is an explicit heap allocation; dense
//! clusters in hot paths are visible cost. Companion to
//! `clone-density` and `format-density`: each one counts a different
//! allocation site.

use syn::visit::{self, Visit};
use syn::ExprCall;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `boxed-allocation-density` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct BoxedAllocationDensity;

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
            default_warning: Some(Threshold::new(5.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = BoxVisitor { count: 0 };
                v.visit_block(body);
                f64::from(v.count)
            })
        })
    }
}

const RATIONALE: &str = "\
`Box::new` heap-allocates its argument. Most uses are deliberate (trait \
objects, recursive types, large stack moves), but a high count in a \
single function suggests the allocations could batch into one larger \
arena, or that the data wants a different lifetime story.";

const REFACTOR_HINTS: &[&str] = &[
    "Several `Box::new` calls on the same trait often want to be a \
`Vec<Box<dyn T>>` or, for size-bounded variants, an enum dispatch.",
    "If the boxed values share a lifetime, an arena (e.g. `bumpalo`) can \
collapse them into one allocation.",
];

const REFERENCES: &[&str] = &["plan §M4 — continuous lens proliferation."];

struct BoxVisitor {
    count: u32,
}

impl<'ast> Visit<'ast> for BoxVisitor {
    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if is_box_new(node) {
            self.count += 1;
        }
        visit::visit_expr_call(self, node);
    }
}

fn is_box_new(call: &ExprCall) -> bool {
    let syn::Expr::Path(p) = call.func.as_ref() else {
        return false;
    };
    let segs: Vec<_> = p.path.segments.iter().collect();
    if segs.len() < 2 {
        return false;
    }
    let last_two = &segs[segs.len() - 2..];
    last_two[0].ident == "Box" && last_two[1].ident == "new"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        BoxedAllocationDensity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_box_new_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn one_box_new() {
        let src = "fn f() { let _b = Box::new(1); }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn three_box_news_sum() {
        let src = "fn f() { Box::new(1); Box::new(2); Box::new(3); }";
        assert_eq!(n_of(src, "f"), 3);
    }

    #[test]
    fn fully_qualified_box_new_counts() {
        let src = "fn f() { std::boxed::Box::new(1); }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn other_news_do_not_count() {
        let src = "fn f() { Vec::new(); String::new(); }";
        assert_eq!(n_of(src, "f"), 0);
    }
}
