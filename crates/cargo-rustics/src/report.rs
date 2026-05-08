//! Report shape used by every reporter.
//!
//! The shape is intentionally a flat list of violations + a summary —
//! shaping happens in the reporter, not the data model. JSON Schema lives
//! at `schemas/rustics-report.schema.json` (committed in M1 alongside the
//! reporter).
//!
//! Field names are *stable across the 0.x line* (plan §4.1). Field
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
    /// Number of violations dropped by `--limit`. Plan §7.2.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub truncated: usize,
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
    /// Free-form rationale (auto-explain default-on, plan §4.2).
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
    /// read multiple dimensions in one place. Plan §4.3.
    #[serde(
        rename = "rustContext",
        default,
        skip_serializing_if = "RustContext::is_empty"
    )]
    pub rust_context: RustContext,
}

/// Plan §4.3 — sidecar measurements that travel with each violation
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
    /// Number of positional parameters.
    #[serde(
        rename = "numberOfParameters",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub number_of_parameters: Option<f64>,
    /// Borrow profile — `{owned, borrowed, mutBorrowed}`. Plan §4.3.
    #[serde(
        rename = "borrowProfile",
        default,
        skip_serializing_if = "BorrowProfile::is_empty"
    )]
    pub borrow_profile: BorrowProfile,
}

/// Plan §4.3 — `borrowProfile` sub-object on `rustContext`. Three
/// separate lenses populate this; the CLI aggregates them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BorrowProfile {
    /// Owned (`T`) parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned: Option<f64>,
    /// Immutable borrows (`&T`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borrowed: Option<f64>,
    /// Mutable borrows (`&mut T`).
    #[serde(
        rename = "mutBorrowed",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mut_borrowed: Option<f64>,
}

impl BorrowProfile {
    /// True iff every field is `None`.
    pub fn is_empty(&self) -> bool {
        self.owned.is_none() && self.borrowed.is_none() && self.mut_borrowed.is_none()
    }
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
            && self.number_of_parameters.is_none()
            && self.borrow_profile.is_empty()
    }
}

impl Report {
    /// Sorts violations by severity (desc) then over-threshold ratio (desc)
    /// then id (asc) to stabilise output across runs.
    pub fn sort_violations(&mut self) {
        self.violations.sort_by(|a, b| {
            severity_rank(b.severity)
                .cmp(&severity_rank(a.severity))
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
            },
            violations,
            truncated: 0,
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
