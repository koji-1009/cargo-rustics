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
    if outcome.skipped_non_top_level > 0 {
        println!(
            "  ({} method/variant/assoc-const declaration(s) reported but not auto-deletable yet)",
            outcome.skipped_non_top_level
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
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let path =
            std::env::temp_dir().join(format!("rustics-cmd-unused-{pid}-{n}-{seq}"));
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

    #[test]
    fn run_in_returns_zero_on_clean_workspace() {
        let tmp = tempdir();
        write_file(
            &tmp,
            "Cargo.toml",
            "[workspace]\nmembers = []\nresolver = \"2\"\n",
        );
        write_file(&tmp, "src/lib.rs", "// nothing public\n");
        let code = run_in(&tmp, report_args()).expect("run_in");
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn run_in_returns_zero_when_candidates_present() {
        let tmp = tempdir();
        write_file(
            &tmp,
            "Cargo.toml",
            "[workspace]\nmembers = []\nresolver = \"2\"\n",
        );
        write_file(&tmp, "src/lib.rs", "pub fn solitary() {}\n");
        // Even when items are surfaced the command stays informational
        // without `--apply`. Exit 0 is the contract.
        let code = run_in(&tmp, report_args()).expect("run_in");
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
