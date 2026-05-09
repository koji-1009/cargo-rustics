//! Configuration loader.
//!
//! M1 supports the bare minimum: per-metric `warning` / `error` thresholds
//! from `rustics.toml`. lists the full surface; M1 implements the
//! threshold table, the rest of the surface (snapshot, dismissals, exclude
//! patterns) plugs into M2/M3.
//!
//! Resolution order:
//! 1. `--config <path>`.
//! 2. `<workspace_root>/rustics.toml` if present.
//! 3. Defaults from each metric's metadata.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level config file shape (`rustics.toml`).
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// `[rustics]` table — top-level options.
    #[serde(default)]
    pub rustics: RusticsTable,
}

/// `[rustics]` table.
#[derive(Debug, Default, Deserialize)]
pub struct RusticsTable {
    /// `[rustics.metrics.<id>]` entries.
    #[serde(default)]
    pub metrics: BTreeMap<String, MetricThresholds>,
    /// `[rustics.exclude]` table.
    #[serde(default)]
    pub exclude: ExcludeTable,
}

/// `[rustics.exclude]` — file-walker exclusions
///
/// Patterns are matched against the workspace-relative path. The matcher
/// is intentionally minimal at M1:
///
/// * `<prefix>/**` — true if the path begins with `prefix/`.
/// * `**/<basename>` — true if the path ends with `basename`.
/// * literal — true if the path begins with the literal.
///
/// Full glob support (`*` segment wildcards, alternations) is M2 alongside
/// the `--config <path>` flag.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct ExcludeTable {
    /// Glob-ish patterns examples: `target/**`, `**/build.rs`.
    #[serde(default)]
    pub patterns: Vec<String>,
}

impl ExcludeTable {
    /// True iff `relative` matches at least one configured pattern.
    pub fn matches(&self, relative: &str) -> bool {
        self.patterns.iter().any(|p| pattern_matches(p, relative))
    }
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/**") {
        let needle = format!("{prefix}/");
        return path == prefix || path.starts_with(&needle);
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return path.ends_with(suffix) || path == suffix;
    }
    path.starts_with(pattern)
}

/// `[rustics.metrics.<id>]` entry.
#[derive(Debug, Default, Deserialize, Clone, Copy)]
pub struct MetricThresholds {
    /// Override the default warning threshold; `None` keeps the metric's default.
    pub warning: Option<f64>,
    /// Override the default error threshold; `None` keeps the metric's default.
    pub error: Option<f64>,
    /// `false` disables the metric for this run.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Config {
    /// Loads `rustics.toml` from `workspace_root` if present; otherwise
    /// returns defaults. Missing file is not an error — most projects
    /// ride on the metric defaults.
    pub fn load_from(workspace_root: &Path) -> Result<Self> {
        let path = workspace_root.join("rustics.toml");
        if !path.is_file() {
            return Ok(Self::default());
        }
        Self::load_from_explicit_path(&path)
    }

    /// Loads from an explicit path (used by `--config`). Errors if the
    /// path does not exist; missing-file is only acceptable for the
    /// implicit `rustics.toml` lookup.
    pub fn load_from_explicit_path(path: &Path) -> Result<Self> {
        let bytes =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&bytes).with_context(|| format!("parse {}", path.display()))?;
        Ok(cfg)
    }

    /// Returns the threshold override (if any) for the given metric id.
    pub fn metric(&self, id: &str) -> Option<MetricThresholds> {
        self.rustics.metrics.get(id).copied()
    }

    /// Returns the file-walker exclude table.
    pub fn exclude(&self) -> &ExcludeTable {
        &self.rustics.exclude
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SUFFIX: AtomicUsize = AtomicUsize::new(0);

    fn unique_tempdir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = SUFFIX.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rustics-cfg-{label}-{pid}-{n}"));
        fs::create_dir_all(&dir).expect("mkdir tempdir");
        dir
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = unique_tempdir("missing");
        let cfg = Config::load_from(&dir).expect("default");
        assert!(cfg.metric("anything").is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_metric_overrides() {
        let dir = unique_tempdir("parse");
        fs::write(
            dir.join("rustics.toml"),
            r#"
[rustics.metrics.cyclomatic-complexity]
warning = 12
error = 25
"#,
        )
        .unwrap();
        let cfg = Config::load_from(&dir).expect("parse");
        let cc = cfg.metric("cyclomatic-complexity").expect("present");
        assert_eq!(cc.warning, Some(12.0));
        assert_eq!(cc.error, Some(25.0));
        assert!(cc.enabled);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enabled_defaults_to_true() {
        let dir = unique_tempdir("enabled");
        fs::write(
            dir.join("rustics.toml"),
            r#"
[rustics.metrics.x]
warning = 1
"#,
        )
        .unwrap();
        let cfg = Config::load_from(&dir).expect("parse");
        assert!(cfg.metric("x").unwrap().enabled);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pattern_matches_prefix_suffix_and_literal() {
        assert!(pattern_matches("target/**", "target/foo.rs"));
        assert!(pattern_matches("target/**", "target"));
        assert!(!pattern_matches("target/**", "src/foo.rs"));

        assert!(pattern_matches("**/build.rs", "crates/foo/build.rs"));
        assert!(pattern_matches("**/build.rs", "build.rs"));
        assert!(!pattern_matches("**/build.rs", "crates/foo/lib.rs"));

        assert!(pattern_matches("vendor/", "vendor/foo.rs"));
        assert!(!pattern_matches("vendor/", "src/foo.rs"));
    }

    #[test]
    fn parses_exclude_patterns() {
        let dir = unique_tempdir("excl");
        fs::write(
            dir.join("rustics.toml"),
            r#"
[rustics.exclude]
patterns = ["tests/projects/**", "**/build.rs"]
"#,
        )
        .unwrap();
        let cfg = Config::load_from(&dir).expect("parse");
        let excl = cfg.exclude();
        assert!(excl.matches("tests/projects/small-cli/src/lib.rs"));
        assert!(excl.matches("crates/foo/build.rs"));
        assert!(!excl.matches("crates/rustics/src/lib.rs"));
        fs::remove_dir_all(&dir).ok();
    }
}
