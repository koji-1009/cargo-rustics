//! Golden tests for the reporter formats.
//!
//! Plan §12.3. Locking these formats in golden files prevents accidental
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
    "plan §2.5 — Type-system-aware adjustments (sealed-aware match).",
];

/// Builds the canonical fixture report shared by every golden test.
fn fixture_report() -> Report {
    let make_violation = |id: &str,
                          file: &str,
                          line: usize,
                          scope: &str,
                          kind: ScopeKind,
                          value: f64,
                          threshold: f64,
                          severity: MetricSeverity| {
        Violation {
            id: id.into(),
            file: file.into(),
            line,
            scope: scope.into(),
            scope_kind: kind,
            metric: "cyclomatic-complexity".into(),
            value,
            threshold,
            severity,
            rationale: Some(RATIONALE.to_string()),
            refactor_hints: REFACTOR_HINTS.iter().map(|s| s.to_string()).collect(),
            references: REFERENCES.iter().map(|s| s.to_string()).collect(),
            rust_context: Default::default(),
        }
    };
    Report {
        version: 1,
        generated_at: "2026-05-08T00:00:00Z".into(),
        summary: Summary {
            files_analyzed: 2,
            violations: 2,
            warnings: 1,
            errors: 1,
        },
        violations: vec![
            make_violation(
                "11112222aaaabbbb",
                "crates/demo/src/parser.rs",
                12,
                "parser::Parser::parse",
                ScopeKind::Method,
                25.0,
                20.0,
                MetricSeverity::Error,
            ),
            make_violation(
                "33334444ccccdddd",
                "crates/demo/src/lib.rs",
                4,
                "f",
                ScopeKind::FreeFunction,
                11.0,
                10.0,
                MetricSeverity::Warning,
            ),
        ],
        truncated: 0,
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
