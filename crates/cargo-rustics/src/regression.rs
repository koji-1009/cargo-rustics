//! `cargo rustics regression` — AI-loop closer.
//!
//! Plan §1.4, §4.5, §5.1 (core command). Compares two `Report`s by
//! violation id and produces:
//!
//! * `improved` — ids present in `before` but absent in `after`.
//! * `regressed` — ids present in `after` but absent in `before`.
//! * `unchanged` — ids present in both.
//!
//! A `verdict` summarises the diff at a glance: `clean` /
//! `improved` / `regressed` / `mixed` / `unchanged`. A more nuanced
//! cosmetic-refactor detector (plan §4.5) needs richer measurement data
//! than the violation-list shape; that lands when the snapshot format
//! grows to carry all per-scope measurements (M2 follow-up).

use std::collections::HashMap;

use serde::Serialize;

use crate::report::{Report, Violation};

/// Output of [`compute`].
#[derive(Debug, Clone, Serialize)]
pub struct RegressionReport {
    /// Contract version of this report shape (currently `1`).
    pub version: u32,
    /// `before` snapshot summary (counts only).
    pub before: SnapshotSummary,
    /// `after` snapshot summary (counts only).
    pub after: SnapshotSummary,
    /// Headline counts of the diff.
    pub diff: DiffCounts,
    /// One-word `verdict` — quick read for humans / agents.
    pub verdict: Verdict,
    /// Violations resolved between `before` and `after`.
    pub improved: Vec<Violation>,
    /// New violations introduced in `after`.
    pub regressed: Vec<Violation>,
    /// Violations whose stable id is in both snapshots — same problem,
    /// same place. The `after` form is reported (line numbers / values
    /// may have moved).
    pub unchanged: Vec<Violation>,
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
    /// Violations that disappeared between `before` and `after`.
    pub improved: usize,
    /// Violations that appeared in `after`.
    pub regressed: usize,
    /// Violations whose stable id is in both snapshots.
    pub unchanged: usize,
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
    let before_by_id = index_by_id(before.violations);
    let after_by_id = index_by_id(after.violations);

    let buckets = bucketise(&before_by_id, &after_by_id);
    let diff = DiffCounts {
        improved: buckets.improved.len(),
        regressed: buckets.regressed.len(),
        unchanged: buckets.unchanged.len(),
    };
    let verdict = verdict_for(&diff);

    RegressionReport {
        version: 1,
        before: before_summary,
        after: after_summary,
        diff,
        verdict,
        improved: buckets.improved,
        regressed: buckets.regressed,
        unchanged: buckets.unchanged,
    }
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
}

fn bucketise(before: &HashMap<String, Violation>, after: &HashMap<String, Violation>) -> Buckets {
    let mut improved = Vec::new();
    let mut regressed = Vec::new();
    let mut unchanged = Vec::new();
    for (id, v) in before {
        if !after.contains_key(id) {
            improved.push(v.clone());
        }
    }
    for (id, v) in after {
        if before.contains_key(id) {
            unchanged.push(v.clone());
        } else {
            regressed.push(v.clone());
        }
    }
    sort_by_id(&mut improved);
    sort_by_id(&mut regressed);
    sort_by_id(&mut unchanged);
    Buckets {
        improved,
        regressed,
        unchanged,
    }
}

fn sort_by_id(v: &mut [Violation]) {
    v.sort_by(|a, b| a.id.cmp(&b.id));
}

fn verdict_for(diff: &DiffCounts) -> Verdict {
    match (diff.improved, diff.regressed, diff.unchanged) {
        (0, 0, 0) => Verdict::Clean,
        (_, 0, _) if diff.improved > 0 => Verdict::Improved,
        (0, _, _) if diff.regressed > 0 => Verdict::Regressed,
        (i, r, _) if i > 0 && r > 0 => Verdict::Mixed,
        _ => Verdict::Unchanged,
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
        }
    }

    #[test]
    fn clean_when_both_empty() {
        let r = compute(report(vec![]), report(vec![]));
        assert_eq!(r.verdict, Verdict::Clean);
        assert!(r.improved.is_empty());
        assert!(r.regressed.is_empty());
        assert!(r.unchanged.is_empty());
    }

    #[test]
    fn improved_when_only_disappeared() {
        let before = report(vec![v("aaa"), v("bbb")]);
        let after = report(vec![]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Improved);
        assert_eq!(r.improved.len(), 2);
        assert_eq!(r.regressed.len(), 0);
    }

    #[test]
    fn regressed_when_only_appeared() {
        let before = report(vec![]);
        let after = report(vec![v("aaa")]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Regressed);
        assert_eq!(r.regressed.len(), 1);
    }

    #[test]
    fn mixed_when_both_directions() {
        let before = report(vec![v("aaa")]);
        let after = report(vec![v("bbb")]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Mixed);
        assert_eq!(r.improved.len(), 1);
        assert_eq!(r.regressed.len(), 1);
    }

    #[test]
    fn unchanged_when_same_ids() {
        let before = report(vec![v("aaa"), v("bbb")]);
        let after = report(vec![v("aaa"), v("bbb")]);
        let r = compute(before, after);
        assert_eq!(r.verdict, Verdict::Unchanged);
        assert_eq!(r.unchanged.len(), 2);
        assert!(r.improved.is_empty());
        assert!(r.regressed.is_empty());
    }

    #[test]
    fn outputs_are_id_sorted() {
        let before = report(vec![v("zzz"), v("aaa")]);
        let after = report(vec![]);
        let r = compute(before, after);
        let ids: Vec<_> = r.improved.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, vec!["aaa".to_string(), "zzz".to_string()]);
    }
}
