//! Scope identification.
//!
//! A *scope* names the syntactic neighbourhood that a measurement applies to.
//! For function-level metrics that means a function path like
//! `parser::Parser::parse`. For module/crate-level metrics it would be the
//! module path. The path is built solely from the AST — no symbol resolution —
//! which keeps Layer 1 (syn-only) self-contained.
//!
//! Path format follows the AI-report contract (plan §4.1):
//! `<module>::<Type>::<method>` with `::` as the separator.

use serde::{Deserialize, Serialize};

/// Kind of scope a metric value belongs to.
///
/// The kinds are kept coarse on purpose — an AI consumer should be able to
/// rank a violation list without learning the entire AST taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScopeKind {
    /// Free-standing `fn` (including `pub fn`) at module level.
    #[default]
    FreeFunction,
    /// `fn` inside an `impl` block (inherent or trait impl).
    Method,
    /// `fn` inside a `trait` definition (provided or required).
    TraitMethod,
    /// Module (`mod foo { ... }` or a file-as-module).
    Module,
    /// `impl` block as a whole.
    ImplBlock,
    /// `trait` definition.
    TraitDef,
}

/// A reference to a syntactic scope.
///
/// `path` is `module::Type::method` form. `line` is the 1-based line number
/// where the scope's syntax starts in the source file. Both are used by the
/// CLI to compute the stable violation id (`sha256("<file>|<scope>|<metric>")`,
/// see plan §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScopeRef {
    /// Dotted scope path; e.g. `parser::Parser::parse`.
    pub path: String,
    /// What kind of syntactic node `path` resolves to.
    pub kind: ScopeKind,
    /// 1-based line number where the scope opens in the source file.
    pub line: usize,
}

impl ScopeRef {
    /// Constructs a new scope reference. Trims leading `::` if present so the
    /// path is canonical (the visitor builds the path top-down which can leave
    /// a leading separator at the crate root).
    pub fn new(path: impl Into<String>, kind: ScopeKind, line: usize) -> Self {
        let mut path = path.into();
        while path.starts_with("::") {
            path.drain(..2);
        }
        Self { path, kind, line }
    }
}
