//! Borrow profile — three companion lenses that report parameter
//! ownership/borrowing shape per function.
//!
//! + §4.3. The plan's `borrowProfile` is a structured
//! sub-object (`{ owned, borrowed, mutBorrowed }`); we ship three
//! single-value lenses (`borrow-profile-owned` /
//! `borrow-profile-borrowed` / `borrow-profile-mut`) and let the CLI
//! aggregate them into the `rustContext.borrowProfile` block. This
//! keeps `MetricCalculator::measure` single-value and the JSON
//! contract structurally valid.
//!
//! Counted: positional parameters only — the `self` receiver is
//! excluded throughout (it is a receiver, not a parameter the caller
//! chooses).

use syn::{FnArg, Signature, Type};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_functions;

/// `borrow-profile-owned` — count of *owned* (non-reference) positional
/// parameters.
#[derive(Debug, Default, Clone, Copy)]
pub struct BorrowProfileOwned;

impl MetricCalculator for BorrowProfileOwned {
    fn id(&self) -> &'static str {
        "borrow-profile-owned"
    }
    fn metadata(&self) -> MetricMetadata {
        owned_metadata()
    }
    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            let n = count_params(frame.signature, ParamShape::Owned);
            Some(f64::from(n))
        })
    }
}

/// `borrow-profile-borrowed` — count of `&T` (immutable-reference)
/// positional parameters.
#[derive(Debug, Default, Clone, Copy)]
pub struct BorrowProfileBorrowed;

impl MetricCalculator for BorrowProfileBorrowed {
    fn id(&self) -> &'static str {
        "borrow-profile-borrowed"
    }
    fn metadata(&self) -> MetricMetadata {
        borrowed_metadata()
    }
    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            let n = count_params(frame.signature, ParamShape::Borrowed);
            Some(f64::from(n))
        })
    }
}

/// `borrow-profile-mut` — count of `&mut T` positional parameters.
#[derive(Debug, Default, Clone, Copy)]
pub struct BorrowProfileMut;

impl MetricCalculator for BorrowProfileMut {
    fn id(&self) -> &'static str {
        "borrow-profile-mut"
    }
    fn metadata(&self) -> MetricMetadata {
        mut_metadata()
    }
    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            let n = count_params(frame.signature, ParamShape::MutBorrowed);
            Some(f64::from(n))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamShape {
    Owned,
    Borrowed,
    MutBorrowed,
}

fn count_params(sig: &Signature, want: ParamShape) -> u32 {
    sig.inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pt) => Some(shape_of(&pt.ty)),
            FnArg::Receiver(_) => None,
        })
        .filter(|shape| *shape == want)
        .count() as u32
}

fn shape_of(ty: &Type) -> ParamShape {
    match ty {
        Type::Reference(r) if r.mutability.is_some() => ParamShape::MutBorrowed,
        Type::Reference(_) => ParamShape::Borrowed,
        _ => ParamShape::Owned,
    }
}

fn owned_metadata() -> MetricMetadata {
    MetricMetadata {
        id: "borrow-profile-owned",
        display_name: "Borrow Profile — owned",
        category: MetricCategory::RustErgonomics,
        polarity: MetricPolarity::Informational,
        default_warning: None,
        default_error: None,
        rationale: "Count of positional parameters passed by value. Companion to \
                    borrow-profile-borrowed and borrow-profile-mut; the rustContext \
                    block surfaces them together as `{owned, borrowed, mutBorrowed}`.",
        refactor_hints: &[
            "If most parameters are owned, the function takes ownership of its inputs — \
             that is sometimes the right call (caller-side moves) and sometimes \
             accidental (caller wanted to borrow).",
        ],
        references: &[],
    }
}

fn borrowed_metadata() -> MetricMetadata {
    MetricMetadata {
        id: "borrow-profile-borrowed",
        display_name: "Borrow Profile — borrowed (`&T`)",
        category: MetricCategory::RustErgonomics,
        polarity: MetricPolarity::Informational,
        default_warning: None,
        default_error: None,
        rationale: "Count of positional parameters of shape `&T`. The borrow-friendly \
                    parameter shape; high counts simply mean the function reads a lot.",
        refactor_hints: &[
            "If `&T` parameters cluster on a single conceptual struct (`&Config`, \
             `&Logger`, `&Db`), bundling them into a context type can shrink the \
             signature.",
        ],
        references: &[],
    }
}

fn mut_metadata() -> MetricMetadata {
    MetricMetadata {
        id: "borrow-profile-mut",
        display_name: "Borrow Profile — mut-borrowed (`&mut T`)",
        category: MetricCategory::RustErgonomics,
        polarity: MetricPolarity::Informational,
        default_warning: None,
        default_error: None,
        rationale: "Count of positional parameters of shape `&mut T`. Each is a place \
                    the function mutates state the caller still owns; dense \
                    `&mut`-passing is a sign the function is doing several mutations \
                    in tandem.",
        refactor_hints: &[
            "If multiple `&mut` parameters are always mutated together, replacing them \
             with a single `&mut Bundle` keeps the borrow checker happier and reads \
             clearer.",
            "When the function only writes to one field of an `&mut`, consider \
             accepting a closure that performs the write instead.",
        ],
        references: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure_owned(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        BorrowProfileOwned.measure(&input)
    }
    fn measure_borrowed(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        BorrowProfileBorrowed.measure(&input)
    }
    fn measure_mut(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        BorrowProfileMut.measure(&input)
    }

    fn first_value(ms: Vec<MetricMeasurement>) -> u32 {
        ms.into_iter().next().unwrap().value as u32
    }

    #[test]
    fn empty_signature_is_zero_in_each_axis() {
        assert_eq!(first_value(measure_owned("fn f() {}")), 0);
        assert_eq!(first_value(measure_borrowed("fn f() {}")), 0);
        assert_eq!(first_value(measure_mut("fn f() {}")), 0);
    }

    #[test]
    fn mixed_signature_splits() {
        let src = "fn f(a: i32, b: &str, c: &mut Vec<u8>, d: i32) {}";
        assert_eq!(first_value(measure_owned(src)), 2);
        assert_eq!(first_value(measure_borrowed(src)), 1);
        assert_eq!(first_value(measure_mut(src)), 1);
    }

    #[test]
    fn self_receiver_does_not_count() {
        let src = "struct S; impl S { fn f(&self, x: i32, y: &str) {} }";
        assert_eq!(first_value(measure_owned(src)), 1);
        assert_eq!(first_value(measure_borrowed(src)), 1);
        assert_eq!(first_value(measure_mut(src)), 0);
    }
}
