//! `rfc` — Response For a Class (Chidamber & Kemerer 1994).
//!
//! `RFC = |M| + |R|` where `M` is the set of methods defined in the
//! class and `R` is the set of *distinct* methods directly called
//! by methods of the class (each called method counted once).
//! Intuitively: how many methods can be invoked in response to a
//! message arriving at the class. CK validated RFC as a tester's-
//! burden indicator — the larger the response set, the more cases
//! exercise even a single entry point.
//!
//! Rust mapping (per inherent `impl T { … }` block):
//! * Methods defined `M` = `fn` items in the block.
//! * Called methods `R` = distinct method names reached by any of
//!   those fn bodies, *excluding* names already in `M` (CK's set
//!   union). Both `self.foo()` / `other.foo()` (`ExprMethodCall`)
//!   and `Type::foo(…)` / `Self::foo(…)` (path-call) contribute.
//!   Free-function calls (`some_fn(…)` with no receiver) are not
//!   counted — RFC is about method-message dispatch.
//!
//! Trait impls are skipped for the same reason as `lcom4`: the
//! method set is the trait's contract, and counting calls inside
//! it conflates "what the trait demands" with "what this code
//! reaches for".
//!
//! Default thresholds: warning 50 (CK 1994 § 3.5 advisory), error
//! 100 — the conventional escalation in published OO-design tools.
//!
//! ## Known limitations (AST-only, no name resolution)
//!
//! * **`module::helper()` free-function calls** are indistinguishable
//!   from `Type::associated_fn()` at the AST level — both parse as
//!   an `ExprCall` with a multi-segment path and ` qself = None`.
//!   The visitor counts the trailing segment as a method name
//!   either way, slightly inflating R for code that uses module-
//!   grouped helpers heavily. Distinguishing them would require a
//!   name-resolution layer (rust-analyzer / cargo-expand).
//! * **`<Self as Trait>::method()`** (qualified self path) is *not*
//!   counted — the visitor matches only `qself: None`. False
//!   negative on the disambiguation idiom.
//! * **Tokens inside macro bodies** (`vec![…]`, `format!(…)`) are
//!   not walked by `syn::Visit`, so calls hidden inside macro
//!   invocations are invisible to RFC.
//!
//! Reference:
//! * Chidamber, S. R., & Kemerer, C. F. (1994). A metrics suite for
//!   object oriented design. IEEE Transactions on Software
//!   Engineering, 20(6), 476-493.

use std::collections::BTreeSet;

use syn::visit::{self, Visit};
use syn::{ExprCall, ExprMethodCall, ExprPath, ImplItem};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// Response For a Class calculator.
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
        measure_impls(input.ast, |frame| {
            // Same trait-impl exclusion as lcom4 — the method set
            // is dictated by the trait, not by the impl author.
            if frame.item.trait_.is_some() {
                return None;
            }
            let methods: BTreeSet<String> = frame
                .item
                .items
                .iter()
                .filter_map(|i| match i {
                    ImplItem::Fn(f) => Some(f.sig.ident.to_string()),
                    _ => None,
                })
                .collect();
            if methods.is_empty() {
                return Some(0.0);
            }
            let mut called: BTreeSet<String> = BTreeSet::new();
            for item in &frame.item.items {
                if let ImplItem::Fn(method) = item {
                    let mut walker = CallNameCollector { out: &mut called };
                    walker.visit_block(&method.block);
                }
            }
            // CK union: M plus methods in R that aren't in M.
            let m = methods.len();
            let r_external = called.difference(&methods).count();
            Some((m + r_external) as f64)
        })
    }
}

const RATIONALE: &str = "\
Response For a Class (RFC, CK 1994) counts the methods that can be \
invoked in response to a message arriving at this impl block — the \
methods defined here plus the distinct methods they call. A high \
RFC means even a single entry point pulls in many other methods, \
which inflates the test-case surface and the reading load when \
following control flow. Validated as a defect predictor in Basili \
et al. (1996) and many follow-ups.";

const REFACTOR_HINTS: &[&str] = &[
    "If most of `R` (the called set) routes through one helper \
type, consider depending on that type as a constructor parameter \
instead of inlining the calls — the response surface narrows.",
    "Methods that delegate to many other methods are good \
candidates for the strategy / template-method shape: pull the \
varying bits behind a small trait so the impl block calls only one \
abstract method.",
    "If RFC is high because `M` is large (many fn items in the \
block), the impl is doing several jobs — see `lcom4` for whether \
those methods cluster into separable types.",
];

const REFERENCES: &[&str] = &[
    "Chidamber, S. R., & Kemerer, C. F. (1994). A metrics suite for \
object oriented design. IEEE TSE 20(6).",
    "Basili, Briand & Melo (1996). A validation of object-oriented \
design metrics as quality indicators.",
];

/// Collects every method name reached by `self.foo()`, `other.foo()`,
/// and `Type::foo(…)` / `Self::foo(…)` calls. Free-function calls
/// (`some_fn(…)` — a path with one segment used in a call position)
/// are ignored: RFC is about method-message dispatch.
struct CallNameCollector<'a> {
    out: &'a mut BTreeSet<String>,
}

impl<'a, 'ast> Visit<'ast> for CallNameCollector<'a> {
    // A nested `impl T { … }` or top-level `fn helper(…)` declared
    // inside a method body belongs to a different scope. Its calls
    // are part of that inner unit's response set, not the outer
    // impl's. Stop recursion at those boundaries.
    fn visit_item_impl(&mut self, _node: &'ast syn::ItemImpl) {}
    fn visit_item_fn(&mut self, _node: &'ast syn::ItemFn) {}

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        self.out.insert(node.method.to_string());
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let syn::Expr::Path(ExprPath { path, qself: None, .. }) = node.func.as_ref() {
            // `Type::method(…)` — at least two segments, last one is
            // the method name. A bare single-segment path is a
            // free function call (`println!` is a macro, not a call;
            // already filtered by syn). Skip those.
            if path.segments.len() >= 2 {
                if let Some(last) = path.segments.last() {
                    self.out.insert(last.ident.to_string());
                }
            }
        }
        visit::visit_expr_call(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        Rfc.measure(&input)
    }

    fn rfc_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_impl_is_zero() {
        let src = "struct F; impl F {}";
        assert_eq!(rfc_of(src, "F"), 0);
    }

    #[test]
    fn methods_with_no_calls_count_only_m() {
        // Two methods, no calls → RFC = |M| = 2.
        let src = "struct F; impl F { fn a(&self) {} fn b(&self) {} }";
        assert_eq!(rfc_of(src, "F"), 2);
    }

    #[test]
    fn external_method_call_adds_to_response() {
        // M = {a}; R = {Vec::new ("new" via path call) + v.push
        // ("push" via method call)} = 2 distinct names. RFC = 3.
        let src = r#"
            struct F;
            impl F {
                fn a(&self) { let mut v = Vec::<i32>::new(); v.push(1); }
            }
        "#;
        assert_eq!(rfc_of(src, "F"), 3);
    }

    #[test]
    fn self_call_to_own_method_is_already_in_m() {
        // M = {a, b}. b calls self.a(). a is already in M, so R adds
        // nothing. RFC = 2.
        let src = r#"
            struct F;
            impl F {
                fn a(&self) {}
                fn b(&self) { self.a(); }
            }
        "#;
        assert_eq!(rfc_of(src, "F"), 2);
    }

    #[test]
    fn self_path_call_counts_when_external() {
        // M = {a}. a calls Vec::new() which adds {new}. RFC = 2.
        let src = r#"
            struct F;
            impl F {
                fn a(&self) { let _ = Vec::<i32>::new(); }
            }
        "#;
        assert_eq!(rfc_of(src, "F"), 2);
    }

    #[test]
    fn distinct_calls_collapse_duplicates() {
        // M = {a}. push called twice — counted once. RFC = 2.
        let src = r#"
            struct F;
            impl F {
                fn a(&self) {
                    let mut v = Vec::<i32>::new();
                    v.push(1);
                    v.push(2);
                }
            }
        "#;
        // Calls: Vec::new (new), v.push (push). Both new to R. RFC = 1 + 2 = 3.
        assert_eq!(rfc_of(src, "F"), 3);
    }

    #[test]
    fn trait_impls_are_skipped() {
        let src = r#"
            struct F;
            trait T { fn a(&self); }
            impl T for F { fn a(&self) {} }
        "#;
        assert!(measure(src).is_empty());
    }

    #[test]
    fn nested_impl_calls_do_not_leak() {
        // Pre-fix: the walker recursed into `impl Inner { fn h() { other_method(); } }`
        // declared inside method `a`, adding `other_method` to outer
        // F's response set. After the fix the nested impl is opaque.
        let src = r#"
            struct F;
            impl F {
                fn a(&self) {
                    struct Inner;
                    impl Inner { fn h(&self) { let v: Vec<i32> = vec![]; let _ = v.iter().map(|x| x + 1); } }
                }
            }
        "#;
        // M = {a}; R = {} (the nested impl's calls are scoped out;
        // the macro `vec!` and the closure body live inside the
        // nested impl). RFC = 1.
        assert_eq!(rfc_of(src, "F"), 1);
    }

    #[test]
    fn metadata_cites_chidamber_and_kemerer() {
        let md = Rfc.metadata();
        assert!(md.references.iter().any(|r| r.contains("Chidamber")));
        assert!(md.references.iter().any(|r| r.contains("1994")));
    }
}
