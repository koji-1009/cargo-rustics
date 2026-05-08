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
use rustics::{violation_id, MetricSeverity, ScopeKind};
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
        rust_context: crate::report::RustContext::default(),
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

    #[test]
    fn message_without_code_is_dropped() {
        // No `code` field on the message → we cannot tell if it's clippy
        // or rustc; drop it.
        let s = r#"{"reason":"compiler-message","message":{"message":"x","level":"warning","code":null,"spans":[]}}"#;
        assert!(parse_stream(s).is_empty());
    }

    #[test]
    fn message_without_spans_is_dropped() {
        let s = r#"{"reason":"compiler-message","message":{"message":"x","level":"warning","code":{"code":"clippy::abc"},"spans":[]}}"#;
        assert!(parse_stream(s).is_empty());
    }

    #[test]
    fn message_falls_back_to_first_span_when_none_primary() {
        // is_primary=false on the first (and only) span — the helper
        // falls back to spans[0].
        let s = r#"{"reason":"compiler-message","message":{"message":"x","level":"warning","code":{"code":"clippy::abc"},"spans":[{"file_name":"y.rs","line_start":3,"is_primary":false}]}}"#;
        let v = parse_stream(s);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].file, "y.rs");
        assert_eq!(v[0].line, 3);
    }

    #[test]
    fn unknown_level_maps_to_info_severity() {
        let s = line(
            "compiler-message",
            "clippy::abc",
            "note",
            "src/x.rs",
            1,
        );
        let v = parse_stream(&s);
        assert_eq!(v[0].severity, MetricSeverity::Info);
    }

    #[test]
    fn empty_message_is_dropped() {
        // No message field at all.
        let s = r#"{"reason":"compiler-message"}"#;
        assert!(parse_stream(s).is_empty());
    }

    #[test]
    fn primary_span_picks_primary_when_multiple() {
        let spans = vec![
            MessageSpan {
                file_name: "a.rs".into(),
                line_start: 1,
                is_primary: false,
            },
            MessageSpan {
                file_name: "b.rs".into(),
                line_start: 2,
                is_primary: true,
            },
        ];
        let s = primary_span(&spans).expect("primary");
        assert_eq!(s.file_name, "b.rs");
    }

    #[test]
    fn primary_span_returns_none_for_empty() {
        assert!(primary_span(&[]).is_none());
    }

    #[test]
    fn load_reads_file_into_violations() {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustics-clippy-{pid}-{n}.json"));
        let body = line(
            "compiler-message",
            "clippy::needless_borrow",
            "warning",
            "src/x.rs",
            5,
        );
        std::fs::write(&path, &body).unwrap();
        let v = load(&path).unwrap();
        assert_eq!(v.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_errors_when_file_missing() {
        let err =
            load(Path::new("/no/such/__rustics_clippy_test__.json")).unwrap_err();
        assert!(format!("{err:#}").contains("read clippy json"));
    }
}
