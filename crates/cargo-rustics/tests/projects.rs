//! Integration tests that run cargo-rustics against hand-crafted
//! fixture projects under `tests/projects/`. Plan §12.4.
//!
//! Each fixture is designed to fire a *specific* set of lenses so the
//! tests can assert "at least N violations across at least M distinct
//! metrics". The fixtures are kept small (single digit files each) and
//! checked into the repo so the test environment is fully hermetic.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cargo-rustics"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

#[test]
fn small_cli_fixture_produces_violations() {
    let fixture = workspace_root().join("tests/projects/small-cli");
    assert!(fixture.is_dir(), "fixture missing at {}", fixture.display());

    let out = Command::new(binary())
        .args(["analyze", "--reporter", "json"])
        .current_dir(&fixture)
        .output()
        .expect("run analyze");
    assert!(
        out.status.success(),
        "analyze should exit 0 without --fatal-warnings; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("analyze should produce valid JSON");

    assert_eq!(report["version"], 1);
    let violations = report["violations"].as_array().expect("violations array");
    assert!(
        !violations.is_empty(),
        "small-cli fixture should emit at least one violation"
    );

    // Every violation must carry the AI-report contract surface fields.
    for v in violations {
        for field in [
            "id",
            "file",
            "line",
            "scope",
            "scopeKind",
            "metric",
            "value",
            "threshold",
            "severity",
        ] {
            assert!(
                v.get(field).is_some(),
                "violation missing `{field}` field: {v}"
            );
        }
    }

    // Each fixture file targets a different lens; we expect at least
    // three distinct metrics to fire.
    let mut metrics: Vec<&str> = violations
        .iter()
        .filter_map(|v| v.get("metric").and_then(|m| m.as_str()))
        .collect();
    metrics.sort_unstable();
    metrics.dedup();
    assert!(
        metrics.len() >= 3,
        "expected ≥3 distinct metrics to fire, got {}: {:?}",
        metrics.len(),
        metrics
    );
}

#[test]
fn small_cli_fatal_warnings_returns_nonzero() {
    let fixture = workspace_root().join("tests/projects/small-cli");
    let out = Command::new(binary())
        .args(["analyze", "--fatal-warnings"])
        .current_dir(&fixture)
        .output()
        .expect("run analyze");
    assert!(
        !out.status.success(),
        "small-cli fixture should fail under --fatal-warnings"
    );
}

#[test]
fn small_cli_filtered_by_metric() {
    // `--metric cyclomatic-complexity` should narrow the report to
    // exactly that lens's violations.
    let fixture = workspace_root().join("tests/projects/small-cli");
    let out = Command::new(binary())
        .args([
            "analyze",
            "--reporter",
            "json",
            "--metric",
            "cyclomatic-complexity",
        ])
        .current_dir(&fixture)
        .output()
        .expect("run analyze");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let violations = report["violations"].as_array().expect("violations array");
    for v in violations {
        assert_eq!(v["metric"], "cyclomatic-complexity");
    }
}
