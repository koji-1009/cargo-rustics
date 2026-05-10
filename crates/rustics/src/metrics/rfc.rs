//! `rfc` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// rfc calculator.
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

    fn measure(&self, _input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        // TODO: port to ra_ap_syntax.
        Vec::new()
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
