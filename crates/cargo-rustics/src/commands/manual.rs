//! `cargo rustics manual` — print the embedded operator's manual.
//!
//! The manual is `include_str!`'d at compile time so the install version
//! and printed version cannot drift apart (plan §5.4). This is the entry
//! point of the AI loop (plan §1.4): an agent runs `manual` once, then
//! `analyze`, then refactors, then re-runs `analyze`.
//!
//! There is intentionally no partial-retrieval flag. An earlier
//! `--lens <id>` filter looked like an AI loop UX win — load only the
//! relevant lens's section into context — but the framing was wrong:
//! deciding "which lens do I need?" requires the agent to first read
//! a TOC, which costs more tokens than just dumping the full manual
//! (and adds a routing-decision step the agent may get wrong).
//! `cargo rustics manual` is a one-shot full-context handoff.

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

/// Runs the `manual` subcommand. Prints the full manual to stdout.
pub fn run() -> Result<u8> {
    let mut out = std::io::stdout().lock();
    out.write_all(MANUAL.as_bytes())?;
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
        assert_eq!(run().unwrap(), 0);
    }

    #[test]
    fn embedded_manual_ends_with_newline() {
        // `run()` writes the constant verbatim; the contract is that
        // the file ends with a newline so piping is line-buffered-friendly.
        // If a future edit drops it, this test fails first.
        assert!(MANUAL.ends_with('\n'));
    }

    /// Drift gate: every lens registered in `rustics::builtin_metrics()`
    /// must have a `### \`<id>\`` section in `doc/manual.md`. Without
    /// this, a contributor adding a new lens can ship without
    /// documenting it (the metadata in code is the SSOT and gets
    /// rendered into AI/JSON reports, but the human-readable manual
    /// has its own threshold-rationale prose). Test fails first so
    /// the omission is visible at PR time.
    #[test]
    fn manual_documents_every_built_in_lens() {
        let mut missing: Vec<&str> = Vec::new();
        for metric in rustics::builtin_metrics() {
            let id = metric.id();
            let needle = format!("### `{id}`");
            if !MANUAL.contains(&needle) {
                missing.push(id);
            }
        }
        assert!(
            missing.is_empty(),
            "doc/manual.md is missing sections for: {missing:?}"
        );
    }

    /// Inverse drift gate: every `### \`<id>\`` *inside the lens
    /// section* of the manual must correspond to a registered lens.
    /// Stale sections (lens removed but manual not updated) are dead
    /// weight in AI context. Detection scoped to the markdown region
    /// between the `## Lenses` header and the next H2 so reporter
    /// section H3s (`### \`console\`` etc.) aren't false positives.
    #[test]
    fn manual_does_not_document_removed_lenses() {
        use std::collections::HashSet;
        let mut known: HashSet<&'static str> = rustics::builtin_metrics()
            .iter()
            .map(|m| m.id())
            .collect();
        // Cross-file lenses live outside `builtin_metrics()` (they
        // are computed by the CLI's cross-file pass, not the
        // per-file `MetricCalculator` pipeline) but still belong in
        // the manual.
        for id in ["trait-impl-fanout", "afferent-coupling"] {
            known.insert(id);
        }
        let re = regex_lite_for_h3();
        let mut stale: Vec<String> = Vec::new();
        let mut in_lens_region = false;
        for line in MANUAL.lines() {
            if line.starts_with("## ") {
                in_lens_region = line.starts_with("## Lenses");
                continue;
            }
            if !in_lens_region {
                continue;
            }
            if let Some(id) = re(line) {
                if !known.contains(id.as_str()) {
                    stale.push(id);
                }
            }
        }
        assert!(
            stale.is_empty(),
            "doc/manual.md has stale lens sections: {stale:?}"
        );
    }

    /// Tiny stand-in for a regex: extracts `id` from
    /// `### \`<id>\`` (with optional trailing words after `\``).
    /// Returns None for non-lens H3s (those whose id contains spaces,
    /// e.g. `### \`cargo rustics analyze\``).
    fn regex_lite_for_h3() -> impl Fn(&str) -> Option<String> {
        |line: &str| {
            let prefix = "### `";
            let rest = line.strip_prefix(prefix)?;
            let end = rest.find('`')?;
            let id = rest[..end].to_string();
            if id.contains(' ') {
                None
            } else {
                Some(id)
            }
        }
    }
}
