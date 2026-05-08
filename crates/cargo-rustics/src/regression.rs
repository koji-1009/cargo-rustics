//! `cargo rustics regression` — AI-loop closer.
//!
//! Plan §1.4, §4.5, §5.1 (core command). Compares two `Report`s by
//! violation id and produces five buckets, mirroring dartrics's verdict
//! granularity:
//!
//! * `removed` — id in `before` only. The violation is gone.
//! * `added` — id in `after` only. A new violation appeared.
//! * `improved` — id in both, value moved in the direction the lens
//!   considers "better" (lower for `lower-is-better` lenses, etc.).
//! * `regressed` — id in both, value moved the wrong way.
//! * `unchanged` — id in both, value equal (or the lens is informational).
//!
//! A `verdict` summarises the diff at a glance: `clean` / `improved` /
//! `regressed` / `mixed` / `unchanged`. The cosmetic-refactor detector
//! (plan §4.5) is layered on top via the `measurements:` block when both
//! snapshots carry it.

use std::collections::HashMap;

use rustics::{builtin_metrics, MetricPolarity};
use serde::Serialize;

use crate::report::{Report, Violation};

/// Output of [`compute`].
#[derive(Debug, Clone, Serialize)]
pub struct RegressionReport {
    /// Contract version of this report shape (`2` since the addition of
    /// `added` / `removed` buckets — `1` was id-only buckets).
    pub version: u32,
    /// `before` snapshot summary (counts only).
    pub before: SnapshotSummary,
    /// `after` snapshot summary (counts only).
    pub after: SnapshotSummary,
    /// Headline counts of the diff.
    pub diff: DiffCounts,
    /// One-word `verdict` — quick read for humans / agents.
    pub verdict: Verdict,
    /// Id in both snapshots, value moved the lens-correct direction
    /// (lower for `lower-is-better`). The `after` form is reported.
    pub improved: Vec<Violation>,
    /// Id in both snapshots, value moved the wrong direction. The
    /// `after` form is reported.
    pub regressed: Vec<Violation>,
    /// Id in both snapshots, value equal. Or the lens is `informational`.
    pub unchanged: Vec<Violation>,
    /// New violations: id in `after` only.
    pub added: Vec<Violation>,
    /// Resolved violations: id in `before` only.
    pub removed: Vec<Violation>,
    /// Cosmetic-refactor signals + verdict (plan §4.5). Populated only
    /// when both snapshots carry the `measurements:` block.
    #[serde(rename = "cosmeticAnalysis", skip_serializing_if = "Option::is_none")]
    pub cosmetic_analysis: Option<CosmeticAnalysis>,
}

/// Plan §4.5 — signal table + verdict for the AI-loop refactor sniff.
#[derive(Debug, Clone, Serialize)]
pub struct CosmeticAnalysis {
    /// Per-axis numeric signals.
    pub signals: CosmeticSignals,
    /// One-word verdict from the heuristic.
    pub verdict: CosmeticVerdict,
}

/// Plan §4.5 — raw signals an agent can read independently.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CosmeticSignals {
    /// Functions that exist in `after` but not in `before` (matched
    /// by full scope path). New helper functions show up here.
    #[serde(rename = "helpersAdded")]
    pub helpers_added: i64,
    /// Total SLOC change across the whole repository.
    #[serde(rename = "slocDelta")]
    pub sloc_delta: i64,
    /// Reduction in summed cyclomatic complexity (positive = better).
    #[serde(rename = "ccReduction")]
    pub cc_reduction: i64,
    /// Net new clone calls (positive = worse).
    #[serde(rename = "clonesAdded")]
    pub clones_added: i64,
    /// Net new lines inside `unsafe { ... }` blocks (positive = worse).
    #[serde(rename = "unsafeBlocksAdded")]
    pub unsafe_blocks_added: i64,
    /// Net new `impl Trait` occurrences in signatures.
    #[serde(rename = "implTraitAdded")]
    pub impl_trait_added: i64,
    /// Net new `dyn Trait` occurrences in signatures.
    #[serde(rename = "dynAdded")]
    pub dyn_added: i64,
}

/// One-word verdict for the cosmetic check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CosmeticVerdict {
    /// No signals fire; nothing changed.
    Clean,
    /// Some helpers added, SLOC grew, but CC didn't drop much —
    /// likely a cosmetic refactor.
    LikelyCosmetic,
    /// Some signals fire but the picture is mixed.
    Mixed,
}

/// Summary of one side of a regression diff.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotSummary {
    /// Timestamp the snapshot was generated.
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    /// Total violation count in the snapshot.
    pub violations: usize,
    /// Warning-severity count.
    pub warnings: usize,
    /// Error-severity count.
    pub errors: usize,
}

/// Headline counts of the diff.
#[derive(Debug, Clone, Serialize)]
pub struct DiffCounts {
    /// Id-in-both, value got better.
    pub improved: usize,
    /// Id-in-both, value got worse.
    pub regressed: usize,
    /// Id-in-both, value equal.
    pub unchanged: usize,
    /// Id-in-after-only.
    pub added: usize,
    /// Id-in-before-only.
    pub removed: usize,
}

/// One-word verdict produced from [`DiffCounts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Both snapshots had no violations.
    Clean,
    /// Net improvement — issues went away, no new ones.
    Improved,
    /// Net regression — new issues, none resolved.
    Regressed,
    /// Some improved, some regressed.
    Mixed,
    /// Same set of violations on both sides.
    Unchanged,
}

/// Diffs `before` and `after` and produces a [`RegressionReport`].
pub fn compute(before: Report, after: Report) -> RegressionReport {
    let before_summary = summarise(&before);
    let after_summary = summarise(&after);
    let cosmetic_analysis = compute_cosmetic_analysis(&before.measurements, &after.measurements);
    let before_by_id = index_by_id(before.violations);
    let after_by_id = index_by_id(after.violations);
    let polarities = polarity_index();

    let buckets = bucketise(&before_by_id, &after_by_id, &polarities);
    let diff = DiffCounts {
        improved: buckets.improved.len(),
        regressed: buckets.regressed.len(),
        unchanged: buckets.unchanged.len(),
        added: buckets.added.len(),
        removed: buckets.removed.len(),
    };
    let verdict = verdict_for(&diff);

    RegressionReport {
        version: 2,
        before: before_summary,
        after: after_summary,
        diff,
        verdict,
        improved: buckets.improved,
        regressed: buckets.regressed,
        unchanged: buckets.unchanged,
        added: buckets.added,
        removed: buckets.removed,
        cosmetic_analysis,
    }
}

/// Builds a `metric_id -> polarity` map once from the lens catalogue.
fn polarity_index() -> HashMap<String, MetricPolarity> {
    builtin_metrics()
        .iter()
        .map(|m| (m.id().to_string(), m.metadata().polarity))
        .collect()
}

/// Builds the `cosmeticAnalysis:` block from the two snapshots'
/// `measurements:` blocks. Returns `None` when either side is empty
/// (older or violation-only snapshots), keeping the field absent in
/// the output.
fn compute_cosmetic_analysis(
    before: &[crate::report::MeasurementRecord],
    after: &[crate::report::MeasurementRecord],
) -> Option<CosmeticAnalysis> {
    if before.is_empty() && after.is_empty() {
        return None;
    }
    let signals = CosmeticSignals {
        helpers_added: helpers_added(before, after),
        sloc_delta: metric_total_delta(before, after, "source-lines-of-code"),
        cc_reduction: -metric_total_delta(before, after, "cyclomatic-complexity"),
        clones_added: metric_total_delta(before, after, "clone-density"),
        unsafe_blocks_added: metric_total_delta(before, after, "unsafe-block-scope"),
        impl_trait_added: metric_total_delta(before, after, "impl-trait-fanout"),
        dyn_added: metric_total_delta(before, after, "dyn-density"),
    };
    let verdict = cosmetic_verdict(&signals);
    Some(CosmeticAnalysis { signals, verdict })
}

/// Counts function scopes present in `after` but absent in `before`.
/// Matched by `(file, scope)` keys collected from every CC measurement
/// (CC is the most reliable proxy for "is this a function?").
fn helpers_added(
    before: &[crate::report::MeasurementRecord],
    after: &[crate::report::MeasurementRecord],
) -> i64 {
    use std::collections::HashSet;
    let scopes_in = |xs: &[crate::report::MeasurementRecord]| -> HashSet<(String, String)> {
        xs.iter()
            .filter(|m| m.metric == "cyclomatic-complexity")
            .map(|m| (m.file.clone(), m.scope.clone()))
            .collect()
    };
    let b = scopes_in(before);
    let a = scopes_in(after);
    a.difference(&b).count() as i64
}

/// Sum delta of `<after total>` − `<before total>` for one metric id
/// across every measurement. Returns 0 when neither snapshot has the
/// metric.
fn metric_total_delta(
    before: &[crate::report::MeasurementRecord],
    after: &[crate::report::MeasurementRecord],
    metric: &str,
) -> i64 {
    let total = |xs: &[crate::report::MeasurementRecord]| -> f64 {
        xs.iter()
            .filter(|m| m.metric == metric)
            .map(|m| m.value)
            .sum()
    };
    (total(after) - total(before)).round() as i64
}

/// Plan §4.5 verdict heuristic: tinyHelpersAdded ≥ 3 AND slocDelta >
/// 4·helpers AND ccReduction < 2·helpers ⇒ likely-cosmetic.
fn cosmetic_verdict(s: &CosmeticSignals) -> CosmeticVerdict {
    if no_signals_fired(s) {
        CosmeticVerdict::Clean
    } else if matches_likely_cosmetic(s) {
        CosmeticVerdict::LikelyCosmetic
    } else {
        CosmeticVerdict::Mixed
    }
}

fn no_signals_fired(s: &CosmeticSignals) -> bool {
    [
        s.helpers_added,
        s.sloc_delta,
        s.cc_reduction,
        s.clones_added,
        s.unsafe_blocks_added,
        s.impl_trait_added,
        s.dyn_added,
    ]
    .iter()
    .all(|n| *n == 0)
}

fn matches_likely_cosmetic(s: &CosmeticSignals) -> bool {
    let helpers = s.helpers_added;
    helpers >= 3 && s.sloc_delta > 4 * helpers && s.cc_reduction < 2 * helpers
}

fn summarise(report: &Report) -> SnapshotSummary {
    SnapshotSummary {
        generated_at: report.generated_at.clone(),
        violations: report.summary.violations,
        warnings: report.summary.warnings,
        errors: report.summary.errors,
    }
}

fn index_by_id(violations: Vec<Violation>) -> HashMap<String, Violation> {
    violations.into_iter().map(|v| (v.id.clone(), v)).collect()
}

struct Buckets {
    improved: Vec<Violation>,
    regressed: Vec<Violation>,
    unchanged: Vec<Violation>,
    added: Vec<Violation>,
    removed: Vec<Violation>,
}

fn bucketise(
    before: &HashMap<String, Violation>,
    after: &HashMap<String, Violation>,
    polarities: &HashMap<String, MetricPolarity>,
) -> Buckets {
    let mut buckets = Buckets {
        improved: Vec::new(),
        regressed: Vec::new(),
        unchanged: Vec::new(),
        added: Vec::new(),
        removed: Vec::new(),
    };
    for (id, before_v) in before {
        match after.get(id) {
            None => buckets.removed.push(before_v.clone()),
            Some(after_v) => sort_id_in_both(before_v, after_v, polarities, &mut buckets),
        }
    }
    for (id, after_v) in after {
        if !before.contains_key(id) {
            buckets.added.push(after_v.clone());
        }
    }
    sort_by_id(&mut buckets.improved);
    sort_by_id(&mut buckets.regressed);
    sort_by_id(&mut buckets.unchanged);
    sort_by_id(&mut buckets.added);
    sort_by_id(&mut buckets.removed);
    buckets
}

/// Classifies a violation that exists in both snapshots by comparing
/// values via the lens's polarity. Pushes the `after` form into the
/// matching bucket.
fn sort_id_in_both(
    before: &Violation,
    after: &Violation,
    polarities: &HashMap<String, MetricPolarity>,
    buckets: &mut Buckets,
) {
    let polarity = polarities
        .get(&after.metric)
        .copied()
        .unwrap_or(MetricPolarity::LowerIsBetter);
    match value_change(before.value, after.value, polarity) {
        ValueChange::Better => buckets.improved.push(after.clone()),
        ValueChange::Worse => buckets.regressed.push(after.clone()),
        ValueChange::Same => buckets.unchanged.push(after.clone()),
    }
}

#[derive(Debug, Clone, Copy)]
enum ValueChange {
    Better,
    Worse,
    Same,
}

fn value_change(before: f64, after: f64, polarity: MetricPolarity) -> ValueChange {
    if (after - before).abs() < f64::EPSILON {
        return ValueChange::Same;
    }
    match polarity {
        MetricPolarity::LowerIsBetter => {
            if after < before {
                ValueChange::Better
            } else {
                ValueChange::Worse
            }
        }
        MetricPolarity::HigherIsBetter => {
            if after > before {
                ValueChange::Better
            } else {
                ValueChange::Worse
            }
        }
        // Informational metrics have no direction; "the value moved"
        // is simply noise, not an improvement or regression.
        MetricPolarity::Informational => ValueChange::Same,
    }
}

fn sort_by_id(v: &mut [Violation]) {
    v.sort_by(|a, b| a.id.cmp(&b.id));
}

fn verdict_for(diff: &DiffCounts) -> Verdict {
    let any_improved = diff.improved + diff.removed > 0;
    let any_regressed = diff.regressed + diff.added > 0;
    match (any_improved, any_regressed) {
        (false, false) if diff.unchanged == 0 => Verdict::Clean,
        (false, false) => Verdict::Unchanged,
        (true, false) => Verdict::Improved,
        (false, true) => Verdict::Regressed,
        (true, true) => Verdict::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Summary;
    use rustics::{MetricSeverity, ScopeKind};

    fn v(id: &str) -> Violation {
        Violation {
            id: id.into(),
            file: "src/x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "cyclomatic-complexity".into(),
            value: 11.0,
            threshold: 10.0,
            severity: MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        }
    }

    fn report(violations: Vec<Violation>) -> Report {
        Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: violations.len(),
                warnings: violations
                    .iter()
                    .filter(|v| v.severity == MetricSeverity::Warning)
                    .count(),
                errors: violations
                    .iter()
                    .filter(|v| v.severity == MetricSeverity::Error)
                    .count(),
            },
            violations,
            truncated: 0,
            measurements: vec![],
        }
    }

    #[test]
    fn clean_when_both_empty() {
        let r = compute(report(vec![]), report(vec![]));
        assert_eq!(r.verdict, Verdict::Clean);
        assert!(r.improved.is_empty());
        assert!(r.regressed.is_empty());
        assert!(r.unchanged.is_empty());
        assert!(r.added.is_empty());
        assert!(r.removed.is_empty());
    }

    #[test]
    fn removed_when_only_disappeared() {
        let before = report(vec![v("aaa"), v("bbb")]);
        let after = report(vec![]);
        let r = compute(before, after);
        // Disappearing violations are improvement signal; verdict is Improved.
        assert_eq!(r.verdict, Verdict::Improved);
        assert_eq!(r.removed.len(), 2);
        assert_eq!(r.added.len(), 0);
        assert!(r.improved.is_empty(), "improved is now id-in-both only");
    }

    #[test]
    fn added_when_only_appeared() {
        let before = report(vec![]);
        let after = report(vec![v("aaa")]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Regressed);
        assert_eq!(r.added.len(), 1);
        assert!(r.regressed.is_empty(), "regressed is now id-in-both only");
    }

    #[test]
    fn mixed_when_both_added_and_removed() {
        let before = report(vec![v("aaa")]);
        let after = report(vec![v("bbb")]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Mixed);
        assert_eq!(r.added.len(), 1);
        assert_eq!(r.removed.len(), 1);
    }

    #[test]
    fn unchanged_when_same_ids_and_same_values() {
        let before = report(vec![v("aaa"), v("bbb")]);
        let after = report(vec![v("aaa"), v("bbb")]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Unchanged);
        assert_eq!(r.unchanged.len(), 2);
        assert!(r.improved.is_empty());
        assert!(r.regressed.is_empty());
    }

    #[test]
    fn improved_when_value_dropped_for_lower_is_better() {
        // CC is lower-is-better. Same id, value 11→8 → improved.
        let mut before_v = v("aaa");
        before_v.value = 11.0;
        let mut after_v = v("aaa");
        after_v.value = 8.0;
        let r = compute(report(vec![before_v]), report(vec![after_v]));
        assert_eq!(r.verdict, Verdict::Improved);
        assert_eq!(r.improved.len(), 1);
        assert_eq!(r.improved[0].value, 8.0);
        assert!(r.regressed.is_empty());
    }

    #[test]
    fn regressed_when_value_grew_for_lower_is_better() {
        let mut before_v = v("aaa");
        before_v.value = 11.0;
        let mut after_v = v("aaa");
        after_v.value = 18.0;
        let r = compute(report(vec![before_v]), report(vec![after_v]));
        assert_eq!(r.verdict, Verdict::Regressed);
        assert_eq!(r.regressed.len(), 1);
    }

    #[test]
    fn informational_metric_value_change_is_unchanged() {
        // borrow-profile-* are informational; a value change carries no
        // "better/worse" semantics, so the violation lands in `unchanged`.
        let mk = |value: f64| Violation {
            id: "aaa".into(),
            file: "src/x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "borrow-profile-owned".into(),
            value,
            threshold: 100.0,
            severity: MetricSeverity::Info,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        };
        let r = compute(report(vec![mk(2.0)]), report(vec![mk(5.0)]));
        assert_eq!(r.unchanged.len(), 1);
        assert!(r.improved.is_empty());
        assert!(r.regressed.is_empty());
    }

    #[test]
    fn outputs_are_id_sorted() {
        let before = report(vec![v("zzz"), v("aaa")]);
        let after = report(vec![]);
        let r = compute(before, after);
        let ids: Vec<_> = r.removed.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, vec!["aaa".to_string(), "zzz".to_string()]);
    }

    #[test]
    fn report_version_is_two() {
        // Renaming improved/regressed semantics + adding added/removed
        // is a breaking change to the regression-report contract.
        let r = compute(report(vec![]), report(vec![]));
        assert_eq!(r.version, 2);
    }
}
