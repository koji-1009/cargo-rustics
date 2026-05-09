//! `cargo rustics manual` — print the embedded operator's manual.
//!
//! The manual is `include_str!`'d at compile time so the install version
//! and printed version cannot drift apart (plan §5.4). This is the entry
//! point of the AI loop (plan §1.4): an agent runs `manual` once, then
//! `analyze`, then refactors, then re-runs `analyze`.

use std::io::Write;

use anyhow::Result;

use crate::cli::ManualArgs;

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

/// Runs the `manual` subcommand. Without `--lens <id>`, prints the
/// whole manual; with the flag, prints only the matching `### `-level
/// section so an AI loop can load focused context.
pub fn run(args: ManualArgs) -> Result<u8> {
    let mut out = std::io::stdout().lock();
    let body = match &args.lens {
        Some(id) => extract_section(MANUAL, id)
            .ok_or_else(|| anyhow::anyhow!(
                "manual: no `### \\`{id}\\`` section found. Lens ids are kebab-case; \
                 run `cargo rustics rules` for the catalogue."
            ))?,
        None => MANUAL,
    };
    out.write_all(body.as_bytes())?;
    if !body.ends_with('\n') {
        out.write_all(b"\n")?;
    }
    Ok(0)
}

/// Extracts the markdown section whose H3 heading is `\`<id>\`` (with the
/// id rendered in code-span backticks, the convention `doc/manual.md`
/// uses for every lens). The returned slice runs from the H3 line up
/// to (but not including) the next H3 heading or end-of-string.
fn extract_section<'a>(manual: &'a str, lens_id: &str) -> Option<&'a str> {
    let needle = format!("### `{lens_id}`");
    let start = manual.find(&needle)?;
    let rest = &manual[start..];
    // The next section starts at the next `### ` at column 0. Search
    // for `\n### ` (so we don't match an in-line `### ` inside a code
    // block — extremely unlikely in this manual but cheap to guard).
    let end = rest[needle.len()..]
        .find("\n### ")
        .map(|idx| needle.len() + idx + 1) // include the trailing `\n`
        .unwrap_or(rest.len());
    Some(&rest[..end])
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
    fn run_returns_zero_for_full_manual() {
        // Drives the live `run()` path; output goes to stdout.
        assert_eq!(run(ManualArgs { lens: None }).unwrap(), 0);
    }

    #[test]
    fn extract_section_returns_just_the_lens() {
        let body = extract_section(MANUAL, "cyclomatic-complexity").unwrap();
        // Header is the first line.
        assert!(body.starts_with("### `cyclomatic-complexity`"));
        // Doesn't bleed into the next lens section.
        assert!(!body.contains("### `cognitive-complexity`"));
    }

    #[test]
    fn extract_section_returns_none_for_unknown() {
        assert!(extract_section(MANUAL, "no-such-lens-id").is_none());
    }

    #[test]
    fn run_with_lens_filter_succeeds_for_known_id() {
        let args = ManualArgs {
            lens: Some("cyclomatic-complexity".to_string()),
        };
        assert_eq!(run(args).unwrap(), 0);
    }

    #[test]
    fn run_with_lens_filter_errors_for_unknown_id() {
        let args = ManualArgs {
            lens: Some("nonexistent-lens".to_string()),
        };
        let err = run(args).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent-lens"));
        assert!(msg.contains("rules"));
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
            // Expected heading: `### \`<id>\`` followed by a space
            // (sealed-aware suffix) or a newline.
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
        let known: HashSet<&'static str> = rustics::builtin_metrics()
            .iter()
            .map(|m| m.id())
            .collect();
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
    /// We don't pull in `regex` for one usage. Returns None for
    /// non-lens H3s (those whose id contains spaces, e.g.
    /// `### \`cargo rustics analyze\``).
    fn regex_lite_for_h3() -> impl Fn(&str) -> Option<String> {
        |line: &str| {
            let prefix = "### `";
            let rest = line.strip_prefix(prefix)?;
            let end = rest.find('`')?;
            let id = rest[..end].to_string();
            // Lens ids are kebab-case single tokens. CLI subcommand
            // sections like `cargo rustics analyze` get filtered here.
            if id.contains(' ') {
                None
            } else {
                Some(id)
            }
        }
    }
}
