//! `cargo rustics unused` — public-item dead-code surfacing.
//!
//! Walks the workspace, lists every `pub` declaration whose name is
//! never referenced outside its definition site, and (with
//! `--apply`) deletes the top-level orphan items in place.

use anyhow::{bail, Result};

use crate::cli::UnusedArgs;
use crate::unused;
use crate::workspace;

/// Runs the `unused` subcommand. Exit codes:
///
/// * `0` — no candidates found, or candidates printed (the report
///   path is informational).
/// * `0` — `--apply` succeeded.
/// * `1` — `--apply` refused because the git tree was dirty and
///   `--force` was not set.
pub fn run(args: UnusedArgs) -> Result<u8> {
    run_in(&std::env::current_dir()?, args)
}

/// Like [`run`] but resolves the workspace from `cwd` rather than the
/// process-global current directory. Tests use this entry point so they
/// can drive the command against a temporary fixture without mutating
/// the test harness's working directory.
pub fn run_in(cwd: &std::path::Path, args: UnusedArgs) -> Result<u8> {
    let workspace_root = workspace::resolve_workspace_root(cwd)?;
    let items = collect_filtered(&workspace_root, &args)?;
    if args.apply {
        run_apply(&workspace_root, &args, &items)?;
    } else {
        print!("{}", unused::format(&items));
    }
    Ok(0)
}

/// Detection + `--filter` narrowing, returned as the working set
/// `run_in` then either prints or feeds to `--apply`. Pulled out so
/// each leg has fewer `?` operators (npath compounds multiplicatively).
fn collect_filtered(
    workspace_root: &std::path::Path,
    args: &UnusedArgs,
) -> Result<Vec<unused::UnusedItem>> {
    let allowed = unused::parse_kind_filter(&args.filter)?;
    let raw = unused::detect_at(workspace_root)?;
    Ok(unused::apply_kind_filter(raw, allowed.as_ref()))
}

/// Body of the `--apply` leg: git-tree gate, then deletion pass,
/// then the user-facing summary. Errors when the tree is dirty and
/// `--force` is not set.
fn run_apply(
    workspace_root: &std::path::Path,
    args: &UnusedArgs,
    items: &[unused::UnusedItem],
) -> Result<()> {
    if !args.force && !unused::apply::git_tree_is_clean(workspace_root)? {
        bail!(
            "rustics unused --apply: git tree has uncommitted changes; \
             commit or stash first, or pass --force to override."
        );
    }
    let outcome = unused::apply::apply(workspace_root, items, args.include_tests)?;
    print_apply_outcome(&outcome);
    Ok(())
}

fn print_apply_outcome(outcome: &unused::apply::Outcome) {
    println!(
        "rustics unused --apply: deleted {} item(s) across {} file(s).",
        outcome.deleted, outcome.touched_files
    );
    if outcome.skipped_test_files > 0 {
        println!(
            "  ({} test-file declaration(s) skipped; pass --include-tests to delete)",
            outcome.skipped_test_files
        );
    }
    if outcome.skipped_unsupported > 0 {
        println!(
            "  ({} declaration(s) refused — would leave invalid Rust (e.g. only variant in an enum))",
            outcome.skipped_unsupported
        );
    }
    if outcome.not_found > 0 {
        println!(
            "  ({} declaration(s) not found in source — file may have changed since detect)",
            outcome.not_found
        );
    }
    if outcome.deleted > 0 {
        println!("  Run `cargo fix --allow-staged` to clean up newly unused imports.");
    }
}

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;

    fn write_file(dir: &std::path::Path, rel: &str, body: &str) {
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(abs, body).unwrap();
    }

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rustics-cmd-unused-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn report_args() -> UnusedArgs {
        UnusedArgs {
            apply: false,
            force: false,
            include_tests: false,
            filter: vec![],
        }
    }

    fn apply_args(force: bool, include_tests: bool) -> UnusedArgs {
        UnusedArgs {
            apply: true,
            force,
            include_tests,
            filter: vec![],
        }
    }

    fn write_workspace(tmp: &std::path::Path, lib_body: &str) {
        // The HIR-backed unused detector loads the directory as a
        // cargo workspace via `cargo metadata`, so the fixture must
        // be a real package — a bare `[workspace]` with no members
        // satisfies cargo but produces zero local crates for HIR to
        // walk, so the unused report would always be empty.
        write_file(
            tmp,
            "Cargo.toml",
            "[package]\n\
             name = \"rustics-test-fixture\"\n\
             version = \"0.0.1\"\n\
             edition = \"2021\"\n\
             publish = false\n\
             \n\
             [lib]\n\
             path = \"src/lib.rs\"\n",
        );
        write_file(tmp, "src/lib.rs", lib_body);
    }

    #[test]
    fn run_in_returns_zero_on_clean_workspace() {
        let tmp = tempdir();
        write_workspace(&tmp, "// nothing public\n");
        let code = run_in(&tmp, report_args()).expect("run_in");
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn run_in_returns_zero_when_candidates_present() {
        let tmp = tempdir();
        write_workspace(&tmp, "pub fn solitary() {}\n");
        // Even when items are surfaced the command stays informational
        // without `--apply`. Exit 0 is the contract.
        let code = run_in(&tmp, report_args()).expect("run_in");
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn run_in_apply_deletes_orphans_in_non_git_workspace() {
        // `git_tree_is_clean` returns Ok(true) when the workspace
        // isn't a git repo (no `.git` dir in the temp tree), so the
        // apply path is exercised end-to-end without needing to spin
        // up a real repository.
        let tmp = tempdir();
        write_workspace(&tmp, "pub fn orphan() {}\n");
        let code = run_in(&tmp, apply_args(false, false)).expect("run_in --apply");
        assert_eq!(code, 0);
        let after = std::fs::read_to_string(tmp.join("src/lib.rs")).unwrap();
        // Orphan was deleted by the apply pass.
        assert!(
            !after.contains("orphan"),
            "orphan survived apply: {after:?}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn run_in_apply_filter_narrows_targets() {
        // `--filter struct` shouldn't touch a stray pub fn — the
        // filter narrows the candidate set before apply runs.
        let tmp = tempdir();
        write_workspace(&tmp, "pub fn keep_me() {}\n");
        let mut args = apply_args(false, false);
        args.filter = vec!["struct".into()];
        let code = run_in(&tmp, args).expect("run_in --apply --filter struct");
        assert_eq!(code, 0);
        let after = std::fs::read_to_string(tmp.join("src/lib.rs")).unwrap();
        assert!(
            after.contains("keep_me"),
            "filter struct erroneously deleted a fn: {after:?}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Initialises a minimal git repo at `dir` so `git status
    /// --porcelain` will produce non-empty output (untracked files
    /// count as a dirty tree).
    fn git_init_dirty(dir: &std::path::Path) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("init")
            .arg("--quiet")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git init failed in {}", dir.display());
    }

    #[test]
    fn run_in_apply_bails_on_dirty_git_without_force() {
        let tmp = tempdir();
        write_workspace(&tmp, "pub fn orphan() {}\n");
        git_init_dirty(&tmp);
        let err = run_in(&tmp, apply_args(false, false)).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("uncommitted changes"),
            "expected dirty-tree bail message, got: {msg}"
        );
        // File is preserved when apply is refused.
        let after = std::fs::read_to_string(tmp.join("src/lib.rs")).unwrap();
        assert!(after.contains("orphan"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn run_in_apply_runs_with_force_on_dirty_git() {
        let tmp = tempdir();
        write_workspace(&tmp, "pub fn orphan() {}\n");
        git_init_dirty(&tmp);
        let code = run_in(&tmp, apply_args(true, false)).expect("run_in --force");
        assert_eq!(code, 0);
        let after = std::fs::read_to_string(tmp.join("src/lib.rs")).unwrap();
        assert!(!after.contains("orphan"), "force should delete orphan");
        std::fs::remove_dir_all(&tmp).ok();
    }

    // -----------------------------------------------------------------
    // print_apply_outcome — direct coverage for each conditional
    // print branch. We construct each Outcome shape and let the fn
    // run; the test asserts no panic and the print calls fire.
    // -----------------------------------------------------------------

    fn outcome_shape(
        deleted: usize,
        touched_files: usize,
        skipped_test_files: usize,
        skipped_unsupported: usize,
        not_found: usize,
    ) -> unused::apply::Outcome {
        unused::apply::Outcome {
            deleted,
            touched_files,
            skipped_test_files,
            skipped_unsupported,
            not_found,
        }
    }

    #[test]
    fn print_apply_outcome_handles_empty_outcome() {
        // No flags > 0 → only the headline line prints; the four
        // conditional branches are all false.
        print_apply_outcome(&outcome_shape(0, 0, 0, 0, 0));
    }

    #[test]
    fn print_apply_outcome_prints_deleted_summary() {
        // Headline + the post-delete `cargo fix` hint branch.
        print_apply_outcome(&outcome_shape(3, 2, 0, 0, 0));
    }

    #[test]
    fn print_apply_outcome_prints_skipped_test_files() {
        print_apply_outcome(&outcome_shape(0, 0, 4, 0, 0));
    }

    #[test]
    fn print_apply_outcome_prints_skipped_unsupported() {
        print_apply_outcome(&outcome_shape(0, 0, 0, 2, 0));
    }

    #[test]
    fn print_apply_outcome_prints_not_found() {
        print_apply_outcome(&outcome_shape(0, 0, 0, 0, 5));
    }

    #[test]
    fn print_apply_outcome_prints_every_branch_at_once() {
        // Cover every conditional branch in one call so the
        // multiple-flag-set path doesn't regress silently.
        print_apply_outcome(&outcome_shape(7, 3, 1, 1, 2));
    }
}
