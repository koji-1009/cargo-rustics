//! `generic-arity` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// generic-arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct GenericArity;

impl MetricCalculator for GenericArity {
    fn id(&self) -> &'static str {
        "generic-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Generic Arity (type params + where bounds)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(4.0)),
            default_error: Some(Threshold::new(7.0)),
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
A function whose signature has many type parameters and where-bounds is \
asking the reader to mentally solve a small trait-resolution puzzle. The \
number summarises how much of the bound surface the call site has to \
satisfy. Past 4 the signature is best read with rustdoc rendering.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace some generic parameters with `impl Trait` arguments — the bound \
moves out of the visible signature.",
    "Group co-occurring bounds into a single trait alias (`trait My: A + B + C \
{}` then `T: My`). The `where` clause shrinks.",
    "If a parameter is always instantiated with the same type, just use that \
type directly.",
];

const REFERENCES: &[&str] = &[];
