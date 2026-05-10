//! Input handed to a [`crate::MetricCalculator`].
//!
//! Single source of truth for what a lens has to look at: the
//! workspace-relative path, the raw source text, and a parsed
//! `ra_ap_syntax::SourceFile`. Lenses that need name resolution or
//! macro-expanded HIR do their own work via the optional
//! [`SemanticContext`] reference; lenses that only need the AST
//! ignore the field.

use std::path::Path;

use ra_ap_syntax::SourceFile;

/// Read-only view of a single Rust source file for metric calculation.
///
/// `MetricInput` is borrowed; metrics are not allowed to mutate the
/// AST or the source. Owned ownership lives in the CLI's analyzer —
/// the same input is shared across all enabled metrics in parallel.
pub struct MetricInput<'a> {
    /// Path of the file relative to the workspace root.
    ///
    /// Relative-to-workspace is required by the violation-id
    /// contract so that two clones of the same workspace produce the
    /// same ids.
    pub file: &'a Path,
    /// Original source text. Provided so metrics that need raw
    /// lines (SLOC, Halstead-volume token counting, …) don't have
    /// to re-read the file.
    pub source: &'a str,
    /// Parsed `ra_ap_syntax::SourceFile`. Lenses walk this with
    /// `SyntaxNode::descendants` / `AstNode::cast`. The CST exposes
    /// trivia (whitespace, comments) the syn AST hides, so SLOC
    /// computation against `source` and structure walks against
    /// `tree` give consistent line counts.
    pub tree: &'a SourceFile,
}

impl<'a> MetricInput<'a> {
    /// Constructs a new metric input.
    pub fn new(file: &'a Path, source: &'a str, tree: &'a SourceFile) -> Self {
        Self { file, source, tree }
    }
}
