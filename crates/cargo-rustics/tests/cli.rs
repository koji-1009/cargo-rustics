//! End-to-end smoke tests that spawn the cargo-rustics binary as a
//! subprocess and pipe a fixture project at it. These are the //! integration story.
//!
//! The fixture is created on the fly in a unique tempdir so the tests
//! are hermetic and parallel-safe.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static SUFFIX: AtomicUsize = AtomicUsize::new(0);

fn unique_tempdir(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = SUFFIX.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rustics-it-{label}-{pid}-{n}"));
    fs::create_dir_all(&dir).expect("mkdir tempdir");
    dir
}

fn binary() -> PathBuf {
    // `CARGO_BIN_EXE_<name>` is set by cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_cargo-rustics"))
}

fn write_fixture(root: &Path) {
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "demo"
version = "0.0.1"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
pub fn calm() -> i32 { 1 }

// CC overshoots the default error threshold (20).
pub fn busy(x: i32) -> i32 {
    let mut acc = 0;
    if x > 0 { acc += 1; }
    if x > 1 { acc += 1; }
    if x > 2 { acc += 1; }
    if x > 3 { acc += 1; }
    if x > 4 { acc += 1; }
    if x > 5 { acc += 1; }
    if x > 6 { acc += 1; }
    if x > 7 { acc += 1; }
    if x > 8 { acc += 1; }
    if x > 9 { acc += 1; }
    if x > 10 { acc += 1; }
    if x > 11 { acc += 1; }
    if x > 12 { acc += 1; }
    if x > 13 { acc += 1; }
    if x > 14 { acc += 1; }
    if x > 15 { acc += 1; }
    if x > 16 { acc += 1; }
    if x > 17 { acc += 1; }
    if x > 18 { acc += 1; }
    if x > 19 { acc += 1; }
    if x > 20 { acc += 1; }
    if x > 21 { acc += 1; }
    if x > 22 { acc += 1; }
    if x > 23 { acc += 1; }
    if x > 24 { acc += 1; }
    acc
}
"#,
    )
    .unwrap();
}

#[test]
fn manual_subcommand_prints_heading() {
    let out = Command::new(binary())
        .arg("manual")
        .output()
        .expect("run binary");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("# cargo-rustics — operator's manual"));
}

#[test]
fn analyze_clean_fixture_exits_zero() {
    let root = unique_tempdir("clean");
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "demo"
version = "0.0.1"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn ok() -> i32 { 1 }\n").unwrap();

    let out = Command::new(binary())
        .args(["analyze", "--reporter", "console"])
        .current_dir(&root)
        .output()
        .expect("run analyze");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("clean"));
    fs::remove_dir_all(&root).ok();
}

#[test]
fn analyze_busy_fixture_emits_violation() {
    let root = unique_tempdir("busy");
    write_fixture(&root);

    let out = Command::new(binary())
        .args(["analyze", "--reporter", "ai"])
        .current_dir(&root)
        .output()
        .expect("run analyze");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("# rustics ai-report v1\n"));
    assert!(stdout.contains("metric: cyclomatic-complexity"));
    assert!(stdout.contains("severity: error"));
    fs::remove_dir_all(&root).ok();
}

#[test]
fn fatal_warnings_returns_nonzero_when_busy() {
    let root = unique_tempdir("fatal");
    write_fixture(&root);

    let out = Command::new(binary())
        .args(["analyze", "--fatal-warnings"])
        .current_dir(&root)
        .output()
        .expect("run analyze");
    assert!(!out.status.success(), "expected non-zero exit");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn cargo_plugin_token_is_accepted() {
    // When invoked as `cargo rustics manual`, cargo passes
    // ["cargo-rustics", "rustics", "manual"]. The binary must accept that.
    let out = Command::new(binary())
        .args(["rustics", "manual"])
        .output()
        .expect("run binary");
    assert!(out.status.success());
}

#[test]
fn rules_lists_cyclomatic_complexity() {
    let out = Command::new(binary())
        .args(["rules"])
        .output()
        .expect("run");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cyclomatic-complexity"));
    assert!(stdout.contains("rationale"));
}

#[test]
fn unknown_metric_id_is_rejected() {
    let out = Command::new(binary())
        .args(["analyze", "--metric", "does-not-exist"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown metric id"));
}
