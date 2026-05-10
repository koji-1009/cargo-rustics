//! AI reporter — strict YAML 1.2, header-anchored.
//!
//! Tuned for LLM consumption: predictable indentation, the contract header
//! at the top, every field in a fixed order, multi-line strings as YAML
//! literal-block scalars (`|`). Every string field is run through
//! `scalar_string` so a parser can round-trip the output. We deliberately
//! do *not* depend on `serde_yaml` — we write the format ourselves so we
//! can pin the exact output the AI report contract promises.
//!
//! The function set is split so each helper stays small enough to clear the
//! self-application Cyclomatic Complexity threshold.

use std::io::Write;

use anyhow::Result;

use rustics::MetricSeverity;

use crate::report::{Report, Violation};
use crate::reporters::{ai_default_options, ReportOptions};

/// Writes the AI-report YAML-ish form to `out` with default options
/// (auto-explain on for every violation). Embedding-host convenience.
#[allow(dead_code)] // public convenience API; the CLI uses `write_with`.
pub fn write(report: &Report, out: &mut dyn Write) -> Result<()> {
    write_with(report, &ai_default_options(), out)
}

/// Writes the AI-report YAML-ish form to `out`, honouring `opts`:
/// `--no-auto-explain` clears `auto_explain` so the per-violation
/// rationale / refactor hints / references blocks are skipped to save
/// tokens; `--explain <metric-id>` re-enables them per lens.
pub fn write_with(report: &Report, opts: &ReportOptions, out: &mut dyn Write) -> Result<()> {
    write_header(report, out)?;
    write_summary(&report.summary, report.unused.len(), out)?;
    write_violations(&report.violations, opts, out)?;
    write_unused(&report.unused, out)?;
    write_stale_dismissals(&report.stale_dismissals, out)?;
    write_truncated(report.truncated, out)?;
    Ok(())
}

fn write_unused(items: &[crate::unused::UnusedItem], out: &mut dyn Write) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(out, "unused:")?;
    for u in items {
        writeln!(out, "  - file: {}", scalar_string(&u.file))?;
        writeln!(out, "    line: {}", u.line)?;
        writeln!(out, "    name: {}", scalar_string(&u.name))?;
        writeln!(out, "    kind: {}", scalar_string(&u.kind))?;
        if let Some(parent) = &u.parent {
            writeln!(out, "    parent: {}", scalar_string(parent))?;
        }
    }
    Ok(())
}

fn write_stale_dismissals(
    stale: &[crate::report::StaleDismissal],
    out: &mut dyn Write,
) -> Result<()> {
    if stale.is_empty() {
        return Ok(());
    }
    writeln!(out, "staleDismissals:")?;
    for d in stale {
        writeln!(out, "  - file: {}", scalar_string(&d.file))?;
        writeln!(out, "    scope: {}", scalar_string(&d.scope))?;
        writeln!(out, "    metric: {}", scalar_string(&d.metric))?;
        writeln!(out, "    reason: {}", scalar_string(&d.reason))?;
    }
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

fn write_summary(
    summary: &crate::report::Summary,
    unused_count: usize,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "summary:")?;
    writeln!(out, "  filesAnalyzed: {}", summary.files_analyzed)?;
    writeln!(out, "  violations: {}", summary.violations)?;
    writeln!(out, "  warnings: {}", summary.warnings)?;
    writeln!(out, "  errors: {}", summary.errors)?;
    if unused_count > 0 {
        writeln!(out, "  unused: {unused_count}")?;
    }
    write_summary_justified(summary, out)?;
    Ok(())
}

/// Emits the `warningsJustified` / `errorsJustified` lines only when
/// non-zero. AI agents subtract these from `warnings` / `errors` to
/// get the count they actually have refactor work on.
fn write_summary_justified(summary: &crate::report::Summary, out: &mut dyn Write) -> Result<()> {
    if summary.warnings_justified > 0 {
        writeln!(out, "  warningsJustified: {}", summary.warnings_justified)?;
    }
    if summary.errors_justified > 0 {
        writeln!(out, "  errorsJustified: {}", summary.errors_justified)?;
    }
    Ok(())
}

fn write_violations(
    violations: &[Violation],
    opts: &ReportOptions,
    out: &mut dyn Write,
) -> Result<()> {
    if violations.is_empty() {
        writeln!(out, "violations: []")?;
        return Ok(());
    }
    writeln!(out, "violations:")?;
    for v in violations {
        write_one_violation(v, opts, out)?;
    }
    Ok(())
}

fn write_one_violation(v: &Violation, opts: &ReportOptions, out: &mut dyn Write) -> Result<()> {
    write_violation_core(v, out)?;
    write_complexity_justified(v, out)?;
    if opts.should_explain(&v.metric) {
        write_explain(v, out)?;
        write_string_list("    refactorHints:", &v.refactor_hints, out)?;
        write_string_list("    references:", &v.references, out)?;
    }
    write_rust_context(&v.rust_context, out)?;
    Ok(())
}

/// Renders the `complexityJustified` block when set. Tells the AI agent
/// "this complex shape is covered by tests — leave it alone".
fn write_complexity_justified(v: &Violation, out: &mut dyn Write) -> Result<()> {
    let Some(j) = &v.complexity_justified else {
        return Ok(());
    };
    let basis = match j.by {
        crate::report::JustificationBasis::Line => "line",
        crate::report::JustificationBasis::Branch => "branch",
    };
    writeln!(out, "    complexityJustified:")?;
    writeln!(out, "      by: {basis}")?;
    writeln!(out, "      threshold: {}", format_number(j.threshold))?;
    writeln!(out, "      actual: {}", format_number(j.actual))?;
    Ok(())
}

fn write_rust_context(ctx: &crate::report::RustContext, out: &mut dyn Write) -> Result<()> {
    if ctx.is_empty() {
        return Ok(());
    }
    writeln!(out, "    rustContext:")?;
    write_rust_context_scalars(ctx, out)?;
    Ok(())
}

fn write_rust_context_scalars(ctx: &crate::report::RustContext, out: &mut dyn Write) -> Result<()> {
    write_optional_field(out, "      lifetimeArity", ctx.lifetime_arity)?;
    write_optional_field(out, "      genericArity", ctx.generic_arity)?;
    write_optional_field(out, "      panicSites", ctx.panic_sites)?;
    write_optional_field(out, "      unsafeBlocks", ctx.unsafe_blocks)?;
    Ok(())
}

fn write_optional_field(out: &mut dyn Write, label: &str, value: Option<f64>) -> Result<()> {
    let Some(v) = value else { return Ok(()) };
    writeln!(out, "{label}: {}", format_number(v))?;
    Ok(())
}

fn write_violation_core(v: &Violation, out: &mut dyn Write) -> Result<()> {
    // Every string value is run through `scalar_string` so the output
    // is *strict* YAML 1.2 — file paths with spaces, scopes with
    // colon-space, etc. all survive a YAML parser.
    write_violation_locator(v, out)?;
    write_violation_metric(v, out)?;
    Ok(())
}

fn write_violation_locator(v: &Violation, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "  - id: {}", scalar_string(&v.id))?;
    writeln!(out, "    file: {}", scalar_string(&v.file))?;
    writeln!(out, "    line: {}", v.line)?;
    writeln!(out, "    scope: {}", scalar_string(&v.scope))?;
    writeln!(out, "    scopeKind: {}", scope_kind_word(v.scope_kind))?;
    Ok(())
}

/// Renders `ScopeKind` as a kebab-case string. AI agents read this to
/// pick the right refactor framing — a `trait-method` violation means
/// reviewing the trait contract; a `free-function` violation only
/// touches the call sites.
fn scope_kind_word(kind: rustics::ScopeKind) -> &'static str {
    match kind {
        rustics::ScopeKind::FreeFunction => "free-function",
        rustics::ScopeKind::Method => "method",
        rustics::ScopeKind::TraitMethod => "trait-method",
        rustics::ScopeKind::Module => "module",
        rustics::ScopeKind::ImplBlock => "impl-block",
        rustics::ScopeKind::TraitDef => "trait-def",
    }
}

fn write_violation_metric(v: &Violation, out: &mut dyn Write) -> Result<()> {
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
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations: vec![],
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
            unused: vec![],
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
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations: vec![],
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
            unused: vec![],
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
                warnings_justified: 0,
                errors_justified: 0,
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

    #[test]
    fn no_auto_explain_suppresses_rationale_and_hints() {
        let v = Violation {
            id: "abc".into(),
            file: "x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "cyclomatic-complexity".into(),
            value: 25.0,
            threshold: 10.0,
            severity: rustics::MetricSeverity::Warning,
            rationale: Some("EXPENSIVE explanation that costs tokens".into()),
            refactor_hints: vec!["a hint".into()],
            references: vec!["a ref".into()],
            rust_context: Default::default(),
            complexity_justified: None,
        };
        let mut buf = Vec::new();
        // auto_explain=false, no per-metric override → suppress.
        write_one_violation(&v, &ReportOptions::default(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("explain:"), "rationale must be suppressed");
        assert!(!s.contains("refactorHints"));
        assert!(!s.contains("references"));
        // Violation core still present.
        assert!(s.contains("    metric: cyclomatic-complexity"));
    }

    #[test]
    fn explain_metrics_re_enables_inline_for_named_lens() {
        use std::collections::HashSet;
        let v = Violation {
            id: "abc".into(),
            file: "x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "panic-density".into(),
            value: 7.0,
            threshold: 5.0,
            severity: rustics::MetricSeverity::Warning,
            rationale: Some("panic explanation".into()),
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        };
        let mut explain_metrics = HashSet::new();
        explain_metrics.insert("panic-density".to_string());
        let opts = ReportOptions {
            auto_explain: false,
            explain_metrics,
        };
        let mut buf = Vec::new();
        write_one_violation(&v, &opts, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("panic explanation"));
    }

    #[test]
    fn severity_word_covers_each_variant() {
        // Reachable via `--from-clippy` for Info (clippy `note` level)
        // and via standard lens thresholds for Error/Warning. The AI
        // reporter promises a stable lower-case word per severity in
        // the contract — pin every arm.
        assert_eq!(severity_word(rustics::MetricSeverity::Error), "error");
        assert_eq!(severity_word(rustics::MetricSeverity::Warning), "warning");
        assert_eq!(severity_word(rustics::MetricSeverity::Info), "info");
    }

    #[test]
    fn complexity_justified_branch_basis_renders_word_branch() {
        // The lcov reader is line-only today, so the runtime path
        // never produces a Branch-basis justification. The type is
        // public though, so an embedder can construct one — the AI
        // reporter must format `by: branch` correctly.
        use crate::report::{ComplexityJustification, JustificationBasis};
        let v = Violation {
            id: "abc".into(),
            file: "x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "cyclomatic-complexity".into(),
            value: 25.0,
            threshold: 10.0,
            severity: rustics::MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: Some(ComplexityJustification {
                by: JustificationBasis::Branch,
                threshold: 0.80,
                actual: 0.85,
            }),
        };
        let mut buf = Vec::new();
        write_complexity_justified(&v, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("      by: branch"));
        assert!(s.contains("      threshold: 0.8"));
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

    fn report_with_unused(items: Vec<crate::unused::UnusedItem>) -> Report {
        Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 1,
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
            unused: items,
        }
    }

    #[test]
    fn unused_block_renders_each_item_with_optional_parent() {
        // The unify-analyze-unused branch threads `report.unused` into
        // the AI report as a top-level `unused:` block. Pin the YAML
        // shape both with and without `parent` since the latter omits
        // the line entirely (skip_serializing_if-equivalent on the
        // hand-rolled writer).
        let r = report_with_unused(vec![
            unused_item("variant", Some("Color")),
            unused_item("orphan", None),
        ]);
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("unused:"), "got: {s}");
        assert!(s.contains("    name: variant"), "got: {s}");
        assert!(s.contains("    parent: Color"), "got: {s}");
        assert!(s.contains("    name: orphan"), "got: {s}");
        // The bare-name item must not produce a parent line.
        let orphan_section = s.split("name: orphan").nth(1).unwrap_or("");
        let next_item_break = orphan_section
            .find("  - file:")
            .unwrap_or(orphan_section.len());
        assert!(
            !orphan_section[..next_item_break].contains("parent:"),
            "orphan item should have no parent line, got: {orphan_section}"
        );
    }

    #[test]
    fn summary_unused_count_appears_only_when_nonzero() {
        // The unify-analyze-unused branch added `summary.unused: N` —
        // present only when the count is > 0 so existing parsers don't
        // see a stray zero key on clean runs.
        let with = report_with_unused(vec![unused_item("orphan", None)]);
        let without = report_with_unused(vec![]);
        let mut buf_with = Vec::new();
        let mut buf_without = Vec::new();
        write(&with, &mut buf_with).unwrap();
        write(&without, &mut buf_without).unwrap();
        let s_with = String::from_utf8(buf_with).unwrap();
        let s_without = String::from_utf8(buf_without).unwrap();
        assert!(s_with.contains("  unused: 1"), "got: {s_with}");
        assert!(!s_without.contains("  unused:"), "got: {s_without}");
    }

    #[test]
    fn complexity_justified_block_renders_under_violation() {
        use crate::report::{ComplexityJustification, JustificationBasis};
        let v = Violation {
            id: "abc".into(),
            file: "x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "cyclomatic-complexity".into(),
            value: 25.0,
            threshold: 10.0,
            severity: rustics::MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: Some(ComplexityJustification {
                by: JustificationBasis::Line,
                threshold: 0.95,
                actual: 0.965,
            }),
        };
        let mut buf = Vec::new();
        write_one_violation(&v, &ai_default_options(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("    complexityJustified:"));
        assert!(s.contains("      by: line"));
        assert!(s.contains("      threshold: 0.95"));
        assert!(s.contains("      actual: 0.965"));
    }
}
