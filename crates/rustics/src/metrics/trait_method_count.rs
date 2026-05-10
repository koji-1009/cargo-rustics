//! `trait-method-count` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// trait-method-count calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct TraitMethodCount;

impl MetricCalculator for TraitMethodCount {
    fn id(&self) -> &'static str {
        "trait-method-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Trait Method Count",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(15.0)),
            default_error: Some(Threshold::new(30.0)),
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
A trait with many methods imposes a heavy contract on every implementor. \
Past 15 methods, splitting into smaller traits (with the original as a \
super-trait that combines them) usually makes implementations easier to \
write and read.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the trait into a hierarchy: `trait Read`, `trait Write`, then \
`trait ReadWrite: Read + Write {}` for the combined contract.",
    "Move methods that have natural defaults into a separate `*Ext` trait \
implemented blanket on the original.",
];

const REFERENCES: &[&str] = &[];
