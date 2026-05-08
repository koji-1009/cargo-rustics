//! AI reporter — strict YAML 1.2, header-anchored.
//!
//! Tuned for LLM consumption: predictable indentation, the contract header
//! at the top, every field in a fixed order, multi-line strings as YAML
//! literal-block scalars (`|`). Every string field is run through
//! `scalar_string` so a parser can round-trip the output. We deliberately
//! do *not* depend on `serde_yaml` — we write the format ourselves so we
//! can pin the exact output the AI report contract promises (plan §4.1,
//! §4.3).
//!
//! The function set is split so each helper stays small enough to clear the
//! self-application Cyclomatic Complexity threshold (plan §1.2 — the tool
//! must pass its own lenses on its own code).

use std::io::Write;

use anyhow::Result;

use rustics::MetricSeverity;

use crate::report::{Report, Violation};

/// Writes the AI-report YAML-ish form to `out`.
pub fn write(report: &Report, out: &mut dyn Write) -> Result<()> {
    write_header(report, out)?;
    write_summary(&report.summary, out)?;
    write_violations(&report.violations, out)?;
    write_truncated(report.truncated, out)?;
    Ok(())
}

fn write_truncated(truncated: usize, out: &mut dyn Write) -> Result<()> {
    if truncated > 0 {
        writeln!(out, "truncated: {truncated}")?;
    }
    Ok(())
}

fn write_header(report: &Report, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "# rustics ai-report v{}", report.version)?;
    writeln!(out, "version: {}", report.version)?;
    writeln!(out, "generatedAt: {}", scalar_string(&report.generated_at))?;
    Ok(())
}

fn write_summary(summary: &crate::report::Summary, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "summary:")?;
    writeln!(out, "  filesAnalyzed: {}", summary.files_analyzed)?;
    writeln!(out, "  violations: {}", summary.violations)?;
    writeln!(out, "  warnings: {}", summary.warnings)?;
    writeln!(out, "  errors: {}", summary.errors)?;
    Ok(())
}

fn write_violations(violations: &[Violation], out: &mut dyn Write) -> Result<()> {
    if violations.is_empty() {
        writeln!(out, "violations: []")?;
        return Ok(());
    }
    writeln!(out, "violations:")?;
    for v in violations {
        write_one_violation(v, out)?;
    }
    Ok(())
}

fn write_one_violation(v: &Violation, out: &mut dyn Write) -> Result<()> {
    write_violation_core(v, out)?;
    write_explain(v, out)?;
    write_string_list("    refactorHints:", &v.refactor_hints, out)?;
    write_string_list("    references:", &v.references, out)?;
    Ok(())
}

fn write_violation_core(v: &Violation, out: &mut dyn Write) -> Result<()> {
    // Every string value is run through `scalar_string` so the output
    // is *strict* YAML 1.2 — file paths with spaces, scopes with
    // colon-space, etc. all survive a YAML parser.
    writeln!(out, "  - id: {}", scalar_string(&v.id))?;
    writeln!(out, "    file: {}", scalar_string(&v.file))?;
    writeln!(out, "    line: {}", v.line)?;
    writeln!(out, "    scope: {}", scalar_string(&v.scope))?;
    writeln!(out, "    metric: {}", scalar_string(&v.metric))?;
    writeln!(out, "    value: {}", format_number(v.value))?;
    writeln!(out, "    threshold: {}", format_number(v.threshold))?;
    writeln!(out, "    severity: {}", severity_word(v.severity))?;
    Ok(())
}

fn write_explain(v: &Violation, out: &mut dyn Write) -> Result<()> {
    let Some(rationale) = &v.rationale else {
        return Ok(());
    };
    writeln!(out, "    explain: |")?;
    for line in rationale.lines() {
        writeln!(out, "      {line}")?;
    }
    Ok(())
}

fn write_string_list(header: &str, items: &[String], out: &mut dyn Write) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(out, "{header}")?;
    for item in items {
        writeln!(out, "      - {}", scalar_string(item))?;
    }
    Ok(())
}

fn severity_word(s: MetricSeverity) -> &'static str {
    match s {
        MetricSeverity::Error => "error",
        MetricSeverity::Warning => "warning",
        MetricSeverity::Info => "info",
    }
}

fn format_number(v: f64) -> String {
    if (v - v.trunc()).abs() < f64::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Conservatively quotes a single-line string so YAML parsers (and LLMs
/// trained on YAML) read it as one scalar value.
fn scalar_string(s: &str) -> String {
    if s.contains('\n') {
        // Multi-line — caller should use a literal block instead. We
        // collapse newlines so we don't break our own output, but report
        // hints currently never contain newlines.
        return format!("\"{}\"", s.replace('\n', " ").replace('"', "\\\""));
    }
    if needs_quoting(s) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn needs_quoting(s: &str) -> bool {
    s.is_empty()
        || s.starts_with('-')
        || s.starts_with(':')
        || s.contains(": ")
        || s.contains('#')
        || s.contains('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Summary, Violation};
    use rustics::ScopeKind;

    #[test]
    fn header_is_first_line() {
        let r = Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: 0,
                warnings: 0,
                errors: 0,
            },
            violations: vec![],
            truncated: 0,
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("# rustics ai-report v1\n"));
    }

    #[test]
    fn empty_violations_emit_inline_list() {
        let r = Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: 0,
                warnings: 0,
                errors: 0,
            },
            violations: vec![],
            truncated: 0,
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("violations: []"));
    }

    #[test]
    fn violation_block_renders_every_field() {
        let r = Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![Violation {
                id: "abcd".into(),
                file: "src/x.rs".into(),
                line: 10,
                scope: "x::f".into(),
                scope_kind: ScopeKind::FreeFunction,
                metric: "cyclomatic-complexity".into(),
                value: 14.0,
                threshold: 10.0,
                severity: MetricSeverity::Warning,
                rationale: Some("Multi\nline".into()),
                refactor_hints: vec!["hint a".into(), "hint b".into()],
                references: vec!["ref a".into()],
                rust_context: Default::default(),
            }],
            truncated: 0,
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("- id: abcd"));
        assert!(s.contains("severity: warning"));
        assert!(s.contains("explain: |"));
        assert!(s.contains("      Multi"));
        assert!(s.contains("      line"));
        assert!(s.contains("      - hint a"));
    }

    #[test]
    fn quoting_kicks_in_for_yaml_specials() {
        // ':' followed by space, '#', leading '-' all need quoting.
        assert!(needs_quoting("a: b"));
        assert!(needs_quoting("# tag"));
        assert!(needs_quoting("-leading"));
        assert!(!needs_quoting("plain word"));
    }
}
