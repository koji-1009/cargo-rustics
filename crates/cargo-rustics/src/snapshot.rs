//! Snapshot persistence — port of dartrics's `cache` / `baseline`
//! snapshot modes (<https://pub.dev/packages/dartrics>).
//!
//! `cargo rustics analyze --snapshot-mode <cache|baseline>` writes the
//! finished `Report` plus a `analyzedFiles` map (path → sha256) to a
//! known location. `cargo rustics regression --before <cache|baseline>`
//! then loads the snapshot back without the user having to manage paths
//! manually.
//!
//! Two modes:
//!
//! * `cache` (`target/.rustics-cache/snapshot.json`) — under `target/`,
//!   gitignored by Cargo's default rules. Used for "what did my last
//!   local run look like" between iterations.
//! * `baseline` (`<workspace>/rustics-snapshot.json`) — committed by
//!   convention. CI compares the PR's analyze output against this file
//!   to surface regressions.
//!
//! The file SHA-256 list is git-independent: a developer using `jj` or
//! `sapling`, or running on a dirty index where `git diff` would
//! mis-classify, can still get a sound regression diff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::report::Report;

/// Output of `analyze --snapshot-mode <…>`. The `report` is the same
/// shape every reporter emits; `analyzed_files` is what makes the
/// snapshot useful for regression: a content fingerprint per file lets
/// the next run compare *content* rather than relying on git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Snapshot format version. `1` for now; bumped on breaking changes
    /// alongside the AI-report contract version.
    pub version: u32,
    /// The persisted report.
    pub report: Report,
    /// Workspace-relative file paths (`/` separators) → SHA-256 hex of
    /// the file contents at snapshot time.
    #[serde(rename = "analyzedFiles", default)]
    pub analyzed_files: BTreeMap<String, String>,
}

/// Persistence mode for `analyze --snapshot-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    /// `target/.rustics-cache/snapshot.json` — local, gitignored.
    Cache,
    /// `<workspace>/rustics-snapshot.json` — commit this and CI uses it
    /// as the regression baseline.
    Baseline,
}

impl SnapshotMode {
    /// Returns the absolute path the snapshot should be written to /
    /// read from, for `workspace_root`.
    pub fn path_in(self, workspace_root: &Path) -> PathBuf {
        match self {
            SnapshotMode::Cache => workspace_root
                .join("target")
                .join(".rustics-cache")
                .join("snapshot.json"),
            SnapshotMode::Baseline => workspace_root.join("rustics-snapshot.json"),
        }
    }

    /// Resolves a `--before` keyword to the matching mode. Returns
    /// `None` if the value is a literal path that the caller should
    /// load directly.
    pub fn from_keyword(value: &str) -> Option<Self> {
        match value {
            "cache" => Some(SnapshotMode::Cache),
            "baseline" => Some(SnapshotMode::Baseline),
            _ => None,
        }
    }
}

/// Computes SHA-256 hex of every absolute path in `files`, keyed by
/// the workspace-relative path the analyzer assigned. Paths that fail
/// to read are omitted (the report still ships, the file just won't
/// have a fingerprint).
pub fn compute_file_hashes(files: &[crate::discover::DiscoveredFile]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for f in files {
        let Ok(bytes) = std::fs::read(&f.absolute) else {
            continue;
        };
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        out.insert(f.relative.clone(), hex_lowercase(&digest));
    }
    out
}

fn hex_lowercase(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Writes `snapshot` to the location for `mode` under `workspace_root`,
/// creating the parent directory if necessary.
pub fn write(mode: SnapshotMode, workspace_root: &Path, snapshot: &Snapshot) -> Result<PathBuf> {
    let path = mode.path_in(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create snapshot directory {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(snapshot).context("serialise snapshot")?;
    std::fs::write(&path, json).with_context(|| format!("write snapshot to {}", path.display()))?;
    Ok(path)
}

/// Reads a snapshot from `path`. The path may come from
/// [`SnapshotMode::path_in`] or be a literal user-supplied file.
#[allow(dead_code)] // public API; the CLI uses `read_report_compat`.
pub fn read(path: &Path) -> Result<Snapshot> {
    let bytes = std::fs::read_to_string(path)
        .with_context(|| format!("read snapshot {}", path.display()))?;
    serde_json::from_str(&bytes).with_context(|| format!("parse snapshot {}", path.display()))
}

/// Backward-compatible read: accepts either a v2 [`Snapshot`] or a
/// raw v1 [`Report`] payload (what `cargo rustics analyze --reporter
/// json` already produces). Returns the inner report either way so the
/// regression command keeps working with both shapes.
pub fn read_report_compat(path: &Path) -> Result<Report> {
    let bytes = std::fs::read_to_string(path)
        .with_context(|| format!("read snapshot {}", path.display()))?;
    if let Ok(snapshot) = serde_json::from_str::<Snapshot>(&bytes) {
        return Ok(snapshot.report);
    }
    serde_json::from_str(&bytes).with_context(|| format!("parse snapshot {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tempdir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("rustics-snap-{label}-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn from_keyword_recognises_both() {
        assert_eq!(SnapshotMode::from_keyword("cache"), Some(SnapshotMode::Cache));
        assert_eq!(
            SnapshotMode::from_keyword("baseline"),
            Some(SnapshotMode::Baseline)
        );
        assert_eq!(SnapshotMode::from_keyword("/tmp/foo.json"), None);
    }

    #[test]
    fn path_in_uses_target_for_cache_and_root_for_baseline() {
        assert_eq!(
            SnapshotMode::Cache.path_in(Path::new("/ws")),
            PathBuf::from("/ws/target/.rustics-cache/snapshot.json"),
        );
        assert_eq!(
            SnapshotMode::Baseline.path_in(Path::new("/ws")),
            PathBuf::from("/ws/rustics-snapshot.json"),
        );
    }

    #[test]
    fn hex_lowercase_is_zero_padded() {
        assert_eq!(hex_lowercase(&[0x01, 0x0a, 0xff]), "010aff");
    }

    #[test]
    fn compute_file_hashes_skips_unreadable() {
        use crate::discover::DiscoveredFile;
        let dir = tempdir("hash");
        std::fs::write(dir.join("a.rs"), b"fn a() {}\n").unwrap();
        let files = vec![
            DiscoveredFile {
                absolute: dir.join("a.rs"),
                relative: "a.rs".into(),
            },
            DiscoveredFile {
                absolute: dir.join("missing.rs"),
                relative: "missing.rs".into(),
            },
        ];
        let map = compute_file_hashes(&files);
        assert!(map.contains_key("a.rs"));
        assert!(!map.contains_key("missing.rs"));
        // SHA-256 hex is 64 chars.
        assert_eq!(map.get("a.rs").unwrap().len(), 64);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn fake_snapshot() -> Snapshot {
        let mut analyzed_files = BTreeMap::new();
        analyzed_files.insert("a.rs".to_string(), "deadbeef".repeat(8));
        Snapshot {
            version: 1,
            report: Report {
                version: 1,
                generated_at: "T".into(),
                ..Default::default()
            },
            analyzed_files,
        }
    }

    #[test]
    fn write_and_read_roundtrip_for_cache() {
        let dir = tempdir("rt-cache");
        let snap = fake_snapshot();
        let path = write(SnapshotMode::Cache, &dir, &snap).unwrap();
        assert_eq!(path, dir.join("target/.rustics-cache/snapshot.json"));
        let back = read(&path).unwrap();
        assert_eq!(back.version, 1);
        assert!(back.analyzed_files.contains_key("a.rs"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_and_read_roundtrip_for_baseline() {
        let dir = tempdir("rt-base");
        let snap = fake_snapshot();
        let path = write(SnapshotMode::Baseline, &dir, &snap).unwrap();
        assert_eq!(path, dir.join("rustics-snapshot.json"));
        let back = read(&path).unwrap();
        assert_eq!(back.version, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_report_compat_accepts_snapshot_envelope() {
        let dir = tempdir("compat-snap");
        let snap = fake_snapshot();
        let path = write(SnapshotMode::Cache, &dir, &snap).unwrap();
        let report = read_report_compat(&path).unwrap();
        assert_eq!(report.version, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_report_compat_accepts_bare_report() {
        // What `cargo rustics analyze --reporter json` produces today —
        // a `Report` with no envelope. read_report_compat still parses it.
        let dir = tempdir("compat-bare");
        let report = Report {
            version: 1,
            generated_at: "T".into(),
            ..Default::default()
        };
        let path = dir.join("bare.json");
        std::fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();
        let back = read_report_compat(&path).unwrap();
        assert_eq!(back.version, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_errors_on_missing_path() {
        let err = read(Path::new("/no/such/__rustics_snap_test__.json")).unwrap_err();
        assert!(format!("{err:#}").contains("read snapshot"));
    }

    #[test]
    fn write_creates_missing_parent() {
        let dir = tempdir("mkparent");
        // The snapshot path's parent (target/.rustics-cache) doesn't
        // exist yet — write() must create it.
        assert!(!dir.join("target").exists());
        let _ = write(SnapshotMode::Cache, &dir, &fake_snapshot()).unwrap();
        assert!(dir.join("target/.rustics-cache").is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }
}
