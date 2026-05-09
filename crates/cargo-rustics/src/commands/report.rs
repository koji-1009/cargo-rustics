//! `cargo rustics report <input.json>` — reformat an existing snapshot.
//!
//! The CI workflow that produced the JSON does not have to
//! re-run analysis to get the AI-format or console version of the same
//! report.

use std::io::{Read, Write};

use anyhow::{Context, Result};

use crate::cli::ReportArgs;
use crate::report::Report;
use crate::reporters;

/// Runs the `report` subcommand. Always exits `0` on a parseable input.
pub fn run(args: ReportArgs) -> Result<u8> {
    let raw = if args.input.as_os_str() == "-" {
        read_stdin()?
    } else {
        std::fs::read_to_string(&args.input)
            .with_context(|| format!("read snapshot {}", args.input.display()))?
    };
    let report: Report = serde_json::from_str(&raw).context("parse JSON snapshot")?;
    let mut out = std::io::stdout().lock();
    let opts = reporters::ReportOptions {
        auto_explain: matches!(args.reporter, crate::cli::Reporter::Ai) && !args.no_auto_explain,
        explain_metrics: args.explain_metrics.iter().cloned().collect(),
    };
    reporters::write_with(args.reporter, &report, &opts, &mut out)?;
    out.flush().ok();
    Ok(0)
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("read snapshot from stdin")?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;
    use crate::cli::Reporter;
    use crate::report::{Summary, Violation};
    use rustics::{MetricSeverity, ScopeKind};

    fn sample_report() -> Report {
        Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: 1,
                warnings: 1,
                errors: 0,
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations: vec![Violation {
                id: "abc".into(),
                file: "f.rs".into(),
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
            }],
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
        }
    }

    #[test]
    fn round_trip_through_json_then_ai() {
        let report = sample_report();
        let json = serde_json::to_string(&report).unwrap();
        let parsed: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.violations.len(), 1);
        assert_eq!(parsed.violations[0].id, "abc");
    }

    fn write_tmp_json(report: &Report) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let path =
            std::env::temp_dir().join(format!("rustics-report-test-{pid}-{n}-{seq}.json"));
        std::fs::write(&path, serde_json::to_string(report).unwrap()).unwrap();
        path
    }

    #[test]
    fn run_reads_path_and_succeeds() {
        let report = sample_report();
        let path = write_tmp_json(&report);
        let args = ReportArgs {
            input: path.clone(),
            reporter: Reporter::Json,
            no_auto_explain: false,
            explain_metrics: vec![],
        };
        let code = run(args).unwrap();
        assert_eq!(code, 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn run_errors_when_path_missing() {
        let args = ReportArgs {
            input: std::path::PathBuf::from("/no/such/__rustics_test_missing__.json"),
            reporter: Reporter::Json,
            no_auto_explain: false,
            explain_metrics: vec![],
        };
        let err = run(args).unwrap_err();
        assert!(format!("{err:#}").contains("read snapshot"));
    }

    #[test]
    fn run_errors_on_invalid_json() {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let path = std::env::temp_dir()
            .join(format!("rustics-report-test-bad-{pid}-{n}-{seq}.json"));
        std::fs::write(&path, "not really json").unwrap();
        let args = ReportArgs {
            input: path.clone(),
            reporter: Reporter::Json,
            no_auto_explain: false,
            explain_metrics: vec![],
        };
        let err = run(args).unwrap_err();
        assert!(format!("{err:#}").contains("parse JSON snapshot"));
        std::fs::remove_file(&path).ok();
    }
}
