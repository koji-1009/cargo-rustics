//! `cargo run -p rustics-ra --example cc -- <manifest_dir>`
//!
//! Prints HIR-derived cyclomatic complexity per function. Designed
//! for diff-against-syn comparison: the line shape `<file>:<line>
//! <scope> <cc>` is the same as the syn-based reporter's CSV-ish
//! row, sortable for textual diff.

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
    let mut rows = rustics_ra::cc::measure_at(&dir)?;
    let elapsed = t.elapsed();
    eprintln!(
        "done in {:.2}s — {} fn measurements",
        elapsed.as_secs_f64(),
        rows.len()
    );
    rows.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.scope.cmp(&b.scope))
            .then(a.line.cmp(&b.line))
    });
    for row in rows {
        println!("{}:{} {} {}", row.file, row.line + 1, row.scope, row.cc);
    }
    Ok(())
}
