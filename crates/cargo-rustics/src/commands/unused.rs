//! `cargo rustics unused` — public-item dead-code surfacing.
//!
//! Plan §M3 / §7.1. Walks the workspace, collects every public item
//! whose name does not appear anywhere outside its declaration, and
//! prints them.

use anyhow::Result;

use crate::unused;
use crate::workspace;

/// Runs the `unused` subcommand. Exit codes:
///
/// * `0` — no candidates found.
/// * `0` (still) — candidates printed; the command is informational at
///   M3 first slice. `--apply` (deletion) lands later in M3.
pub fn run() -> Result<u8> {
    let cwd = std::env::current_dir()?;
    let workspace_root = workspace::resolve_workspace_root(&cwd)?;
    let items = unused::detect_at(&workspace_root)?;
    print!("{}", unused::format(&items));
    Ok(0)
}
