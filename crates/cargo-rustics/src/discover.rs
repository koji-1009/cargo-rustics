//! Source-file discovery.
//!
//! Walks the workspace from `--root` (default cwd) and yields every `.rs`
//! file that is not under `target/`. We use the `ignore` crate so
//! `.gitignore` is respected automatically.

use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

/// Discovered source file.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    /// Absolute path on disk.
    pub absolute: PathBuf,
    /// Workspace-root-relative path with `/` separators (plan §4.1).
    pub relative: String,
}

/// Walks `root` and returns every `.rs` file that is not under `target/`.
///
/// `workspace_root` is the prefix used to compute the relative path for the
/// AI-report contract. The walker honours `.gitignore`.
pub fn discover_rust_files(root: &Path, workspace_root: &Path) -> Result<Vec<DiscoveredFile>> {
    let walker = WalkBuilder::new(root)
        .hidden(false) // ignore handles hidden via .gitignore; allow `.cargo` etc.
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .filter_entry(|entry| {
            // Always skip `target/`. Cargo's build output is colossal and
            // never user code.
            !entry
                .file_name()
                .to_str()
                .map(|name| name == "target")
                .unwrap_or(false)
        })
        .build();

    let mut out = Vec::new();
    for entry in walker {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let relative = make_relative(workspace_root, path);
        out.push(DiscoveredFile {
            absolute: path.to_path_buf(),
            relative,
        });
    }
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

fn make_relative(workspace_root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(workspace_root).unwrap_or(file);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
        let dir = std::env::temp_dir().join(format!("rustics-disc-{label}-{pid}-{n}"));
        fs::create_dir_all(&dir).expect("mkdir tempdir");
        dir
    }

    #[test]
    fn make_relative_uses_forward_slash() {
        // Build a path that we know contains a directory separator.
        let workspace = PathBuf::from("/ws");
        let file = PathBuf::from("/ws/crates/foo/src/lib.rs");
        let rel = make_relative(&workspace, &file);
        assert_eq!(rel, "crates/foo/src/lib.rs");
    }

    #[test]
    fn discover_finds_rust_files_and_sorts() {
        let root = unique_tempdir("discover");
        fs::write(root.join("a.rs"), "").unwrap();
        fs::write(root.join("b.rs"), "").unwrap();
        fs::write(root.join("c.txt"), "").unwrap();
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("ignored.rs"), "").unwrap();

        let files = discover_rust_files(&root, &root).expect("walk");
        let names: Vec<_> = files.iter().map(|f| f.relative.clone()).collect();
        assert_eq!(names, vec!["a.rs", "b.rs"]);
        fs::remove_dir_all(&root).ok();
    }
}
