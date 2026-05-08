//! Console reporter — human-readable, lined up.
//!
//! Designed for terminals, not for piping into another tool. Use `json`
//! for that, or `ai` for an LLM. The console format pads columns by the
//! widest entry in the violation list, which keeps the output compact even
//! when most violations are short.

use std::io::Write;

use anyhow::Result;

use rustics::MetricSeverity;

use crate::report::Report;
use crate::reporters::ReportOptions;

/// Writes the human-readable form of `report` to `out` with default
/// options (no inline rationale). Embedding-host convenience.
#[allow(dead_code)] // public convenience API; the CLI uses `write_with`.
pub fn write(report: &Report, out: &mut dyn Write) -> Result<()> {
    write_with(report, &ReportOptions::lean(), out)
}

/// Like [`write`] but with `--explain <metric-id>` honoured: any
/// violation whose metric is in `opts.explain_metrics` gets its
/// rationale + refactor hints printed inline, indented under the row.
pub fn write_with(report: &Report, opts: &ReportOptions, out: &mut dyn Write) -> Result<()> {
    if report.violations.is_empty() {
        writeln!(
            out,
            "rustics: clean. {} files analysed.",
            report.summary.files_analyzed
        )?;
        return Ok(());
    }

    let (file_w, scope_w, metric_w) = column_widths(report);
    writeln!(out, "rustics — {} violation(s):", report.summary.violations)?;
    for v in &report.violations {
        let suffix = justified_suffix(v);
        writeln!(
            out,
            "  {sev}  {file:<file_w$}:{line:<5}  {scope:<scope_w$}  {metric:<metric_w$}  {value} (>{threshold}){suffix}",
            sev = severity_tag(v.severity),
            file = v.file,
            line = v.line,
            scope = v.scope,
            metric = v.metric,
            value = format_value(v.value),
            threshold = format_value(v.threshold),
        )?;
        if opts.should_explain(&v.metric) {
            write_inline_explain(v, out)?;
        }
    }
    writeln!(
        out,
        "summary: {} files, {} warnings, {} errors",
        report.summary.files_analyzed, report.summary.warnings, report.summary.errors
    )?;
    Ok(())
}

/// Renders rationale + refactor hints under a violation row, indented
/// so the table layout still scans visually. Only fires when the
/// violation's lens is in `--explain` (or, for the AI reporter,
/// auto-explain is on).
fn write_inline_explain(
    v: &crate::report::Violation,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(rationale) = &v.rationale {
        for line in rationale.lines() {
            writeln!(out, "      | {line}")?;
        }
    }
    for hint in &v.refactor_hints {
        writeln!(out, "      → {hint}")?;
    }
    Ok(())
}

fn column_widths(report: &Report) -> (usize, usize, usize) {
    let mut file_w = 0;
    let mut scope_w = 0;
    let mut metric_w = 0;
    for v in &report.violations {
        file_w = file_w.max(v.file.len());
        scope_w = scope_w.max(v.scope.len());
        metric_w = metric_w.max(v.metric.len());
    }
    (file_w, scope_w, metric_w)
}

fn severity_tag(s: MetricSeverity) -> &'static str {
    match s {
        MetricSeverity::Error => "ERROR ",
        MetricSeverity::Warning => "WARN  ",
        MetricSeverity::Info => "INFO  ",
    }
}

fn format_value(v: f64) -> String {
    if (v - v.trunc()).abs() < f64::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// `(justified by 96.5% line coverage)` suffix for a justified
/// complexity-class violation; empty string otherwise.
fn justified_suffix(v: &crate::report::Violation) -> String {
    let Some(j) = &v.complexity_justified else {
        return String::new();
    };
    let basis = match j.by {
        crate::report::JustificationBasis::Line => "line",
        crate::report::JustificationBasis::Branch => "branch",
    };
    format!(" (justified by {:.1}% {basis} coverage)", j.actual * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Summary, Violation};
    use rustics::ScopeKind;

    fn empty_report() -> Report {
        Report {
            version: 1,
            generated_at: "2026-05-08T00:00:00Z".into(),
            summary: Summary {
                files_analyzed: 7,
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
    fn clean_run_says_clean() {
        let mut buf = Vec::new();
        write(&empty_report(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("clean"));
        assert!(s.contains("7 files"));
    }

    #[test]
    fn violations_emit_one_line_each() {
        let r = Report {
            version: 1,
            generated_at: "".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![Violation {
                id: "abcdef0123456789".into(),
                file: "src/a.rs".into(),
                line: 42,
                scope: "f".into(),
                scope_kind: ScopeKind::FreeFunction,
                metric: "cyclomatic-complexity".into(),
                value: 14.0,
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
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("WARN"));
        assert!(s.contains("src/a.rs:42"));
        assert!(s.contains("cyclomatic-complexity"));
        assert!(s.contains("14"));
    }

    #[test]
    fn integer_values_print_without_decimal() {
        assert_eq!(format_value(14.0), "14");
        assert_eq!(format_value(14.5), "14.50");
    }

    #[test]
    fn justified_violations_show_coverage_suffix() {
        use crate::report::{ComplexityJustification, JustificationBasis};
        let r = Report {
            version: 1,
            generated_at: "".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![Violation {
                id: "abc".into(),
                file: "src/a.rs".into(),
                line: 1,
                scope: "f".into(),
                scope_kind: ScopeKind::FreeFunction,
                metric: "cyclomatic-complexity".into(),
                value: 25.0,
                threshold: 10.0,
                severity: MetricSeverity::Warning,
                rationale: None,
                refactor_hints: vec![],
                references: vec![],
                rust_context: Default::default(),
                complexity_justified: Some(ComplexityJustification {
                    by: JustificationBasis::Line,
                    threshold: 0.95,
                    actual: 0.965,
                }),
            }],
            truncated: 0,
            measurements: vec![],
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("(justified by 96.5% line coverage)"));
    }
}
