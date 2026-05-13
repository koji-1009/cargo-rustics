//! HIR-aware impl-block cohesion lenses (LCOM4 + RFC + WMC).
//!
//! Replaces `rustics::metrics::lcom4 / rfc / wmc` (per-file AST
//! walkers) with HIR-resolved variants that handle aliased `self`
//! via type resolution, disambiguate method-calls from
//! free-function calls in RFC's R set, and inherit
//! `function_complexity`'s sealed-match-aware CC for WMC.

use std::path::Path;

use rustics::{violation_id, MetricSeverity, ScopeKind};

use crate::discover::DiscoveredFile;
use crate::report::{MeasurementRecord, Violation};

use super::CrossFilePass;

const LCOM4_WARNING: f64 = 2.0;
const LCOM4_ERROR: f64 = 4.0;
const RFC_WARNING: f64 = 50.0;
const RFC_ERROR: f64 = 100.0;
const WMC_WARNING: f64 = 50.0;
const WMC_ERROR: f64 = 100.0;

/// Drives the HIR cohesion walker and shapes results into the
/// cross-file pass. Returns empty on workspace-load failure so the
/// analyze pipeline degrades gracefully.
pub(super) fn run(workspace_root: &Path, files: &[DiscoveredFile]) -> CrossFilePass {
    let raw = match rustics_ra::metrics::impl_cohesion::detect_at(workspace_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "rustics: HIR impl-cohesion pass could not load workspace at {}: {e:#}",
                workspace_root.display()
            );
            return CrossFilePass::default();
        }
    };
    let workspace_prefix = workspace_root.to_string_lossy().into_owned();
    let mut pass = CrossFilePass::default();
    for c in raw {
        let relative = strip_workspace_prefix(&c.file, &workspace_prefix);
        if !discovered_match(files, &relative) {
            continue;
        }
        emit_impl_lenses(&relative, &c, &mut pass);
    }
    pass
}

fn emit_impl_lenses(
    relative: &str,
    c: &rustics_ra::metrics::impl_cohesion::ImplCohesion,
    pass: &mut CrossFilePass,
) {
    if let Some(lcom4) = c.lcom4 {
        let value = f64::from(lcom4);
        emit_one(
            pass,
            relative,
            c,
            "lcom4",
            value,
            Some((LCOM4_WARNING, LCOM4_ERROR)),
            LCOM4_RATIONALE,
            LCOM4_REFACTOR_HINTS,
            LCOM4_REFERENCES,
        );
    }
    let rfc = f64::from(c.rfc);
    emit_one(
        pass,
        relative,
        c,
        "rfc",
        rfc,
        Some((RFC_WARNING, RFC_ERROR)),
        RFC_RATIONALE,
        RFC_REFACTOR_HINTS,
        RFC_REFERENCES,
    );
    let wmc = f64::from(c.wmc);
    emit_one(
        pass,
        relative,
        c,
        "wmc",
        wmc,
        Some((WMC_WARNING, WMC_ERROR)),
        WMC_RATIONALE,
        WMC_REFACTOR_HINTS,
        WMC_REFERENCES,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_one(
    pass: &mut CrossFilePass,
    relative: &str,
    c: &rustics_ra::metrics::impl_cohesion::ImplCohesion,
    metric: &str,
    value: f64,
    thresholds: Option<(f64, f64)>,
    rationale: &str,
    refactor_hints: &[&str],
    references: &[&str],
) {
    pass.measurements.push(MeasurementRecord {
        file: relative.to_string(),
        scope: c.scope.clone(),
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
        c.line,
        &c.scope,
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

fn severity_for_f64(value: f64, warning: f64, error: f64) -> Option<(MetricSeverity, f64)> {
    if value > error {
        Some((MetricSeverity::Error, error))
    } else if value > warning {
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
        scope_kind: ScopeKind::ImplBlock,
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

const LCOM4_RATIONALE: &str = "\
LCOM4 (Hitz & Montazeri 1995) reports the number of disjoint \
method clusters in an inherent `impl` block. LCOM4 = 1 means every \
method reaches every other through field-share or self-call edges; \
LCOM4 ≥ 2 suggests the impl could split into separate types. The \
HIR walker resolves aliased `self` (`let s = self; s.field`) and \
qualified self-calls (`Self::method`, `<Self as Trait>::method`) \
through type resolution, so the cohesion graph is exact rather \
than literal-`self` only.";

const LCOM4_REFACTOR_HINTS: &[&str] = &[
    "If methods cluster around two disjoint field sets, split the impl into two struct + impl pairs.",
    "If a cluster of methods shares no state with the rest, move them onto a sibling type the original delegates to.",
];

const LCOM4_REFERENCES: &[&str] = &[
    "Hitz, M., & Montazeri, B. (1995). Measuring Coupling and Cohesion In Object-Oriented Systems.",
];

const RFC_RATIONALE: &str = "\
RFC (Chidamber & Kemerer 1994) = |M ∪ R| where M is the set of \
methods on the impl and R is the set of methods called from M. \
The HIR walker resolves each call expression and only counts \
associated functions (methods) in R — free-function calls \
(`module::helper()`) don't enter R, so the count reflects true \
method-message dispatch.";

const RFC_REFACTOR_HINTS: &[&str] = &[
    "If R is large, push some of the called helpers behind a trait that the impl depends on — the contract narrows the response set.",
    "If a method delegates to many sub-methods, consider whether the impl should split into smaller types with their own R.",
];

const RFC_REFERENCES: &[&str] =
    &["Chidamber, S. R., & Kemerer, C. F. (1994). A metrics suite for object oriented design."];

const WMC_RATIONALE: &str = "\
WMC (Chidamber & Kemerer 1994) = sum of cyclomatic complexity \
across methods in an inherent `impl` block. The HIR walker uses \
the same sealed-match-aware CC as `cyclomatic-complexity` itself, \
so WMC and per-method CC stay in sync — a fix that lowers CC on \
one method lowers WMC by the same delta.";

const WMC_REFACTOR_HINTS: &[&str] = &[
    "Lower the CC of the dominant methods first — WMC is dominated by the largest CC contributor.",
    "Split the impl by responsibility: half the methods on a sibling type drops WMC by the sum of their CC.",
];

const WMC_REFERENCES: &[&str] =
    &["Chidamber, S. R., & Kemerer, C. F. (1994). A metrics suite for object oriented design."];
