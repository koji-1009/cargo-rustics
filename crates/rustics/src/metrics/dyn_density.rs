//! `dyn-density` — count of `dyn Trait` occurrences in a function
//! signature.
//!
//! + §6.1. Informational, paired with
//! [`crate::ImplTraitFanout`]. Captures dynamic-dispatch surface in the
//! signature: `&dyn Trait`, `Box<dyn Trait>`, `Arc<dyn Trait>`, `Vec<Box<dyn
//! Trait>>`, … all add up.

use syn::{FnArg, ReturnType, Signature, Type, TypeTraitObject};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_functions;

/// `dyn-density` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct DynDensity;

impl MetricCalculator for DynDensity {
    fn id(&self) -> &'static str {
        "dyn-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "dyn-Trait Density",
            category: MetricCategory::RustPerformance,
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
            Some(f64::from(count_dyn_in_sig(frame.signature)))
        })
    }
}

const RATIONALE: &str = "\
Each `dyn Trait` in the signature is one virtual-dispatch boundary the \
runtime has to honour. Dynamic dispatch is sometimes the right answer \
(plug-in architectures, heterogeneous collections); sometimes it is the \
path of least resistance for a generic that did not fit. Informational \
— the value feeds the `rustContext` block that travels with each \
violation.";

const REFACTOR_HINTS: &[&str] = &[
    "If only a small set of types implements the trait, prefer a generic \
parameter or an enum to keep the dispatch static.",
    "Inside hot loops, converting `Box<dyn T>` to `T: Trait` (a generic \
parameter) often removes the per-call indirection.",
];

const REFERENCES: &[&str] = &[
];

fn count_dyn_in_sig(sig: &Signature) -> u32 {
    let mut total = 0u32;
    for input in &sig.inputs {
        if let FnArg::Typed(pt) = input {
            total += count_dyn_in_ty(&pt.ty);
        }
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        total += count_dyn_in_ty(ty);
    }
    total
}

fn count_dyn_in_ty(ty: &Type) -> u32 {
    if let Some(inner) = unwrap_simple(ty) {
        return count_dyn_in_ty(inner);
    }
    match ty {
        Type::TraitObject(TypeTraitObject {
            dyn_token: Some(_), ..
        }) => 1,
        Type::Tuple(t) => t.elems.iter().map(count_dyn_in_ty).sum(),
        Type::Array(a) => count_dyn_in_ty(&a.elem),
        Type::Slice(s) => count_dyn_in_ty(&s.elem),
        Type::Path(tp) => count_in_path_args(tp, count_dyn_in_ty),
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

/// Walks the angle-bracketed generic arguments of a path and sums
/// `count_one` over their type arguments.
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
        DynDensity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_dyn_is_zero() {
        assert_eq!(n_of("fn f(x: i32) {}", "f"), 0);
    }

    #[test]
    fn ref_dyn_counts() {
        let src = "fn f(x: &dyn std::fmt::Debug) {}";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn box_dyn_counts() {
        let src = "fn f(x: Box<dyn std::fmt::Debug>) {}";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn vec_of_box_dyn_counts_inner() {
        let src = "fn f(x: Vec<Box<dyn std::fmt::Debug>>) {}";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn impl_trait_does_not_count() {
        let src = "fn f(x: impl std::fmt::Debug) {}";
        assert_eq!(n_of(src, "f"), 0);
    }
}
