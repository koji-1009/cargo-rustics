//! `lifetime-arity` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// lifetime-arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct LifetimeArity;

impl MetricCalculator for LifetimeArity {
    fn id(&self) -> &'static str {
        "lifetime-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Lifetime Arity",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Two-and-fewer is comfortable; three is the smell threshold;
            // five and we are at "rewrite this signature" territory.
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(5.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, _input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // TODO: port to ra_ap_syntax.
        Vec::new()
    }
}

const RATIONALE: &str = "\
Each lifetime parameter is one referential constraint a reader has to track. \
Two is normal; past three the signature is asking the reader to mentally \
solve a small constraint puzzle before they can call the function. Lifetime \
elision exists precisely so simple functions don't have to spell them out — \
when elision can't apply, the signature becomes a contract that requires \
study.";

const REFACTOR_HINTS: &[&str] = &[
    "Push the lifetimes into a struct: `struct Borrow<'a> { ... }`. The \
function becomes `fn f(b: Borrow<'_>) -> ...`, with one named-lifetime \
binding instead of N.",
    "Take ownership where possible — `String` instead of `&'a str`, `Vec<T>` \
instead of `&'a [T]`. The borrow-checker bookkeeping disappears.",
    "If the lifetimes only relate one input to one output, elision usually \
applies — try removing the explicit lifetime first and see if rustc \
infers it.",
];

const REFERENCES: &[&str] = &[];
