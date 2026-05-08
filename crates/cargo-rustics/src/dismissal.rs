//! Dismissal — "this violation is fine, here's why".
//!
//! Plan §4.4. Two surfaces:
//!
//! * **Sidecar `.rustics-dismissals.toml`** — file at the workspace
//!   root listing `[[dismissals]]` entries. Source-controlled.
//! * **Doc-comment** — `/// rustics:dismiss <metric> reason="..."` on
//!   the function being dismissed. Co-located with the code.
//!
//! Validation rules at M1+:
//!
//! * `require_reason: true` (default). A dismissal whose reason is
//!   shorter than `min_reason_length` (default 20) is *rejected* —
//!   the violation stays live and a `dismissalRejected` warning is
//!   emitted.
//! * Sidecar entry that does not match any live violation by
//!   `(file, scope, metric)` is *stale* — it stays in the file but
//!   the report's `staleDismissals:` block lists it.
//! * Doc-comment + sidecar collision — sidecar wins.
//!
//! `--strict-dismiss` (CLI flag, plan §7.2) suppresses every
//! dismissal regardless of validity. Useful in CI / final-review
//! mode.
//!
//! M1+ ships sidecar dismissal validation and filter only; doc-comment
//! parsing lands in the next slice once we wire `attrs` through the
//! analyzer (the data is already collected by the visitor — plan task
//! is to surface it on `FileMetricRecord`).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::report::Violation;

/// File on disk: `.rustics-dismissals.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DismissalsFile {
    /// `[[dismissals]]` entries in source order.
    #[serde(default)]
    pub dismissals: Vec<Dismissal>,
}

/// One dismissal record.
///
/// Plan §4.4 — `file`, `scope`, `metric` together identify the
/// violation; `reason` documents the call. `by` and `at` are the
/// audit trail; both are optional but encouraged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dismissal {
    /// Workspace-relative file path with `/` separators.
    pub file: String,
    /// Scope path (`module::Type::method`).
    pub scope: String,
    /// Metric id (`cyclomatic-complexity`).
    pub metric: String,
    /// Free-form reason. Plan default: ≥ 20 chars.
    pub reason: String,
    /// Author handle (e.g. `claude-opus-4-7`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// ISO-8601 timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Configuration knobs for dismissal validation. Plan §8.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DismissalRules {
    /// Reject dismissals whose `reason` is missing or below the
    /// minimum length.
    #[serde(default = "default_require_reason")]
    pub require_reason: bool,
    /// Minimum acceptable `reason` length when `require_reason` is on.
    #[serde(default = "default_min_reason_length")]
    pub min_reason_length: usize,
    /// Emit a `staleDismissals:` block for sidecar entries that do
    /// not match any live violation.
    #[serde(default = "default_warn_stale")]
    pub warn_stale: bool,
}

fn default_require_reason() -> bool {
    true
}

fn default_min_reason_length() -> usize {
    20
}

fn default_warn_stale() -> bool {
    true
}

impl Default for DismissalRules {
    fn default() -> Self {
        Self {
            require_reason: default_require_reason(),
            min_reason_length: default_min_reason_length(),
            warn_stale: default_warn_stale(),
        }
    }
}

/// Loads `.rustics-dismissals.toml` from `workspace_root` if present.
///
/// Missing file is not an error — most projects do not yet have one,
/// and `dismiss` is opt-in.
pub fn load_sidecar(workspace_root: &Path) -> Result<DismissalsFile> {
    let path = workspace_root.join(".rustics-dismissals.toml");
    if !path.is_file() {
        return Ok(DismissalsFile::default());
    }
    let bytes =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let file: DismissalsFile =
        toml::from_str(&bytes).with_context(|| format!("parse {}", path.display()))?;
    Ok(file)
}

/// Indexed dismissal set with hit-tracking for stale detection.
pub struct DismissalIndex<'a> {
    entries: Vec<DismissalEntry<'a>>,
    rules: DismissalRules,
    strict: bool,
}

struct DismissalEntry<'a> {
    dismissal: &'a Dismissal,
    valid: bool,
    rejection_reason: Option<&'static str>,
    used: std::cell::Cell<bool>,
}

impl<'a> DismissalIndex<'a> {
    /// Builds an index from a sidecar file and the rules.
    pub fn new(file: &'a DismissalsFile, rules: DismissalRules, strict: bool) -> Self {
        let entries = file
            .dismissals
            .iter()
            .map(|d| DismissalEntry::new(d, &rules))
            .collect();
        Self {
            entries,
            rules,
            strict,
        }
    }

    /// Returns whether any *valid* dismissal matches this violation.
    /// Marks the matching entry as used.
    pub fn matches(&self, v: &Violation) -> bool {
        if self.strict {
            return false;
        }
        for entry in &self.entries {
            if !entry.valid {
                continue;
            }
            if entry_matches(entry.dismissal, v) {
                entry.used.set(true);
                return true;
            }
        }
        false
    }

    /// Sidecar entries that were rejected (reason missing / too short).
    /// Plan §4.4 — these become `dismissalRejected` records in the report.
    pub fn rejected(&self) -> Vec<DismissalRejection<'_>> {
        self.entries
            .iter()
            .filter_map(|e| {
                e.rejection_reason.map(|r| DismissalRejection {
                    dismissal: e.dismissal,
                    reason: r,
                })
            })
            .collect()
    }

    /// Sidecar entries that did not match any live violation.
    /// Plan §4.4 — these become the `staleDismissals:` block.
    pub fn stale(&self) -> Vec<&Dismissal> {
        if !self.rules.warn_stale {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.valid && !e.used.get())
            .map(|e| e.dismissal)
            .collect()
    }
}

impl<'a> DismissalEntry<'a> {
    fn new(d: &'a Dismissal, rules: &DismissalRules) -> Self {
        let rejection = if rules.require_reason && d.reason.trim().len() < rules.min_reason_length {
            Some("reason too short")
        } else {
            None
        };
        Self {
            dismissal: d,
            valid: rejection.is_none(),
            rejection_reason: rejection,
            used: std::cell::Cell::new(false),
        }
    }
}

fn entry_matches(d: &Dismissal, v: &Violation) -> bool {
    d.file == v.file && d.scope == v.scope && d.metric == v.metric
}

/// Display row for `dismissalRejected:` block.
pub struct DismissalRejection<'a> {
    /// The original sidecar entry.
    pub dismissal: &'a Dismissal,
    /// One-line rejection reason.
    pub reason: &'static str,
}

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;
    use crate::report::Summary;
    use rustics::{MetricSeverity, ScopeKind};

    fn dismissal(file: &str, scope: &str, metric: &str, reason: &str) -> Dismissal {
        Dismissal {
            file: file.into(),
            scope: scope.into(),
            metric: metric.into(),
            reason: reason.into(),
            by: None,
            at: None,
        }
    }

    fn violation(file: &str, scope: &str, metric: &str) -> Violation {
        Violation {
            id: "abc".into(),
            file: file.into(),
            line: 1,
            scope: scope.into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: metric.into(),
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

    #[test]
    fn matching_dismissal_filters_violation() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/x.rs",
                "f",
                "cyclomatic-complexity",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        assert!(idx.matches(&violation("src/x.rs", "f", "cyclomatic-complexity")));
    }

    #[test]
    fn unmatched_violation_passes_through() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/other.rs",
                "f",
                "cyclomatic-complexity",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        assert!(!idx.matches(&violation("src/x.rs", "f", "cyclomatic-complexity")));
    }

    #[test]
    fn short_reason_is_rejected() {
        let file = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "short")],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        assert!(!idx.matches(&violation("src/x.rs", "f", "cc")));
        let rejected = idx.rejected();
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].reason.contains("too short"));
    }

    #[test]
    fn stale_dismissal_is_reported() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/old.rs",
                "ghost",
                "cc",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        // No matches.
        assert_eq!(idx.stale().len(), 1);
    }

    #[test]
    fn strict_mode_skips_all_dismissals() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/x.rs",
                "f",
                "cc",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), /* strict */ true);
        assert!(!idx.matches(&violation("src/x.rs", "f", "cc")));
    }

    #[test]
    fn helper_summary_for_test_coverage() {
        // Touch the Summary type via a default to keep the test crate's
        // unused-import warnings quiet when this module runs solo.
        let _ = Summary {
            files_analyzed: 0,
            violations: 0,
            warnings: 0,
            errors: 0,
            warnings_justified: 0,
            errors_justified: 0,
        };
    }

    #[test]
    fn defaults_match_plan_documented_values() {
        let r = DismissalRules::default();
        assert!(r.require_reason);
        assert_eq!(r.min_reason_length, 20);
        assert!(r.warn_stale);
    }

    #[test]
    fn warn_stale_false_returns_no_stale() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/old.rs",
                "ghost",
                "cc",
                "twenty character reason here",
            )],
        };
        let rules = DismissalRules {
            warn_stale: false,
            ..Default::default()
        };
        let idx = DismissalIndex::new(&file, rules, false);
        assert!(idx.stale().is_empty());
    }

    #[test]
    fn used_dismissal_is_not_stale() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/x.rs",
                "f",
                "cc",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        // Match it once.
        assert!(idx.matches(&violation("src/x.rs", "f", "cc")));
        // Now it's used → not stale.
        assert!(idx.stale().is_empty());
    }

    #[test]
    fn invalid_dismissal_does_not_count_toward_stale() {
        let file = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "short")],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        // The entry is invalid (rejected) → it's not stale, it's
        // rejected; rejected and stale are disjoint sets.
        assert!(idx.stale().is_empty());
        assert_eq!(idx.rejected().len(), 1);
    }

    fn write_workspace_with_sidecar(toml: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let dir = std::env::temp_dir().join(format!("rustics-dismiss-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        if !toml.is_empty() {
            std::fs::write(dir.join(".rustics-dismissals.toml"), toml).unwrap();
        }
        dir
    }

    #[test]
    fn load_sidecar_returns_default_when_absent() {
        let dir = write_workspace_with_sidecar("");
        let f = load_sidecar(&dir).unwrap();
        assert!(f.dismissals.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_sidecar_parses_toml() {
        let dir = write_workspace_with_sidecar(
            r#"[[dismissals]]
file = "src/x.rs"
scope = "f"
metric = "cyclomatic-complexity"
reason = "twenty character reason here"
"#,
        );
        let f = load_sidecar(&dir).unwrap();
        assert_eq!(f.dismissals.len(), 1);
        assert_eq!(f.dismissals[0].file, "src/x.rs");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_sidecar_errors_on_invalid_toml() {
        let dir = write_workspace_with_sidecar("[[ malformed\n");
        let err = load_sidecar(&dir).unwrap_err();
        assert!(format!("{err:#}").contains("parse"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
