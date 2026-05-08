//! `--expanded-macros` — re-run lenses on cargo-expand's macro-expanded
//! output.
//!
//! Plan §7.2 / M3 task #54. Spawns `cargo expand` per workspace
//! package, captures the expanded source, and feeds it back through
//! the file walker as a synthetic `<package>/__expanded__.rs` entry.
//! Lens output then reflects the post-expansion AST — useful when
//! large proc-macros (`#[tokio::main]`, derive blanket traits, …)
//! hide the actual control flow from the un-expanded source.
//!
//! The integration is opt-in. If `cargo-expand` is not installed we
//! print a stderr note and return an empty set; the analyzer
//! continues with the un-expanded source.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::discover::DiscoveredFile;

/// Runs `cargo expand --lib` from `workspace_root` and returns the
/// expanded source as a single synthetic file. Returns `Ok(None)` if
/// `cargo-expand` is not installed; returns `Err(_)` on other
/// failures (broken manifest, etc).
pub fn expand_workspace(workspace_root: &Path) -> Result<Option<DiscoveredFile>> {
    if !cargo_expand_available() {
        eprintln!(
            "rustics: --expanded-macros set but `cargo expand` is not available. \
             Install with `cargo install cargo-expand` and re-run. Continuing on \
             the un-expanded AST."
        );
        return Ok(None);
    }
    let output = Command::new("cargo")
        .args(["expand", "--lib"])
        .current_dir(workspace_root)
        .output()
        .with_context(|| format!("invoke cargo expand at {}", workspace_root.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("rustics: cargo expand failed: {stderr}");
        return Ok(None);
    }
    let source = String::from_utf8(output.stdout).context("cargo expand stdout not UTF-8")?;
    let synthetic = synthetic_file_path(workspace_root);
    if let Err(e) = std::fs::write(&synthetic, &source) {
        eprintln!(
            "rustics: cargo expand could not persist expanded source at {}: {e}",
            synthetic.display()
        );
        return Ok(None);
    }
    Ok(Some(DiscoveredFile {
        absolute: synthetic,
        relative: ".rustics-expanded.rs".to_string(),
    }))
}

/// Returns the temporary path that holds the persisted expanded
/// source. Stable so successive runs overwrite the same file rather
/// than littering `target/`.
pub fn synthetic_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target").join(".rustics-expanded.rs")
}

/// Returns `true` iff `cargo expand` is on PATH.
fn cargo_expand_available() -> bool {
    Command::new("cargo")
        .args(["expand", "--help"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_file_path_is_under_target() {
        let p = synthetic_file_path(Path::new("/ws"));
        assert!(p.ends_with("target/.rustics-expanded.rs"));
    }
}
