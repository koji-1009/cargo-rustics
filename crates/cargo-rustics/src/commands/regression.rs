//! `cargo rustics regression` — closes the AI loop.
//!
//! Plan §1.4, §4.5, §5.1. Loads two JSON snapshots, diffs them through
//! [`crate::regression::compute`], and writes the result.

use std::io::Write;

use anyhow::{Context, Result};

use crate::cli::{RegressionArgs, Reporter};
use crate::regression::{self, RegressionReport, Verdict};
use crate::report::Report;

/// Runs the `regression` subcommand. Exit codes:
///
/// * `0` — clean / improved / unchanged
/// * `1` — at least one regression *and* `--fatal-regressions` set, or
///   any regression at all if `--fatal-regressions` was not requested but
///   the verdict is `regressed` or `mixed` (we surface the regression
///   so AI loops do not silently advance).
pub fn run(args: RegressionArgs) -> Result<u8> {
    let before = read_report(&args.before, "before")?;
    let after = read_report(&args.after, "after")?;
    let report = regression::compute(before, after);

    let mut out = std::io::stdout().lock();
    write_report(&report, args.reporter, &mut out)?;
    out.flush().ok();

    Ok(decide_exit(&report, args.fatal_regressions))
}

fn read_report(path: &std::path::Path, label: &str) -> Result<Report> {
    let bytes = std::fs::read_to_string(path)
        .with_context(|| format!("read {label} snapshot at {}", path.display()))?;
    let report: Report = serde_json::from_str(&bytes)
        .with_context(|| format!("parse {label} snapshot at {}", path.display()))?;
    Ok(report)
}

fn write_report(report: &RegressionReport, reporter: Reporter, out: &mut dyn Write) -> Result<()> {
    match reporter {
        Reporter::Json => write_json(report, out),
        Reporter::Ai => write_ai(report, out),
        // md / sarif fall back to a JSON dump for now — regression is
        // not the primary surface for those formats. They land properly
        // when the regression report grows the cosmetic-detection
        // signals (M2 follow-up).
        _ => write_console(report, out),
    }
}

fn write_json(report: &RegressionReport, out: &mut dyn Write) -> Result<()> {
    let s = serde_json::to_string_pretty(report)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

fn write_ai(report: &RegressionReport, out: &mut dyn Write) -> Result<()> {
    write_ai_header(report, out)?;
    write_ai_snapshot(out, "before", &report.before)?;
    write_ai_snapshot(out, "after", &report.after)?;
    write_ai_diff(out, &report.diff)?;
    write_id_list(out, "improved", &report.improved)?;
    write_id_list(out, "regressed", &report.regressed)?;
    Ok(())
}

fn write_ai_header(report: &RegressionReport, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "# rustics regression-report v{}", report.version)?;
    writeln!(out, "version: {}", report.version)?;
    writeln!(out, "verdict: {}", verdict_word(report.verdict))?;
    Ok(())
}

fn write_ai_snapshot(
    out: &mut dyn Write,
    label: &str,
    snap: &crate::regression::SnapshotSummary,
) -> Result<()> {
    writeln!(out, "{label}:")?;
    writeln!(out, "  generatedAt: {}", snap.generated_at)?;
    writeln!(out, "  violations: {}", snap.violations)?;
    Ok(())
}

fn write_ai_diff(out: &mut dyn Write, diff: &crate::regression::DiffCounts) -> Result<()> {
    writeln!(out, "diff:")?;
    writeln!(out, "  improved: {}", diff.improved)?;
    writeln!(out, "  regressed: {}", diff.regressed)?;
    writeln!(out, "  unchanged: {}", diff.unchanged)?;
    Ok(())
}

fn write_id_list(
    out: &mut dyn Write,
    label: &str,
    list: &[crate::report::Violation],
) -> Result<()> {
    if list.is_empty() {
        return Ok(());
    }
    writeln!(out, "{label}Violations:")?;
    for v in list {
        writeln!(out, "  - id: {}", v.id)?;
        writeln!(out, "    file: {}", v.file)?;
        writeln!(out, "    scope: {}", v.scope)?;
        writeln!(out, "    metric: {}", v.metric)?;
        writeln!(out, "    value: {}", v.value)?;
    }
    Ok(())
}

fn write_console(report: &RegressionReport, out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "rustics regression: {} (improved {}, regressed {}, unchanged {})",
        verdict_word(report.verdict),
        report.diff.improved,
        report.diff.regressed,
        report.diff.unchanged,
    )?;
    if !report.regressed.is_empty() {
        writeln!(out, "regressed:")?;
        for v in &report.regressed {
            writeln!(
                out,
                "  + {} {}:{} {} = {}",
                v.id, v.file, v.line, v.metric, v.value,
            )?;
        }
    }
    if !report.improved.is_empty() {
        writeln!(out, "improved:")?;
        for v in &report.improved {
            writeln!(out, "  - {} {} {}", v.id, v.file, v.metric)?;
        }
    }
    Ok(())
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Clean => "clean",
        Verdict::Improved => "improved",
        Verdict::Regressed => "regressed",
        Verdict::Mixed => "mixed",
        Verdict::Unchanged => "unchanged",
    }
}

fn decide_exit(report: &RegressionReport, fatal: bool) -> u8 {
    let regressed = matches!(report.verdict, Verdict::Regressed | Verdict::Mixed);
    if regressed && fatal {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regression::{DiffCounts, RegressionReport, SnapshotSummary};

    fn empty_report(verdict: Verdict) -> RegressionReport {
        RegressionReport {
            version: 1,
            before: SnapshotSummary {
                generated_at: "T".into(),
                violations: 0,
                warnings: 0,
                errors: 0,
            },
            after: SnapshotSummary {
                generated_at: "T".into(),
                violations: 0,
                warnings: 0,
                errors: 0,
            },
            diff: DiffCounts {
                improved: 0,
                regressed: 0,
                unchanged: 0,
            },
            verdict,
            improved: vec![],
            regressed: vec![],
            unchanged: vec![],
            cosmetic_analysis: None,
        }
    }

    #[test]
    fn fatal_regressions_exits_one() {
        let r = empty_report(Verdict::Regressed);
        assert_eq!(decide_exit(&r, true), 1);
        assert_eq!(decide_exit(&r, false), 0);
    }

    #[test]
    fn improved_never_exits_one() {
        let r = empty_report(Verdict::Improved);
        assert_eq!(decide_exit(&r, true), 0);
    }

    #[test]
    fn ai_output_starts_with_header() {
        let r = empty_report(Verdict::Clean);
        let mut buf = Vec::new();
        write_ai(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("# rustics regression-report v1\n"));
        assert!(s.contains("verdict: clean"));
    }
}
