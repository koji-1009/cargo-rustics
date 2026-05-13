//! HIR-aware function-level complexity lenses (CC + Cognitive +
//! NPath) dispatched through `rustics_ra::metrics::function_complexity`.
//!
//! Replaces the per-file AST walkers in `rustics::metrics::
//! cyclomatic_complexity`, `cognitive_complexity`, and
//! `npath_complexity` with HIR-refined variants:
//!
//! * **CC** — same McCabe formula plus sealed-match exactness via
//!   HIR type resolution.
//! * **Cognitive** — Sonar's B1 + B2 + B3 plus the B1 direct-
//!   recursion rule the AST lens was missing.
//! * **NPath** — Nejmeh's multiplicative path count plus the same
//!   sealed-match refinement as CC.
//!
//! Per-function measurements + per-function violations, just like
//! the per-file lenses they replace. Thresholds match the AST
//! defaults so the backend swap changes accuracy, not contract.

use std::path::Path;

use rustics::{violation_id, MetricSeverity, ScopeKind};

use crate::discover::DiscoveredFile;
use crate::report::{MeasurementRecord, Violation};

use super::CrossFilePass;

const CC_WARNING: f64 = 10.0;
const CC_ERROR: f64 = 20.0;
const COGNITIVE_WARNING: f64 = 15.0;
const COGNITIVE_ERROR: f64 = 25.0;
// NPath is off-by-default in the AST lens — no automatic thresholds.

/// Runs the HIR function-complexity walker and shapes results into
/// the cross-file pass. Returns empty if the workspace fails to
/// load (no Cargo.toml, ra_ap_load_cargo errors); the analyze
/// pipeline degrades gracefully.
pub(super) fn run(workspace_root: &Path, files: &[DiscoveredFile]) -> CrossFilePass {
    let raw = match rustics_ra::metrics::function_complexity::detect_at(workspace_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "rustics: HIR function-complexity pass could not load workspace at {}: {e:#}",
                workspace_root.display()
            );
            return CrossFilePass::default();
        }
    };
    let workspace_prefix = workspace_root.to_string_lossy().into_owned();
    let mut pass = CrossFilePass::default();
    for fc in raw {
        let relative = strip_workspace_prefix(&fc.file, &workspace_prefix);
        if !discovered_match(files, &relative) {
            continue;
        }
        emit_function_lenses(&relative, &fc, &mut pass);
    }
    pass
}

/// Emits the three measurements (CC, Cognitive, NPath) for one
/// function and any threshold-crossing violations. Pulled out of
/// `run` so the orchestrator stays narrow.
fn emit_function_lenses(
    relative: &str,
    fc: &rustics_ra::metrics::function_complexity::FunctionComplexity,
    pass: &mut CrossFilePass,
) {
    let cc = f64::from(fc.cyclomatic);
    emit_one(
        pass,
        relative,
        fc,
        "cyclomatic-complexity",
        cc,
        Some((CC_WARNING, CC_ERROR)),
        CC_RATIONALE,
        CC_REFACTOR_HINTS,
        CC_REFERENCES,
    );
    let cog = f64::from(fc.cognitive);
    emit_one(
        pass,
        relative,
        fc,
        "cognitive-complexity",
        cog,
        Some((COGNITIVE_WARNING, COGNITIVE_ERROR)),
        COG_RATIONALE,
        COG_REFACTOR_HINTS,
        COG_REFERENCES,
    );
    emit_one(
        pass,
        relative,
        fc,
        "npath-complexity",
        fc.npath,
        None,
        "",
        &[],
        &[],
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_one(
    pass: &mut CrossFilePass,
    relative: &str,
    fc: &rustics_ra::metrics::function_complexity::FunctionComplexity,
    metric: &str,
    value: f64,
    thresholds: Option<(f64, f64)>,
    rationale: &str,
    refactor_hints: &[&str],
    references: &[&str],
) {
    pass.measurements.push(MeasurementRecord {
        file: relative.to_string(),
        scope: fc.scope.clone(),
        metric: metric.into(),
        value,
    });
    let Some((warning, error)) = thresholds else {
        return;
    };
    let Some((severity, threshold)) = severity_for_f64(value, warning, error) else {
        return;
    };
    pass.violations.push(violation(
        relative,
        fc.line,
        &fc.scope,
        metric,
        value,
        threshold,
        severity,
        rationale,
        refactor_hints,
        references,
    ));
}

fn strip_workspace_prefix(file: &str, workspace_prefix: &str) -> String {
    file.strip_prefix(workspace_prefix)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| file.to_string())
}

fn discovered_match(files: &[DiscoveredFile], relative: &str) -> bool {
    files.iter().any(|f| f.relative == relative)
}

/// Severity ladder for `f64`-typed values. `super::severity_for`
/// is `u32`-typed; CC / Cognitive / NPath round through `f64`
/// because npath can exceed `u32::MAX` on pathological functions.
fn severity_for_f64(value: f64, warning: f64, error: f64) -> Option<(MetricSeverity, f64)> {
    if value >= error {
        Some((MetricSeverity::Error, error))
    } else if value >= warning {
        Some((MetricSeverity::Warning, warning))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn violation(
    relative: &str,
    line: u32,
    scope: &str,
    metric: &str,
    value: f64,
    threshold: f64,
    severity: MetricSeverity,
    rationale: &str,
    refactor_hints: &[&str],
    references: &[&str],
) -> Violation {
    let id = violation_id(relative, scope, metric);
    Violation {
        id,
        file: relative.to_string(),
        line: line as usize,
        scope: scope.to_string(),
        scope_kind: ScopeKind::FreeFunction,
        metric: metric.to_string(),
        value,
        threshold,
        severity,
        rationale: Some(rationale.to_string()),
        refactor_hints: refactor_hints.iter().map(|s| s.to_string()).collect(),
        references: references.iter().map(|s| s.to_string()).collect(),
        rust_context: Default::default(),
        complexity_justified: None,
    }
}

const CC_RATIONALE: &str = "\
Cyclomatic Complexity counts independent execution paths in a function. \
McCabe established 10 as the empirical break-even where defect rates start \
climbing. The HIR-backed walker refines the sealed-match adjustment by \
resolving the scrutinee's type — only true enum matches without a `_` arm \
contribute 0; matches on `bool` / numeric / string scrutinees count each \
arm as a real branch.";

const CC_REFACTOR_HINTS: &[&str] = &[
    "Extract independent branches into named helpers — each is a unit the \
reader can scan separately.",
    "Replace nested `if`/`else` chains with a `match` on a small enum. \
Sealed-aware CC charges 0 for the new match.",
    "Lift early-exit checks (`return Err(...)` / `return None`) so the \
function reads as a happy-path sequence with guards on top.",
];

const CC_REFERENCES: &[&str] = &[
    "McCabe, T. J. (1976). A Complexity Measure. IEEE Transactions on Software Engineering, 2(4), 308-320.",
];

const COG_RATIONALE: &str = "\
Cognitive Complexity (SonarSource 2018) penalises control-flow plus \
nesting plus logical-op sequences plus direct recursion (B1's +1). The \
HIR-backed walker resolves each call expression and credits the B1 +1 \
when the callee is the enclosing function — a check the AST walker \
cannot perform reliably across module-prefixed call shapes.";

const COG_REFACTOR_HINTS: &[&str] = &[
    "Lift the deepest nested block into a named helper.",
    "Replace deep `if`/`else` chains with a `match` on a small enum.",
];

const COG_REFERENCES: &[&str] = &[
    "Campbell, G. A. (2018). Cognitive Complexity: A new way of measuring understandability. SonarSource white paper (industry source, not peer-reviewed).",
];
