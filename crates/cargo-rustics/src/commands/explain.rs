//! `cargo rustics explain <id>` — reverse-lookup a violation id.
//!
//! Plan §5.2. The AI loop's idiom is:
//!
//! ```sh
//! cargo rustics analyze --reporter json > report.json
//! cargo rustics explain a3f1c4e9b2d8f7c5 --snapshot report.json
//! ```
//!
//! The output is the lens metadata that produced the violation —
//! rationale, refactor hints, references — plus the violation's own
//! file/scope/metric/value so the agent has full context without
//! re-running analyze.

use std::io::Read;

use anyhow::{bail, Context, Result};

use crate::cli::ExplainArgs;
use crate::report::{Report, Violation};

/// Runs the `explain` subcommand. Exit codes:
///
/// * `0` — id was found and printed.
/// * `1` — id was not present in the snapshot.
pub fn run(args: ExplainArgs) -> Result<u8> {
    let report = read_snapshot(&args)?;
    let Some(violation) = report.violations.iter().find(|v| v.id == args.id) else {
        bail!("violation id `{}` not found in snapshot", args.id);
    };
    print_violation(violation);
    Ok(0)
}

fn read_snapshot(args: &ExplainArgs) -> Result<Report> {
    let raw = match &args.snapshot {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("read snapshot at {}", path.display()))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("read snapshot from stdin")?;
            buf
        }
    };
    let report: Report = serde_json::from_str(&raw).context("parse snapshot")?;
    Ok(report)
}

fn print_violation(v: &Violation) {
    print_violation_identity(v);
    print_violation_metric(v);
    print_violation_rationale(v);
    print_violation_string_list("refactorHints", &v.refactor_hints);
    print_violation_string_list("references", &v.references);
}

/// Header + locator (`id`, `file`, `line`). Kept tiny on purpose so the
/// `format-density` lens stays comfortably below its warning threshold —
/// see the dogfooding refactor in commit history.
fn print_violation_identity(v: &Violation) {
    println!("# rustics explain v1");
    println!("id: {}", v.id);
    println!("file: {}", v.file);
    println!("line: {}", v.line);
}

fn print_violation_metric(v: &Violation) {
    println!("scope: {}", v.scope);
    println!("metric: {}", v.metric);
    println!("value: {}", v.value);
    println!("threshold: {}", v.threshold);
    println!("severity: {:?}", v.severity);
}

fn print_violation_rationale(v: &Violation) {
    let Some(rationale) = &v.rationale else { return };
    println!("rationale: |");
    for line in rationale.lines() {
        println!("  {line}");
    }
}

fn print_violation_string_list(label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!("{label}:");
    for item in items {
        println!("  - {item}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Summary, Violation};
    use rustics::{MetricSeverity, ScopeKind};

    fn sample_violation() -> Violation {
        Violation {
            id: "abc".into(),
            file: "x".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "cyclomatic-complexity".into(),
            value: 11.0,
            threshold: 10.0,
            severity: MetricSeverity::Warning,
            rationale: Some("a\nb".into()),
            refactor_hints: vec!["hint1".into()],
            references: vec!["ref1".into()],
            rust_context: Default::default(),
        }
    }

    fn sample_report() -> Report {
        Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![sample_violation()],
            truncated: 0,
            measurements: vec![],
        }
    }

    #[test]
    fn id_lookup_succeeds_when_present() {
        let report = sample_report();
        let found = report.violations.iter().find(|v| v.id == "abc");
        assert!(found.is_some());
    }

    #[test]
    fn print_helpers_drive_each_branch() {
        // Just verify that the helpers don't panic; output goes to
        // stdout. Live-runs each branch so coverage records every line.
        let v = sample_violation();
        print_violation_identity(&v);
        print_violation_metric(&v);
        print_violation_rationale(&v);
        print_violation_string_list("refactorHints", &v.refactor_hints);
        print_violation_string_list("references", &v.references);
        // None / empty branches.
        let mut bare = sample_violation();
        bare.rationale = None;
        bare.refactor_hints = vec![];
        bare.references = vec![];
        print_violation_rationale(&bare);
        print_violation_string_list("refactorHints", &bare.refactor_hints);
        print_violation(&v);
    }

    fn write_tmp_json(report: &Report) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustics-explain-test-{pid}-{n}.json"));
        std::fs::write(&path, serde_json::to_string(report).unwrap()).unwrap();
        path
    }

    #[test]
    fn run_succeeds_when_id_is_present() {
        let path = write_tmp_json(&sample_report());
        let args = ExplainArgs {
            id: "abc".to_string(),
            snapshot: Some(path.clone()),
        };
        let code = run(args).unwrap();
        assert_eq!(code, 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn run_errors_when_id_is_missing() {
        let path = write_tmp_json(&sample_report());
        let args = ExplainArgs {
            id: "missing".to_string(),
            snapshot: Some(path.clone()),
        };
        let err = run(args).unwrap_err();
        assert!(format!("{err:#}").contains("not found"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_snapshot_errors_on_missing_path() {
        let args = ExplainArgs {
            id: "x".into(),
            snapshot: Some(std::path::PathBuf::from(
                "/no/such/__rustics_explain_test__.json",
            )),
        };
        let err = read_snapshot(&args).unwrap_err();
        assert!(format!("{err:#}").contains("read snapshot"));
    }

    #[test]
    fn read_snapshot_errors_on_invalid_json() {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir()
            .join(format!("rustics-explain-bad-{pid}-{n}.json"));
        std::fs::write(&path, "garbage").unwrap();
        let args = ExplainArgs {
            id: "x".into(),
            snapshot: Some(path.clone()),
        };
        let err = read_snapshot(&args).unwrap_err();
        assert!(format!("{err:#}").contains("parse snapshot"));
        std::fs::remove_file(&path).ok();
    }
}
