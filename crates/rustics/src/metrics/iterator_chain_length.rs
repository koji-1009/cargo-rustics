//! `iterator-chain-length` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// iterator-chain-length calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct IteratorChainLength;

impl MetricCalculator for IteratorChainLength {
    fn id(&self) -> &'static str {
        "iterator-chain-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Iterator Chain Length",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Three-link iterator chains are everywhere; six is the
            // smell threshold; ten is "name your intermediate values".
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
A method-call chain is a one-liner that hides each step's intent. \
Iterator pipelines naturally chain three or four links \
(`.iter().filter().map().sum()`); past six the reader has to mentally \
hold a long pipeline of transformations. Naming an intermediate value \
restores legibility.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the chain at the first stateful step (`fold`, `try_fold`, \
`scan`, `inspect`) — extract the prefix into a named local binding.",
    "Long chains often hide an early-return path that wants to be a \
plain `for` loop. The CC drops slightly and the early-return reads \
explicitly.",
    "If the chain ends with `collect()`, see if a `for` loop with \
`Vec::push` is clearer at the call site.",
];

const REFERENCES: &[&str] = &[];
