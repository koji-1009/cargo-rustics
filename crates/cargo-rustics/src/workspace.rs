//! Workspace root detection.
//!
//! Wraps `cargo_metadata` so the analyzer always knows where to root
//! workspace-relative file paths (plan §4.1, Q-T4). When metadata is
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
        assert!(nearest_manifest(&root).is_none());
        fs::remove_dir_all(&root).ok();
    }
}
