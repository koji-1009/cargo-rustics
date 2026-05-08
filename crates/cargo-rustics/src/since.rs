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

    /// Tiny tempdir helper, identical to `unused.rs::tests::tempdir`.
    fn tempdir() -> TempDir {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustics-since-test-{pid}-{n}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        // Split the builder chain into two segments so the
        // iterator-chain-length lens stays under its 6-link threshold —
        // we just dogfooded this on ourselves.
        let mut cmd = std::process::Command::new("git");
        with_test_identity(cmd.args(args).current_dir(dir));
        let st = cmd.output().expect("git invoke");
        assert!(
            st.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&st.stderr)
        );
    }

    /// Stamps deterministic author/committer identities on `cmd` so the
    /// test's commits don't depend on the host's git config (which may
    /// not have a name set, or may require gpg signing).
    fn with_test_identity(cmd: &mut std::process::Command) {
        cmd.env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "t@t");
    }

    fn init_repo() -> TempDir {
        let tmp = tempdir();
        run_git(tmp.path(), &["init", "-q", "-b", "main"]);
        run_git(tmp.path(), &["config", "commit.gpgsign", "false"]);
        run_git(tmp.path(), &["config", "tag.gpgsign", "false"]);
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("b.rs"), "fn b() {}").unwrap();
        run_git(tmp.path(), &["add", "."]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);
        tmp
    }

    #[test]
    fn changed_files_lists_modified_paths() {
        let tmp = init_repo();
        // tag the initial commit so we have a stable reference to diff
        // against, then mutate one file and stage it.
        run_git(tmp.path(), &["tag", "base"]);
        std::fs::write(tmp.path().join("a.rs"), "fn a() { let _ = 1; }").unwrap();
        run_git(tmp.path(), &["add", "a.rs"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "edit a"]);
        let set = changed_files("base", tmp.path()).unwrap();
        assert!(set.contains("a.rs"), "set = {set:?}");
        assert!(!set.contains("b.rs"));
    }

    #[test]
    fn changed_files_empty_when_no_diff() {
        let tmp = init_repo();
        run_git(tmp.path(), &["tag", "base"]);
        // No further commits → diff base...HEAD is empty.
        let set = changed_files("base", tmp.path()).unwrap();
        assert!(set.is_empty(), "expected empty, got {set:?}");
    }

    #[test]
    fn changed_files_errors_on_unknown_ref() {
        let tmp = init_repo();
        let err = changed_files("does-not-exist", tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("git diff failed"), "msg = {msg}");
    }

    #[test]
    fn changed_files_errors_when_git_invocation_fails() {
        // current_dir does not exist → git can't even start in it.
        let err = changed_files("HEAD", Path::new("/no/such/path/for/git/diff")).unwrap_err();
        // Either spawn-fail or git-fail — both are acceptable error
        // shapes; we only assert that we surface a context line.
        let msg = format!("{err:#}");
        assert!(
            msg.contains("git diff") || msg.contains("invoke git"),
            "msg = {msg}"
        );
    }
}
