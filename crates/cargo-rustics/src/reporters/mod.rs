//! Output formatters.
//!
//! Each reporter takes the `Report` shape and writes to a `Write` sink.
//! Reporters are stateless — the `Report` is already sorted (plan §4) and
//! contains every field the AI agent needs to act.

pub mod ai;
pub mod console;
pub mod json;

#[cfg(test)]
mod golden_tests;

use std::io::Write;

use anyhow::Result;

use crate::cli::Reporter;
use crate::report::Report;

/// Writes `report` in the chosen format to `out`.
pub fn write(reporter: Reporter, report: &Report, out: &mut dyn Write) -> Result<()> {
    match reporter {
        Reporter::Console => console::write(report, out),
        Reporter::Json => json::write(report, out),
        Reporter::Ai => ai::write(report, out),
    }
}
