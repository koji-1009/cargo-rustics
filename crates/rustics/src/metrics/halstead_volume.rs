//! `halstead-volume` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// halstead-volume calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct HalsteadVolume;

impl MetricCalculator for HalsteadVolume {
    fn id(&self) -> &'static str {
        "halstead-volume"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Halstead Volume",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // listed warning 1000 as a starting point; // self-application calibration shows ordinary Rust functions cluster ~700–1500 due
            // to verbose punctuation. We raise the warning to 1500 and
            // the error to 3000 so the lens flags genuine outliers.
            default_warning: Some(Threshold::new(1500.0)),
            default_error: Some(Threshold::new(3000.0)),
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
Halstead Volume measures the size of a program in *information-theoretic* \
units — the number of bits required to write down the operator and \
operand stream. It is sensitive to both length and vocabulary, so a \
function that uses many distinct names scores higher than one that reuses \
the same handful even when the line counts agree. The metric was \
proposed in 1977; it survived because the measurement target — \
'how big is this implementation, including its vocabulary' — is real.";

const REFACTOR_HINTS: &[&str] = &[
    "If the function uses many one-off names (`x_a`, `x_b`, `tmp1`, `tmp2`), \
collapse them into a struct or enum so the vocabulary shrinks.",
    "Long arithmetic / formatting expressions can often move to a helper. The \
helper's name replaces a stretch of operators and operands at the call \
site.",
    "Lift repeated literal constants to `const` definitions at module level. \
The function's operand vocabulary contracts.",
];

const REFERENCES: &[&str] = &[
    "Halstead, M. H. (1977). Elements of Software Science. Elsevier.",
];
