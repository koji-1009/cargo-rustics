//! Shared visitor helpers for metric calculators.
//!
//! Layer 2 migration stub. The original visitor was syn-based
//! (`syn::visit::Visit`); the replacement walks
//! `ra_ap_syntax::SourceFile` via `AstNode::cast` / `descendants`.
//! Until the full helper surface lands, lenses do their own walks
//! against `MetricInput::tree`. Re-exports below keep public types
//! that other modules import; their content is placeholder.

use ra_ap_syntax::SourceFile;

use crate::measurement::MetricMeasurement;
use crate::scope::ScopeKind;

/// Function-frame placeholder — the future visitor will hand one
/// frame per `fn` declaration to lens callbacks. For now nothing
/// walks the tree and lenses receive no frames.
#[allow(dead_code)]
pub struct FunctionFrame<'a> {
    /// Scope path: `module::Type::method`.
    pub scope: crate::scope::ScopeRef,
    /// Whether the fn is free / method / trait method.
    pub kind: FunctionKind,
    /// Path-anchor; lifetimes pinned via the SourceFile borrow.
    _marker: std::marker::PhantomData<&'a SourceFile>,
}

/// Function kind discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    /// Free-standing `fn` at module level.
    Free,
    /// `fn` inside an `impl` block.
    Method,
    /// `fn` inside a `trait` definition (provided body).
    TraitProvided,
    /// `fn` inside a `trait` definition (signature only).
    TraitRequired,
}

impl FunctionKind {
    /// Maps a function kind to the corresponding `ScopeKind` for
    /// reporting.
    pub fn to_scope_kind(self) -> ScopeKind {
        match self {
            FunctionKind::Free => ScopeKind::FreeFunction,
            FunctionKind::Method => ScopeKind::Method,
            FunctionKind::TraitProvided | FunctionKind::TraitRequired => ScopeKind::TraitMethod,
        }
    }
}

/// Impl-block frame placeholder.
#[allow(dead_code)]
pub struct ImplFrame<'a> {
    /// Scope path of the `Self` type.
    pub scope: crate::scope::ScopeRef,
    _marker: std::marker::PhantomData<&'a SourceFile>,
}

/// Trait-block frame placeholder.
#[allow(dead_code)]
pub struct TraitFrame<'a> {
    /// Scope path of the trait.
    pub scope: crate::scope::ScopeRef,
    _marker: std::marker::PhantomData<&'a SourceFile>,
}

/// Walks every function in the file and invokes `_emit` per frame.
/// Layer 2 migration stub: returns no measurements until the
/// ra_ap_syntax-based walker lands. Signature kept so call sites
/// in lens stubs continue to type-check.
#[allow(dead_code)]
pub fn measure_functions<F>(_tree: &SourceFile, _emit: F) -> Vec<MetricMeasurement>
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    Vec::new()
}

/// Walks every inherent / trait `impl` in the file and invokes
/// `_emit` per frame. Layer 2 migration stub.
#[allow(dead_code)]
pub fn measure_impls<F>(_tree: &SourceFile, _emit: F) -> Vec<MetricMeasurement>
where
    F: FnMut(ImplFrame<'_>) -> Option<f64>,
{
    Vec::new()
}

/// Walks every `trait` definition in the file and invokes `_emit`
/// per frame. Layer 2 migration stub.
#[allow(dead_code)]
pub fn measure_traits<F>(_tree: &SourceFile, _emit: F) -> Vec<MetricMeasurement>
where
    F: FnMut(TraitFrame<'_>) -> Option<f64>,
{
    Vec::new()
}

/// Walks every function in the file and calls the visitor for each
/// frame. Layer 2 migration stub — kept as a public function so
/// embedding code that imports it still compiles.
pub fn walk_functions<F>(_tree: &SourceFile, _f: F)
where
    F: FnMut(FunctionFrame<'_>),
{
}

/// Walks every `impl` block in the file. Layer 2 migration stub.
pub fn walk_impls<F>(_tree: &SourceFile, _f: F)
where
    F: FnMut(ImplFrame<'_>),
{
}

/// Walks every `trait` definition in the file. Layer 2 migration stub.
pub fn walk_traits<F>(_tree: &SourceFile, _f: F)
where
    F: FnMut(TraitFrame<'_>),
{
}
