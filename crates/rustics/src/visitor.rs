//! Shared function walker.
//!
//! Function-level lenses — Cyclomatic Complexity, SLOC, Method Length,
//! Number Of Parameters, …  — all need the same prelude: walk a `syn::File`,
//! identify each function/method, build the canonical `module::Type::method`
//! scope path, hand the function back to the lens. This module owns that
//! prelude so each lens implementation stays focused on its measurement
//! and stays small enough to clear the self-application Cyclomatic
//! Complexity threshold (plan §1.2).
//!
//! The independence principle (plan §3.2) is preserved: this module is
//! infrastructure, not state. Lenses share *how* they walk, never *what*
//! another lens measured.

use std::cell::RefCell;

use syn::visit::{self, Visit};
use syn::{
    Attribute, Block, ImplItem, ImplItemFn, ItemFn, ItemImpl, ItemMod, ItemTrait, Meta, Signature,
    TraitItem, TraitItemFn, Type,
};

use crate::scope::{ScopeKind, ScopeRef};

/// What kind of function-shaped item the visitor produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    /// `fn` at module level (free function).
    Free,
    /// `fn` inside an `impl` block.
    Method,
    /// `fn` inside a `trait` definition with a default body.
    TraitProvided,
    /// `fn` inside a `trait` definition without a body (required).
    TraitRequired,
}

impl FunctionKind {
    fn to_scope_kind(self) -> ScopeKind {
        match self {
            FunctionKind::Free => ScopeKind::FreeFunction,
            FunctionKind::Method => ScopeKind::Method,
            FunctionKind::TraitProvided | FunctionKind::TraitRequired => ScopeKind::TraitMethod,
        }
    }
}

/// One function the walker hands to the lens callback.
///
/// `body` is `None` for trait-required methods (no default impl). Lenses
/// that measure on the body should early-return; lenses that measure on
/// the signature alone (e.g. `number-of-parameters`, `lifetime-arity`)
/// continue regardless.
pub struct FunctionFrame<'a> {
    /// Canonical `module::Type::method` scope path, with line.
    pub scope: ScopeRef,
    /// Reference to the function's signature (parameters, lifetimes,
    /// generics, return type, async-ness).
    pub signature: &'a Signature,
    /// Function body, or `None` for required trait methods.
    pub body: Option<&'a Block>,
    /// What sort of function this is.
    pub kind: FunctionKind,
    /// Outer attributes attached to the function item (`#[test]`,
    /// `#[cfg(test)]`, doc-comments, …).
    pub attrs: &'a [Attribute],
    /// `true` if any enclosing `mod` is annotated `#[cfg(test)]`.
    pub in_test_module: bool,
}

impl FunctionFrame<'_> {
    /// True iff this frame is part of test-only code: either inside a
    /// `#[cfg(test)]` module (transitively), or annotated `#[test]` /
    /// `#[cfg(test)]` directly. Lenses whose semantic differs in test
    /// code (e.g. [`crate::PanicDensity`]) consult this and skip the
    /// frame; other lenses ignore it.
    pub fn is_test(&self) -> bool {
        self.in_test_module || self.attrs.iter().any(is_test_or_cfg_test_attr)
    }
}

fn is_test_or_cfg_test_attr(attr: &Attribute) -> bool {
    if let Some(last) = attr.path().segments.last() {
        if last.ident == "test" {
            return true;
        }
    }
    if let Meta::List(list) = &attr.meta {
        if list.path.is_ident("cfg") {
            return list.tokens.to_string().trim() == "test";
        }
    }
    false
}

/// Walks `file`, calling `visit` once per function-shaped item.
///
/// Order is source order — the lens callback can rely on stable ordering
/// for snapshot/golden tests. If the file carries an inner `#![cfg(test)]`
/// attribute every emitted frame is flagged `in_test_module: true`.
pub fn walk_functions<F>(file: &syn::File, mut visit: F)
where
    F: FnMut(FunctionFrame<'_>),
{
    let mut walker = ScopeWalker::new();
    if file.attrs.iter().any(is_test_or_cfg_test_attr) {
        walker.test_module_depth = 1;
    }
    let mut adapter = Adapter {
        walker,
        emit: &mut |frame| visit(frame),
    };
    adapter.visit_file(file);
}

/// Same as [`walk_functions`] but `visit` returns a measurement that is
/// collected into a `Vec`. Convenience for the common case where a lens
/// emits one number per function.
pub fn measure_functions<F>(file: &syn::File, mut compute: F) -> Vec<crate::MetricMeasurement>
where
    F: FnMut(&FunctionFrame<'_>) -> Option<f64>,
{
    let mut out = Vec::new();
    walk_functions(file, |frame| {
        if let Some(value) = compute(&frame) {
            out.push(crate::MetricMeasurement::new(frame.scope.clone(), value));
        }
    });
    out
}

/// One `impl Type` / `impl Trait for Type` block handed to the lens
/// callback by [`walk_impls`].
pub struct ImplFrame<'a> {
    /// `Type` (for inherent impls) or `Type` (the receiver, for trait
    /// impls). The trait name is dropped — the receiver type is what
    /// disambiguates instances at call sites.
    pub scope: ScopeRef,
    /// The full `impl` block.
    pub item: &'a ItemImpl,
    /// Outer attributes on the impl item (`#[cfg(test)]`, `#[automatically_derived]`, …).
    pub attrs: &'a [Attribute],
    /// True if any enclosing `mod` is `#[cfg(test)]`.
    pub in_test_module: bool,
}

impl ImplFrame<'_> {
    /// Same as [`FunctionFrame::is_test`] but on the impl item.
    pub fn is_test(&self) -> bool {
        self.in_test_module || self.attrs.iter().any(is_test_or_cfg_test_attr)
    }
}

/// Walks `file` and calls `visit` once per `impl` block.
pub fn walk_impls<F>(file: &syn::File, mut visit: F)
where
    F: FnMut(ImplFrame<'_>),
{
    let walker = ScopeWalker::new();
    let mut adapter = ImplAdapter {
        walker,
        emit: &mut |frame| visit(frame),
    };
    adapter.visit_file(file);
}

/// Convenience: emits `f64` per impl block via [`walk_impls`].
pub fn measure_impls<F>(file: &syn::File, mut compute: F) -> Vec<crate::MetricMeasurement>
where
    F: FnMut(&ImplFrame<'_>) -> Option<f64>,
{
    let mut out = Vec::new();
    walk_impls(file, |frame| {
        if let Some(v) = compute(&frame) {
            out.push(crate::MetricMeasurement::new(frame.scope.clone(), v));
        }
    });
    out
}

/// One `trait` definition handed to the lens callback by [`walk_traits`].
pub struct TraitFrame<'a> {
    /// `Trait` (the trait name).
    pub scope: ScopeRef,
    /// The full `trait` definition.
    pub item: &'a ItemTrait,
    /// Outer attributes on the trait item.
    pub attrs: &'a [Attribute],
    /// True if any enclosing `mod` is `#[cfg(test)]`.
    pub in_test_module: bool,
}

impl TraitFrame<'_> {
    /// Same as [`FunctionFrame::is_test`] but on the trait item.
    pub fn is_test(&self) -> bool {
        self.in_test_module || self.attrs.iter().any(is_test_or_cfg_test_attr)
    }
}

/// Walks `file` and calls `visit` once per `trait` definition.
pub fn walk_traits<F>(file: &syn::File, mut visit: F)
where
    F: FnMut(TraitFrame<'_>),
{
    let walker = ScopeWalker::new();
    let mut adapter = TraitAdapter {
        walker,
        emit: &mut |frame| visit(frame),
    };
    adapter.visit_file(file);
}

/// Convenience: emits `f64` per trait definition via [`walk_traits`].
pub fn measure_traits<F>(file: &syn::File, mut compute: F) -> Vec<crate::MetricMeasurement>
where
    F: FnMut(&TraitFrame<'_>) -> Option<f64>,
{
    let mut out = Vec::new();
    walk_traits(file, |frame| {
        if let Some(v) = compute(&frame) {
            out.push(crate::MetricMeasurement::new(frame.scope.clone(), v));
        }
    });
    out
}

// --- internals ---------------------------------------------------------

struct ScopeWalker {
    module_path: Vec<String>,
    impl_type: Option<String>,
    trait_name: Option<String>,
    /// Increments when entering a `#[cfg(test)]` module; the frame's
    /// `in_test_module` is `test_module_depth > 0` at the time of emission.
    test_module_depth: u32,
    /// Used so the syn::Visit impl can borrow-check freely while still
    /// emitting frames upward via the &mut callback.
    _marker: RefCell<()>,
}

impl ScopeWalker {
    fn new() -> Self {
        Self {
            module_path: Vec::new(),
            impl_type: None,
            trait_name: None,
            test_module_depth: 0,
            _marker: RefCell::new(()),
        }
    }

    fn in_test_module(&self) -> bool {
        self.test_module_depth > 0
    }

    fn make_scope_path(&self, fn_name: &str) -> String {
        let mut parts: Vec<&str> = self.module_path.iter().map(String::as_str).collect();
        if let Some(t) = self.impl_type.as_deref() {
            parts.push(t);
        } else if let Some(t) = self.trait_name.as_deref() {
            parts.push(t);
        }
        parts.push(fn_name);
        parts.join("::")
    }
}

struct Adapter<'cb> {
    walker: ScopeWalker,
    emit: &'cb mut dyn FnMut(FunctionFrame<'_>),
}

impl<'ast, 'cb> Visit<'ast> for Adapter<'cb> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        let entered_test = node.attrs.iter().any(is_test_or_cfg_test_attr);
        if entered_test {
            self.walker.test_module_depth += 1;
        }
        self.walker.module_path.push(node.ident.to_string());
        visit::visit_item_mod(self, node);
        self.walker.module_path.pop();
        if entered_test {
            self.walker.test_module_depth -= 1;
        }
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let kind = FunctionKind::Free;
        let scope = ScopeRef::new(
            self.walker.make_scope_path(&node.sig.ident.to_string()),
            kind.to_scope_kind(),
            node.sig.fn_token.span.start().line,
        );
        (self.emit)(FunctionFrame {
            scope,
            signature: &node.sig,
            body: Some(node.block.as_ref()),
            kind,
            attrs: &node.attrs,
            in_test_module: self.walker.in_test_module(),
        });
        visit::visit_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        let prev = self.walker.impl_type.take();
        self.walker.impl_type = type_name(&node.self_ty);
        for item in &node.items {
            if let ImplItem::Fn(method) = item {
                self.visit_impl_item_fn(method);
            } else {
                visit::visit_impl_item(self, item);
            }
        }
        self.walker.impl_type = prev;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let kind = FunctionKind::Method;
        let scope = ScopeRef::new(
            self.walker.make_scope_path(&node.sig.ident.to_string()),
            kind.to_scope_kind(),
            node.sig.fn_token.span.start().line,
        );
        (self.emit)(FunctionFrame {
            scope,
            signature: &node.sig,
            body: Some(&node.block),
            kind,
            attrs: &node.attrs,
            in_test_module: self.walker.in_test_module(),
        });
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast ItemTrait) {
        let prev = self.walker.trait_name.take();
        self.walker.trait_name = Some(node.ident.to_string());
        for item in &node.items {
            if let TraitItem::Fn(method) = item {
                self.visit_trait_item_fn(method);
            } else {
                visit::visit_trait_item(self, item);
            }
        }
        self.walker.trait_name = prev;
    }

    fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
        let kind = if node.default.is_some() {
            FunctionKind::TraitProvided
        } else {
            FunctionKind::TraitRequired
        };
        let scope = ScopeRef::new(
            self.walker.make_scope_path(&node.sig.ident.to_string()),
            kind.to_scope_kind(),
            node.sig.fn_token.span.start().line,
        );
        (self.emit)(FunctionFrame {
            scope,
            signature: &node.sig,
            body: node.default.as_ref(),
            kind,
            attrs: &node.attrs,
            in_test_module: self.walker.in_test_module(),
        });
        visit::visit_trait_item_fn(self, node);
    }
}

struct ImplAdapter<'cb> {
    walker: ScopeWalker,
    emit: &'cb mut dyn FnMut(ImplFrame<'_>),
}

impl<'ast, 'cb> Visit<'ast> for ImplAdapter<'cb> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        let entered_test = node.attrs.iter().any(is_test_or_cfg_test_attr);
        if entered_test {
            self.walker.test_module_depth += 1;
        }
        self.walker.module_path.push(node.ident.to_string());
        visit::visit_item_mod(self, node);
        self.walker.module_path.pop();
        if entered_test {
            self.walker.test_module_depth -= 1;
        }
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        let receiver = type_name(&node.self_ty).unwrap_or_else(|| "<anon>".to_string());
        let scope = ScopeRef::new(
            self.walker.make_scope_path(&receiver),
            ScopeKind::ImplBlock,
            node.impl_token.span.start().line,
        );
        (self.emit)(ImplFrame {
            scope,
            item: node,
            attrs: &node.attrs,
            in_test_module: self.walker.test_module_depth > 0,
        });
        // Don't recurse into the impl's items — we don't currently need
        // nested impls (impls inside fn bodies are syntactically rare
        // and not part of the M1 catalogue).
    }
}

struct TraitAdapter<'cb> {
    walker: ScopeWalker,
    emit: &'cb mut dyn FnMut(TraitFrame<'_>),
}

impl<'ast, 'cb> Visit<'ast> for TraitAdapter<'cb> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        let entered_test = node.attrs.iter().any(is_test_or_cfg_test_attr);
        if entered_test {
            self.walker.test_module_depth += 1;
        }
        self.walker.module_path.push(node.ident.to_string());
        visit::visit_item_mod(self, node);
        self.walker.module_path.pop();
        if entered_test {
            self.walker.test_module_depth -= 1;
        }
    }

    fn visit_item_trait(&mut self, node: &'ast ItemTrait) {
        let name = node.ident.to_string();
        let scope = ScopeRef::new(
            self.walker.make_scope_path(&name),
            ScopeKind::TraitDef,
            node.trait_token.span.start().line,
        );
        (self.emit)(TraitFrame {
            scope,
            item: node,
            attrs: &node.attrs,
            in_test_module: self.walker.test_module_depth > 0,
        });
    }
}

/// Returns the surface-level type name for an `impl Type` head. Generic
/// parameters and lifetimes are stripped — the metric scope path is for
/// human + AI display, not name-mangled symbol resolution.
fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(tp) => tp.path.segments.last().map(|seg| seg.ident.to_string()),
        Type::Reference(r) => type_name(&r.elem),
        Type::Paren(p) => type_name(&p.elem),
        Type::Group(g) => type_name(&g.elem),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("parse")
    }

    fn collect(file: &syn::File) -> Vec<(String, FunctionKind)> {
        let mut out = Vec::new();
        walk_functions(file, |frame| {
            out.push((frame.scope.path.clone(), frame.kind));
        });
        out
    }

    #[test]
    fn free_function_is_picked_up() {
        let f = parse("fn f() {}");
        assert_eq!(collect(&f), vec![("f".into(), FunctionKind::Free)]);
    }

    #[test]
    fn impl_method_uses_type_prefix() {
        let f = parse("struct Foo; impl Foo { fn m(&self) {} }");
        assert_eq!(collect(&f), vec![("Foo::m".into(), FunctionKind::Method)]);
    }

    #[test]
    fn trait_for_type_uses_receiver_type() {
        let f = parse(
            r#"
            struct Foo;
            trait T { fn t(&self); }
            impl T for Foo { fn t(&self) {} }
        "#,
        );
        let v = collect(&f);
        assert!(v.contains(&("T::t".into(), FunctionKind::TraitRequired)));
        assert!(v.contains(&("Foo::t".into(), FunctionKind::Method)));
    }

    #[test]
    fn nested_modules_prefix_scope() {
        let f = parse("mod a { mod b { fn f() {} } }");
        assert_eq!(collect(&f), vec![("a::b::f".into(), FunctionKind::Free)]);
    }

    #[test]
    fn trait_required_has_no_body() {
        let f = parse("trait T { fn f(); fn g() {} }");
        let mut frames: Vec<(String, FunctionKind, bool)> = Vec::new();
        walk_functions(&f, |frame| {
            frames.push((frame.scope.path.clone(), frame.kind, frame.body.is_some()));
        });
        assert!(frames.contains(&("T::f".into(), FunctionKind::TraitRequired, false)));
        assert!(frames.contains(&("T::g".into(), FunctionKind::TraitProvided, true)));
    }

    #[test]
    fn measure_functions_filters_none() {
        let f = parse("fn a() {} fn b() {} fn c() {}");
        let ms = measure_functions(&f, |frame| {
            if frame.scope.path == "b" {
                None
            } else {
                Some(1.0)
            }
        });
        let names: Vec<_> = ms.iter().map(|m| m.scope.path.clone()).collect();
        assert_eq!(names, vec!["a".to_string(), "c".to_string()]);
    }
}
