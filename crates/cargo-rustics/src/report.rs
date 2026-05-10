//! Report shape used by every reporter.
//!
//! The shape is intentionally a flat list of violations + a summary —
//! shaping happens in the reporter, not the data model. JSON Schema lives
//! at `schemas/rustics-report.schema.json` (committed alongside the
//! reporter).
//!
//! Field names are *stable across the 0.x line*. Field
//! additions are not breaking; renames or removals bump the contract
//! header to `v2`.

use serde::{Deserialize, Serialize};

use rustics::{MetricSeverity, ScopeKind};

/// Top-level report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// AI-report contract version (currently 1).
    pub version: u32,
    /// ISO-8601 timestamp of when the report was produced.
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    /// Aggregate statistics.
    pub summary: Summary,
    /// Violations sorted by (severity desc, value-over-threshold desc, id asc).
    pub violations: Vec<Violation>,
    /// Number of violations dropped by `--limit`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub truncated: usize,
    /// Every per-scope measurement collected during the run, regardless
    /// of whether the value crossed a threshold. Snapshots that include
    /// these support `cargo rustics regression`'s cosmetic-detection
    /// signals. Empty / absent means the run produced
    /// violations only — older snapshots and the "violation-only"
    /// reporters drop this field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<MeasurementRecord>,
    /// Sidecar dismissals that no longer match any live violation.
    /// Surfacing them in the report (not just stderr) lets an AI agent
    /// or PR reviewer act on cleanup. Empty → field omitted.
    #[serde(
        rename = "staleDismissals",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub stale_dismissals: Vec<StaleDismissal>,
}

/// One stale-dismissal entry — sidecar lines that pointed at a violation
/// no longer present. Surfaced to reporters so the AI loop can prompt
/// "remove this dismissal" without parsing stderr.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleDismissal {
    /// Workspace-relative file path the dismissal targeted.
    pub file: String,
    /// Scope path the dismissal targeted (`module::Type::method`).
    pub scope: String,
    /// Lens id the dismissal targeted (kebab-case).
    pub metric: String,
    /// The reason text from the sidecar entry.
    pub reason: String,
}

/// One per-scope measurement, snapshot-friendly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementRecord {
    /// Workspace-relative path with `/` separators.
    pub file: String,
    /// `module::Type::method` scope path.
    pub scope: String,
    /// Lens id (kebab-case).
    pub metric: String,
    /// Measured numeric value.
    pub value: f64,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Aggregate counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    /// Total `.rs` files analyzed.
    #[serde(rename = "filesAnalyzed")]
    pub files_analyzed: usize,
    /// Total violation count.
    pub violations: usize,
    /// Number of `severity == warning` violations.
    pub warnings: usize,
    /// Number of `severity == error` violations.
    pub errors: usize,
    /// Subset of `warnings` that carry `complexityJustified`. The
    /// violation still counts toward `warnings` (so consumers that
    /// don't read this field still see the headline number) but an
    /// agent can subtract this from `warnings` to get the count it
    /// actually has work to do on.
    #[serde(rename = "warningsJustified", default, skip_serializing_if = "is_zero")]
    pub warnings_justified: usize,
    /// Same idea as [`Self::warnings_justified`] but for `severity ==
    /// error` violations whose host file's coverage cleared the
    /// `complexityJustified` bar.
    #[serde(rename = "errorsJustified", default, skip_serializing_if = "is_zero")]
    pub errors_justified: usize,
}

/// A single violation record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Violation {
    /// Stable id (`sha256(<file>|<scope>|<metric>)[..16]`).
    pub id: String,
    /// Workspace-root-relative file path with `/` separators.
    pub file: String,
    /// 1-based line number where the scope begins.
    pub line: usize,
    /// Scope path (e.g. `module::Type::method`).
    pub scope: String,
    /// Scope kind for tooling that wants to filter (e.g. trait methods only).
    #[serde(rename = "scopeKind")]
    pub scope_kind: ScopeKind,
    /// Lens id (kebab-case).
    pub metric: String,
    /// Measured value.
    pub value: f64,
    /// Threshold the value crossed.
    pub threshold: f64,
    /// Severity of this violation.
    pub severity: MetricSeverity,
    /// Free-form rationale (auto-explain default-on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Concrete refactor hints.
    #[serde(
        rename = "refactorHints",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub refactor_hints: Vec<String>,
    /// Original-source citations.
    #[serde(rename = "references", default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    /// Other lens values at the same scope, attached so the AI agent can
    /// read multiple dimensions in one place.
    #[serde(
        rename = "rustContext",
        default,
        skip_serializing_if = "RustContext::is_empty"
    )]
    pub rust_context: RustContext,
    /// Marks complexity-class violations whose host file is well covered
    /// by tests as "earned complexity". An AI agent reading the report
    /// should *not* try to refactor a `complexityJustified` violation —
    /// the tests prove the shape works. Inspired by dartrics's
    /// `complexityJustified` flag (https://pub.dev/packages/dartrics).
    #[serde(
        rename = "complexityJustified",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub complexity_justified: Option<ComplexityJustification>,
}

/// Reason a complexity-class violation is allowed to stand: enough
/// coverage to consider the shape "earned".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ComplexityJustification {
    /// Which coverage dimension cleared the bar. `Branch` is reserved
    /// for future use (the lcov reader is line-only).
    pub by: JustificationBasis,
    /// Threshold the coverage met or exceeded (in `[0.0, 1.0]`).
    pub threshold: f64,
    /// Actual coverage ratio that triggered the justification.
    pub actual: f64,
}

/// Coverage dimension used to justify a complexity-class violation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JustificationBasis {
    /// Line coverage — `lcov LH/LF`.
    Line,
    /// Branch coverage — reserved (lcov `BRF/BRH` parsing lands later).
    Branch,
}

/// Lens IDs whose violations can be justified by high test coverage.
/// Other lenses (e.g. `clone-density`, `panic-density`, `lifetime-arity`)
/// describe shapes that tests can't make "OK" — they signal cost or
/// risk regardless of coverage.
pub const COMPLEXITY_CLASS_METRICS: &[&str] = &[
    "cyclomatic-complexity",
    "cognitive-complexity",
    "halstead-volume",
    "source-lines-of-code",
];

/// Default thresholds: ≥ 95% line coverage justifies. Branch threshold
/// reserved for future use.
pub const COMPLEXITY_JUSTIFIED_LINE_THRESHOLD: f64 = 0.95;

/// — sidecar measurements that travel with each violation
/// so an AI agent can correlate dimensions without round-tripping
/// through the full lens catalogue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RustContext {
    /// Lifetime parameters on the function signature.
    #[serde(
        rename = "lifetimeArity",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub lifetime_arity: Option<f64>,
    /// Type parameters + where bounds.
    #[serde(
        rename = "genericArity",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub generic_arity: Option<f64>,
    /// `.clone()` / `.to_owned()` / `.to_string()` count in the body.
    #[serde(
        rename = "cloneSites",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub clone_sites: Option<f64>,
    /// `.unwrap()` / `.expect()` / panic-class macro count.
    #[serde(
        rename = "panicSites",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub panic_sites: Option<f64>,
    /// Total lines of `unsafe { ... }` blocks in the body.
    #[serde(
        rename = "unsafeBlocks",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub unsafe_blocks: Option<f64>,
}

impl RustContext {
    /// True iff every field is `None` — used by serde to skip an empty
    /// `rustContext` block in the output.
    pub fn is_empty(&self) -> bool {
        self.lifetime_arity.is_none()
            && self.generic_arity.is_none()
            && self.clone_sites.is_none()
            && self.panic_sites.is_none()
            && self.unsafe_blocks.is_none()
    }
}

impl Report {
    /// Sorts violations: justified-by-coverage entries to the bottom of
    /// the list (an AI agent should reach for the unjustified ones
    /// first), then by severity (desc), over-threshold ratio (desc),
    /// id (asc) for stability.
    pub fn sort_violations(&mut self) {
        self.violations.sort_by(|a, b| {
            a.complexity_justified
                .is_some()
                .cmp(&b.complexity_justified.is_some())
                .then_with(|| severity_rank(b.severity).cmp(&severity_rank(a.severity)))
                .then_with(|| {
                    let ratio_a = ratio(a.value, a.threshold);
                    let ratio_b = ratio(b.value, b.threshold);
                    ratio_b
                        .partial_cmp(&ratio_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.id.cmp(&b.id))
        });
    }
}

fn severity_rank(s: MetricSeverity) -> u8 {
    match s {
        MetricSeverity::Error => 2,
        MetricSeverity::Warning => 1,
        MetricSeverity::Info => 0,
    }
}

/// Sort key: how far over the threshold a violation sits. Threshold
/// can legitimately be 0 if a user sets `warning = 0` in their config
/// for a `lower-is-better` lens (every nonzero measurement violates).
/// In that degenerate case we fall back to the raw value so the
/// pairwise ordering still reflects severity of crossing.
fn ratio(value: f64, threshold: f64) -> f64 {
    if threshold == 0.0 {
        value
    } else {
        value / threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str, sev: MetricSeverity, value: f64, threshold: f64) -> Violation {
        Violation {
            id: id.into(),
            file: "x".into(),
            line: 1,
            scope: "y".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "m".into(),
            value,
            threshold,
            severity: sev,
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
            generated_at: "".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: violations.len(),
                warnings: 0,
                errors: 0,
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations,
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
        }
    }

    #[test]
    fn sort_puts_errors_first() {
        let mut r = report(vec![
            v("a", MetricSeverity::Warning, 11.0, 10.0),
            v("b", MetricSeverity::Error, 21.0, 20.0),
        ]);
        r.sort_violations();
        assert_eq!(r.violations[0].id, "b");
    }

    #[test]
    fn justified_violations_sort_to_end() {
        let mut a = v("a", MetricSeverity::Error, 25.0, 10.0); // big error
        a.complexity_justified = Some(ComplexityJustification {
            by: JustificationBasis::Line,
            threshold: 0.95,
            actual: 0.97,
        });
        let b = v("b", MetricSeverity::Warning, 11.0, 10.0); // small warning
        let mut r = report(vec![a, b]);
        r.sort_violations();
        // The small unjustified warning beats the big justified error
        // because the AI agent should reach for the unjustified work
        // first.
        assert_eq!(r.violations[0].id, "b");
        assert_eq!(r.violations[1].id, "a");
    }

    #[test]
    fn complexity_justification_serde_roundtrip() {
        let j = ComplexityJustification {
            by: JustificationBasis::Line,
            threshold: 0.95,
            actual: 0.96,
        };
        let json = serde_json::to_string(&j).unwrap();
        assert!(json.contains("\"by\":\"line\""));
        assert!(json.contains("\"threshold\":0.95"));
        let back: ComplexityJustification = serde_json::from_str(&json).unwrap();
        assert_eq!(back.by, JustificationBasis::Line);
    }

    #[test]
    fn sort_handles_zero_threshold_via_value_fallback() {
        // `ratio()` returns `value` directly when the threshold is zero
        // — the only way that arm is reached today is via a user
        // config that sets `warning = 0` for a `lower-is-better` lens.
        // Sort then orders by raw value (desc).
        let a = v("a", MetricSeverity::Warning, 5.0, 0.0);
        let b = v("b", MetricSeverity::Warning, 9.0, 0.0);
        let mut r = report(vec![a, b]);
        r.sort_violations();
        let ids: Vec<_> = r.violations.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }

    #[test]
    fn sort_ranks_info_below_warning() {
        // The Info severity is part of `rustics::MetricSeverity`'s
        // public API; library embedders (rustics-lsp, rustics-build,
        // ad-hoc tools) may construct Info-severity violations even
        // though no built-in lens currently produces one. Sort must
        // still rank Info below Warning so an agent reaches for the
        // warning / error work first.
        let mut r = report(vec![
            v("a", MetricSeverity::Info, 5.0, 1.0),
            v("b", MetricSeverity::Warning, 11.0, 10.0),
            v("c", MetricSeverity::Error, 25.0, 20.0),
        ]);
        r.sort_violations();
        let ids: Vec<_> = r.violations.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
    }

    #[test]
    fn complexity_class_metrics_includes_classics() {
        for id in [
            "cyclomatic-complexity",
            "cognitive-complexity",
            "halstead-volume",
            "source-lines-of-code",
        ] {
            assert!(
                COMPLEXITY_CLASS_METRICS.contains(&id),
                "{id} missing from complexity-class set"
            );
        }
        // Cost / risk metrics must NOT be in the complexity-class set —
        // tests can't make `clone-density` "OK".
        assert!(!COMPLEXITY_CLASS_METRICS.contains(&"clone-density"));
        assert!(!COMPLEXITY_CLASS_METRICS.contains(&"panic-density"));
        assert!(!COMPLEXITY_CLASS_METRICS.contains(&"lifetime-arity"));
    }

    #[test]
    fn sort_breaks_ties_by_ratio() {
        let mut r = report(vec![
            v("a", MetricSeverity::Warning, 11.0, 10.0), // 1.10
            v("b", MetricSeverity::Warning, 30.0, 10.0), // 3.00
            v("c", MetricSeverity::Warning, 20.0, 10.0), // 2.00
        ]);
        r.sort_violations();
        let ids: Vec<_> = r.violations.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }
}
