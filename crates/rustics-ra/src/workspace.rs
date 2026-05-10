//! Workspace loading via `ra_ap_load-cargo`.
//!
//! Wraps the boilerplate of building a `LoadCargoConfig`,
//! discovering the cargo project, running the macro server, and
//! returning a usable `AnalysisHost` + `Vfs`. Callers stay focused
//! on lens logic.

use anyhow::{Context, Result};
use std::path::Path;

use ra_ap_ide::AnalysisHost;
use ra_ap_load_cargo::{load_workspace_at, LoadCargoConfig, ProcMacroServerChoice};
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_vfs::Vfs;

/// Loaded workspace — keep both pieces alive for the duration of any
/// lens analysis. `host` exposes the high-level `Analysis` snapshot;
/// `vfs` maps `FileId` ↔ filesystem path.
pub struct LoadedWorkspace {
    pub host: AnalysisHost,
    pub vfs: Vfs,
}

/// Loads the cargo workspace rooted at `manifest_dir`. `manifest_dir`
/// must contain a `Cargo.toml`. Macro expansion is enabled (Sysroot
/// mode `Discover`) so HIR-level analysis sees post-expansion code.
pub fn load(manifest_dir: &Path) -> Result<LoadedWorkspace> {
    let cargo_config = CargoConfig {
        sysroot: Some(RustLibSource::Discover),
        ..Default::default()
    };
    let load_config = LoadCargoConfig {
        load_out_dirs_from_check: true,
        with_proc_macro_server: ProcMacroServerChoice::Sysroot,
        prefill_caches: false,
        num_worker_threads: 1,
        proc_macro_processes: 1,
    };
    let no_progress = &|_| ();
    let (db, vfs, _proc_macro) = load_workspace_at(
        manifest_dir,
        &cargo_config,
        &load_config,
        no_progress,
    )
    .with_context(|| format!("load workspace at {}", manifest_dir.display()))?;
    let host = AnalysisHost::with_database(db);
    Ok(LoadedWorkspace { host, vfs })
}
