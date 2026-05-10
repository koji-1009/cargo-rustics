//! `proc-macro-presence` — Layer 2 migration stub.
//!
//! The real implementation will be re-added on top of
//! `ra_ap_syntax`. Until then `measure()` returns an empty vec
//! and the lens contributes no measurements; metadata is preserved
//! so `cargo rustics rules` and `explain` keep working.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};

/// proc-macro-presence calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcMacroPresence;

impl MetricCalculator for ProcMacroPresence {
    fn id(&self) -> &'static str {
        "proc-macro-presence"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Proc-Macro Presence",
            category: MetricCategory::Macro,
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
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
Functions decorated with proc-macro attributes (e.g. `#[tokio::main]`, \
`#[axum::handler]`, `#[serde::Serialize]`) execute code from another \
crate at compile time. The expanded body can be larger than the source \
suggests; reading the source alone misses the actual control flow. The \
metric flags such functions so the AI report can hint that the lens \
output is incomplete when the proc-macro is doing a lot of work.";

const REFACTOR_HINTS: &[&str] = &[
    "If the proc-macro is expanding into substantial logic, run \
`cargo rustics analyze --expanded-macros` to measure the \
post-expansion AST.",
    "Consider whether the proc-macro is essential or merely convenient — \
some attribute macros can be replaced by a plain function for code \
that the team has to read often.",
];

const REFERENCES: &[&str] = &[
];
