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
    println!("# rustics explain v1");
    println!("id: {}", v.id);
    println!("file: {}", v.file);
    println!("line: {}", v.line);
    println!("scope: {}", v.scope);
    println!("metric: {}", v.metric);
    println!("value: {}", v.value);
    println!("threshold: {}", v.threshold);
    println!("severity: {:?}", v.severity);
    if let Some(rationale) = &v.rationale {
        println!("rationale: |");
        for line in rationale.lines() {
            println!("  {line}");
        }
    }
    if !v.refactor_hints.is_empty() {
        println!("refactorHints:");
        for h in &v.refactor_hints {
            println!("  - {h}");
        }
    }
    if !v.references.is_empty() {
        println!("references:");
        for r in &v.references {
            println!("  - {r}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Summary, Violation};
    use rustics::{MetricSeverity, ScopeKind};

    #[test]
    fn id_lookup_succeeds_when_present() {
        let v = Violation {
            id: "abc".into(),
            file: "x".into(),
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
        };
        let report = Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![v],
            truncated: 0,
            measurements: vec![],
        };
        let found = report.violations.iter().find(|v| v.id == "abc");
        assert!(found.is_some());
    }
}
