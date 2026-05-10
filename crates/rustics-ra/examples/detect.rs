//! `cargo run -p rustics-ra --example detect -- <manifest_dir>`
//!
//! Smoke-tests the HIR-based unused detector against an arbitrary
//! Cargo project. Mostly exists for the spike — once the API
//! settles, the detector wires into `cargo-rustics --resolved`.

use std::env;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    eprintln!("loading workspace at {} ...", dir.display());
    let t = Instant::now();
    let items = rustics_ra::unused::detect_at(&dir)?;
    let elapsed = t.elapsed();
    eprintln!(
        "done in {:.2}s — {} unused items",
        elapsed.as_secs_f64(),
        items.len()
    );
    for item in &items {
        println!(
            "  {kind} {name} — {file}:{line}",
            kind = item.kind,
            name = item.name,
            file = item.file,
            line = item.line + 1, // ra is 0-based; humans are 1-based
        );
    }
    Ok(())
}
