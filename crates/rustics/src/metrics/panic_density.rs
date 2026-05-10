//! Panic density — count of `panic!` / `unwrap` / `expect` /
//! `todo!` / `unimplemented!` / `unreachable!` per function.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Panic density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanicDensity;

impl MetricCalculator for PanicDensity {
    fn id(&self) -> &'static str {
        "panic-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Panic Density",
            category: MetricCategory::RustSafety,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(8.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            // Test bodies legitimately panic / assert / unwrap their
            // way through fixture data; we don't want assertion
            // noise to dominate the score.
            if frame.is_test() {
                return None;
            }
            frame
                .item
                .body()
                .map(|body| f64::from(count_panics(body.syntax())))
        })
    }
}

const PANIC_METHOD_NAMES: &[&str] = &["unwrap", "expect", "unwrap_or_else"];
const PANIC_MACRO_NAMES: &[&str] =
    &["panic", "todo", "unimplemented", "unreachable", "assert", "assert_eq", "assert_ne"];

fn count_panics(node: &SyntaxNode) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        if let Some(m) = ast::MethodCallExpr::cast(desc.clone()) {
            if m.name_ref()
                .is_some_and(|nr| PANIC_METHOD_NAMES.contains(&nr.text().as_str()))
            {
                n += 1;
            }
            continue;
        }
        if let Some(mac) = ast::MacroCall::cast(desc) {
            let name = mac
                .path()
                .and_then(|p| p.segment())
                .and_then(|s| s.name_ref())
                .map(|n| n.text().to_string())
                .unwrap_or_default();
            if PANIC_MACRO_NAMES.contains(&name.as_str()) {
                n += 1;
            }
        }
    }
    n
}

const RATIONALE: &str = "\
Panic density counts call sites that abort the program on failure: \
`unwrap` / `expect` on `Option`/`Result`, plus `panic!` / `todo!` / \
`unimplemented!` / `unreachable!` / assertions. Dense use signals a \
function that 'gives up' rather than handling errors — fine in tests, \
worth flagging in production paths.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace `unwrap()` with `?` and let the caller decide.",
    "Convert `expect(\"...\")` strings into typed errors that the caller can match.",
    "If a panic site really is unreachable, document the invariant and use `unreachable!()` deliberately so the count is honest.",
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
        PanicDensity.measure(&input)
    }

    #[test]
    fn empty_fn_zero() {
        assert_eq!(measure("fn f() {}")[0].value, 0.0);
    }

    #[test]
    fn unwrap_expect_count() {
        let src = "fn f(o: Option<i32>) { o.unwrap(); o.expect(\"x\"); }";
        assert_eq!(measure(src)[0].value, 2.0);
    }

    #[test]
    fn panic_macros_count() {
        let src = "fn f() { panic!(); todo!(); unreachable!(); }";
        assert_eq!(measure(src)[0].value, 3.0);
    }
}
