//! Workspace root detection.
//!
//! Wraps `cargo_metadata` so the analyzer always knows where to root
//! workspace-relative file paths. When metadata is
//! unavailable — e.g. running outside a Cargo project — the analysis root
//! is used as the workspace root. That keeps the tool useful for
//! ad-hoc directories of `.rs` files (educational fixtures, the
//! `tests/projects/` integration crates, …) without losing the relative-path
//! invariant.

use std::path::{Path, PathBuf};

use anyhow::Result;
use cargo_metadata::MetadataCommand;

/// Resolves the workspace root for an analysis.
///
/// `analysis_root` is treated as the starting point of the search. The
/// returned path is *absolute* and is used as the prefix for every file path
/// in the report.
pub fn resolve_workspace_root(analysis_root: &Path) -> Result<PathBuf> {
    // `cargo_metadata` looks for a `Cargo.toml`, walking upwards from
    // `--manifest-path`. We pass the closest `Cargo.toml` if present;
    // otherwise we just probe the parent directories ourselves so we don't
    // call cargo subprocess unnecessarily.
    if let Some(manifest) = nearest_manifest(analysis_root) {
        let metadata = MetadataCommand::new()
            .manifest_path(manifest)
            .no_deps()
            .exec();
        if let Ok(m) = metadata {
            return Ok(m.workspace_root.into_std_path_buf());
        }
    }
    Ok(absolute(analysis_root))
}

fn nearest_manifest(start: &Path) -> Option<PathBuf> {
    let abs = absolute(start);
    let mut here: &Path = abs.as_path();
    loop {
        let candidate = here.join("Cargo.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        match here.parent() {
            Some(parent) => here = parent,
            None => return None,
        }
    }
}

/// Returns an absolute path. Uses `std::path::absolute` if available, falling
/// back to `current_dir().join(...)` otherwise.
fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(p))
        .unwrap_or_else(|_| p.to_path_buf())
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
        let dir = std::env::temp_dir().join(format!("rustics-ws-{label}-{pid}-{n}"));
        fs::create_dir_all(&dir).expect("mkdir tempdir");
        dir
    }

    #[test]
    fn nearest_manifest_finds_workspace_root() {
        let root = unique_tempdir("root");
        let manifest = root.join("Cargo.toml");
        fs::write(&manifest, "[workspace]\nmembers = []\n").unwrap();
        let nested = root.join("src/foo");
        fs::create_dir_all(&nested).unwrap();

        let found = nearest_manifest(&nested).expect("found manifest");
        assert_eq!(found, manifest);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nearest_manifest_returns_none_when_absent() {
        let root = unique_tempdir("nomf");
        // The walk from a tempdir under /tmp will keep going up the
        // filesystem; if any ancestor of /tmp has a Cargo.toml the walk
        // will find it. To make this deterministic we drop into a
        // path that is rooted at "/" — the walk terminates at the FS
        // root which has no Cargo.toml.
        // Use an absolute path that does not exist beneath any
        // Cargo.toml ancestor; on POSIX hosts /tmp's parent is `/`.
        let _ = root; // dir is created but the walk uses `/` directly.
        assert!(nearest_manifest(Path::new("/")).is_none());
        fs::remove_dir_all(unique_tempdir("nomf-cleanup")).ok();
    }

    #[test]
    fn absolute_returns_input_when_already_absolute() {
        let p = Path::new("/etc/passwd");
        assert_eq!(absolute(p), p.to_path_buf());
    }

    #[test]
    fn absolute_prefixes_relative_with_cwd() {
        let abs = absolute(Path::new("relative-name-12345"));
        assert!(abs.is_absolute(), "expected absolute, got {abs:?}");
        assert!(abs.ends_with("relative-name-12345"));
    }

    #[test]
    fn resolve_workspace_root_falls_back_to_input_when_no_manifest() {
        let root = unique_tempdir("nomanifest");
        // No Cargo.toml under root — resolve falls back to absolute(root).
        let resolved = resolve_workspace_root(&root).unwrap();
        assert_eq!(resolved, root);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_workspace_root_uses_metadata_when_manifest_present() {
        // Build a tiny synthetic workspace with a real package so
        // cargo_metadata succeeds. We use a single-package layout
        // because no separate workspace declaration is needed for
        // metadata to resolve a workspace_root.
        let root = unique_tempdir("ws");
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "rustics-ws-fixture"
version = "0.0.0"
edition = "2021"
"#,
        )
        .unwrap();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "// nothing\n").unwrap();
        // cargo metadata must succeed; the workspace root is `root`
        // (canonicalised by cargo).
        let resolved = resolve_workspace_root(&root).unwrap();
        // Compare canonical paths: cargo_metadata canonicalises the
        // workspace root, our `unique_tempdir` does not.
        let expected = std::fs::canonicalize(&root).unwrap();
        let resolved = std::fs::canonicalize(&resolved).unwrap();
        assert_eq!(resolved, expected);
        fs::remove_dir_all(&root).ok();
    }
}
