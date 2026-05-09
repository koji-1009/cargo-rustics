//! Input handed to a [`crate::MetricCalculator`].
//!
//! The CLI parses each `.rs` file once with `syn::parse_file` and passes the resulting AST plus its source string
//! to every enabled metric. Holding both lets metrics fall back to the source
//! text when they need data the AST omits (line counts, raw token slices, …).

use std::path::Path;

/// Read-only view of a single Rust source file for metric calculation.
///
/// `MetricInput` is borrowed; metrics are not allowed to mutate the AST or
/// the source. Owned ownership lives in the CLI's analyzer — the same input
/// is shared across all enabled metrics in parallel.
pub struct MetricInput<'a> {
    /// Path of the file relative to the workspace root.
    ///
    /// Relative-to-workspace is required by the violation-id contract (plan
    /// §4.1, Q-T4) so that two clones of the same workspace produce the same
    /// ids.
    pub file: &'a Path,
    /// Original source text. Provided so metrics that need raw lines (SLOC,
    /// Halstead-volume token counting against the source, …) don't have to
    /// re-read the file.
    pub source: &'a str,
    /// Parsed `syn::File`. Metrics walk this with `syn::visit::Visit`.
    pub ast: &'a syn::File,
}

impl<'a> MetricInput<'a> {
    /// Constructs a new metric input.
    pub fn new(file: &'a Path, source: &'a str, ast: &'a syn::File) -> Self {
        Self { file, source, ast }
    }
}
