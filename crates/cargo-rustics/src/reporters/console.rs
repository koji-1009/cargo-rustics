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
    write_with(report, &ReportOptions::default(), out)
}

/// Like [`write`] but with `--explain <metric-id>` honoured: any
/// violation whose metric is in `opts.explain_metrics` gets its
/// rationale + refactor hints printed inline, indented under the row.
pub fn write_with(report: &Report, opts: &ReportOptions, out: &mut dyn Write) -> Result<()> {
    if report.violations.is_empty() && report.unused.is_empty() {
        writeln!(
            out,
            "rustics: clean. {} files analysed.",
            report.summary.files_analyzed
        )?;
        return Ok(());
    }
    write_violations_block(report, opts, out)?;
    write_unused_block(report, out)?;
    write_summary_line(report, out)?;
    Ok(())
}

fn write_violations_block(
    report: &Report,
    opts: &ReportOptions,
    out: &mut dyn Write,
) -> Result<()> {
    if report.violations.is_empty() {
        return Ok(());
    }
    let (file_w, scope_w, metric_w) = column_widths(report);
    writeln!(out, "rustics — {} violation(s):", report.summary.violations)?;
    for v in &report.violations {
        let suffix = justified_suffix(v);
        writeln!(
            out,
            "  {sev}  {kind:<6}  {file:<file_w$}:{line:<5}  {scope:<scope_w$}  {metric:<metric_w$}  {value} (>{threshold}){suffix}",
            sev = severity_tag(v.severity),
            kind = scope_kind_short(v.scope_kind),
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
    Ok(())
}

fn write_summary_line(report: &Report, out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "summary: {} files, {} warnings, {} errors, {} unused",
        report.summary.files_analyzed,
        report.summary.warnings,
        report.summary.errors,
        report.unused.len(),
    )?;
    Ok(())
}

/// Renders the public-API reachability findings as a separate block
/// under the violations list. One line per finding, format matching
/// the standalone `cargo rustics unused` output.
fn write_unused_block(report: &Report, out: &mut dyn Write) -> Result<()> {
    if report.unused.is_empty() {
        return Ok(());
    }
    writeln!(
        out,
        "rustics unused — {} candidate(s):",
        report.unused.len()
    )?;
    for u in &report.unused {
        let display_name = match &u.parent {
            Some(parent) => format!("{parent}::{}", u.name),
            None => u.name.clone(),
        };
        writeln!(
            out,
            "  {kind:<6}  {name} — {file}:{line}",
            kind = u.kind,
            name = display_name,
            file = u.file,
            line = u.line,
        )?;
    }
    Ok(())
}

/// Renders rationale + refactor hints under a violation row, indented
/// so the table layout still scans visually. Only fires when the
/// violation's lens is in `--explain` (or, for the AI reporter,
/// auto-explain is on).
fn write_inline_explain(v: &crate::report::Violation, out: &mut dyn Write) -> Result<()> {
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

/// Six-character kind tag for the row prefix. AI agents use the AI
/// reporter's `scopeKind:` field directly; humans benefit from seeing
/// at-a-glance whether a violation is in a `fn` vs a `trait` vs a
/// whole-`impl` block.
fn scope_kind_short(kind: rustics::ScopeKind) -> &'static str {
    match kind {
        rustics::ScopeKind::FreeFunction => "fn    ",
        rustics::ScopeKind::Method => "method",
        rustics::ScopeKind::TraitMethod => "trait ",
        rustics::ScopeKind::Module => "module",
        rustics::ScopeKind::ImplBlock => "impl  ",
        rustics::ScopeKind::TraitDef => "tdef  ",
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
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations: vec![],
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
            unused: vec![],
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
                warnings_justified: 0,
                errors_justified: 0,
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
            stale_dismissals: vec![],
            unused: vec![],
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
    fn info_severity_renders_with_info_tag() {
        // Reachable via `--from-clippy` when clippy emits a level that
        // isn't error/warning (`note`). The console reporter must
        // render the row, not panic / drop the violation.
        let mut r = report_with_one_violation("clippy::needless_borrow", None, &[]);
        r.violations[0].severity = MetricSeverity::Info;
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("INFO"));
        assert!(s.contains("clippy::needless_borrow"));
    }

    #[test]
    fn integer_values_print_without_decimal() {
        assert_eq!(format_value(14.0), "14");
        assert_eq!(format_value(14.5), "14.50");
    }

    fn report_with_one_violation(metric: &str, rationale: Option<&str>, hints: &[&str]) -> Report {
        Report {
            version: 1,
            generated_at: "".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 1,
                errors: 0,
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations: vec![Violation {
                id: "abc".into(),
                file: "src/a.rs".into(),
                line: 1,
                scope: "f".into(),
                scope_kind: ScopeKind::FreeFunction,
                metric: metric.into(),
                value: 11.0,
                threshold: 10.0,
                severity: MetricSeverity::Warning,
                rationale: rationale.map(String::from),
                refactor_hints: hints.iter().map(|s| s.to_string()).collect(),
                references: vec![],
                rust_context: Default::default(),
                complexity_justified: None,
            }],
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
            unused: vec![],
        }
    }

    #[test]
    fn explain_metrics_inlines_rationale_and_hints_under_row() {
        // The dartrics-port `--explain <metric-id>` flag arrives at the
        // console reporter as `opts.explain_metrics`. The rationale +
        // hints must show up under the row using the documented
        // `      | <line>` and `      → <hint>` prefixes.
        use std::collections::HashSet;
        let r = report_with_one_violation(
            "cyclomatic-complexity",
            Some("first line\nsecond line"),
            &["extract a helper", "use a match"],
        );
        let mut explain = HashSet::new();
        explain.insert("cyclomatic-complexity".to_string());
        let opts = ReportOptions {
            auto_explain: false,
            explain_metrics: explain,
        };
        let mut buf = Vec::new();
        write_with(&r, &opts, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("      | first line"));
        assert!(s.contains("      | second line"));
        assert!(s.contains("      → extract a helper"));
        assert!(s.contains("      → use a match"));
    }

    #[test]
    fn explain_metrics_does_not_fire_for_unmatched_lens() {
        // Filter says clone-density; the violation is cyclomatic-
        // complexity → no inline explain.
        use std::collections::HashSet;
        let r = report_with_one_violation(
            "cyclomatic-complexity",
            Some("must not appear"),
            &["must not appear hint"],
        );
        let mut explain = HashSet::new();
        explain.insert("clone-density".to_string());
        let opts = ReportOptions {
            auto_explain: false,
            explain_metrics: explain,
        };
        let mut buf = Vec::new();
        write_with(&r, &opts, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("must not appear"));
        assert!(!s.contains("must not appear hint"));
    }

    #[test]
    fn justified_suffix_renders_each_basis() {
        // The `Branch` variant of `JustificationBasis` is reserved for
        // when the lcov reader gains `BRF/BRH` parsing; the type is
        // already public so `justified_suffix` must format it. Drive
        // the helper directly with both variants.
        use crate::report::{ComplexityJustification, JustificationBasis};
        let mut v = report_with_one_violation("cyclomatic-complexity", None, &[])
            .violations
            .into_iter()
            .next()
            .unwrap();
        v.complexity_justified = Some(ComplexityJustification {
            by: JustificationBasis::Line,
            threshold: 0.95,
            actual: 0.965,
        });
        assert_eq!(justified_suffix(&v), " (justified by 96.5% line coverage)");
        v.complexity_justified = Some(ComplexityJustification {
            by: JustificationBasis::Branch,
            threshold: 0.80,
            actual: 0.85,
        });
        assert_eq!(
            justified_suffix(&v),
            " (justified by 85.0% branch coverage)"
        );
        v.complexity_justified = None;
        assert_eq!(justified_suffix(&v), "");
    }

    fn unused_item(name: &str, parent: Option<&str>) -> crate::unused::UnusedItem {
        crate::unused::UnusedItem {
            file: "src/u.rs".into(),
            line: 7,
            name: name.into(),
            kind: "fn".into(),
            parent: parent.map(String::from),
        }
    }

    #[test]
    fn unused_block_renders_heading_and_each_item() {
        // The unify-analyze-unused branch routes `report.unused` into
        // the console reporter as a separate block under the violations
        // list. Verify the heading carries the count, both `Type::name`
        // and bare-name display variants render, and the summary line
        // exposes the unused count.
        let mut r = report_with_one_violation("cyclomatic-complexity", None, &[]);
        r.unused = vec![
            unused_item("variant", Some("Color")),
            unused_item("orphan", None),
        ];
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("rustics unused — 2 candidate(s):"), "got: {s}");
        assert!(s.contains("Color::variant"), "got: {s}");
        assert!(s.contains("orphan"), "got: {s}");
        assert!(s.contains("src/u.rs:7"), "got: {s}");
        assert!(s.contains("2 unused"), "summary line missing count: {s}");
    }

    #[test]
    fn unused_only_report_skips_clean_message() {
        // No metric violations but unused items remain: the "clean"
        // early-out must not fire — the user still has work to look at.
        let mut r = empty_report();
        r.unused = vec![unused_item("orphan", None)];
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("clean"));
        assert!(s.contains("rustics unused — 1 candidate(s):"));
    }

    #[test]
    fn scope_kind_short_covers_every_variant() {
        // The console row prefix uses a six-character padded tag per
        // scope kind so a reviewer can scan the column at a glance.
        // Only `FreeFunction` was driven through the row renderer
        // before — pin every arm directly. The width matters: the
        // row format string assumes `{kind:<6}` but the static
        // strings are already six chars, so width-padding is a no-op.
        assert_eq!(scope_kind_short(rustics::ScopeKind::FreeFunction), "fn    ");
        assert_eq!(scope_kind_short(rustics::ScopeKind::Method), "method");
        assert_eq!(scope_kind_short(rustics::ScopeKind::TraitMethod), "trait ");
        assert_eq!(scope_kind_short(rustics::ScopeKind::Module), "module");
        assert_eq!(scope_kind_short(rustics::ScopeKind::ImplBlock), "impl  ");
        assert_eq!(scope_kind_short(rustics::ScopeKind::TraitDef), "tdef  ");
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
                warnings_justified: 0,
                errors_justified: 0,
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
            stale_dismissals: vec![],
            unused: vec![],
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("(justified by 96.5% line coverage)"));
    }
}
