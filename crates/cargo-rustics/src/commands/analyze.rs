//! `cargo rustics analyze` — the body of the loop.
//!
//! Walks the workspace, runs every enabled lens against every `.rs` file's
//! AST in parallel, and emits a report in the chosen format. This module
//! is the only place where lens output meets thresholds — the lens
//! library is threshold-agnostic on purpose (plan §3.2 — independence).

use std::io::Write;

use anyhow::{bail, Result};

use rustics::{
    ai_report_contract_version, builtin_metrics, violation_id, MetricCalculator, MetricMetadata,
    MetricSeverity, Threshold,
};

use crate::cli::AnalyzeArgs;
use crate::config::{Config, MetricThresholds};
use crate::discover;
use crate::report::{Report, Summary, Violation};
use crate::reporters;
use crate::runner::{self, FileMetricRecord};
use crate::workspace;

/// Runs the `analyze` subcommand. Returns the exit code:
/// * `0` clean (or warnings without `--fatal-warnings`)
/// * `1` violation present and `--fatal-warnings` was set, or any error severity
pub fn run(args: AnalyzeArgs) -> Result<u8> {
    let analysis_root = match args.root {
        Some(ref p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let workspace_root = workspace::resolve_workspace_root(&analysis_root)?;
    let config = Config::load_from(&workspace_root)?;
    let metrics = pick_metrics(&args)?;

    let files = discover::discover_rust_files(&analysis_root, &workspace_root)?;
    if args.verbose {
        eprintln!(
            "rustics: workspace={} files={} metrics={}",
            workspace_root.display(),
            files.len(),
            metrics.len()
        );
    }
    let concurrency = args.concurrency.unwrap_or_else(default_concurrency);
    let output = runner::run(&files, &metrics, concurrency);

    for err in &output.parse_errors {
        eprintln!("rustics: parse error in {}: {}", err.relative, err.message);
    }

    let mut report = build_report(&output.records, &config, output.files_analyzed);
    report.sort_violations();

    let mut out = std::io::stdout().lock();
    reporters::write(args.reporter, &report, &mut out)?;
    out.flush().ok();

    let exit = decide_exit(&report, args.fatal_warnings);
    Ok(exit)
}

fn default_concurrency() -> usize {
    let n = std::thread::available_parallelism()
        .map(|nz| nz.get())
        .unwrap_or(1);
    n.clamp(1, 16)
}

/// Selects the metric set per `--metric` / `--exclude-metric`.
fn pick_metrics(args: &AnalyzeArgs) -> Result<Vec<Box<dyn MetricCalculator>>> {
    let all = builtin_metrics();
    let known: Vec<&'static str> = all.iter().map(|m| m.id()).collect();

    let mut include = expand_csv(&args.include_metrics);
    let exclude = expand_csv(&args.exclude_metrics);

    for id in include.iter().chain(exclude.iter()) {
        if !known.iter().any(|k| *k == id) {
            bail!("unknown metric id `{id}`; run `cargo rustics rules` for the catalogue");
        }
    }

    if include.is_empty() {
        include = known.iter().map(|s| s.to_string()).collect();
    }
    let mut chosen: Vec<Box<dyn MetricCalculator>> = Vec::new();
    for m in all {
        let id = m.id().to_string();
        if include.contains(&id) && !exclude.contains(&id) {
            chosen.push(m);
        }
    }
    Ok(chosen)
}

fn expand_csv(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for v in values {
        for part in v.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
    out
}

fn build_report(records: &[FileMetricRecord], config: &Config, files_analyzed: usize) -> Report {
    let mut violations = Vec::new();
    for rec in records {
        let thresholds = thresholds_for(&rec.metadata, config);
        if !thresholds.enabled {
            continue;
        }
        for measurement in &rec.measurements {
            if let Some(v) = build_violation(rec, measurement, &thresholds) {
                violations.push(v);
            }
        }
    }
    let warnings = violations
        .iter()
        .filter(|v| v.severity == MetricSeverity::Warning)
        .count();
    let errors = violations
        .iter()
        .filter(|v| v.severity == MetricSeverity::Error)
        .count();
    Report {
        version: ai_report_contract_version(),
        generated_at: now_iso8601(),
        summary: Summary {
            files_analyzed,
            violations: violations.len(),
            warnings,
            errors,
        },
        violations,
    }
}

/// Resolves the effective thresholds for a metric, giving config overrides
/// precedence over the metric's defaults.
struct EffectiveThresholds {
    enabled: bool,
    polarity: rustics::MetricPolarity,
    warning: Option<Threshold>,
    error: Option<Threshold>,
}

fn thresholds_for(meta: &MetricMetadata, config: &Config) -> EffectiveThresholds {
    let override_ = config.metric(meta.id);
    let (warning, error, enabled) = match override_ {
        Some(MetricThresholds {
            warning,
            error,
            enabled,
        }) => (
            warning.map(Threshold::new).or(meta.default_warning),
            error.map(Threshold::new).or(meta.default_error),
            enabled,
        ),
        None => (meta.default_warning, meta.default_error, true),
    };
    EffectiveThresholds {
        enabled,
        polarity: meta.polarity,
        warning,
        error,
    }
}

/// Derives a Rust-ish module path from a workspace-relative source file path.
///
/// `crates/foo-bar/src/baz/qux.rs` → `baz::qux`.
/// `crates/foo/src/lib.rs` → `""`.
/// `crates/foo/src/baz/mod.rs` → `baz`.
///
/// The prefix is prepended to the per-scope path the visitor produces so the
/// AI report's `scope:` field is a complete `module::Type::method` identifier
/// even though the metric library walks one file at a time and never sees the
/// crate's `mod` declarations (plan §4.1).
fn file_to_module_prefix(relative: &str) -> String {
    let path = std::path::Path::new(relative);
    let mut after_src: Vec<String> = path
        .iter()
        .skip_while(|p| p.to_str() != Some("src"))
        .skip(1)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if let Some(last) = after_src.last_mut() {
        if let Some(stripped) = last.strip_suffix(".rs") {
            *last = stripped.to_string();
        }
    }
    if matches!(
        after_src.last().map(String::as_str),
        Some("lib" | "main" | "mod")
    ) {
        after_src.pop();
    }
    after_src.join("::")
}

fn join_scope(prefix: &str, inner: &str) -> String {
    if prefix.is_empty() {
        inner.to_string()
    } else if inner.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}::{inner}")
    }
}

fn build_violation(
    rec: &FileMetricRecord,
    measurement: &rustics::MetricMeasurement,
    thresholds: &EffectiveThresholds,
) -> Option<Violation> {
    let (severity, threshold_value) = pick_severity(measurement.value, thresholds)?;
    let module_prefix = file_to_module_prefix(&rec.relative);
    let scope_path = join_scope(&module_prefix, &measurement.scope.path);
    let id = violation_id(&rec.relative, &scope_path, &rec.metric);
    Some(Violation {
        id,
        file: rec.relative.clone(),
        line: measurement.scope.line,
        scope: scope_path,
        scope_kind: measurement.scope.kind,
        metric: rec.metric.clone(),
        value: measurement.value,
        threshold: threshold_value,
        severity,
        rationale: Some(rec.metadata.rationale.to_string()),
        refactor_hints: collect_strings(rec.metadata.refactor_hints),
        references: collect_strings(rec.metadata.references),
    })
}

/// Picks the severity tier (and the threshold value the measurement
/// crossed) for a measured `value`, or returns `None` if the value is
/// below every configured threshold.
fn pick_severity(value: f64, thresholds: &EffectiveThresholds) -> Option<(MetricSeverity, f64)> {
    if let Some(t) = thresholds.error.as_ref() {
        if t.is_violated_by(value, thresholds.polarity) {
            return Some((MetricSeverity::Error, t.value));
        }
    }
    if let Some(t) = thresholds.warning.as_ref() {
        if t.is_violated_by(value, thresholds.polarity) {
            return Some((MetricSeverity::Warning, t.value));
        }
    }
    None
}

fn collect_strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

fn decide_exit(report: &Report, fatal_warnings: bool) -> u8 {
    if report.summary.errors > 0 {
        return 1;
    }
    if fatal_warnings && report.summary.warnings > 0 {
        return 1;
    }
    0
}

/// Pulls a coarse ISO-8601 UTC timestamp without depending on `chrono`.
///
/// `SystemTime::now()` is the only source we need; we format it via the
/// "seconds since 1970" -> calendar conversion below. This stays accurate
/// for the foreseeable future and avoids a non-trivial dep.
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    epoch_to_iso8601(secs)
}

/// Converts seconds-since-Unix-epoch to `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Pure function; no leap seconds, UTC only. Tested with a few well-known
/// timestamps.
fn epoch_to_iso8601(mut secs: i64) -> String {
    let mut days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    secs = secs_of_day;
    let hour = (secs / 3600) as u32;
    let minute = ((secs % 3600) / 60) as u32;
    let second = (secs % 60) as u32;

    // Civil-from-days (Howard Hinnant's algorithm).
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_metric_is_rejected() {
        let args = AnalyzeArgs {
            root: None,
            reporter: crate::cli::Reporter::Console,
            include_metrics: vec!["does-not-exist".into()],
            exclude_metrics: vec![],
            fatal_warnings: false,
            concurrency: None,
            verbose: false,
        };
        match pick_metrics(&args) {
            Ok(_) => panic!("expected unknown-metric error"),
            Err(e) => assert!(e.to_string().contains("unknown metric id")),
        }
    }

    #[test]
    fn csv_metric_list_is_split() {
        let v = expand_csv(&["a,b".into(), "c".into(), "".into(), " d ".into()]);
        assert_eq!(v, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn iso8601_known_epoch() {
        // 2026-05-08T00:00:00Z = 1778198400
        assert_eq!(epoch_to_iso8601(1_778_198_400), "2026-05-08T00:00:00Z");
        // Unix epoch.
        assert_eq!(epoch_to_iso8601(0), "1970-01-01T00:00:00Z");
        // Y2K.
        assert_eq!(epoch_to_iso8601(946_684_800), "2000-01-01T00:00:00Z");
    }

    #[test]
    fn fatal_warnings_exit_when_warnings_present() {
        let r = Report {
            version: 1,
            generated_at: "".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![],
        };
        assert_eq!(decide_exit(&r, true), 1);
        assert_eq!(decide_exit(&r, false), 0);
    }

    #[test]
    fn module_prefix_strips_lib_main_mod() {
        assert_eq!(file_to_module_prefix("crates/foo/src/lib.rs"), "");
        assert_eq!(file_to_module_prefix("crates/foo/src/main.rs"), "");
        assert_eq!(file_to_module_prefix("crates/foo/src/baz/mod.rs"), "baz");
    }

    #[test]
    fn module_prefix_keeps_directory_segments() {
        assert_eq!(
            file_to_module_prefix("crates/cargo-rustics/src/reporters/ai.rs"),
            "reporters::ai"
        );
    }

    #[test]
    fn join_scope_handles_empties() {
        assert_eq!(join_scope("", "f"), "f");
        assert_eq!(join_scope("a::b", "f"), "a::b::f");
        assert_eq!(join_scope("a::b", ""), "a::b");
    }

    #[test]
    fn errors_always_exit_one() {
        let r = Report {
            version: 1,
            generated_at: "".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 0,
                errors: 1,
            },
            violations: vec![],
        };
        assert_eq!(decide_exit(&r, false), 1);
    }
}
