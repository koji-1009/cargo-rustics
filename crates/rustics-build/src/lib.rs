//! `rustics-build` — `build.rs` integration for the rustics lens
//! catalogue.
//!
//! Plan §5.6 / M3 task #51. Drop a [`Gate`] into `build.rs` and the
//! build fails when any configured threshold is crossed:
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     rustics_build::Gate::new()
//!         .threshold("cyclomatic-complexity", 15)
//!         .threshold("clone-density", 8)
//!         .source_root("src")
//!         .run();
//! }
//! ```
//!
//! `Gate::run` panics on threshold violations; build scripts treat
//! panics as build failures, so the gate halts compilation with the
//! offending function/scope/value in the error message.
//!
//! The crate is intentionally thin — it shares the `rustics` library
//! with `cargo-rustics` so the lens definitions stay single-source.
//! The `cargo` plugin is the rich, full-featured surface; this crate
//! is for projects that want the gate baked into every build without
//! needing a CI step.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rustics::{builtin_metrics, MetricCalculator, MetricInput};

/// Builder-style configuration for a `build.rs` lens gate.
///
/// Default: every workspace `.rs` file under `<crate>/src/` is walked,
/// every built-in lens runs, no thresholds are configured (so nothing
/// fails). Add thresholds with [`Gate::threshold`].
pub struct Gate {
    source_root: PathBuf,
    thresholds: HashMap<String, f64>,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    /// Constructs a gate rooted at the calling crate's `src/`
    /// directory. `build.rs` runs from `CARGO_MANIFEST_DIR`, so
    /// `src` is the conventional location.
    pub fn new() -> Self {
        Self {
            source_root: PathBuf::from("src"),
            thresholds: HashMap::new(),
        }
    }

    /// Overrides the source root that the gate walks. Relative paths
    /// are resolved from `CARGO_MANIFEST_DIR`.
    pub fn source_root<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.source_root = path.as_ref().to_path_buf();
        self
    }

    /// Adds (or replaces) a metric threshold. Crossing it fails the
    /// build. Metric ids are kebab-case as listed by `cargo rustics
    /// rules`.
    pub fn threshold(mut self, metric_id: &str, value: f64) -> Self {
        self.thresholds.insert(metric_id.to_string(), value);
        self
    }

    /// Runs the gate. Panics with a build-script-friendly message if
    /// any threshold is crossed. Intended for `build.rs` callers.
    pub fn run(self) {
        if let Err(failures) = self.try_run() {
            panic!("{}", format_failures(&failures));
        }
    }

    /// Same as [`Gate::run`] but returns the failure list instead of
    /// panicking. Useful for tests / programmatic use.
    pub fn try_run(self) -> Result<(), Vec<GateFailure>> {
        let metrics = builtin_metrics();
        let files = walk_rust_files(&self.source_root);
        let mut failures = Vec::new();
        for file in &files {
            check_one_file(file, &metrics, &self.thresholds, &mut failures);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }
}

/// One threshold breach captured by [`Gate::try_run`].
#[derive(Debug, Clone)]
pub struct GateFailure {
    /// Source file containing the offending scope.
    pub file: PathBuf,
    /// Scope path (`module::Type::method`).
    pub scope: String,
    /// Lens id (kebab-case).
    pub metric: String,
    /// Measured value.
    pub value: f64,
    /// Threshold the value crossed.
    pub threshold: f64,
}

fn check_one_file(
    file: &Path,
    metrics: &[Box<dyn MetricCalculator>],
    thresholds: &HashMap<String, f64>,
    out: &mut Vec<GateFailure>,
) {
    let Ok(source) = std::fs::read_to_string(file) else {
        return;
    };
    let Ok(ast) = syn::parse_file(&source) else {
        return;
    };
    let input = MetricInput::new(file, &source, &ast);
    for metric in metrics {
        let Some(threshold) = thresholds.get(metric.id()) else {
            continue;
        };
        for measurement in metric.measure(&input) {
            if measurement.value > *threshold {
                out.push(GateFailure {
                    file: file.to_path_buf(),
                    scope: measurement.scope.path,
                    metric: metric.id().to_string(),
                    value: measurement.value,
                    threshold: *threshold,
                });
            }
        }
    }
}

fn walk_rust_files(root: &Path) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        out.push(path.to_path_buf());
    }
    out.sort();
    out
}

fn format_failures(failures: &[GateFailure]) -> String {
    let mut s = format!("rustics-build: {} threshold breach(es):\n", failures.len());
    for f in failures {
        s.push_str(&format!(
            "  {metric}  {scope}  {value} (>{threshold}) — {file}\n",
            metric = f.metric,
            scope = f.scope,
            value = f.value,
            threshold = f.threshold,
            file = f.file.display(),
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SUFFIX: AtomicUsize = AtomicUsize::new(0);

    fn unique_tempdir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = SUFFIX.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rustics-build-{label}-{pid}-{n}"));
        fs::create_dir_all(&dir).expect("mkdir tempdir");
        dir
    }

    #[test]
    fn no_thresholds_passes() {
        let dir = unique_tempdir("notr");
        fs::write(
            dir.join("a.rs"),
            "fn f(x: i32) -> i32 { if x > 0 { 1 } else { 0 } }",
        )
        .unwrap();
        let result = Gate::new().source_root(&dir).try_run();
        assert!(result.is_ok());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cc_threshold_zero_catches_any_branch() {
        let dir = unique_tempdir("cc");
        fs::write(
            dir.join("a.rs"),
            "fn f(x: i32) -> i32 { if x > 0 { 1 } else { 0 } }",
        )
        .unwrap();
        let result = Gate::new()
            .source_root(&dir)
            .threshold("cyclomatic-complexity", 1.0)
            .try_run();
        match result {
            Ok(_) => panic!("expected gate failure"),
            Err(fs) => {
                assert!(fs.iter().any(|f| f.metric == "cyclomatic-complexity"));
            }
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_metric_id_is_silently_skipped() {
        let dir = unique_tempdir("unknown");
        fs::write(dir.join("a.rs"), "fn f() {}").unwrap();
        let result = Gate::new()
            .source_root(&dir)
            .threshold("does-not-exist", 5.0)
            .try_run();
        assert!(result.is_ok());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_source_root_does_not_panic() {
        let result = Gate::new()
            .source_root("/no/such/dir")
            .threshold("cyclomatic-complexity", 1.0)
            .try_run();
        assert!(result.is_ok()); // No files → no failures.
    }
}
