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
    run_in(&std::env::current_dir()?)
}

/// Like [`run`] but resolves the workspace from `cwd` rather than the
/// process-global current directory. Tests use this entry point so they
/// can drive the command against a temporary fixture without mutating
/// the test harness's working directory.
pub fn run_in(cwd: &std::path::Path) -> Result<u8> {
    let workspace_root = workspace::resolve_workspace_root(cwd)?;
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
    let mut known: std::collections::HashSet<&'static str> =
        metrics.iter().map(|m| m.id()).collect();
    // Cross-file lenses live outside `MetricCalculator` (computed by
    // the cross-file pass in `analyze.rs`) but `rustics.toml` can
    // still override their thresholds — so doctor must accept their
    // ids without flagging "unknown metric id".
    known.extend(crate::cross_file::CROSS_FILE_METRIC_IDS);
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
        check_threshold_value("warning", id, override_.warning, out);
        check_threshold_value("error", id, override_.error, out);
    }
}

/// Validates a single threshold value from `rustics.toml`. NaN and
/// negative values silently disable the gate (the comparison
/// `measurement > NaN` is always false for `lower-is-better`), so
/// catch them at config-load time with a clear message.
fn check_threshold_value(field: &str, id: &str, value: Option<f64>, out: &mut Vec<Issue>) {
    let Some(v) = value else { return };
    if v.is_nan() {
        out.push(Issue {
            severity: IssueSeverity::Error,
            message: format!("{id}: {field} threshold is NaN — must be a finite number"),
        });
    } else if v.is_infinite() {
        out.push(Issue {
            severity: IssueSeverity::Error,
            message: format!("{id}: {field} threshold is infinite — must be finite"),
        });
    } else if v < 0.0 {
        out.push(Issue {
            severity: IssueSeverity::Error,
            message: format!(
                "{id}: {field} threshold {v} is negative — only `>= 0` makes \
                 sense for a `lower-is-better` lens"
            ),
        });
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
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
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
    fn cross_file_metric_id_is_accepted() {
        // Cross-file lens ids (computed outside `MetricCalculator`)
        // are still configurable via `[rustics.metrics.<id>]`, so
        // doctor must honour them the same way `analyze --metric`
        // does — no "unknown metric id" rejection.
        let cfg = config_with_metric("afferent-coupling", Some(15.0), Some(30.0));
        let issues = check(&cfg, &builtin_metrics());
        assert!(
            !issues.iter().any(|i| i.message.contains("unknown metric")),
            "cross-file id 'afferent-coupling' was rejected: {issues:?}"
        );
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

    #[test]
    fn polarity_word_renders_each_variant() {
        assert_eq!(polarity_word(MetricPolarity::LowerIsBetter), "lower-is-better");
        assert_eq!(polarity_word(MetricPolarity::HigherIsBetter), "higher-is-better");
        assert_eq!(polarity_word(MetricPolarity::Informational), "informational");
    }

    #[test]
    fn threshold_pair_ok_handles_each_polarity() {
        // LowerIsBetter — warning ≤ error is OK.
        assert!(threshold_pair_ok(5.0, 10.0, MetricPolarity::LowerIsBetter));
        assert!(!threshold_pair_ok(10.0, 5.0, MetricPolarity::LowerIsBetter));
        // HigherIsBetter — warning ≥ error is OK.
        assert!(threshold_pair_ok(0.9, 0.5, MetricPolarity::HigherIsBetter));
        assert!(!threshold_pair_ok(0.1, 0.9, MetricPolarity::HigherIsBetter));
        // Informational accepts any pair.
        assert!(threshold_pair_ok(1.0, 2.0, MetricPolarity::Informational));
        assert!(threshold_pair_ok(2.0, 1.0, MetricPolarity::Informational));
    }

    #[test]
    fn has_errors_distinguishes_severity() {
        assert!(!has_errors(&[]));
        assert!(!has_errors(&[Issue {
            severity: IssueSeverity::Info,
            message: "x".into(),
        }]));
        assert!(has_errors(&[Issue {
            severity: IssueSeverity::Error,
            message: "x".into(),
        }]));
    }

    #[test]
    fn print_report_handles_empty_and_populated() {
        // Just verifies no panic — output goes to stdout. Live coverage
        // of the println paths.
        print_report(std::path::Path::new("/tmp/x"), &[]);
        print_report(
            std::path::Path::new("/tmp/y"),
            &[
                Issue { severity: IssueSeverity::Info, message: "i".into() },
                Issue { severity: IssueSeverity::Error, message: "e".into() },
            ],
        );
    }

    fn write_workspace_with_config(toml: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let dir = std::env::temp_dir().join(format!("rustics-doctor-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = []\nresolver = \"2\"\n",
        )
        .unwrap();
        if !toml.is_empty() {
            std::fs::write(dir.join("rustics.toml"), toml).unwrap();
        }
        dir
    }

    /// `run()` resolves the workspace from process cwd, so we drive it
    /// through `check` + `has_errors` indirectly here. The
    /// `print_report` test above already covers the printing path; this
    /// test asserts the assembled exit-decision logic against a config
    /// that does have a real lens override.
    #[test]
    fn check_with_full_config_returns_no_issues_when_clean() {
        let dir = write_workspace_with_config(
            "[rustics.metrics.cyclomatic-complexity]\nwarning = 8\nerror = 20\n",
        );
        let cfg = Config::load_from(&dir).unwrap();
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.is_empty(), "issues = {issues:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn negative_threshold_is_flagged() {
        let cfg = config_with_metric("cyclomatic-complexity", Some(-1.0), None);
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("negative")));
    }

    #[test]
    fn nan_threshold_is_flagged() {
        let cfg = config_with_metric("cyclomatic-complexity", Some(f64::NAN), None);
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("NaN")));
    }

    #[test]
    fn infinite_threshold_is_flagged() {
        let cfg = config_with_metric("cyclomatic-complexity", None, Some(f64::INFINITY));
        let issues = check(&cfg, &builtin_metrics());
        assert!(issues.iter().any(|i| i.message.contains("infinite")));
    }

    #[test]
    fn zero_threshold_is_allowed() {
        // `warning = 0` is a valid user choice (every measurement
        // violates) — must NOT be flagged.
        let cfg = config_with_metric("cyclomatic-complexity", Some(0.0), Some(20.0));
        let issues = check(&cfg, &builtin_metrics());
        // Zero passes the value-validity check; the inversion check is
        // also satisfied (0 <= 20). No issues.
        assert!(issues.is_empty(), "zero should not be flagged: {issues:?}");
    }

    #[test]
    fn check_with_full_config_surfaces_inversion() {
        let dir = write_workspace_with_config(
            "[rustics.metrics.cyclomatic-complexity]\nwarning = 50\nerror = 5\n",
        );
        let cfg = Config::load_from(&dir).unwrap();
        let issues = check(&cfg, &builtin_metrics());
        assert!(has_errors(&issues));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_in_returns_zero_for_clean_workspace_with_no_config() {
        // No rustics.toml at the workspace root → doctor surfaces an
        // INFO entry but exits 0 (clean).
        let dir = write_workspace_with_config("");
        let code = run_in(&dir).unwrap();
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_in_returns_zero_for_clean_workspace_with_valid_config() {
        let dir = write_workspace_with_config(
            "[rustics.metrics.cyclomatic-complexity]\nwarning = 8\nerror = 20\n",
        );
        let code = run_in(&dir).unwrap();
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_in_returns_one_when_config_has_errors() {
        let dir = write_workspace_with_config(
            "[rustics.metrics.does-not-exist]\nwarning = 1\n",
        );
        let code = run_in(&dir).unwrap();
        assert_eq!(code, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
