//! Borrow profile — three companion lenses (Owned / Borrowed /
//! MutBorrowed). Layer 2 migration stub. See sibling stubs for the
//! migration plan.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

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
    fn measure(&self, _input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // TODO: port to ra_ap_syntax.
        Vec::new()
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
    fn measure(&self, _input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // TODO: port to ra_ap_syntax.
        Vec::new()
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
    fn measure(&self, _input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // TODO: port to ra_ap_syntax.
        Vec::new()
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
