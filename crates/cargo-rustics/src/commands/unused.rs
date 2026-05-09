//! `cargo rustics unused` — public-item dead-code surfacing.
//!
//!
//! whose name does not appear anywhere outside its declaration, and
//! prints them.

use anyhow::Result;

use crate::unused;
use crate::workspace;

/// Runs the `unused` subcommand. Exit codes:
///
/// * `0` — no candidates found.
/// * `0` (still) — candidates printed; the command is informational at
///   M3 first slice. `--apply` (deletion) lands later in M3.
pub fn run() -> Result<u8> {
    run_in(&std::env::current_dir()?)
}

/// Like [`run`] but resolves the workspace from `cwd` rather than the
/// process-global current directory. Tests use this entry point so they
/// can drive the command against a temporary fixture without mutating
/// the test harness's working directory.
pub fn run_in(cwd: &std::path::Path) -> Result<u8> {
    let workspace_root = workspace::resolve_workspace_root(cwd)?;
    let items = unused::detect_at(&workspace_root)?;
    print!("{}", unused::format(&items));
    Ok(0)
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

    #[test]
    fn run_in_returns_zero_on_clean_workspace() {
        let tmp = tempdir();
        write_file(
            &tmp,
            "Cargo.toml",
            "[workspace]\nmembers = []\nresolver = \"2\"\n",
        );
        write_file(&tmp, "src/lib.rs", "// nothing public\n");
        let code = run_in(&tmp).expect("run_in");
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
        // at M3 first slice — exit 0 is the contract.
        let code = run_in(&tmp).expect("run_in");
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
