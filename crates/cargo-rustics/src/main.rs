//! `cargo-rustics` — the CLI entry point.
//!
//! cargo plugins are normal binaries that cargo invokes as
//! `cargo-rustics rustics <args>` when the user types `cargo rustics
//! <args>`. We detect that calling convention and skip the duplicate
//! `rustics` token before handing off to clap. Direct invocation (running
//! the binary outside cargo) is also supported.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod config;
mod discover;
mod report;
mod reporters;
mod runner;
mod workspace;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let args = strip_cargo_subcommand_token(std::env::args().collect::<Vec<_>>());
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            // Use clap's own formatting; let the OS-style exit code follow
            // sysexits.h: 64 on usage errors, 0 on `--help` / `--version`.
            let kind = e.kind();
            e.print().ok();
            return match kind {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    ExitCode::from(0)
                }
                _ => ExitCode::from(64),
            };
        }
    };
    match dispatch(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(70)
        }
    }
}

/// Skips the duplicate "rustics" token cargo prepends when running as a plugin.
///
/// Direct invocation passes through unchanged. Pulled out as a free function
/// so it can be unit-tested without touching `std::env::args`.
fn strip_cargo_subcommand_token(args: Vec<String>) -> Vec<String> {
    if args.len() >= 2 && args[1] == "rustics" {
        let mut out = Vec::with_capacity(args.len() - 1);
        out.push(args[0].clone());
        out.extend(args.into_iter().skip(2));
        out
    } else {
        args
    }
}

fn dispatch(cli: Cli) -> Result<u8> {
    match cli.command {
        Command::Analyze(args) => commands::analyze::run(args),
        Command::Manual => commands::manual::run(),
        Command::Rules(args) => commands::rules::run(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_plugin_token_is_skipped() {
        let args = vec![
            "/path/to/cargo-rustics".to_string(),
            "rustics".to_string(),
            "manual".to_string(),
        ];
        let stripped = strip_cargo_subcommand_token(args);
        assert_eq!(stripped, vec!["/path/to/cargo-rustics", "manual"]);
    }

    #[test]
    fn direct_invocation_passes_through() {
        let args = vec!["cargo-rustics".to_string(), "manual".to_string()];
        let stripped = strip_cargo_subcommand_token(args.clone());
        assert_eq!(stripped, args);
    }

    #[test]
    fn empty_args_pass_through() {
        let args: Vec<String> = vec![];
        let stripped = strip_cargo_subcommand_token(args.clone());
        assert_eq!(stripped, args);
    }
}
