//! `--since <ref>` filter — keep only violations in files changed vs a
//! git reference.
//!
//! Plan §7.2. The whole-workspace analysis is preserved (cross-file
//! signals stay accurate); the filter applies only to the report
//! consumer surface.
//!
//! Implementation calls `git diff --name-only --diff-filter=ACMRT
//! <ref>...HEAD` as a subprocess. We avoid pulling in `gix` here; the
//! plan §11.3 reserves `gix` for the regression command's tree
//! resolution and our needs are simpler.
//!
//! `<ref>` can be any reference git understands: `main`, `HEAD~1`,
//! `origin/main`, a SHA, …

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::report::Violation;

/// Returns the set of paths (workspace-relative, `/` separators) that
/// changed between `git_ref` and `HEAD`. Empty set means "no changes".
pub fn changed_files(git_ref: &str, workspace_root: &Path) -> Result<HashSet<String>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=ACMRT"])
        .arg(format!("{git_ref}...HEAD"))
        .current_dir(workspace_root)
        .output()
        .with_context(|| format!("invoke git diff against {git_ref}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git diff failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut set = HashSet::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        set.insert(line.to_string());
    }
    Ok(set)
}

/// Filters `violations` in place, keeping only entries whose `file`
/// path is in `changed`. Workspace-relative paths use `/` separators
/// on every platform, matching what `git diff --name-only` emits.
pub fn filter(violations: &mut Vec<Violation>, changed: &HashSet<String>) {
    violations.retain(|v| changed.contains(&v.file));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Violation;
    use rustics::{MetricSeverity, ScopeKind};

    fn v(file: &str) -> Violation {
        Violation {
            id: "abc".into(),
            file: file.into(),
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
            rust_context: Default::default(),
        }
    }

    #[test]
    fn keeps_only_changed_files() {
        let mut violations = vec![v("a.rs"), v("b.rs"), v("c.rs")];
        let changed: HashSet<String> = ["a.rs", "c.rs"].iter().map(|s| s.to_string()).collect();
        filter(&mut violations, &changed);
        let names: Vec<_> = violations.iter().map(|v| v.file.clone()).collect();
        assert_eq!(names, vec!["a.rs", "c.rs"]);
    }

    #[test]
    fn empty_changed_set_filters_everything() {
        let mut violations = vec![v("a.rs")];
        filter(&mut violations, &HashSet::new());
        assert!(violations.is_empty());
    }
}
