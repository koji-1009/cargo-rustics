//! `result-chain-depth` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// result-chain-depth calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResultChainDepth;

impl MetricCalculator for ResultChainDepth {
    fn id(&self) -> &'static str {
        "result-chain-depth"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Result Chain Depth (?)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Calibrated for Rust idioms: `?` is cheap, the chain has to
            // be *long* before reading suffers.
            default_warning: Some(Threshold::new(6.0)),
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
Each `?` is a place this expression can early-return on `Err`. Inference \
makes the error path mechanical, so a long chain is *legible* — but past \
six links a reader still has to track which `?` corresponds to which \
fallible step in the chain. The threshold is generous compared to the \
hand-rolled `match Result { … }` ladder it replaces.";

const REFACTOR_HINTS: &[&str] = &[
    "Break a long chain into named local bindings: `let x = a()?; let y = \
x.b()?; …`. Each step now has a name and the chain depth resets.",
    "If the chain is mostly `.method()?`, consider whether `.method()` should \
return the type already (some failure points may collapse).",
    "Use combinators (`map`, `and_then`, `?` as a single tail) sparingly when \
the steps are heterogeneous — the named-binding form usually reads clearer.",
];

const REFERENCES: &[&str] = &[
];
