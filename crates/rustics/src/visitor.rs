//! Shared visitor helpers for metric calculators.
//!
//! Each helper walks a `ra_ap_syntax::SourceFile` once and invokes
//! a callback per function / impl / trait scope. Lenses build their
//! `MetricMeasurement` lists by returning `Some(value)` from the
//! callback.

use ra_ap_syntax::{
    ast::{self, AstNode, HasName},
    SourceFile, SyntaxNode,
};

use crate::measurement::MetricMeasurement;
use crate::scope::{ScopeKind, ScopeRef};

/// Function-frame handed to lens callbacks. Carries the resolved
/// scope path, the AST node (for body / signature inspection), and
/// the function kind (free / method / trait method).
pub struct FunctionFrame<'a> {
    /// Resolved scope path: `module::Type::method`.
    pub scope: ScopeRef,
    /// The fn node itself. Walk `signature()` for parameters,
    /// `body()` for the block. Use `syntax()` to descend into
    /// expressions.
    pub item: ast::Fn,
    /// Whether the fn is free / method / trait method.
    pub kind: FunctionKind,
    /// Lifetime witness for `'a`.
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
    /// Maps a function kind to the corresponding `ScopeKind`.
    pub fn to_scope_kind(self) -> ScopeKind {
        match self {
            FunctionKind::Free => ScopeKind::FreeFunction,
            FunctionKind::Method => ScopeKind::Method,
            FunctionKind::TraitProvided | FunctionKind::TraitRequired => ScopeKind::TraitMethod,
        }
    }
}

/// Impl-block frame — the `impl Foo { ... }` (inherent) or
/// `impl Trait for Foo { ... }` block.
pub struct ImplFrame<'a> {
    /// Scope path of the `Self` type.
    pub scope: ScopeRef,
    /// The `impl` node. `trait_()` is `Some` for trait impls.
    pub item: ast::Impl,
    _marker: std::marker::PhantomData<&'a SourceFile>,
}

/// Trait-block frame — `trait Foo { ... }`.
pub struct TraitFrame<'a> {
    /// Scope path of the trait.
    pub scope: ScopeRef,
    /// The trait node.
    pub item: ast::Trait,
    _marker: std::marker::PhantomData<&'a SourceFile>,
}

/// Walks every function in `tree` and invokes `emit` per frame.
/// Returns one [`MetricMeasurement`] per `Some` value the callback
/// produced.
pub fn measure_functions<F>(tree: &SourceFile, emit: F) -> Vec<MetricMeasurement>
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    let mut out = Vec::new();
    let mut scope_chain: Vec<String> = Vec::new();
    let mut sink = MeasurementSink {
        emit,
        out: &mut out,
    };
    walk_for_fns(tree.syntax(), &mut scope_chain, &mut sink);
    out
}

/// Walks every `impl` block in `tree` and invokes `emit`. Useful for
/// class-level lenses (LCOM4 / RFC / WMC).
pub fn measure_impls<F>(tree: &SourceFile, mut emit: F) -> Vec<MetricMeasurement>
where
    F: FnMut(ImplFrame<'_>) -> Option<f64>,
{
    let mut out = Vec::new();
    for desc in tree.syntax().descendants() {
        let Some(impl_) = ast::Impl::cast(desc) else {
            continue;
        };
        let scope = impl_scope(&impl_);
        let frame = ImplFrame {
            scope: scope.clone(),
            item: impl_,
            _marker: std::marker::PhantomData,
        };
        if let Some(value) = emit(frame) {
            out.push(MetricMeasurement::new(scope, value));
        }
    }
    out
}

/// Walks every `trait` definition in `tree` and invokes `emit`.
pub fn measure_traits<F>(tree: &SourceFile, mut emit: F) -> Vec<MetricMeasurement>
where
    F: FnMut(TraitFrame<'_>) -> Option<f64>,
{
    let mut out = Vec::new();
    for desc in tree.syntax().descendants() {
        let Some(trait_) = ast::Trait::cast(desc) else {
            continue;
        };
        let Some(name) = trait_.name() else {
            continue;
        };
        let line = line_of(trait_.syntax());
        let scope = ScopeRef::new(name.text().to_string(), ScopeKind::TraitDef, line);
        let frame = TraitFrame {
            scope: scope.clone(),
            item: trait_,
            _marker: std::marker::PhantomData,
        };
        if let Some(value) = emit(frame) {
            out.push(MetricMeasurement::new(scope, value));
        }
    }
    out
}

/// Walks every function in the file and calls `f` per frame.
/// Convenience wrapper for lenses that already accumulate state
/// outside the callback.
pub fn walk_functions<F>(tree: &SourceFile, mut f: F)
where
    F: FnMut(FunctionFrame<'_>),
{
    measure_functions(tree, |frame| {
        f(frame);
        None
    });
}

/// Walks every `impl` block in the file and calls `f`.
pub fn walk_impls<F>(tree: &SourceFile, mut f: F)
where
    F: FnMut(ImplFrame<'_>),
{
    measure_impls(tree, |frame| {
        f(frame);
        None
    });
}

/// Walks every `trait` definition in the file and calls `f`.
pub fn walk_traits<F>(tree: &SourceFile, mut f: F)
where
    F: FnMut(TraitFrame<'_>),
{
    measure_traits(tree, |frame| {
        f(frame);
        None
    });
}

// -----------------------------------------------------------------
// Internals
// -----------------------------------------------------------------

struct MeasurementSink<'a, F: FnMut(FunctionFrame<'_>) -> Option<f64>> {
    emit: F,
    out: &'a mut Vec<MetricMeasurement>,
}

fn walk_for_fns<F>(node: &SyntaxNode, scope_chain: &mut Vec<String>, sink: &mut MeasurementSink<'_, F>)
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    for child in node.children() {
        if let Some(fn_) = ast::Fn::cast(child.clone()) {
            visit_fn(&fn_, scope_chain, sink);
            continue;
        }
        if let Some(m) = ast::Module::cast(child.clone()) {
            visit_module(&m, scope_chain, sink);
            continue;
        }
        if let Some(i) = ast::Impl::cast(child.clone()) {
            visit_impl(&i, scope_chain, sink);
            continue;
        }
        if let Some(t) = ast::Trait::cast(child.clone()) {
            visit_trait(&t, scope_chain, sink);
            continue;
        }
        // Unknown container — recurse to handle nested blocks.
        walk_for_fns(&child, scope_chain, sink);
    }
}

fn visit_fn<F>(fn_: &ast::Fn, scope_chain: &[String], sink: &mut MeasurementSink<'_, F>)
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    let Some(name) = fn_.name() else {
        return;
    };
    let kind = function_kind_for_chain(scope_chain, fn_);
    let scope_path = join_scope(scope_chain, &name.text());
    let line = line_of(fn_.syntax());
    let scope = ScopeRef::new(scope_path, kind.to_scope_kind(), line);
    let frame = FunctionFrame {
        scope: scope.clone(),
        item: fn_.clone(),
        kind,
        _marker: std::marker::PhantomData,
    };
    if let Some(value) = (sink.emit)(frame) {
        sink.out.push(MetricMeasurement::new(scope, value));
    }
}

fn visit_module<F>(m: &ast::Module, scope_chain: &mut Vec<String>, sink: &mut MeasurementSink<'_, F>)
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    let pushed = m.name().map(|n| {
        scope_chain.push(n.text().to_string());
    });
    if let Some(item_list) = m.item_list() {
        walk_for_fns(item_list.syntax(), scope_chain, sink);
    }
    if pushed.is_some() {
        scope_chain.pop();
    }
}

fn visit_impl<F>(i: &ast::Impl, scope_chain: &mut Vec<String>, sink: &mut MeasurementSink<'_, F>)
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    let parent_name = impl_self_name(i);
    scope_chain.push(parent_name);
    if let Some(assoc) = i.assoc_item_list() {
        walk_for_fns(assoc.syntax(), scope_chain, sink);
    }
    scope_chain.pop();
}

fn visit_trait<F>(t: &ast::Trait, scope_chain: &mut Vec<String>, sink: &mut MeasurementSink<'_, F>)
where
    F: FnMut(FunctionFrame<'_>) -> Option<f64>,
{
    let pushed = t.name().map(|n| {
        scope_chain.push(n.text().to_string());
    });
    if let Some(assoc) = t.assoc_item_list() {
        walk_for_fns(assoc.syntax(), scope_chain, sink);
    }
    if pushed.is_some() {
        scope_chain.pop();
    }
}

/// Determines the `FunctionKind` from the surrounding scope chain
/// and the function's own shape. The last scope-chain entry tells
/// us whether we're inside an impl or trait.
fn function_kind_for_chain(scope_chain: &[String], fn_: &ast::Fn) -> FunctionKind {
    // Walk ancestors to find the nearest impl / trait container.
    for ancestor in fn_.syntax().ancestors().skip(1) {
        if ast::Impl::can_cast(ancestor.kind()) {
            return FunctionKind::Method;
        }
        if ast::Trait::can_cast(ancestor.kind()) {
            return if fn_.body().is_some() {
                FunctionKind::TraitProvided
            } else {
                FunctionKind::TraitRequired
            };
        }
        if ast::Fn::can_cast(ancestor.kind()) {
            // Nested fn inside a fn body — uncommon; fall through to
            // free.
            break;
        }
    }
    let _ = scope_chain;
    FunctionKind::Free
}

fn impl_self_name(i: &ast::Impl) -> String {
    i.self_ty()
        .as_ref()
        .and_then(|ty| match ty {
            ast::Type::PathType(p) => p.path().and_then(|p| p.segment()).map(|s| s.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn impl_scope(i: &ast::Impl) -> ScopeRef {
    let line = line_of(i.syntax());
    ScopeRef::new(impl_self_name(i), ScopeKind::ImplBlock, line)
}

fn join_scope(scope_chain: &[String], name: &str) -> String {
    let mut path = scope_chain.join("::");
    if !path.is_empty() {
        path.push_str("::");
    }
    path.push_str(name);
    path
}

/// 1-based line number of `node`'s start position. Computed by
/// counting newlines in the file's text up to the node's range
/// start. Cheap because we only use it for top-level fn/impl/trait
/// nodes.
pub(crate) fn line_of(node: &SyntaxNode) -> usize {
    let offset: usize = node.text_range().start().into();
    let root_text = node
        .ancestors()
        .last()
        .map(|root| root.text().to_string())
        .unwrap_or_default();
    let prefix = root_text
        .get(..offset)
        .unwrap_or_default();
    prefix.bytes().filter(|b| *b == b'\n').count() + 1
}
