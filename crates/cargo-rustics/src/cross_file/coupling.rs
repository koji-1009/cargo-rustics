//! Cross-file Martin coupling: Afferent Coupling (Ca) +
//! Instability (I = Ce_internal / (Ce_internal + Ca)).
//!
//! HIR-backed via `rustics_ra::metrics::coupling_graph`. The
//! AST-based predecessor of this module walked each file's `use`
//! items and resolved targets via longest-prefix matching against
//! cargo metadata's crate list; that approximation tied the
//! resolution to file *paths* rather than semantic identity, which
//! mis-attributed re-exported items and lost edges that flowed
//! through `pub use` chains. HIR runs name resolution + macro
//! expansion through rust-analyzer-as-library, so each `use` leaf
//! resolves to the canonical `Definition` and the dependency graph
//! reflects the real workspace topology.
//!
//! Granularity is per-file. Each `.rs` file is one vertex; the
//! per-file `Ca` is the count of *other* workspace files that
//! reach into this one through a HIR-resolved `use`. `Ce_internal`
//! is the count of workspace-internal files this file's `use`
//! statements reach (separate from the `efferent-coupling` lens,
//! which counts *external* crates). Instability is the standard
//! ratio. Files isolated from the workspace dependency graph (no
//! in / no out) report `instability = 0` rather than NaN.
//!
//! References:
//! * Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of
//!   Dependencies.
//! * Distance-from-Main-Sequence was implemented and then *removed*
//!   under the multicollinearity rule. Self-application showed `D ↔
//!   instability r = −0.994` (n = 86). With `A ≈ 0` for natural
//!   Rust struct-only modules, D collapses to `1 − I`. Keeping the
//!   simpler signal.

use std::path::Path;

use rustics::{violation_id, MetricSeverity, ScopeKind};

use crate::report::{MeasurementRecord, Violation};

use super::{CrossFilePass, ParsedFile};

const AFFERENT_WARNING: u32 = 20;
const AFFERENT_ERROR: u32 = 40;

/// Drives the HIR-backed coupling pass and shapes the result into a
/// [`CrossFilePass`]. Emits, per workspace file:
///
/// * One `afferent-coupling` measurement (always — so `regression`
///   sees sub-threshold Ca drifts).
/// * One `afferent-coupling` violation if Ca > [`AFFERENT_WARNING`].
/// * One `instability` measurement (informational; no violation
///   shape — instability is a ratio, not a count).
///
/// On `cargo metadata` / `ra_ap_load_cargo` failure (e.g. workspace
/// without a Cargo.toml) the pass returns empty and logs the error;
/// the analyze pipeline degrades gracefully.
pub(super) fn run(workspace_root: &Path, _parsed: &[ParsedFile]) -> CrossFilePass {
    let raw = match rustics_ra::metrics::coupling_graph::detect_at(workspace_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "rustics: HIR coupling pass could not load workspace at {}: {e:#}",
                workspace_root.display()
            );
            return CrossFilePass::default();
        }
    };
    let workspace_prefix = workspace_root.to_string_lossy().into_owned();
    let mut measurements: Vec<MeasurementRecord> = Vec::with_capacity(raw.len() * 2);
    let mut violations: Vec<Violation> = Vec::new();
    for m in raw {
        let relative = strip_workspace_prefix(&m.file, &workspace_prefix);
        measurements.push(MeasurementRecord {
            file: relative.clone(),
            scope: m.scope.clone(),
            metric: "afferent-coupling".into(),
            value: f64::from(m.afferent),
        });
        measurements.push(MeasurementRecord {
            file: relative.clone(),
            scope: m.scope.clone(),
            metric: "instability".into(),
            value: m.instability,
        });
        if let Some((severity, threshold)) =
            super::severity_for(m.afferent, AFFERENT_WARNING, AFFERENT_ERROR)
        {
            violations.push(violation(
                &relative, &m.scope, m.afferent, severity, threshold,
            ));
        }
    }
    CrossFilePass {
        violations,
        measurements,
    }
}

fn strip_workspace_prefix(file: &str, workspace_prefix: &str) -> String {
    file.strip_prefix(workspace_prefix)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| file.to_string())
}

fn violation(
    relative: &str,
    scope: &str,
    count: u32,
    severity: MetricSeverity,
    threshold: u32,
) -> Violation {
    let id = violation_id(relative, scope, "afferent-coupling");
    Violation {
        id,
        file: relative.to_string(),
        line: 1,
        scope: scope.to_string(),
        scope_kind: ScopeKind::Module,
        metric: "afferent-coupling".into(),
        value: f64::from(count),
        threshold: f64::from(threshold),
        severity,
        rationale: Some(format!(
            "{count} workspace files import from `{scope}`. A high \
             afferent-coupling means many places in the codebase break \
             if you change this module's public surface — invest in \
             narrow APIs, backwards-compatible changes, or splitting \
             the module."
        )),
        refactor_hints: REFACTOR_HINTS.iter().map(|s| s.to_string()).collect(),
        references: REFERENCES.iter().map(|s| s.to_string()).collect(),
        rust_context: Default::default(),
        complexity_justified: None,
    }
}

const REFACTOR_HINTS: &[&str] = &[
    "If many files reach into a single deep symbol of this module, \
     publish a focused re-export at a stable path so the spread of \
     transitive dependents narrows.",
    "Keep the module's public surface trait-shaped so dependents bind \
     to a contract, not a concrete implementation.",
    "If the module has both high Ca and high Ce (= high coupling in \
     both directions), it is a likely 'central hub' — consider \
     splitting it by role.",
];

const REFERENCES: &[&str] =
    &["Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of Dependencies."];
