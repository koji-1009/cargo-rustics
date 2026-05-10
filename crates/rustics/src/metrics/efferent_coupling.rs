//! `efferent-coupling` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// efferent-coupling calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EfferentCoupling;

impl MetricCalculator for EfferentCoupling {
    fn id(&self) -> &'static str {
        "efferent-coupling"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Efferent Coupling (Ce)",
            category: MetricCategory::Coupling,
            polarity: MetricPolarity::LowerIsBetter,
            // 20+ outgoing roots in a single module is the "this file
            // is doing too much" signal. The threshold is generous —
            // facade modules legitimately use many things.
            default_warning: Some(Threshold::new(20.0)),
            default_error: Some(Threshold::new(40.0)),
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
Efferent Coupling counts the things a module reaches outward to. A high \
Ce means the module ties many other things together — sometimes that is \
the right design (a facade), more often it means responsibilities have \
landed here that belong elsewhere. Pair with Afferent Coupling for \
the Instability ratio I = Ce / (Ca + Ce).";

const REFACTOR_HINTS: &[&str] = &[
    "Pull `use` statements that only one function uses inside that function — \
the module-level Ce drops without touching the function's behaviour.",
    "If most outgoing edges go to one larger system (auth, persistence), \
extract a small adapter module and have the rest of the file talk through \
the adapter only.",
    "Re-exports from a `prelude` module can collapse many `use` lines into \
one, lowering Ce while keeping the same names available.",
];

const REFERENCES: &[&str] = &[
    "Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of Dependencies.",
];
