//! Configuration loader.
//!
//! M1 supports the bare minimum: per-metric `warning` / `error` thresholds
//! from `rustics.toml`. Plan §8 lists the full surface; M1 implements the
//! threshold table, the rest of the surface (snapshot, dismissals, exclude
//! patterns) plugs into M2/M3.
//!
//! Resolution order:
//! 1. `--config <path>` (M2 — not wired in M1).
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
        let bytes =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&bytes).with_context(|| format!("parse {}", path.display()))?;
        Ok(cfg)
    }

    /// Returns the threshold override (if any) for the given metric id.
    pub fn metric(&self, id: &str) -> Option<MetricThresholds> {
        self.rustics.metrics.get(id).copied()
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
}
