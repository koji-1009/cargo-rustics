//! `cargo rustics ai-loop` — print the embedded AI-loop walkthrough.
//!
//! Mirrors the [`super::manual`] command: the doc text is `include_str!`'d
//! at compile time so the install version and printed version cannot
//! drift apart. The walkthrough is the operational counterpart to the
//! lens-catalogue manual — it shows the actual prompts and commands an
//! AI agent uses to drive cargo-rustics end to end.
//!
//! Inspired by dartrics's `doc/ai-loop.md`
//! (<https://pub.dev/packages/dartrics>).

use std::io::Write;

use anyhow::Result;

const AI_LOOP: &str = include_str!("../../../../doc/ai-loop.md");

#[cfg(test)]
fn ai_loop_text() -> &'static str {
    AI_LOOP
}

/// Runs the `ai-loop` subcommand.
pub fn run() -> Result<u8> {
    let mut out = std::io::stdout().lock();
    out.write_all(AI_LOOP.as_bytes())?;
    // The doc is committed with a trailing newline (enforced by the
    // test below). No defensive `\n` append is needed — the embedding
    // contract is that the file ends with a newline so the output of
    // this command is line-buffered-friendly for piping.
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_starts_with_h1() {
        assert!(AI_LOOP.starts_with("# cargo-rustics — AI loop walkthrough"));
    }

    #[test]
    fn doc_mentions_each_loop_step() {
        // The four-step loop is the core promise; smoke-test that none
        // of the headings drifted.
        assert!(AI_LOOP.contains("Step 1"));
        assert!(AI_LOOP.contains("Step 2"));
        assert!(AI_LOOP.contains("Step 3"));
        assert!(AI_LOOP.contains("Step 4"));
        assert!(AI_LOOP.contains("Step 5"));
    }

    #[test]
    fn doc_mentions_dartrics_innovations() {
        // The point of the AI-loop doc is to surface the reasons for
        // each design choice; verify the headlines are present.
        assert!(AI_LOOP.contains("complexityJustified"));
        assert!(AI_LOOP.contains("--snapshot-mode"));
        assert!(AI_LOOP.contains("cosmeticAnalysis"));
        assert!(AI_LOOP.contains("--no-auto-explain"));
        assert!(AI_LOOP.contains("regression-report v2"));
    }

    #[test]
    fn ai_loop_text_is_non_empty() {
        assert!(!ai_loop_text().is_empty());
    }

    #[test]
    fn run_returns_zero() {
        assert_eq!(run().unwrap(), 0);
    }

    #[test]
    fn embedded_doc_ends_with_newline() {
        // The `run()` body trusts this contract (no defensive `\n`
        // append). If a future edit drops the trailing newline, this
        // test fails first so the contract change is visible.
        assert!(AI_LOOP.ends_with('\n'));
    }
}
