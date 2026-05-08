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
fn regression_diffs_snapshots() {
    use std::fs;
    let fixture = workspace_root().join("tests/projects/small-cli");
    let tmp = std::env::temp_dir().join(format!("rustics-regr-{}", std::process::id()));
    fs::create_dir_all(&tmp).expect("mkdir tmp");

    // before: empty snapshot.
    let before_json = r#"{"version":1,"generatedAt":"T","summary":{"filesAnalyzed":0,"violations":0,"warnings":0,"errors":0},"violations":[]}"#;
    let before_path = tmp.join("before.json");
    fs::write(&before_path, before_json).unwrap();

    // after: real analyze on the small-cli fixture, captures the
    // current violation list.
    let after_path = tmp.join("after.json");
    let after_out = Command::new(binary())
        .args(["analyze", "--reporter", "json"])
        .current_dir(&fixture)
        .output()
        .expect("analyze");
    fs::write(&after_path, after_out.stdout).unwrap();

    // Empty -> populated should be a regression.
    let regr = Command::new(binary())
        .args([
            "regression",
            "--before",
            before_path.to_str().unwrap(),
            "--after",
            after_path.to_str().unwrap(),
            "--reporter",
            "json",
        ])
        .output()
        .expect("regression");
    assert!(
        regr.status.success(),
        "regression should exit 0 without --fatal-regressions"
    );
    let stdout = String::from_utf8_lossy(&regr.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(report["verdict"], "regressed");
    assert!(report["diff"]["regressed"].as_u64().unwrap() > 0);

    // Same fatal flag should fail.
    let regr_fatal = Command::new(binary())
        .args([
            "regression",
            "--before",
            before_path.to_str().unwrap(),
            "--after",
            after_path.to_str().unwrap(),
            "--fatal-regressions",
        ])
        .output()
        .expect("regression");
    assert!(
        !regr_fatal.status.success(),
        "expected non-zero under --fatal-regressions"
    );

    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn explain_round_trips_through_id() {
    use std::fs;
    let fixture = workspace_root().join("tests/projects/small-cli");
    let tmp = std::env::temp_dir().join(format!("rustics-expl-{}", std::process::id()));
    fs::create_dir_all(&tmp).expect("mkdir tmp");

    // analyze the fixture and grab a real violation id.
    let snap = tmp.join("snap.json");
    let analyze_out = Command::new(binary())
        .args(["analyze", "--reporter", "json"])
        .current_dir(&fixture)
        .output()
        .expect("analyze");
    fs::write(&snap, &analyze_out.stdout).unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&analyze_out.stdout).expect("valid JSON");
    let id = report["violations"][0]["id"]
        .as_str()
        .expect("at least one violation in fixture");

    // explain by snapshot path.
    let out = Command::new(binary())
        .args(["explain", id, "--snapshot", snap.to_str().unwrap()])
        .output()
        .expect("explain");
    assert!(out.status.success(), "explain should succeed for known id");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# rustics explain v1"));
    assert!(stdout.contains(&format!("id: {id}")));
    assert!(stdout.contains("rationale: |"));

    // unknown id should fail.
    let out_unknown = Command::new(binary())
        .args([
            "explain",
            "0000000000000000",
            "--snapshot",
            snap.to_str().unwrap(),
        ])
        .output()
        .expect("explain");
    assert!(!out_unknown.status.success());

    fs::remove_dir_all(&tmp).ok();
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
