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

/// Per-load tuning knobs. Each toggle disables one of the heavy
/// stages of `ra_ap_load-cargo`. Default = full fidelity (matches
/// rust-analyzer's IDE setup); turn pieces off to trade accuracy
/// for load time.
#[derive(Debug, Clone, Copy)]
pub struct LoadOpts {
    /// Discover and parse the rust toolchain stdlib. Required for
    /// HIR queries that resolve `std::*` types.
    pub with_sysroot: bool,
    /// Run `cargo check` to populate `OUT_DIR` for build-script
    /// generated source. Required when the workspace has crates
    /// that ship `build.rs` (most non-trivial workspaces do).
    pub run_build_scripts: bool,
    /// Spawn the proc-macro server. Required for `#[derive]`,
    /// `#[tokio::main]`, etc. to produce expanded HIR. Without
    /// it, generated code is invisible to the detector.
    pub with_proc_macro_server: bool,
}

impl Default for LoadOpts {
    fn default() -> Self {
        Self {
            with_sysroot: true,
            run_build_scripts: true,
            with_proc_macro_server: true,
        }
    }
}

/// Loads the cargo workspace rooted at `manifest_dir` with full
/// fidelity. Equivalent to `load_with(manifest_dir, LoadOpts::default())`.
pub fn load(manifest_dir: &Path) -> Result<LoadedWorkspace> {
    load_with(manifest_dir, LoadOpts::default())
}

/// Loads the cargo workspace with the given option set. Used by
/// the `load_bench` example to measure how much each heavy stage
/// contributes to total load time.
pub fn load_with(manifest_dir: &Path, opts: LoadOpts) -> Result<LoadedWorkspace> {
    let cargo_config = CargoConfig {
        sysroot: opts.with_sysroot.then_some(RustLibSource::Discover),
        ..Default::default()
    };
    let load_config = LoadCargoConfig {
        load_out_dirs_from_check: opts.run_build_scripts,
        with_proc_macro_server: if opts.with_proc_macro_server {
            ProcMacroServerChoice::Sysroot
        } else {
            ProcMacroServerChoice::None
        },
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
