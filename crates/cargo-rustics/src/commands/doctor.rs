//! `cargo rustics doctor` — config validation.
//!
//! Plan §5.2. Loads `rustics.toml` (if present) and checks:
//!
//! 1. Every `[rustics.metrics.<id>]` key is a real lens id (typo
//!    detection).
//! 2. Threshold orderings make sense for the lens's polarity
//!    (warning ≤ error for `lower-is-better`, warning ≥ error for
//!    `higher-is-better`).
//! 3. Exclude patterns parse without obvious mistakes (no leading `!`,
//!    no `..` path traversal — those would mean the user is reaching
//!    into a different idiom).
//!
//! Exit codes:
//!
//! * `0` — config is fine (or no `rustics.toml` was found, which is
//!   a valid state — the lens defaults take over).
//! * `1` — config has at least one error.

use anyhow::Result;

use rustics::{builtin_metrics, MetricCalculator, MetricMetadata, MetricPolarity};

use crate::config::{Config, ExcludeTable};
use crate::workspace;

/// Runs the `doctor` subcommand.
pub fn run() -> Result<u8> {
    let analysis_root = std::env::current_dir()?;
    let workspace_root = workspace::resolve_workspace_root(&analysis_root)?;
    let config = Config::load_from(&workspace_root)?;
    let metrics = builtin_metrics();

    let mut issues = check(&config, &metrics);
    if !workspace_root.join("rustics.toml").is_file() {
        issues.push(Issue {
            severity: IssueSeverity::Info,
            message: "no rustics.toml found — using lens defaults".to_string(),
        });
    }
    print_report(&workspace_root, &issues);

    Ok(if has_errors(&issues) { 1 } else { 0 })
}

fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == IssueSeverity::Error)
}

/// One diagnostic finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Severity tag for human display.
    pub severity: IssueSeverity,
    /// One-line message.
    pub message: String,
}

/// Severity of a doctor finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Informational — config is fine but worth noting.
    Info,
    /// Real problem — exit code is non-zero when any present.
    Error,
}

/// Returns every issue [`run`] would print, without printing.
pub fn check(config: &Config, metrics: &[Box<dyn MetricCalculator>]) -> Vec<Issue> {
    let mut issues = Vec::new();
    check_metric_overrides(config, metrics, &mut issues);
    check_exclude(config.exclude(), &mut issues);
    issues
}

fn check_metric_overrides(
    config: &Config,
    metrics: &[Box<dyn MetricCalculator>],
    out: &mut Vec<Issue>,
) {
    let known: std::collections::HashSet<&'static str> = metrics.iter().map(|m| m.id()).collect();
    let by_id: std::collections::HashMap<&'static str, MetricMetadata> =
        metrics.iter().map(|m| (m.id(), m.metadata())).collect();

    for (id, override_) in &config.rustics.metrics {
        if !known.contains(id.as_str()) {
            out.push(Issue {
                severity: IssueSeverity::Error,
                message: format!("unknown metric id `{id}` in rustics.toml"),
            });
            continue;
        }
        let Some(meta) = by_id.get(id.as_str()) else {
            continue;
        };
        if let (Some(w), Some(e)) = (override_.warning, override_.error) {
            if !threshold_pair_ok(w, e, meta.polarity) {
                out.push(Issue {
                    severity: IssueSeverity::Error,
                    message: format!(
                        "{id}: warning {w} and error {e} are inverted for {polarity}",
                        polarity = polarity_word(meta.polarity)
                    ),
                });
            }
        }
    }
}

fn threshold_pair_ok(warning: f64, error: f64, polarity: MetricPolarity) -> bool {
    match polarity {
        MetricPolarity::LowerIsBetter => warning <= error,
        MetricPolarity::HigherIsBetter => warning >= error,
        MetricPolarity::Informational => true,
    }
}

fn check_exclude(exclude: &ExcludeTable, out: &mut Vec<Issue>) {
    for pattern in &exclude.patterns {
        if pattern.starts_with('!') {
            out.push(Issue {
                severity: IssueSeverity::Error,
                message: format!(
                    "exclude pattern `{pattern}` starts with `!` — negation \
                     is gitignore syntax, not supported here"
                ),
            });
        }
        if pattern.contains("..") {
            out.push(Issue {
                severity: IssueSeverity::Error,
                message: format!(
                    "exclude pattern `{pattern}` contains `..` — path \
                     traversal is rejected; use a literal prefix instead"
                ),
            });
        }
    }
}

fn polarity_word(p: MetricPolarity) -> &'static str {
    match p {
        MetricPolarity::LowerIsBetter => "lower-is-better",
        MetricPolarity::HigherIsBetter => "higher-is-better",
        MetricPolarity::Informational => "informational",
    }
}

fn print_report(workspace_root: &std::path::Path, issues: &[Issue]) {
    println!("rustics doctor — workspace: {}", workspace_root.display());
    if issues.is_empty() {
        println!("  no issues — config is healthy.");
        return;
    }
    for issue in issues {
        let tag = match issue.severity {
            IssueSeverity::Info => "INFO ",
            IssueSeverity::Error => "ERROR",
        };
        println!("  {tag}  {}", issue.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExcludeTable, MetricThresholds, RusticsTable};
    use std::collections::BTreeMap;

    fn config_with_metric(id: &str, warning: Option<f64>, error: Option<f64>) -> Config {
        let mut metrics = BTreeMap::new();
        metrics.insert(
            id.to_string(),
            MetricThresholds {
                warning,
                error,
                enabled: true,
            },
        );
        Config {
            rustics: RusticsTable {
                metrics,
                exclude: ExcludeTable::default(),
            },
        }
    }

    fn config_with_exclude(patterns: Vec<&str>) -> Config {
        Config {
            rustics: RusticsTable {
                metrics: BTreeMap::new(),
                exclude: ExcludeTable {
                    patterns: patterns.into_iter().map(String::from).collect(),
                },
            },
        }
    }

    #[test]
    fn unknown_metric_id_is_an_error() {
        let cfg = config_with_metric("does-not-exist", Some(1.0), None);
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("does-not-exist")));
    }

    #[test]
    fn known_metric_with_sane_thresholds_is_clean() {
        // Cyclomatic Complexity is lower-is-better; warning < error.
        let cfg = config_with_metric("cyclomatic-complexity", Some(8.0), Some(20.0));
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.is_empty());
    }

    #[test]
    fn inverted_thresholds_are_flagged() {
        let cfg = config_with_metric("cyclomatic-complexity", Some(50.0), Some(10.0));
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("inverted")));
    }

    #[test]
    fn exclude_negation_pattern_is_flagged() {
        let cfg = config_with_exclude(vec!["!skip"]);
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("negation")));
    }

    #[test]
    fn exclude_traversal_is_flagged() {
        let cfg = config_with_exclude(vec!["../skip"]);
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("traversal")));
    }
}
