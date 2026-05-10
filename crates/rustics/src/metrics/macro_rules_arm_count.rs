//! `macro-rules-arm-count` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// macro-rules-arm-count calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacroRulesArmCount;

impl MetricCalculator for MacroRulesArmCount {
    fn id(&self) -> &'static str {
        "macro-rules-arm-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "macro_rules! Arm Count",
            category: MetricCategory::Macro,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(8.0)),
            default_error: Some(Threshold::new(15.0)),
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
Each arm of a `macro_rules!` definition is one rule the expander tries; \
many arms scale the cognitive load of reading the macro the way many \
`match` arms scale the cognitive load of reading a function. Past eight \
arms, the order-dependence and overlap between rules become hard to keep \
straight.";

const REFACTOR_HINTS: &[&str] = &[
    "If the rules dispatch on a small set of categories, push the categories \
into a helper macro and call it from the main macro's arms.",
    "Procedural macros (`#[proc_macro]`) replace declarative macros for the \
complex cases — when the arm count grows past a dozen, it is usually time \
to convert.",
    "Some `macro_rules!` arms are added defensively (`($($any:tt)*) => {}`); \
make sure those are necessary rather than vestigial.",
];

const REFERENCES: &[&str] = &[];
