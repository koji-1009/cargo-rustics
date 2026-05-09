//! Golden tests for the reporter formats.
//!
//! Locking these formats in golden files prevents accidental
//! breakage of the AI-report contract — every change to a reporter has
//! to land *with* an updated golden file, which makes the change visible
//! in code review.
//!
//! Goldens live at the workspace root under `tests/golden/<reporter>/`.
//! Each test reads its golden file by absolute path (`CARGO_MANIFEST_DIR`
//! plus a relative offset) so the test runs identically from `cargo
//! test` at the workspace root or from the crate directory.

#![cfg(test)]

use std::path::PathBuf;

use rustics::{MetricSeverity, ScopeKind};

use crate::report::{Report, Summary, Violation};
use crate::reporters;

/// Resolves a path relative to the workspace root from inside the
/// `crates/cargo-rustics/` crate directory.
fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const RATIONALE: &str = "Cyclomatic Complexity counts the linearly independent paths through a function body. Higher values correlate with branching density, which raises the cognitive load of reading the function and the test combinatorics needed to cover it. The Rust adjustment (sealed-aware) keeps `match` on enums from being penalised when the compiler is already checking exhaustiveness — the cognitive risk that CC was designed to flag (a missed case) does not exist there.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract one branch arm into a helper function so the surrounding control flow stays readable.",
    "Replace nested `if`/`else` chains with a single `match` on a small enum when possible — the sealed-aware rule then absorbs the branches.",
    "Lift early-return guard clauses to the top with `let ... else { return ... }` so the happy path stays on the function's main spine.",
    "Split a god-function into a state machine: each state becomes its own small function with a low CC.",
];

const REFERENCES: &[&str] = &[
    "McCabe, T. J. (1976). A Complexity Measure. IEEE Trans. Softw. Eng. SE-2(4).",
];

struct Spec {
    id: &'static str,
    file: &'static str,
    line: usize,
    scope: &'static str,
    kind: ScopeKind,
    value: f64,
    threshold: f64,
    severity: MetricSeverity,
}

fn fixture_violation(spec: Spec) -> Violation {
    Violation {
        id: spec.id.into(),
        file: spec.file.into(),
        line: spec.line,
        scope: spec.scope.into(),
        scope_kind: spec.kind,
        metric: "cyclomatic-complexity".into(),
        value: spec.value,
        threshold: spec.threshold,
        severity: spec.severity,
        rationale: Some(RATIONALE.to_string()),
        refactor_hints: REFACTOR_HINTS.iter().map(|s| s.to_string()).collect(),
        references: REFERENCES.iter().map(|s| s.to_string()).collect(),
        rust_context: Default::default(),
        complexity_justified: None,
    }
}

const FIXTURE_VIOLATIONS: &[Spec] = &[
    Spec {
        id: "11112222aaaabbbb",
        file: "crates/demo/src/parser.rs",
        line: 12,
        scope: "parser::Parser::parse",
        kind: ScopeKind::Method,
        value: 25.0,
        threshold: 20.0,
        severity: MetricSeverity::Error,
    },
    Spec {
        id: "33334444ccccdddd",
        file: "crates/demo/src/lib.rs",
        line: 4,
        scope: "f",
        kind: ScopeKind::FreeFunction,
        value: 11.0,
        threshold: 10.0,
        severity: MetricSeverity::Warning,
    },
];

/// Builds the canonical fixture report shared by every golden test.
fn fixture_report() -> Report {
    Report {
        version: 1,
        generated_at: "2026-05-08T00:00:00Z".into(),
        summary: Summary {
            files_analyzed: 2,
            violations: 2,
            warnings: 1,
            errors: 1,
            warnings_justified: 0,
            errors_justified: 0,
        },
        violations: FIXTURE_VIOLATIONS
            .iter()
            .map(|s| {
                fixture_violation(Spec {
                    id: s.id,
                    file: s.file,
                    line: s.line,
                    scope: s.scope,
                    kind: s.kind,
                    value: s.value,
                    threshold: s.threshold,
                    severity: s.severity,
                })
            })
            .collect(),
        truncated: 0,
        measurements: vec![],
        stale_dismissals: vec![],
    }
}

#[test]
fn ai_reporter_matches_golden() {
    let report = fixture_report();
    let mut buf = Vec::new();
    reporters::ai::write(&report, &mut buf).unwrap();
    let actual = String::from_utf8(buf).unwrap();
    let expected = std::fs::read_to_string(workspace_path(
        "tests/golden/ai_reporter/with_violations.expected.yaml",
    ))
    .expect("read ai golden");
    assert_eq!(
        actual, expected,
        "AI reporter output drifted from golden — regenerate the golden \
         after reviewing the diff:\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

#[test]
fn json_reporter_matches_golden() {
    let report = fixture_report();
    let mut buf = Vec::new();
    reporters::json::write(&report, &mut buf).unwrap();
    let actual = String::from_utf8(buf).unwrap();
    let expected = std::fs::read_to_string(workspace_path(
        "tests/golden/json_reporter/with_violations.expected.json",
    ))
    .expect("read json golden");
    assert_eq!(
        actual, expected,
        "JSON reporter output drifted from golden — regenerate the golden \
         after reviewing the diff:\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}
