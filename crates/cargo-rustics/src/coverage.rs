//! Coverage gating — plan §4.3 / §7.2.
//!
//! Reads an `lcov.info` file and attaches per-file line coverage to
//! every violation. The richer per-function and per-branch coverage
//! requires the metric library to expose body line ranges; that lands
//! when the M3 rust-analyzer integration arrives. For M2 we surface
//! the file-level fraction so the AI report can hint "this violation
//! is in a 20%-covered file" without claiming function-level precision.
//!
//! `--coverage` resolution order:
//!
//! 1. Explicit `--coverage <path>`.
//! 2. `target/coverage/lcov.info` if it exists (idiomatic
//!    `cargo-llvm-cov` output).
//! 3. None — violations carry no coverage hint.
//!
//! `--coverage none` forces step 3.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::report::Violation;

/// Per-file coverage totals.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileCoverage {
    /// Total lines instrumented (`LF:` in lcov).
    pub total: u32,
    /// Lines hit at least once (`LH:` in lcov).
    pub hit: u32,
}

impl FileCoverage {
    /// Returns `hit / total`, or `None` if there are no instrumented
    /// lines.
    pub fn ratio(&self) -> Option<f64> {
        if self.total == 0 {
            return None;
        }
        Some(f64::from(self.hit) / f64::from(self.total))
    }
}

/// Resolved coverage index: `file -> totals`.
#[derive(Debug, Clone, Default)]
pub struct CoverageIndex {
    files: HashMap<String, FileCoverage>,
}

impl CoverageIndex {
    /// Returns the coverage row for a file, if known.
    pub fn for_file(&self, file: &str) -> Option<FileCoverage> {
        self.files.get(file).copied()
    }
}

/// Resolves the lcov path from CLI input + workspace conventions.
///
/// `explicit` is the value of `--coverage`. `"none"` (case-insensitive)
/// disables coverage. Empty / `None` falls back to
/// `<workspace>/target/coverage/lcov.info` if present.
pub fn resolve_path(explicit: Option<&str>, workspace_root: &Path) -> Option<PathBuf> {
    if let Some(s) = explicit {
        if s.eq_ignore_ascii_case("none") {
            return None;
        }
        return Some(PathBuf::from(s));
    }
    let default = workspace_root.join("target/coverage/lcov.info");
    if default.is_file() {
        Some(default)
    } else {
        None
    }
}

/// Parses an lcov.info file at `path` into a [`CoverageIndex`].
pub fn load(path: &Path) -> Result<CoverageIndex> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("read lcov {}", path.display()))?;
    Ok(parse(&raw))
}

/// Pure parser for the lcov record stream. Public so unit tests can
/// drive it without disk I/O.
pub fn parse(text: &str) -> CoverageIndex {
    let mut files = HashMap::new();
    let mut current_file: Option<String> = None;
    let mut current = FileCoverage::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SF:") {
            // Reset for new file. Persist any in-flight totals first.
            commit_current(&mut files, &mut current_file, &current);
            current_file = Some(rest.to_string());
            current = FileCoverage::default();
        } else if let Some(rest) = line.strip_prefix("LF:") {
            current.total = rest.parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("LH:") {
            current.hit = rest.parse().unwrap_or(0);
        } else if line == "end_of_record" {
            commit_current(&mut files, &mut current_file, &current);
            current_file = None;
            current = FileCoverage::default();
        }
    }
    commit_current(&mut files, &mut current_file, &current);
    CoverageIndex { files }
}

fn commit_current(
    files: &mut HashMap<String, FileCoverage>,
    current_file: &mut Option<String>,
    current: &FileCoverage,
) {
    if let Some(f) = current_file.take() {
        files.insert(f, *current);
    }
}

/// Attaches the file-level line-coverage ratio to every violation as
/// the `coverage` field (in `[0.0, 1.0]`). Violations whose file is
/// not in the index are left unchanged.
pub fn attach(report_violations: &mut [Violation], index: &CoverageIndex) {
    for v in report_violations.iter_mut() {
        let Some(cov) = index.for_file(&v.file).and_then(|c| c.ratio()) else {
            continue;
        };
        // Coverage is rendered through the existing `rationale`-adjacent
        // path: prepend a `coverage: <pct>` note. A first-class field on
        // `Violation` arrives with the rustContext block (M2 task #44).
        let note = format!("Coverage on this file: {:.1}%.", cov * 100.0);
        match &mut v.rationale {
            Some(existing) => existing.push_str(&format!("\n{note}")),
            None => v.rationale = Some(note),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_record() {
        let lcov = "SF:src/x.rs\nLF:10\nLH:7\nend_of_record\n";
        let idx = parse(lcov);
        let cov = idx.for_file("src/x.rs").unwrap();
        assert_eq!(cov.total, 10);
        assert_eq!(cov.hit, 7);
        assert_eq!(cov.ratio(), Some(0.7));
    }

    #[test]
    fn parse_multiple_records() {
        let lcov = concat!(
            "SF:src/a.rs\nLF:10\nLH:5\nend_of_record\n",
            "SF:src/b.rs\nLF:20\nLH:20\nend_of_record\n",
        );
        let idx = parse(lcov);
        assert_eq!(idx.for_file("src/a.rs").unwrap().ratio(), Some(0.5));
        assert_eq!(idx.for_file("src/b.rs").unwrap().ratio(), Some(1.0));
    }

    #[test]
    fn ratio_zero_total_is_none() {
        let cov = FileCoverage { total: 0, hit: 0 };
        assert_eq!(cov.ratio(), None);
    }

    #[test]
    fn resolve_path_none_disables() {
        let p = resolve_path(Some("none"), Path::new("/ws"));
        assert!(p.is_none());
    }

    #[test]
    fn resolve_path_returns_explicit() {
        let p = resolve_path(Some("/etc/lcov.info"), Path::new("/ws"));
        assert_eq!(p, Some(PathBuf::from("/etc/lcov.info")));
    }

    #[test]
    fn attach_writes_into_rationale() {
        use crate::report::Violation;
        use rustics::{MetricSeverity, ScopeKind};
        let mut violations = vec![Violation {
            id: "abc".into(),
            file: "src/x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "cyclomatic-complexity".into(),
            value: 11.0,
            threshold: 10.0,
            severity: MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
        }];
        let mut files = HashMap::new();
        files.insert("src/x.rs".to_string(), FileCoverage { total: 10, hit: 5 });
        let index = CoverageIndex { files };
        attach(&mut violations, &index);
        let r = violations[0].rationale.as_ref().unwrap();
        assert!(r.contains("Coverage on this file: 50.0%"));
    }
}
