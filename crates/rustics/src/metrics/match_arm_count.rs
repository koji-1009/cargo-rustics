//! `match-arm-count` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// match-arm-count calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MatchArmCount;

impl MetricCalculator for MatchArmCount {
    fn id(&self) -> &'static str {
        "match-arm-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Match Arm Count (sealed-aware)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(7.0)),
            default_error: Some(Threshold::new(12.0)),
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
A `match` with many arms reads as a switch table: the reader holds \
each arm's pattern in working memory while scanning for the one that \
applies. Sealed enums with one arm per variant are the idiomatic \
exhaustive case (compile-time-checked); this lens flags wide matches \
where you, the human reader, are doing the dispatch work that an \
extracted helper enum or `match` on a smaller key could absorb.";

const REFACTOR_HINTS: &[&str] = &[
    "Group arm clusters into a helper enum: \
`enum Action { File(FileOp), Net(NetOp) }` then match those.",
    "Use guard clauses for early arms (`0..10 if x % 2 == 0 => …`) \
to collapse repetitive conditions and reduce visual branching.",
    "If the dispatch is on a string / id, replace the `match` with \
a `HashMap<&'static str, fn(...)>` lookup at the call site.",
    "Wide matches inside `impl Trait for T` can usually be split: \
each variant's arm becomes its own helper method, and the `match` \
shrinks to a one-liner that delegates.",
];

const REFERENCES: &[&str] = &[
];
