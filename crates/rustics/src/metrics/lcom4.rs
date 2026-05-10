//! `lcom4` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// lcom4 calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lcom4;

impl MetricCalculator for Lcom4 {
    fn id(&self) -> &'static str {
        "lcom4"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Lack of Cohesion in Methods, v4 (Hitz & Montazeri 1995)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            // 1 is fully cohesive. 2+ → at least one cluster is
            // separable. We warn at 2 (per Hitz & Montazeri's
            // "needs review") and error at 5 (effectively "the
            // impl block is several types in disguise").
            default_warning: Some(Threshold::new(2.0)),
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
LCOM4 (Hitz & Montazeri 1995) counts disjoint method clusters in an \
inherent impl block — methods that don't share any field access or \
call relationship. A cohesive impl scores 1: every method reaches \
every other through some chain of shared state or calls. Score ≥ 2 \
means the block has independent method clusters that could be split \
into separate types without losing anything. \
Trait impls are skipped: their method set is dictated by the trait \
contract, not a cohesion choice the author can refactor. The metric \
was validated by Marinescu (2002) as a defect-density predictor \
superior to the original CK LCOM definition.";

const REFACTOR_HINTS: &[&str] = &[
    "Group the disjoint clusters into separate types: each cluster \
becomes a struct that owns the fields its methods touch.",
    "If one cluster is a small constructor + helper pair, move it \
into a free function or an `impl T` block dedicated to that role.",
    "Methods that touch *no* fields and aren't called by other methods \
in the impl form their own singleton component. Consider whether \
they belong on the type at all — they might be better as free \
functions.",
];

const REFERENCES: &[&str] = &[
    "Hitz & Montazeri (1995). Measuring coupling and cohesion in \
object-oriented systems. Proc. Int. Symp. on Applied Corporate Computing.",
    "Marinescu (2002). Measurement and quality in object-oriented design \
— validation of LCOM4 as defect predictor.",
];
