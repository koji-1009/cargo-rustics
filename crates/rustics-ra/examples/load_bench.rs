//! `cargo run -p rustics-ra --example load_bench -- <manifest_dir>`
//!
//! Measures end-to-end wall time across the LoadCargoConfig
//! toggle matrix. Each row runs a full CC pass after loading so
//! the lazy salsa-cached HIR queries get triggered (and counted).
//!
//! Without the analysis pass the load alone is sub-second on this
//! workspace; the time we want to optimise lives in the HIR
//! queries the walker forces.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rustics_ra::cc;
use rustics_ra::workspace::{load_with, LoadOpts};

fn main() -> anyhow::Result<()> {
    let dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    print_header(&dir);
    for (label, opts) in configs() {
        run_one(&dir, label, *opts)?;
    }
    Ok(())
}

fn configs() -> &'static [(&'static str, LoadOpts)] {
    &[
        (
            "default (sysroot+build_scripts+proc_macro)",
            LoadOpts {
                with_sysroot: true,
                run_build_scripts: true,
                with_proc_macro_server: true,
            },
        ),
        (
            "no proc_macro server",
            LoadOpts {
                with_sysroot: true,
                run_build_scripts: true,
                with_proc_macro_server: false,
            },
        ),
        (
            "no build scripts",
            LoadOpts {
                with_sysroot: true,
                run_build_scripts: false,
                with_proc_macro_server: true,
            },
        ),
        (
            "no sysroot",
            LoadOpts {
                with_sysroot: false,
                run_build_scripts: true,
                with_proc_macro_server: true,
            },
        ),
        (
            "minimum (no sysroot + no build_scripts + no proc_macro)",
            LoadOpts {
                with_sysroot: false,
                run_build_scripts: false,
                with_proc_macro_server: false,
            },
        ),
    ]
}

fn print_header(dir: &Path) {
    println!("workspace: {}", dir.display());
    println!(
        "{:<60} {:>8} {:>10} {:>9} {:>10} {:>9}",
        "config", "load_s", "hir_cc_s", "hir_fns", "syn_cc_s", "syn_fns"
    );
    println!("{}", "-".repeat(112));
}

fn run_one(dir: &Path, label: &str, opts: LoadOpts) -> anyhow::Result<()> {
    let t_load = Instant::now();
    let workspace = load_with(dir, opts)?;
    let load_s = t_load.elapsed().as_secs_f64();
    let t_hir = Instant::now();
    let hir_rows = cc::measure_loaded(&workspace)?;
    let hir_s = t_hir.elapsed().as_secs_f64();
    let t_syn = Instant::now();
    let syn_rows = cc::measure_loaded_syntax(&workspace)?;
    let syn_s = t_syn.elapsed().as_secs_f64();
    println!(
        "{label:<60} {:>8.2} {:>10.2} {:>9} {:>10.2} {:>9}",
        load_s,
        hir_s,
        hir_rows.len(),
        syn_s,
        syn_rows.len()
    );
    drop(workspace);
    Ok(())
}
