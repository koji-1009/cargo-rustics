//! `await-depth` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// await-depth calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct AwaitDepth;

impl MetricCalculator for AwaitDepth {
    fn id(&self) -> &'static str {
        "await-depth"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Await Depth (nested only)",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
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
Each `.await` is a suspension point; nested awaits within one expression \
mean the function is composing several async operations into a single \
sequenced computation. The metric does not penalise sequential awaits — \
those are a flat list, no harder to read than a flat list of statements. \
Past three links the chain becomes hard to reason about for cancellation \
and error propagation.";

const REFACTOR_HINTS: &[&str] = &[
    "Pull each `.await` into its own `let` binding. Each step gets a name \
and the chain flattens.",
    "If the awaits compose a pipeline, consider an explicit combinator \
(`futures::join!`, `tokio::try_join!`) so the parallel structure is visible.",
    "When the chain mixes `Result` and `Future`, the `await?` form is \
shorthand for two operations — splitting them often makes the error \
handling clearer.",
];

const REFERENCES: &[&str] = &[
];
