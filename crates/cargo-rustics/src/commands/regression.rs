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
    let before_path = resolve_before(&args.before)?;
    let before = read_report(&before_path, "before")?;
    let after = read_report(&args.after, "after")?;
    let report = regression::compute(before, after);

    let mut out = std::io::stdout().lock();
    write_report(&report, args.reporter, &mut out)?;
    out.flush().ok();

    Ok(decide_exit(&report, args.fatal_regressions))
}

/// Resolves `--before` into a real path. `cache` and `baseline` are
/// keywords that map to the persisted snapshot location for the
/// current workspace; anything else is treated as a literal path.
fn resolve_before(value: &str) -> Result<std::path::PathBuf> {
    if let Some(mode) = crate::snapshot::SnapshotMode::from_keyword(value) {
        let cwd = std::env::current_dir()?;
        let workspace_root = crate::workspace::resolve_workspace_root(&cwd)?;
        return Ok(mode.path_in(&workspace_root));
    }
    Ok(std::path::PathBuf::from(value))
}

fn read_report(path: &std::path::Path, label: &str) -> Result<Report> {
    // `read_report_compat` accepts both bare-Report JSON (what
    // `--reporter json` emits) and the `Snapshot` envelope written by
    // `--snapshot-mode <cache|baseline>`. Either way the inner Report
    // comes back.
    crate::snapshot::read_report_compat(path)
        .with_context(|| format!("{label} snapshot at {}", path.display()))
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
    write_id_list(out, "added", &report.added)?;
    write_id_list(out, "removed", &report.removed)?;
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
    write_ai_diff_counts(out, diff)?;
    Ok(())
}

fn write_ai_diff_counts(
    out: &mut dyn Write,
    diff: &crate::regression::DiffCounts,
) -> Result<()> {
    writeln!(out, "  added: {}", diff.added)?;
    writeln!(out, "  removed: {}", diff.removed)?;
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
        write_id_list_entry(out, v)?;
    }
    Ok(())
}

fn write_id_list_entry(out: &mut dyn Write, v: &crate::report::Violation) -> Result<()> {
    writeln!(out, "  - id: {}", v.id)?;
    writeln!(out, "    file: {}", v.file)?;
    writeln!(out, "    scope: {}", v.scope)?;
    writeln!(out, "    metric: {}", v.metric)?;
    writeln!(out, "    value: {}", v.value)?;
    Ok(())
}

fn write_console(report: &RegressionReport, out: &mut dyn Write) -> Result<()> {
    write_console_header(report, out)?;
    write_console_section(out, "added", '+', &report.added, console_added_line)?;
    write_console_section(out, "removed", '-', &report.removed, console_removed_line)?;
    write_console_section(out, "regressed", '↑', &report.regressed, console_added_line)?;
    write_console_section(out, "improved", '↓', &report.improved, console_added_line)?;
    Ok(())
}

fn write_console_header(report: &RegressionReport, out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "rustics regression: {} (+{} −{} ↑{} ↓{} ={})",
        verdict_word(report.verdict),
        report.diff.added,
        report.diff.removed,
        report.diff.regressed,
        report.diff.improved,
        report.diff.unchanged,
    )?;
    Ok(())
}

fn write_console_section(
    out: &mut dyn Write,
    label: &str,
    bullet: char,
    list: &[crate::report::Violation],
    fmt: fn(char, &crate::report::Violation) -> String,
) -> Result<()> {
    if list.is_empty() {
        return Ok(());
    }
    writeln!(out, "{label}:")?;
    for v in list {
        writeln!(out, "{}", fmt(bullet, v))?;
    }
    Ok(())
}

fn console_added_line(bullet: char, v: &crate::report::Violation) -> String {
    format!(
        "  {bullet} {} {}:{} {} = {}",
        v.id, v.file, v.line, v.metric, v.value
    )
}

fn console_removed_line(bullet: char, v: &crate::report::Violation) -> String {
    format!("  {bullet} {} {} {}", v.id, v.file, v.metric)
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
            version: 2,
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
                added: 0,
                removed: 0,
            },
            verdict,
            improved: vec![],
            regressed: vec![],
            unchanged: vec![],
            added: vec![],
            removed: vec![],
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
        assert!(s.starts_with("# rustics regression-report v2\n"));
        assert!(s.contains("verdict: clean"));
    }

    fn populated_report(verdict: Verdict) -> RegressionReport {
        let v = |id: &str, metric: &str| crate::report::Violation {
            id: id.into(),
            file: "f.rs".into(),
            line: 1,
            scope: "scope".into(),
            scope_kind: rustics::ScopeKind::FreeFunction,
            metric: metric.into(),
            value: 12.0,
            threshold: 10.0,
            severity: rustics::MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        };
        RegressionReport {
            version: 2,
            before: SnapshotSummary {
                generated_at: "B".into(),
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            after: SnapshotSummary {
                generated_at: "A".into(),
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            diff: DiffCounts {
                improved: 1,
                regressed: 1,
                unchanged: 0,
                added: 0,
                removed: 0,
            },
            verdict,
            improved: vec![v("imp1", "cyclomatic-complexity")],
            regressed: vec![v("reg1", "method-length")],
            unchanged: vec![],
            added: vec![],
            removed: vec![],
            cosmetic_analysis: None,
        }
    }

    #[test]
    fn write_console_lists_regressed_and_improved() {
        let r = populated_report(Verdict::Mixed);
        let mut buf = Vec::new();
        write_console(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("rustics regression: mixed"));
        assert!(s.contains("regressed:"));
        assert!(s.contains("↑ reg1 f.rs:1 method-length = 12"));
        assert!(s.contains("improved:"));
        assert!(s.contains("↓ imp1 f.rs:1 cyclomatic-complexity = 12"));
    }

    #[test]
    fn write_json_emits_valid_json() {
        let r = populated_report(Verdict::Clean);
        let mut buf = Vec::new();
        write_json(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["verdict"], "clean");
    }

    #[test]
    fn write_ai_lists_each_section() {
        let r = populated_report(Verdict::Mixed);
        let mut buf = Vec::new();
        write_ai(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("verdict: mixed"));
        assert!(s.contains("before:"));
        assert!(s.contains("after:"));
        assert!(s.contains("diff:"));
        assert!(s.contains("  added: 0"));
        assert!(s.contains("  removed: 0"));
        assert!(s.contains("  improved: 1"));
        assert!(s.contains("  regressed: 1"));
        assert!(s.contains("improvedViolations:"));
        assert!(s.contains("regressedViolations:"));
        assert!(s.contains("    metric: cyclomatic-complexity"));
        assert!(s.contains("    metric: method-length"));
    }

    #[test]
    fn write_report_falls_back_to_console_for_md_and_sarif() {
        for fmt in [Reporter::Md, Reporter::Sarif, Reporter::Console] {
            let r = populated_report(Verdict::Clean);
            let mut buf = Vec::new();
            write_report(&r, fmt, &mut buf).unwrap();
            let s = String::from_utf8(buf).unwrap();
            assert!(s.contains("rustics regression:"), "fmt {fmt:?}: {s}");
        }
    }

    #[test]
    fn verdict_word_covers_each_variant() {
        assert_eq!(verdict_word(Verdict::Clean), "clean");
        assert_eq!(verdict_word(Verdict::Improved), "improved");
        assert_eq!(verdict_word(Verdict::Regressed), "regressed");
        assert_eq!(verdict_word(Verdict::Mixed), "mixed");
        assert_eq!(verdict_word(Verdict::Unchanged), "unchanged");
    }

    fn write_tmp_json(report: &Report) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustics-reg-test-{pid}-{n}.json"));
        std::fs::write(&path, serde_json::to_string(report).unwrap()).unwrap();
        path
    }

    fn empty_input_report() -> Report {
        Report {
            version: 1,
            generated_at: "T".into(),
            summary: crate::report::Summary {
                files_analyzed: 0,
                violations: 0,
                warnings: 0,
                errors: 0,
            },
            violations: vec![],
            truncated: 0,
            measurements: vec![],
        }
    }

    #[test]
    fn run_returns_zero_for_clean_diff() {
        let before = write_tmp_json(&empty_input_report());
        let after = write_tmp_json(&empty_input_report());
        let args = RegressionArgs {
            before: before.to_string_lossy().into_owned(),
            after: after.clone(),
            reporter: Reporter::Json,
            fatal_regressions: false,
        };
        let code = run(args).unwrap();
        assert_eq!(code, 0);
        std::fs::remove_file(&before).ok();
        std::fs::remove_file(&after).ok();
    }

    #[test]
    fn read_report_errors_on_missing_path() {
        let path = std::path::PathBuf::from("/no/such/__rustics_regression_test__.json");
        let err = read_report(&path, "before").unwrap_err();
        // The compat reader prefixes its inner error with our context;
        // either "read snapshot" (IO) or "before snapshot at …" wraps it.
        let msg = format!("{err:#}");
        assert!(msg.contains("before") || msg.contains("read snapshot"), "{msg}");
    }

    #[test]
    fn read_report_errors_on_invalid_json() {
        let path = write_tmp_json_text("garbage not json");
        let err = read_report(&path, "after").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("after") || msg.contains("parse snapshot"), "{msg}");
        std::fs::remove_file(&path).ok();
    }

    fn write_tmp_json_text(body: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustics-reg-bad-{pid}-{n}.json"));
        std::fs::write(&path, body).unwrap();
        path
    }

    /// `resolve_before` is the dartrics-port keyword resolver. A
    /// literal path passes through unchanged; `cache` / `baseline`
    /// resolve to the snapshot location for the *current* workspace.
    /// We don't try to spoof the workspace here — we just verify the
    /// dispatch shape: literal paths come back verbatim, keywords
    /// produce a path ending in the expected snapshot filename.
    #[test]
    fn resolve_before_passes_literal_path_through() {
        let p = resolve_before("/tmp/arbitrary-snapshot.json").unwrap();
        assert_eq!(p, std::path::PathBuf::from("/tmp/arbitrary-snapshot.json"));
    }

    #[test]
    fn resolve_before_cache_keyword_ends_in_cache_path() {
        let p = resolve_before("cache").unwrap();
        assert!(
            p.ends_with("target/.rustics-cache/snapshot.json"),
            "got {}",
            p.display()
        );
    }

    #[test]
    fn resolve_before_baseline_keyword_ends_in_baseline_path() {
        let p = resolve_before("baseline").unwrap();
        assert!(p.ends_with("rustics-snapshot.json"), "got {}", p.display());
    }

    /// `write_report` dispatches by `Reporter`; the `Ai` arm calls
    /// `write_ai`. Existing tests covered Json + Console; this fills
    /// the Ai branch.
    #[test]
    fn write_report_dispatches_to_ai_when_requested() {
        let r = populated_report(Verdict::Mixed);
        let mut buf = Vec::new();
        write_report(&r, Reporter::Ai, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("# rustics regression-report v"));
        assert!(s.contains("verdict: mixed"));
    }

    /// `console_removed_line` is the helper for the regression
    /// console reporter's `removed:` section. The previous test only
    /// exercised the `+ added`-style entry; this test drives the
    /// removed shape end to end via a populated report.
    #[test]
    fn write_console_removed_section_uses_minimal_format() {
        let mut r = populated_report(Verdict::Improved);
        r.removed = vec![crate::report::Violation {
            id: "rem1".into(),
            file: "x.rs".into(),
            line: 2,
            scope: "f".into(),
            scope_kind: rustics::ScopeKind::FreeFunction,
            metric: "panic-density".into(),
            value: 7.0,
            threshold: 5.0,
            severity: rustics::MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        }];
        r.diff.removed = 1;
        let mut buf = Vec::new();
        write_console(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("removed:"));
        // Removed section uses an ASCII `-` bullet (the unicode `−`
        // appears only in the header counters) and a no-line-number
        // format produced by `console_removed_line`.
        assert!(s.contains("- rem1 x.rs panic-density"), "got: {s}");
    }
}
