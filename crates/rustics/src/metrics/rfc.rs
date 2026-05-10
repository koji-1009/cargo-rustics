//! Response For a Class (RFC, CK 1994) — `|M ∪ R|` where M is the
//! set of methods defined in an inherent `impl` and R is the set
//! of distinct method names invoked from within those methods.

use std::collections::HashSet;

use ra_ap_syntax::{
    ast::{self, AstNode, HasName},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// RFC calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Rfc;

impl MetricCalculator for Rfc {
    fn id(&self) -> &'static str {
        "rfc"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Response For a Class (CK 1994)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(50.0)),
            default_error: Some(Threshold::new(100.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.tree, |frame| {
            if frame.item.trait_().is_some() {
                return None;
            }
            let al = frame.item.assoc_item_list()?;
            let mut methods: HashSet<String> = HashSet::new();
            let mut invoked: HashSet<String> = HashSet::new();
            for item in al.assoc_items() {
                let ast::AssocItem::Fn(f) = item else {
                    continue;
                };
                let Some(name) = f.name() else { continue };
                methods.insert(name.text().to_string());
                if let Some(body) = f.body() {
                    collect_invocations(body.syntax(), &mut invoked);
                }
            }
            let union: HashSet<&String> = methods.union(&invoked).collect();
            Some(union.len() as f64)
        })
    }
}

fn collect_invocations(node: &SyntaxNode, out: &mut HashSet<String>) {
    for desc in node.descendants() {
        match desc.kind() {
            SyntaxKind::METHOD_CALL_EXPR => record_method_call(&desc, out),
            SyntaxKind::CALL_EXPR => record_path_call(&desc, out),
            _ => {}
        }
    }
}

fn record_method_call(node: &SyntaxNode, out: &mut HashSet<String>) {
    let Some(call) = ast::MethodCallExpr::cast(node.clone()) else {
        return;
    };
    if let Some(name) = call.name_ref() {
        out.insert(name.text().to_string());
    }
}

/// `Type::method(...)` paths only — RFC counts method dispatch, not
/// free-fn shapes (single-segment `helper(...)`).
fn record_path_call(node: &SyntaxNode, out: &mut HashSet<String>) {
    let Some(call) = ast::CallExpr::cast(node.clone()) else {
        return;
    };
    let Some(ast::Expr::PathExpr(p)) = call.expr() else {
        return;
    };
    let Some(path) = p.path() else { return };
    if path.qualifier().is_none() {
        return;
    }
    if let Some(seg) = path.segment().and_then(|s| s.name_ref()) {
        out.insert(seg.text().to_string());
    }
}

const RATIONALE: &str = "\
RFC (CK 1994) is `|M| + |R|`: the set of methods this class declares \
plus the set of distinct method names it invokes. CK validated RFC as \
a tester's-burden indicator — the larger the response set, the more \
cases exercise even a single entry point.";

const REFACTOR_HINTS: &[&str] = &[
    "If the methods reach into many helper types, consider injecting one combined helper instead.",
    "Split methods that delegate widely into a smaller core that does its own work plus a coordinator that calls the core.",
];

const REFERENCES: &[&str] = &["Chidamber, S. R., & Kemerer, C. F. (1994). A metrics suite for object oriented design. IEEE TSE 20(6):476-493."];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        Rfc.measure(&input)
    }

    #[test]
    fn empty_impl_yields_zero() {
        // RFC on an empty impl is `|∅ ∪ ∅| = 0`. The measurement is
        // emitted (one per impl) — the threshold comparison is what
        // skips a 0 from showing up as a violation downstream.
        let m = measure("struct S; impl S {}");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].value, 0.0);
    }

    #[test]
    fn methods_without_calls_count_only_themselves() {
        // Two methods, each empty body — RFC = |{a, b}| = 2.
        let src = "struct S; impl S { fn a(&self) {} fn b(&self) {} }";
        let m = measure(src);
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn method_calls_add_to_response_set() {
        // `a` calls `.foo()` and `.bar()` — RFC = |{a, foo, bar}| = 3.
        let src = "struct S; impl S { fn a(&self, x: &T) { x.foo(); x.bar(); } }";
        let m = measure(src);
        assert_eq!(m[0].value, 3.0);
    }

    #[test]
    fn duplicate_calls_dedupe() {
        // Two calls to `.foo()` count once.
        let src = "struct S; impl S { fn a(&self, x: &T) { x.foo(); x.foo(); } }";
        let m = measure(src);
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn type_qualified_path_call_counts() {
        // `Foo::bar(...)` is a path call with a qualifier — counted.
        let src = "struct S; impl S { fn a(&self) { Foo::bar(); } }";
        let m = measure(src);
        // {a, bar} = 2
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn unqualified_free_fn_call_does_not_count() {
        // Single-segment `helper()` is a free-fn call without dispatch
        // — RFC counts method dispatch, not free fns.
        let src = "struct S; impl S { fn a(&self) { helper(); } }";
        let m = measure(src);
        // {a} only
        assert_eq!(m[0].value, 1.0);
    }

    #[test]
    fn trait_impl_is_skipped() {
        let src = "trait T { fn x(&self); } \
                   struct S; \
                   impl T for S { fn x(&self) { self.x(); } }";
        let m = measure(src);
        assert!(m.is_empty());
    }
}
