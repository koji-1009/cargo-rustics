//! Output of a single metric measurement.
//!
//! A `MetricMeasurement` is the *raw* value at one scope; the CLI compares it
//! against the configured threshold, attaches coverage / dismissals / `Rust
//! context`, and turns it into a `violation` record (plan §4.3). Keeping the
//! library output small means the same trait works for `informational`
//! metrics that never produce violations.

use crate::scope::ScopeRef;

/// One measurement at one scope.
///
/// `value` is `f64` so the same shape works for integer counts (CC,
/// `clone-density`) and ratios/derived metrics (Instability, Distance from
/// Main Sequence — Layer 1, plan §6.3) without a separate variant.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricMeasurement {
    /// Scope this measurement applies to.
    pub scope: ScopeRef,
    /// The measured value.
    pub value: f64,
}

impl MetricMeasurement {
    /// Constructs a new measurement.
    pub fn new(scope: ScopeRef, value: f64) -> Self {
        Self { scope, value }
    }
}
