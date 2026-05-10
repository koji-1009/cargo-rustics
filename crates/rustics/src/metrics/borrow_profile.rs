//! Borrow profile — three companion lenses on parameter shape.

use ra_ap_syntax::ast;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_functions;

/// `borrow-profile-owned` calculator.
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
        measure_functions(input.tree, |frame| {
            Some(f64::from(count_params(&frame.item, ParamShape::Owned)))
        })
    }
}

/// `borrow-profile-borrowed` calculator.
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
        measure_functions(input.tree, |frame| {
            Some(f64::from(count_params(&frame.item, ParamShape::Borrowed)))
        })
    }
}

/// `borrow-profile-mut` calculator.
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
        measure_functions(input.tree, |frame| {
            Some(f64::from(count_params(&frame.item, ParamShape::MutBorrowed)))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamShape {
    Owned,
    Borrowed,
    MutBorrowed,
}

fn count_params(fn_: &ast::Fn, want: ParamShape) -> u32 {
    let Some(params) = fn_.param_list() else {
        return 0;
    };
    let mut n = 0u32;
    for param in params.params() {
        // `self` receivers (`&self`, `self`, `&mut self`) are
        // captured by `params.self_param()` separately; `.params()`
        // yields the positional parameters only.
        if param_matches_shape(&param, want) {
            n += 1;
        }
    }
    n
}

fn param_matches_shape(param: &ast::Param, want: ParamShape) -> bool {
    let Some(ty) = param.ty() else {
        return false;
    };
    let actual = classify_type(&ty);
    actual == want
}

fn classify_type(ty: &ast::Type) -> ParamShape {
    match ty {
        ast::Type::RefType(r) => {
            if r.mut_token().is_some() {
                ParamShape::MutBorrowed
            } else {
                ParamShape::Borrowed
            }
        }
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
