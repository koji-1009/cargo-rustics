//! `early-return-density` — count of explicit `return` statements
//! inside a function body (excluding the implicit trailing return).
//!
//!
//! that never quite committed to its shape. Two or three are usual
//! for guard clauses; past that, the function is often hiding state
//! that wants to live in an explicit `match` or a smaller helper.
//!
//! What counts: every `return ...;` keyword expression. The trailing
//! tail expression of a function (no `return` keyword) is *not*
//! counted — it is a different shape and isn't an early return.

use syn::visit::{self, Visit};
use syn::ExprReturn;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `early-return-density` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EarlyReturnDensity;

impl MetricCalculator for EarlyReturnDensity {
    fn id(&self) -> &'static str {
        "early-return-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Early-return Density",
            category: MetricCategory::Function,
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
                let mut v = ReturnVisitor { count: 0 };
                v.visit_block(body);
                f64::from(v.count)
            })
        })
    }
}

const RATIONALE: &str = "\
A function with many `return` statements is often a switch that did not \
commit to its shape. Two or three early returns guard preconditions; \
past five, the function is usually hiding control flow that wants to \
live in an explicit `match` or be split across helpers.";

const REFACTOR_HINTS: &[&str] = &[
    "Convert a chain of `if cond { return x; }` guards into an explicit \
`match` whose arms compute the result.",
    "If returns split into two clusters (precondition rejection vs. \
business-logic shortcut), the second cluster is often a helper function \
in disguise.",
    "Returns inside a `loop` / `for` are different — they are flow \
control, not guards. Refactoring those tends to make the code worse.",
];

const REFERENCES: &[&str] = &[];

struct ReturnVisitor {
    count: u32,
}

impl<'ast> Visit<'ast> for ReturnVisitor {
    fn visit_expr_return(&mut self, node: &'ast ExprReturn) {
        self.count += 1;
        visit::visit_expr_return(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        EarlyReturnDensity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_return_keyword_is_zero() {
        assert_eq!(n_of("fn f() -> i32 { 1 }", "f"), 0);
    }

    #[test]
    fn one_return() {
        let src = "fn f(x: i32) -> i32 { if x < 0 { return 0; } x }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn three_guards_count_three() {
        let src = r#"
            fn f(x: i32) -> i32 {
                if x < 0 { return 0; }
                if x == 0 { return 1; }
                if x > 100 { return 100; }
                x
            }
        "#;
        assert_eq!(n_of(src, "f"), 3);
    }

    #[test]
    fn implicit_tail_does_not_count() {
        // Tail expression has no `return` keyword.
        let src = "fn f(x: i32) -> i32 { x + 1 }";
        assert_eq!(n_of(src, "f"), 0);
    }
}
