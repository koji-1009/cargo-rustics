//! Clippy bridge — ingest `cargo clippy --message-format=json` output as
//! supplementary lens signal.
//!
//! Plan §5.7 + §10.1. Clippy is a *lint* surface (rule violation:
//! "this is wrong") while rustics's lenses are *quantitative*
//! (dimensions: "this is N borrows deep"). The bridge does not
//! replace Clippy — it imports each warning as one more violation,
//! giving the AI report a single place where both surfaces co-exist.
//!
//! M2 scope: read the JSON, surface each clippy lint as a `Violation`
//! whose metric id is `clippy::<lint>`. Scope path is `<file>:<line>`
//! (no AST mapping at M2 — that lands when M3's rust-analyzer
//! integration ships).

use std::path::Path;

use anyhow::{Context, Result};
use rustics::{violation_id, MetricSeverity, ScopeKind, ScopeRef};
use serde::Deserialize;

use crate::report::Violation;

/// Reads Clippy's `--message-format=json` output from `path` and
/// returns one [`Violation`] per `clippy::<lint>` warning / error.
pub fn load(path: &Path) -> Result<Vec<Violation>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read clippy json {}", path.display()))?;
    Ok(parse_stream(&raw))
}

/// Parses the multi-line clippy stream into violations. Public so
/// tests can drive it without disk I/O.
pub fn parse_stream(stream: &str) -> Vec<Violation> {
    let mut out = Vec::new();
    for line in stream.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(envelope) = serde_json::from_str::<Envelope>(line) else {
            continue;
        };
        if envelope.reason != "compiler-message" {
            continue;
        }
        let Some(msg) = envelope.message else {
            continue;
        };
        if let Some(v) = message_to_violation(&msg) {
            out.push(v);
        }
    }
    out
}

#[derive(Debug, Deserialize)]
struct Envelope {
    reason: String,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    message: String,
    level: String,
    code: Option<MessageCode>,
    spans: Vec<MessageSpan>,
}

#[derive(Debug, Deserialize)]
struct MessageCode {
    code: String,
}

#[derive(Debug, Deserialize)]
struct MessageSpan {
    file_name: String,
    line_start: usize,
    is_primary: bool,
}

fn message_to_violation(msg: &Message) -> Option<Violation> {
    let code = msg.code.as_ref()?.code.clone();
    if !code.starts_with("clippy::") {
        // Plain rustc warnings come through with no `clippy::` prefix; we
        // could surface them later but for M2 we limit to clippy lints.
        return None;
    }
    let span = primary_span(&msg.spans)?;
    let file = span.file_name.clone();
    let line = span.line_start;
    let scope = format!("{file}:{line}");
    let id = violation_id(&file, &scope, &code);
    Some(Violation {
        id,
        file,
        line,
        scope: scope.clone(),
        scope_kind: ScopeKind::Module,
        metric: code,
        value: 1.0,
        threshold: 0.0,
        severity: severity_from_level(&msg.level),
        rationale: Some(msg.message.clone()),
        refactor_hints: vec![],
        references: vec!["Clippy lint — `cargo clippy --explain <code>`".into()],
    })
}

fn primary_span(spans: &[MessageSpan]) -> Option<&MessageSpan> {
    spans
        .iter()
        .find(|s| s.is_primary)
        .or_else(|| spans.first())
}

fn severity_from_level(level: &str) -> MetricSeverity {
    match level {
        "error" => MetricSeverity::Error,
        "warning" => MetricSeverity::Warning,
        _ => MetricSeverity::Info,
    }
}

/// Helper to satisfy unused-import warnings when this module compiles
/// without consumers using `ScopeRef` directly.
#[doc(hidden)]
pub fn _scope_ref(path: &str, line: usize) -> ScopeRef {
    ScopeRef::new(path.to_string(), ScopeKind::Module, line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(reason: &str, code: &str, level: &str, file: &str, line: usize) -> String {
        let body = format!(
            r#"{{"reason":"{reason}","message":{{"message":"oops","level":"{level}","code":{{"code":"{code}"}},"spans":[{{"file_name":"{file}","line_start":{line},"is_primary":true}}]}}}}"#
        );
        body
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse_stream("").is_empty());
    }

    #[test]
    fn non_message_envelope_is_ignored() {
        let s = r#"{"reason":"build-finished","success":true}"#;
        assert!(parse_stream(s).is_empty());
    }

    #[test]
    fn clippy_warning_becomes_violation() {
        let s = line(
            "compiler-message",
            "clippy::needless_borrow",
            "warning",
            "src/x.rs",
            12,
        );
        let v = parse_stream(&s);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].metric, "clippy::needless_borrow");
        assert_eq!(v[0].file, "src/x.rs");
        assert_eq!(v[0].line, 12);
        assert_eq!(v[0].severity, MetricSeverity::Warning);
    }

    #[test]
    fn rustc_warnings_are_ignored() {
        // No `clippy::` prefix -> dropped.
        let s = line(
            "compiler-message",
            "unused_variables",
            "warning",
            "src/x.rs",
            1,
        );
        assert!(parse_stream(&s).is_empty());
    }

    #[test]
    fn error_level_becomes_error_severity() {
        let s = line(
            "compiler-message",
            "clippy::correctness",
            "error",
            "src/x.rs",
            1,
        );
        let v = parse_stream(&s);
        assert_eq!(v[0].severity, MetricSeverity::Error);
    }

    #[test]
    fn invalid_json_lines_are_skipped() {
        let s = "not json\n{\"reason\":\"build-script-executed\"}\n";
        assert!(parse_stream(s).is_empty());
    }
}
