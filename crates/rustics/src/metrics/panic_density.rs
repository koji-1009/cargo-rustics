//! `panic-density` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// panic-density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanicDensity;

impl MetricCalculator for PanicDensity {
    fn id(&self) -> &'static str {
        "panic-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Panic Density (unwrap_or-aware)",
            category: MetricCategory::RustSafety,
            polarity: MetricPolarity::LowerIsBetter,
            // Three panicking paths in one function is the "stop and
            // think" threshold; ten is "this needs an error type".
            default_warning: Some(Threshold::new(3.0)),
            default_error: Some(Threshold::new(10.0)),
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
Each panicking site is a runtime crash waiting for the wrong input. \
`.unwrap()` and `.expect()` look small but they are the same as `panic!()` \
under bad data. A high count says the function is not modelling its error \
cases — it is hoping. The unwrap_or-aware adjustment keeps the lens from \
flagging defensible APIs (`.unwrap_or_default()`, `.unwrap_or_else(...)`) \
that cannot panic by construction.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace `.unwrap()` on `Option` with `.unwrap_or(default)` or \
`.ok_or(error)?`. The `?` keeps the linear shape.",
    "Replace `.expect(\"…\")` on `Result` with `?` and let the caller see the \
real error.",
    "If the panic represents an internal invariant the function genuinely \
guarantees, leave it but document the invariant in a `// SAFETY:` comment \
and consider a `debug_assert!` instead.",
    "Wrap repeated panics into one early-return guard (`let-else`) at the \
top of the function.",
];

const REFERENCES: &[&str] = &[
];
