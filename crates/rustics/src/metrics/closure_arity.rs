//! `closure-arity` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};

/// closure-arity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClosureArity;

impl MetricCalculator for ClosureArity {
    fn id(&self) -> &'static str {
        "closure-arity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Closure Arity",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            // Iterator chains naturally hit 3-5; past that the body
            // is more closures than statements.
            default_warning: Some(Threshold::new(6.0)),
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
Each inline closure introduces a fresh scope with its own captures and \
return-type story. Iterator pipelines often have 3-5 closures; past six, \
the function reads more like a chain of small lambdas than a sequence of \
statements, and the captures + early-return interactions become hard to \
trace.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract a closure that captures more than one local into a named \
local function. The captures become arguments and the body reads like a \
linear sequence.",
    "Long iterator chains often split at the first stateful operation \
(`fold`, `try_fold`, `scan`); the post-split portion can become a plain \
`for` loop without losing brevity.",
    "Closures whose bodies are themselves multi-statement blocks usually \
want to be functions — `|x| { let y = …; let z = …; …  }` is a function \
in disguise.",
];

const REFERENCES: &[&str] = &[];
