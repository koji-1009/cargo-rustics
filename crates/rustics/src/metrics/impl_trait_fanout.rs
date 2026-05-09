//! `impl-trait-fanout` — count of `impl Trait` occurrences in a function
//! signature.
//!
//! + §6.1. Informational at M1 — the value is one of the inputs
//! to the `rustContext` block that lands in M2 alongside the
//! `regression` command. Until then, the lens still runs (it shows up in
//! `cargo rustics rules`) so its catalogue entry is reserved.
//!
//! What counts:
//!
//! * Each `impl Trait` argument type — `fn f(x: impl Iterator<…>)`.
//! * Each `impl Trait` return type — `fn f() -> impl Future<…>`.
//! * Each `impl Trait` inside a tuple / generic / reference type — every
//!   recursive position.

use syn::{FnArg, ReturnType, Signature, Type, TypeImplTrait};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_functions;

/// `impl-trait-fanout` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplTraitFanout;

impl MetricCalculator for ImplTraitFanout {
    fn id(&self) -> &'static str {
        "impl-trait-fanout"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "impl-Trait Fanout",
            category: MetricCategory::RustErgonomics,
            // Informational at M1 — never crosses a threshold.
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            Some(f64::from(count_impl_trait_in_sig(frame.signature)))
        })
    }
}

const RATIONALE: &str = "\
`impl Trait` erases the concrete type at the boundary; every occurrence \
in the signature is one more place where the caller cannot name the \
return value's type without `<…>` annotations or a separate alias. \
Informational at M1 — the value feeds the `rustContext` block (plan \
§4.3) that lands with the regression command.";

const REFACTOR_HINTS: &[&str] = &[
    "If callers need to name the type (store it in a struct, return it from \
their own `fn`), give the function a concrete return type or a type alias.",
    "When `impl Trait` is used because the type is *truly* hidden (RPIT for \
async or iterators), keep it — the count is informational, not a smell.",
];

const REFERENCES: &[&str] = &[
];

fn count_impl_trait_in_sig(sig: &Signature) -> u32 {
    let mut total = 0u32;
    for input in &sig.inputs {
        if let FnArg::Typed(pt) = input {
            total += count_impl_trait_in_ty(&pt.ty);
        }
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        total += count_impl_trait_in_ty(ty);
    }
    total
}

fn count_impl_trait_in_ty(ty: &Type) -> u32 {
    if let Some(inner) = unwrap_simple(ty) {
        return count_impl_trait_in_ty(inner);
    }
    match ty {
        Type::ImplTrait(TypeImplTrait { .. }) => 1,
        Type::Tuple(t) => t.elems.iter().map(count_impl_trait_in_ty).sum(),
        Type::Array(a) => count_impl_trait_in_ty(&a.elem),
        Type::Slice(s) => count_impl_trait_in_ty(&s.elem),
        Type::Path(tp) => count_in_path_args(tp, count_impl_trait_in_ty),
        _ => 0,
    }
}

/// Strips the outer wrapper of a "transparent" type — reference / parens /
/// group all just decorate an inner `Type` and don't change the count.
fn unwrap_simple(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(r) => Some(&r.elem),
        Type::Paren(p) => Some(&p.elem),
        Type::Group(g) => Some(&g.elem),
        _ => None,
    }
}

/// Walks the angle-bracketed generic arguments of a path (`Vec<T>`,
/// `Result<A, B>`) and sums `count_one` over their type arguments.
fn count_in_path_args(tp: &syn::TypePath, count_one: fn(&Type) -> u32) -> u32 {
    tp.path
        .segments
        .iter()
        .map(|seg| match &seg.arguments {
            syn::PathArguments::AngleBracketed(args) => args
                .args
                .iter()
                .filter_map(|a| match a {
                    syn::GenericArgument::Type(t) => Some(count_one(t)),
                    _ => None,
                })
                .sum(),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        ImplTraitFanout.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_impl_trait_is_zero() {
        assert_eq!(n_of("fn f(x: i32) -> i32 { x }", "f"), 0);
    }

    #[test]
    fn impl_trait_in_argument() {
        let src = "fn f(x: impl Iterator<Item = i32>) {}";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn impl_trait_in_return() {
        let src = "fn f() -> impl Iterator<Item = i32> { std::iter::empty() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn impl_trait_inside_box() {
        let src = "fn f() -> Box<impl Iterator<Item = i32>> { todo!() }";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn arg_and_return_sum() {
        let src = "fn f(x: impl Clone) -> impl Iterator<Item = i32> { todo!() }";
        assert_eq!(n_of(src, "f"), 2);
    }
}
