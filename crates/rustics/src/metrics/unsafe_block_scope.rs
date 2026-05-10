//! `unsafe-block-scope` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// unsafe-block-scope calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsafeBlockScope;

impl MetricCalculator for UnsafeBlockScope {
    fn id(&self) -> &'static str {
        "unsafe-block-scope"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Unsafe Block Scope (lines)",
            category: MetricCategory::RustSafety,
            polarity: MetricPolarity::LowerIsBetter,
            // 20 lines of unsafe in one function is already a lot; past
            // 50 the soundness review burden is heavy.
            default_warning: Some(Threshold::new(20.0)),
            default_error: Some(Threshold::new(50.0)),
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
Every line inside an `unsafe { … }` block is a soundness obligation: the \
author has promised the compiler that the operation upholds invariants the \
type system cannot check. Long unsafe blocks scale that promise — five \
lines you can audit; fifty lines you cannot. The metric pushes you to \
keep unsafe locally tight so the surrounding safe code can be trusted on \
its types alone.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull the unsafe block down to the smallest possible expression. The \
surrounding safe code does not need the contract.",
    "Wrap the unsafe operation in a safe abstraction (a small helper that \
returns a checked result) and call it from safe code.",
    "If the same unsafe operation appears in multiple places, extract a \
single safe wrapper; the audit surface stays in one file.",
];

const REFERENCES: &[&str] = &[
];
