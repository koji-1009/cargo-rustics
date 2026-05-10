//! `format-density` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// format-density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct FormatDensity;

impl MetricCalculator for FormatDensity {
    fn id(&self) -> &'static str {
        "format-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Format Density",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::LowerIsBetter,
            // Five `println!` calls in one function is unusual outside
            // a CLI driver; ten is loud.
            default_warning: Some(Threshold::new(5.0)),
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
Each format-class macro builds a `String` through the formatting \
machinery — fine in setup / display code, expensive in hot loops. The \
metric is a companion to clone-density: format calls are *another* \
allocation site that escapes the borrow story.";

const REFACTOR_HINTS: &[&str] = &[
    "Pre-format strings outside a hot loop into a `&str` and reuse them \
inside.",
    "Replace `format!` + `push_str` chains with `write!` on a re-used \
`String`/`Vec<u8>` buffer.",
    "If most calls are `println!`/`eprintln!`, consider whether the function \
should return a value the caller logs at one site instead.",
];

const REFERENCES: &[&str] = &[];
