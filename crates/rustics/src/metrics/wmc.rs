//! `wmc` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// wmc calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Wmc;

impl MetricCalculator for Wmc {
    fn id(&self) -> &'static str {
        "wmc"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Weighted Methods per Class (CK 1994)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            // 50 / 100 split is the SonarSource / Basili-tradition
            // threshold. CK 1994 deliberately leaves it project-set.
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
Weighted Methods per Class (CK 1994) sums the cyclomatic complexity \
of every method in the class — a single number that captures both \
width (how many methods) and depth (how complex each is). High WMC \
correlates empirically with defect density and change-proneness in \
multiple validation studies (Basili et al. 1996, Subramanyam & \
Krishnan 2003). In Rust the natural unit is the `impl` block: one \
score per inherent or trait impl. Past 50 the type is usually \
carrying multiple roles; past 100 the load is rarely defensible.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the impl block by role: separate `impl Foo { /* core */ }` \
from `impl Foo { /* serde */ }` so each block scores independently.",
    "Extract methods that delegate to a helper type; the type's \
constructor becomes one method and the helper carries the complexity.",
    "If the methods share a code structure (e.g. each is a `match` over \
the same variant), collapse the dispatch into a single method that \
takes the variant as a parameter.",
];

const REFERENCES: &[&str] = &[
    "Chidamber & Kemerer (1994). A Metrics Suite for Object Oriented \
Design. IEEE Trans. Softw. Eng. 20(6): 476-493.",
    "Basili, Briand & Melo (1996). A validation of object-oriented \
design metrics as quality indicators. IEEE TSE 22(10): 751-761.",
    "Subramanyam & Krishnan (2003). Empirical analysis of CK metrics \
for object-oriented design complexity. IEEE TSE 29(4): 297-310.",
];
