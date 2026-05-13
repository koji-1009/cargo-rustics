//! Cross-file lenses — measurements / violations that need the
//! whole workspace, not a single file.
//!
//! Per-file lenses live in `rustics::metrics::*` behind the
//! `MetricCalculator` trait; that seam handles the 80% case where
//! every measurement is a function of one AST. Two situations
//! force the second seam:
//!
//! * a metric that aggregates *across* files (Martin's Afferent
//!   Coupling, trait-impl fan-out — counted per-target across all
//!   call sites);
//! * a metric whose granularity is the module / package, not the
//!   function or impl block (Instability `I = Ce / (Ce + Ca)`,
//!   anything that needs cargo metadata).
//!
//! Both submodules under here run after the per-file pass and feed
//! their output into the same [`Report`] the per-file pass built.
//! Their public API is identical: [`CrossFilePass`] containing the
//! union of violations and measurements.
//!
//! ## Adding a new cross-file lens
//!
//! 1. Add a submodule under `cross_file/`.
//! 2. Append its id(s) to [`CROSS_FILE_METRIC_IDS`] (this list is
//!    the single source of truth read by `analyze --metric`,
//!    `doctor`'s rustics.toml validator, and the manual drift
//!    gate).
//! 3. Add a manual entry under `## Lenses` in `doc/manual.md`.
//! 4. Hook the run in [`run_all`].

use std::path::Path;

use rustics::MetricSeverity;

use crate::discover::DiscoveredFile;
use crate::report::{MeasurementRecord, Violation};

pub mod coupling;
pub mod efferent_coupling;
pub mod function_complexity;

/// Result of one cross-file pass — the same shape every cross-file
/// lens emits, so `analyze.rs` merges them into the report with one
/// loop instead of bespoke wiring per pass.
#[derive(Default)]
pub struct CrossFilePass {
    /// Threshold-crossing findings, merged into `report.violations`.
    pub violations: Vec<Violation>,
    /// Per-instance values, merged into `report.measurements` so
    /// `regression`'s cosmetic-detection sees sub-threshold drifts
    /// and so the AI report has the data even when no violation
    /// fired.
    pub measurements: Vec<MeasurementRecord>,
}

impl CrossFilePass {
    /// Folds another pass into this one. Used by [`run_all`] to
    /// concatenate the per-lens passes.
    pub fn extend(&mut self, other: CrossFilePass) {
        self.violations.extend(other.violations);
        self.measurements.extend(other.measurements);
    }
}

/// Canonical ids of every lens computed by this module. The
/// `--metric` filter (`analyze`), the rustics.toml override
/// validator (`doctor`), and the manual drift gate (`manual`) all
/// read this list — adding a new cross-file lens is one edit here.
pub const CROSS_FILE_METRIC_IDS: &[&str] = &[
    "afferent-coupling",
    "instability",
    "efferent-coupling",
    "cyclomatic-complexity",
    "cognitive-complexity",
    "npath-complexity",
];

/// Empty marker carried in `coupling::run`'s signature for
/// historical compatibility — the HIR backend loads its own files
/// through `ra_ap_load_cargo` so the per-file slice isn't used.
/// Removing the parameter would touch every caller and the trait
/// is small enough that the dead-code warning is silenced at the
/// type level instead.
pub(super) struct ParsedFile;

/// Drives every cross-file lens, returning the combined output.
/// This is the single seam `analyze.rs` calls. Both cross-file
/// passes load the workspace through `ra_ap_load_cargo` themselves
/// (HIR resolution requires the whole crate graph, not a flat file
/// list), so no shared per-file AST is computed here.
pub fn run_all(workspace_root: &Path, files: &[DiscoveredFile]) -> CrossFilePass {
    let mut out = CrossFilePass::default();
    out.extend(coupling::run(workspace_root, &[]));
    out.extend(efferent_coupling::run(workspace_root, files));
    out.extend(function_complexity::run(workspace_root, files));
    out
}

/// Shared `count > warning/error` ladder. Used by every cross-file
/// lens that fires on count thresholds. Returns the matching
/// severity *and* the threshold value the violation tripped, so the
/// caller can populate the violation record without re-passing
/// constants.
pub(super) fn severity_for(count: u32, warning: u32, error: u32) -> Option<(MetricSeverity, u32)> {
    if count > error {
        Some((MetricSeverity::Error, error))
    } else if count > warning {
        Some((MetricSeverity::Warning, warning))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_below_warning_is_none() {
        assert!(severity_for(20, 20, 40).is_none());
    }

    #[test]
    fn severity_above_warning_is_warning() {
        let (s, t) = severity_for(21, 20, 40).unwrap();
        assert_eq!(s, MetricSeverity::Warning);
        assert_eq!(t, 20);
    }

    #[test]
    fn severity_above_error_is_error() {
        let (s, t) = severity_for(50, 20, 40).unwrap();
        assert_eq!(s, MetricSeverity::Error);
        assert_eq!(t, 40);
    }

    #[test]
    fn cross_file_metric_ids_unique_and_kebab_case() {
        let mut sorted: Vec<&'static str> = CROSS_FILE_METRIC_IDS.to_vec();
        sorted.sort_unstable();
        let dedup_count = {
            let mut d = sorted.clone();
            d.dedup();
            d.len()
        };
        assert_eq!(sorted.len(), dedup_count, "duplicate cross-file id");
        for id in CROSS_FILE_METRIC_IDS {
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "non-kebab-case id: {id}"
            );
        }
    }
}
