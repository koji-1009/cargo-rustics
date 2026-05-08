//! `cargo rustics manual` — print the embedded operator's manual.
//!
//! The manual is `include_str!`'d at compile time so the install version
//! and printed version cannot drift apart (plan §5.4). This is the entry
//! point of the AI loop (plan §1.4): an agent runs `manual` once, then
//! `analyze`, then refactors, then re-runs `analyze`.

use std::io::Write;

use anyhow::Result;

/// The full text of `doc/manual.md`, baked into the binary at compile time.
///
/// `include_str!` resolves the path at compile time relative to *this file*.
/// Manual edits to `doc/manual.md` propagate to the binary on the next
/// `cargo build` — no separate sync step.
const MANUAL: &str = include_str!("../../../../doc/manual.md");

/// Returns the embedded manual text. Useful for testing and for
/// programmatic embedding by an agent host.
#[cfg(test)]
fn manual_text() -> &'static str {
    MANUAL
}

/// Runs the `manual` subcommand.
pub fn run() -> Result<u8> {
    let mut out = std::io::stdout().lock();
    out.write_all(MANUAL.as_bytes())?;
    if !MANUAL.ends_with('\n') {
        out.write_all(b"\n")?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_starts_with_h1() {
        assert!(MANUAL.starts_with("# cargo-rustics — operator's manual"));
    }

    #[test]
    fn manual_mentions_core_commands() {
        assert!(MANUAL.contains("cargo rustics analyze"));
        assert!(MANUAL.contains("cargo rustics manual"));
        assert!(MANUAL.contains("regression"));
    }

    #[test]
    fn manual_text_is_non_empty() {
        assert!(!manual_text().is_empty());
    }

    #[test]
    fn run_returns_zero() {
        // Drives the live `run()` path; output goes to stdout.
        assert_eq!(run().unwrap(), 0);
    }
}
