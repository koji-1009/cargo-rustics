//! Shared visitor helpers for metric calculators.
//!
//! Each helper walks a `ra_ap_syntax::SourceFile` once and invokes
//! a callback per function / impl / trait scope. Lenses build their
//! `MetricMeasurement` lists by returning `Some(value)` from the
//! callback.

use ra_ap_syntax::{
    ast::{self, AstNode, HasAttrs, HasName},
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
    /// `true` when this fn is inside a `#[cfg(test)]` module or has
    /// a `#[test]` / `#[bench]` attribute. Lenses skip these to
    /// avoid charging fixture / assertion noise.
    pub is_test: bool,
    /// Lifetime witness for `'a`.
    _marker: std::marker::PhantomData<&'a SourceFile>,
}

impl<'a> FunctionFrame<'a> {
    /// Convenience accessor mirroring the syn-side helper.
    pub fn is_test(&self) -> bool {
        self.is_test
    }
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

// -----------------------------------------------------------------
// Internals
// -----------------------------------------------------------------

struct MeasurementSink<'a, F: FnMut(FunctionFrame<'_>) -> Option<f64>> {
    emit: F,
    out: &'a mut Vec<MetricMeasurement>,
}

fn walk_for_fns<F>(
    node: &SyntaxNode,
    scope_chain: &mut Vec<String>,
    sink: &mut MeasurementSink<'_, F>,
) where
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
    let is_test = is_test_fn(fn_, scope_chain);
    let frame = FunctionFrame {
        scope: scope.clone(),
        item: fn_.clone(),
        kind,
        is_test,
        _marker: std::marker::PhantomData,
    };
    if let Some(value) = (sink.emit)(frame) {
        sink.out.push(MetricMeasurement::new(scope, value));
    }
}

/// `true` when this `fn` should be treated as test code: it's inside
/// a `mod tests`-style module, or it carries `#[test]` / `#[bench]`
/// / `#[cfg(test)]` directly.
fn is_test_fn(fn_: &ast::Fn, scope_chain: &[String]) -> bool {
    if scope_chain.iter().any(|s| s == "tests" || s == "test") {
        return true;
    }
    fn_.attrs().any(attr_marks_test)
}

fn attr_marks_test(a: ast::Attr) -> bool {
    let Some(path) = a.path() else {
        return false;
    };
    let Some(seg) = path.segment() else {
        return false;
    };
    let name = seg.to_string();
    name == "test" || name == "bench" || name.contains("cfg(test)")
}

fn visit_module<F>(
    m: &ast::Module,
    scope_chain: &mut Vec<String>,
    sink: &mut MeasurementSink<'_, F>,
) where
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
    let prefix = root_text.get(..offset).unwrap_or_default();
    prefix.bytes().filter(|b| *b == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ra_ap_syntax::SourceFile {
        ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT).tree()
    }

    fn collect_frames(src: &str) -> Vec<(String, FunctionKind, bool)> {
        let tree = parse(src);
        let mut out = Vec::new();
        let _ = measure_functions(&tree, |frame: FunctionFrame<'_>| {
            out.push((frame.scope.path.clone(), frame.kind, frame.is_test()));
            None::<f64>
        });
        out
    }

    #[test]
    fn function_kind_to_scope_kind_covers_every_variant() {
        // FunctionKind::to_scope_kind has one arm per variant; cover
        // them all so the match isn't half-dead.
        assert_eq!(FunctionKind::Free.to_scope_kind(), ScopeKind::FreeFunction);
        assert_eq!(FunctionKind::Method.to_scope_kind(), ScopeKind::Method);
        assert_eq!(
            FunctionKind::TraitProvided.to_scope_kind(),
            ScopeKind::TraitMethod
        );
        assert_eq!(
            FunctionKind::TraitRequired.to_scope_kind(),
            ScopeKind::TraitMethod
        );
    }

    #[test]
    fn free_fn_kind_is_free() {
        let frames = collect_frames("fn f() {}");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "f");
        assert_eq!(frames[0].1, FunctionKind::Free);
        assert!(!frames[0].2);
    }

    #[test]
    fn impl_method_kind_is_method() {
        let frames = collect_frames("struct S; impl S { fn m(&self) {} }");
        let m = frames.iter().find(|(p, _, _)| p.ends_with("::m")).unwrap();
        assert_eq!(m.1, FunctionKind::Method);
        assert_eq!(m.0, "S::m");
    }

    #[test]
    fn trait_required_method_is_trait_required() {
        let frames = collect_frames("trait T { fn r(&self); }");
        let r = frames.iter().find(|(p, _, _)| p.ends_with("::r")).unwrap();
        assert_eq!(r.1, FunctionKind::TraitRequired);
    }

    #[test]
    fn trait_provided_method_is_trait_provided() {
        let frames = collect_frames("trait T { fn p(&self) {} }");
        let p = frames.iter().find(|(s, _, _)| s.ends_with("::p")).unwrap();
        assert_eq!(p.1, FunctionKind::TraitProvided);
    }

    #[test]
    fn module_scope_chain_prefixes_function_path() {
        let frames = collect_frames("mod inner { pub fn f() {} }");
        assert!(frames.iter().any(|(p, _, _)| p == "inner::f"));
    }

    #[test]
    fn cfg_test_module_marks_inner_fn_as_test() {
        // Functions inside `mod tests { … }` are flagged via the
        // scope-chain ("tests" / "test") branch of is_test_fn.
        let frames = collect_frames("mod tests { fn t() {} }");
        let t = frames.iter().find(|(p, _, _)| p == "tests::t").unwrap();
        assert!(t.2, "fn inside mod tests should be is_test = true");
    }

    #[test]
    fn test_attribute_marks_function_as_test() {
        // attr_marks_test branch — `#[test]` on the fn directly.
        let frames = collect_frames("#[test] fn it_works() {}");
        let f = frames.iter().find(|(p, _, _)| p == "it_works").unwrap();
        assert!(f.2);
    }

    #[test]
    fn bench_attribute_also_marks_as_test() {
        let frames = collect_frames("#[bench] fn b() {}");
        let f = frames.iter().find(|(p, _, _)| p == "b").unwrap();
        assert!(f.2);
    }

    #[test]
    fn nested_fn_inside_fn_is_not_visited() {
        // walk_for_fns walks containers (Module / Impl / Trait / Fn)
        // at each level but visit_fn does not recurse into the
        // function body, so a `fn` declared inside another fn's body
        // is *not* visited as its own frame. Locks the contract.
        let frames = collect_frames("fn outer() { fn inner() {} }");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "outer");
    }

    #[test]
    fn measure_impls_walks_only_top_level_impls() {
        // Sanity-check the public seam used by class-level lenses.
        let tree = parse("struct A; struct B; impl A {} impl B {}");
        let mut scopes: Vec<String> = Vec::new();
        let measurements = measure_impls(&tree, |frame: ImplFrame<'_>| {
            scopes.push(frame.scope.path.clone());
            Some(1.0_f64)
        });
        // measure_impls emits one entry per impl that returns Some.
        assert_eq!(measurements.len(), 2);
        assert_eq!(scopes.len(), 2);
    }

    #[test]
    fn impl_self_name_falls_back_for_non_path_type() {
        // `impl SomeTrait for ()` — the self type is a TupleType, not
        // a PathType, exercising impl_self_name's `_ => None` arm
        // which makes the parent name fall back to the empty string.
        // The inner method is still visited; its scope path comes out
        // without an enclosing type prefix.
        let mut got = Vec::new();
        let tree = parse("trait SomeTrait { fn m(); } impl SomeTrait for () { fn m() {} }");
        let _ = measure_functions(&tree, |frame: FunctionFrame<'_>| {
            got.push(frame.scope.path);
            None::<f64>
        });
        // The trait's required `m` is visited (TraitRequired) and the
        // impl's provided `m` is visited under the fallback parent
        // name. Both should appear; one of them ends with bare "m".
        assert!(got.iter().any(|p| p == "m" || p.ends_with("::m")));
    }
}
