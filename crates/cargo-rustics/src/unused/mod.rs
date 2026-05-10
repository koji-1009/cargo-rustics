//! Public-API reachability — name-based heuristic over `syn`'s AST.
//!
//! Walks every workspace `.rs` file, collects `pub` declarations and
//! the reference set, then flags every declaration whose name is never
//! referenced and that isn't an entry-point root.
//!
//! Scope of the heuristic — what's covered, what's not:
//!
//! * **Declarations covered.** Top-level `pub` `fn` / `struct` / `enum`
//!   / `trait` / `type` / `const` / `static` / `union`, every variant
//!   of a `pub enum`, and every `pub fn` / `pub const` inside an
//!   inherent `impl` block. `mod m { ... }` inline modules are
//!   recursed into; trait method bodies are not (the trait's `fn`
//!   declaration is the API surface).
//! * **References counted.** Every `Path` last-segment, every
//!   `ExprMethodCall.method`, every `ExprField` named member. Decl
//!   idents are not paths so they don't double-count themselves.
//! * **Roots.** `fn main`, items with `#[test]` / `#[bench]` /
//!   `#[no_mangle]` / `#[export_name]` / `#[start]` /
//!   `#[proc_macro]` / `#[proc_macro_derive]` /
//!   `#[proc_macro_attribute]` / `#[ctor::ctor]` /
//!   `#[ctor::dtor]`. Items reachable through a `pub use` chain are
//!   counted via the `pub use` path itself (the last segment of the
//!   `UseTree::Path` increments the reference set).
//!
//! Honest limits — these produce false negatives (kept alive when
//! actually unused) or false positives (flagged when actually used)
//! that the caller should know about:
//!
//! * **Homonyms.** Without name resolution two `fn foo`s in different
//!   modules are indistinguishable. If one is referenced both stay
//!   alive.
//! * **proc-macro generated identifiers.** `#[derive(Builder)]` calls
//!   into a `XxxBuilder` constructor that doesn't exist in the
//!   un-expanded source. Run with `--expanded-macros` to suppress
//!   those false positives.
//! * **Recursion / self-reference.** `pub fn foo() { foo(); }` looks
//!   referenced even when no external caller exists.
//! * **Public API consumed only by external crates.** A `pub fn` in
//!   `lib.rs` that's used by another crate but never referenced
//!   inside this workspace will be flagged. That's by design — for
//!   an AI loop, "no internal user, no test" is a legitimate signal
//!   to confirm the API has a consumer somewhere.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use syn::visit::{self, Visit};
use syn::{Attribute, ImplItem, Item, Type, Visibility};

use crate::discover::DiscoveredFile;

pub mod apply;

/// One unused-public finding.
#[derive(Debug, Clone)]
pub struct UnusedItem {
    /// Workspace-relative path.
    pub file: String,
    /// 1-based line number of the item declaration.
    pub line: usize,
    /// Item name (`fn` / `struct` / variant / method).
    pub name: String,
    /// Item kind for display (`fn`, `struct`, `enum`, `variant`,
    /// `method`, …). Stable across versions; printed verbatim by
    /// [`format`].
    pub kind: &'static str,
    /// Containing scope. `None` for top-level items, `Some(enum_name)`
    /// for variants, `Some(type_name)` for inherent impl methods /
    /// associated consts.
    pub parent: Option<String>,
}

/// Walks `files`, returns every `pub` declaration whose name is
/// referenced zero times outside its own declaration site.
pub fn detect(files: &[DiscoveredFile]) -> Result<Vec<UnusedItem>> {
    let mut decls: Vec<DeclSite> = Vec::new();
    let mut refs = ReferenceCollector::default();

    for file in files {
        let source = std::fs::read_to_string(&file.absolute)
            .with_context(|| format!("read {}", file.relative))?;
        let Ok(ast) = syn::parse_file(&source) else {
            continue;
        };
        collect_decls(&file.relative, None, &ast.items, &mut decls);
        refs.visit_file(&ast);
    }

    let counts = refs.counts;
    let mut out: Vec<UnusedItem> = decls
        .into_iter()
        .filter(|d| !d.is_root)
        .filter(|d| counts.get(&d.name).copied().unwrap_or(0) == 0)
        .map(DeclSite::into_unused)
        .collect();
    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Helper for crate-level workspace lookups (kept here so the binary
/// crate's import graph doesn't grow). Honours `rustics.toml`'s
/// `[rustics.exclude]` patterns so test-fixture crates don't show up
/// in the report by default.
pub fn detect_at(workspace_root: &Path) -> Result<Vec<UnusedItem>> {
    let config = crate::config::Config::load_from(workspace_root)?;
    let files =
        crate::discover::discover_rust_files(workspace_root, workspace_root, config.exclude())?;
    detect(&files)
}

/// Every declaration kind the detector emits, in the canonical
/// kebab-case spelling the CLI accepts on `--filter`. Single source of
/// truth for the validator and the tests.
pub const KNOWN_KINDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "trait",
    "type",
    "const",
    "static",
    "union",
    "variant",
    "method",
    "assoc-const",
];

/// Validates `--filter` values from the CLI and returns the allow-set.
/// Returns `Ok(None)` when the user passed no filter (default = all
/// kinds). Returns an error on the first unknown kind so a typo
/// (`--filter functon`) doesn't silently drop the entire report.
pub fn parse_kind_filter(values: &[String]) -> Result<Option<HashSet<String>>> {
    if values.is_empty() {
        return Ok(None);
    }
    let mut allowed = HashSet::new();
    for raw in values {
        for chunk in raw.split(',') {
            let kind = chunk.trim();
            if kind.is_empty() {
                continue;
            }
            if !KNOWN_KINDS.contains(&kind) {
                anyhow::bail!(
                    "unused --filter: unknown kind `{kind}`. Valid kinds: {}",
                    KNOWN_KINDS.join(", ")
                );
            }
            allowed.insert(kind.to_string());
        }
    }
    if allowed.is_empty() {
        return Ok(None);
    }
    Ok(Some(allowed))
}

/// Returns the subset of `items` whose kind is in `allowed`. When
/// `allowed` is `None` (no filter), the input is returned unchanged.
pub fn apply_kind_filter(items: Vec<UnusedItem>, allowed: Option<&HashSet<String>>) -> Vec<UnusedItem> {
    let Some(set) = allowed else {
        return items;
    };
    items.into_iter().filter(|i| set.contains(i.kind)).collect()
}

/// Renders a small reporter-ish text dump for `cargo rustics unused`.
pub fn format(items: &[UnusedItem]) -> String {
    if items.is_empty() {
        return "rustics unused: no candidates found.\n".to_string();
    }
    let mut s = format!("rustics unused: {} candidate(s):\n", items.len());
    for item in items {
        match &item.parent {
            Some(parent) => s.push_str(&format!(
                "  {kind} {parent}::{name} — {file}:{line}\n",
                kind = item.kind,
                parent = parent,
                name = item.name,
                file = item.file,
                line = item.line,
            )),
            None => s.push_str(&format!(
                "  {kind} {name} — {file}:{line}\n",
                kind = item.kind,
                name = item.name,
                file = item.file,
                line = item.line,
            )),
        }
    }
    s
}

#[derive(Debug, Clone)]
struct DeclSite {
    file: String,
    line: usize,
    name: String,
    kind: &'static str,
    parent: Option<String>,
    /// `true` when the decl is an entry point (`fn main`, `#[test]`,
    /// `#[no_mangle]`, …); roots never appear in the unused output.
    is_root: bool,
}

impl DeclSite {
    fn into_unused(self) -> UnusedItem {
        UnusedItem {
            file: self.file,
            line: self.line,
            name: self.name,
            kind: self.kind,
            parent: self.parent,
        }
    }
}

/// Walks `items` and pushes a [`DeclSite`] for every `pub` declaration
/// the heuristic surfaces. `parent` carries the enclosing type name
/// when we recurse into impl blocks.
fn collect_decls(
    file: &str,
    parent: Option<&str>,
    items: &[Item],
    out: &mut Vec<DeclSite>,
) {
    for item in items {
        collect_one_item(file, parent, item, out);
    }
}

fn collect_one_item(file: &str, parent: Option<&str>, item: &Item, out: &mut Vec<DeclSite>) {
    if let Some(decl) = pub_item_decl(file, parent, item) {
        out.push(decl);
    }
    match item {
        Item::Enum(i) if is_pub(&i.vis) => collect_enum_variants(file, &i.ident, i, out),
        // Inherent impl. Trait impls produce signature-driven
        // dispatch, so flagging individual methods would always be a
        // false positive — skip those entirely.
        Item::Impl(i) if i.trait_.is_none() => collect_inherent_impl(file, i, out),
        Item::Mod(m) => collect_mod(file, parent, m, out),
        _ => {}
    }
}

fn collect_enum_variants(
    file: &str,
    enum_ident: &syn::Ident,
    item: &syn::ItemEnum,
    out: &mut Vec<DeclSite>,
) {
    let enum_name = enum_ident.to_string();
    for v in &item.variants {
        out.push(make_decl(file, Some(&enum_name), &v.ident, "variant", false));
    }
}

fn collect_inherent_impl(file: &str, item: &syn::ItemImpl, out: &mut Vec<DeclSite>) {
    let parent_name = type_path_last_segment(&item.self_ty);
    for ii in &item.items {
        collect_impl_item(file, parent_name.as_deref(), ii, out);
    }
}

fn collect_mod(file: &str, parent: Option<&str>, item: &syn::ItemMod, out: &mut Vec<DeclSite>) {
    if let Some((_, items)) = &item.content {
        collect_decls(file, parent, items, out);
    }
}

/// Builds the [`DeclSite`] for a `pub` top-level [`Item`] when one is
/// warranted. The Item-variant breadth is unavoidable (`syn::Item` has
/// 8 declaration kinds the heuristic surfaces), but each arm is a
/// plain tuple read so the per-arm cost stays at one `make_decl` call.
fn pub_item_decl(file: &str, parent: Option<&str>, item: &Item) -> Option<DeclSite> {
    let (ident, kind, is_root): (&syn::Ident, &'static str, bool) = match item {
        Item::Fn(i) if is_pub(&i.vis) => {
            (&i.sig.ident, "fn", is_fn_root(&i.sig.ident, &i.attrs))
        }
        Item::Struct(i) if is_pub(&i.vis) => (&i.ident, "struct", false),
        Item::Enum(i) if is_pub(&i.vis) => (&i.ident, "enum", false),
        Item::Trait(i) if is_pub(&i.vis) => (&i.ident, "trait", false),
        Item::Type(i) if is_pub(&i.vis) => (&i.ident, "type", false),
        Item::Const(i) if is_pub(&i.vis) => (&i.ident, "const", false),
        Item::Static(i) if is_pub(&i.vis) => {
            (&i.ident, "static", i.attrs.iter().any(is_root_attr))
        }
        Item::Union(i) if is_pub(&i.vis) => (&i.ident, "union", false),
        _ => return None,
    };
    Some(make_decl(file, parent, ident, kind, is_root))
}

/// Builds a [`DeclSite`] with the boilerplate fields populated from
/// the call site. `kind` is a stable string written verbatim to the
/// report; `is_root` is the entry-point classification.
fn make_decl(
    file: &str,
    parent: Option<&str>,
    ident: &syn::Ident,
    kind: &'static str,
    is_root: bool,
) -> DeclSite {
    DeclSite {
        file: file.to_string(),
        line: ident.span().start().line,
        name: ident.to_string(),
        kind,
        parent: parent.map(str::to_string),
        is_root,
    }
}

fn collect_impl_item(
    file: &str,
    parent: Option<&str>,
    item: &ImplItem,
    out: &mut Vec<DeclSite>,
) {
    match item {
        ImplItem::Fn(f) if is_pub(&f.vis) => {
            let is_root = is_fn_root(&f.sig.ident, &f.attrs);
            out.push(make_decl(file, parent, &f.sig.ident, "method", is_root));
        }
        ImplItem::Const(c) if is_pub(&c.vis) => {
            out.push(make_decl(file, parent, &c.ident, "assoc-const", false));
        }
        _ => {}
    }
}

/// Returns the last segment of the impl's self type, used as the
/// `parent:` field on impl-item decls. `impl Foo<T>` → `Foo`. When the
/// self type isn't a simple path (e.g. `impl (A, B)`) we return `None`
/// — the methods still get collected, just without a parent label.
fn type_path_last_segment(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Single-segment attribute names that mark the bearer as an entry
/// point (built-in test runners, FFI exports, proc-macro registry).
/// Kept as a `const` table so adding one is a single line.
const ROOT_SINGLE_SEGMENT_ATTRS: &[&str] = &[
    "test",
    "bench",
    "no_mangle",
    "export_name",
    "start",
    "proc_macro",
    "proc_macro_derive",
    "proc_macro_attribute",
];

/// `true` when the function should be treated as an entry point and
/// excluded from the unused report. `fn main` is hardcoded; the
/// rest are attribute-driven.
fn is_fn_root(ident: &syn::Ident, attrs: &[Attribute]) -> bool {
    ident == "main" || attrs.iter().any(is_root_attr)
}

fn is_root_attr(attr: &Attribute) -> bool {
    let path = attr.path();
    if ROOT_SINGLE_SEGMENT_ATTRS
        .iter()
        .any(|name| path.is_ident(name))
    {
        return true;
    }
    // Two-segment forms used by external crates: `ctor::ctor` /
    // `ctor::dtor`, `tokio::main`, `async_std::main`. We honour any
    // `xxx::main` so adding an async runtime doesn't need a new
    // entry here.
    let Some([first, last]) = two_segment_names(path) else {
        return false;
    };
    (first == "ctor" && (last == "ctor" || last == "dtor")) || last == "main"
}

/// Returns the two segment names of a path-attribute when the path is
/// exactly two segments long, otherwise `None`. Pulled out so the
/// branching in [`is_root_attr`] stays linear.
fn two_segment_names(path: &syn::Path) -> Option<[String; 2]> {
    if path.segments.len() != 2 {
        return None;
    }
    Some([
        path.segments[0].ident.to_string(),
        path.segments[1].ident.to_string(),
    ])
}

/// Visits every `Path`, `ExprMethodCall`, named-member `ExprField`,
/// and `UseTree`, accumulating a name → use-count map. Declaration
/// idents (`fn foo`, `struct Foo`, …) are *not* `Path` nodes and
/// therefore never inflate their own count; only the *uses* of those
/// names get credited.
#[derive(Default)]
struct ReferenceCollector {
    counts: HashMap<String, u32>,
}

impl<'ast> Visit<'ast> for ReferenceCollector {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        if let Some(seg) = p.segments.last() {
            *self
                .counts
                .entry(seg.ident.to_string())
                .or_insert(0) += 1;
        }
        visit::visit_path(self, p);
    }

    fn visit_expr_method_call(&mut self, c: &'ast syn::ExprMethodCall) {
        *self.counts.entry(c.method.to_string()).or_insert(0) += 1;
        visit::visit_expr_method_call(self, c);
    }

    fn visit_expr_field(&mut self, f: &'ast syn::ExprField) {
        if let syn::Member::Named(name) = &f.member {
            *self.counts.entry(name.to_string()).or_insert(0) += 1;
        }
        visit::visit_expr_field(self, f);
    }

    fn visit_use_tree(&mut self, t: &'ast syn::UseTree) {
        // `pub use foo::bar::Baz` is a reference to `Baz` (the leaf
        // re-exported name). UseTree itself doesn't contain a Path,
        // so visit_path never fires on it; we walk the chain by hand
        // and credit only the leaf, which is the declaration name
        // being kept alive. The rename target in `use Foo as Bar`
        // is a new local name, not a reference.
        //
        // We deliberately don't fall back to `visit::visit_use_tree`
        // here — the default impl recurses through child UseTrees and
        // would re-enter this override, double-counting the leaf at
        // every level of nesting. UseTree contains no Path or Type
        // children that need visiting beyond what we already do.
        walk_use_tree(t, &mut self.counts);
    }
}

fn walk_use_tree(t: &syn::UseTree, counts: &mut HashMap<String, u32>) {
    match t {
        syn::UseTree::Path(p) => walk_use_tree(&p.tree, counts),
        syn::UseTree::Name(n) => {
            *counts.entry(n.ident.to_string()).or_insert(0) += 1;
        }
        syn::UseTree::Rename(r) => {
            *counts.entry(r.ident.to_string()).or_insert(0) += 1;
        }
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(g) => {
            for inner in &g.items {
                walk_use_tree(inner, counts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("parse")
    }

    fn ref_counts(src: &str) -> HashMap<String, u32> {
        let ast = parse(src);
        let mut c = ReferenceCollector::default();
        c.visit_file(&ast);
        c.counts
    }

    fn decls(src: &str) -> Vec<DeclSite> {
        let ast = parse(src);
        let mut out = Vec::new();
        collect_decls("t.rs", None, &ast.items, &mut out);
        out
    }

    #[test]
    fn top_level_pub_items_are_collected() {
        let src = "pub fn f() {} pub struct S; pub enum E {} pub trait T {} \
                   pub type A = u8; pub const C: u8 = 1; pub static SS: u8 = 1; \
                   pub union U { a: u8, b: u8 }";
        let kinds: Vec<&str> = decls(src).iter().map(|d| d.kind).collect();
        for kind in ["fn", "struct", "enum", "trait", "type", "const", "static", "union"] {
            assert!(kinds.contains(&kind), "missing {kind} in {kinds:?}");
        }
    }

    #[test]
    fn private_items_are_skipped() {
        assert!(decls("fn f() {} struct S; enum E {} const C: u8 = 1;").is_empty());
    }

    #[test]
    fn enum_variants_are_decls_with_parent() {
        let src = "pub enum E { A, B(u8), C { x: u8 } }";
        let all = decls(src);
        let variants: Vec<&DeclSite> = all.iter().filter(|d| d.kind == "variant").collect();
        let names: Vec<&str> = variants.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C"]);
        for d in variants {
            assert_eq!(d.parent.as_deref(), Some("E"));
        }
    }

    #[test]
    fn private_enum_variants_are_skipped() {
        // `pub enum` exposes its variants; a private enum's variants
        // are not part of the public surface.
        let src = "enum E { A, B }";
        let any_variant = decls(src).iter().any(|d| d.kind == "variant");
        assert!(!any_variant);
    }

    #[test]
    fn inherent_impl_pub_methods_are_collected() {
        let src = "pub struct Foo; impl Foo { pub fn m() {} pub const K: u8 = 1; \
                   fn private() {} }";
        let all = decls(src);
        let methods: Vec<&DeclSite> = all.iter().filter(|d| d.kind == "method").collect();
        let names: Vec<&str> = methods.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["m"]);
        assert_eq!(methods[0].parent.as_deref(), Some("Foo"));
        let consts: Vec<&DeclSite> = all.iter().filter(|d| d.kind == "assoc-const").collect();
        assert_eq!(consts.len(), 1);
        assert_eq!(consts[0].name, "K");
        assert_eq!(consts[0].parent.as_deref(), Some("Foo"));
    }

    #[test]
    fn trait_impl_methods_are_not_collected() {
        // Trait impls produce dispatched methods; flagging them as
        // unused would always be a false positive.
        let src = "pub struct Foo; pub trait T { fn m(); } \
                   impl T for Foo { fn m() {} }";
        let all = decls(src);
        let methods: Vec<&DeclSite> = all.iter().filter(|d| d.kind == "method").collect();
        assert!(methods.is_empty());
    }

    #[test]
    fn nested_module_decls_are_recursed_into() {
        let src = "pub mod inner { pub fn deep() {} pub struct Hidden; }";
        let all = decls(src);
        let names: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"deep"));
        assert!(names.contains(&"Hidden"));
    }

    #[test]
    fn fn_main_is_marked_as_root() {
        let src = "pub fn main() {}";
        let d = decls(src);
        assert!(d[0].is_root);
    }

    #[test]
    fn test_attr_marks_root() {
        let src = "#[test] pub fn checks_things() {}";
        let d = decls(src);
        assert!(d[0].is_root);
    }

    #[test]
    fn proc_macro_attrs_mark_root() {
        for attr in ["proc_macro", "proc_macro_derive", "proc_macro_attribute"] {
            let src = format!("#[{attr}] pub fn handler(input: TokenStream) -> TokenStream {{ input }}");
            let d = decls(&src);
            assert!(d[0].is_root, "{attr} did not mark root");
        }
    }

    #[test]
    fn no_mangle_static_is_root() {
        let src = "#[no_mangle] pub static GLOBAL: u8 = 1;";
        let d = decls(src);
        assert!(d[0].is_root);
    }

    #[test]
    fn ctor_attr_marks_root() {
        let src = "#[ctor::ctor] pub fn boot() {}";
        let d = decls(src);
        assert!(d[0].is_root);
    }

    #[test]
    fn xxx_main_two_segment_attr_marks_root() {
        // `tokio::main`, `async_std::main`: anything ending in `::main`
        // is treated as an async-runtime entry attr.
        let src = "#[tokio::main] pub async fn run() {}";
        let d = decls(src);
        assert!(d[0].is_root);
    }

    #[test]
    fn ref_counter_counts_only_path_last_segment() {
        // We treat the last segment as the "name being referenced"
        // because that's the segment that matches a declaration we
        // collect. Intermediate qualifiers (`std`, `io`) are not
        // declarations the heuristic surfaces, so leaving them out of
        // the count keeps the data shape narrow.
        let counts = ref_counts("fn f() { let _ = std::io::stdin(); }");
        assert_eq!(counts.get("stdin").copied(), Some(1));
        assert_eq!(counts.get("io").copied(), None);
        assert_eq!(counts.get("std").copied(), None);
    }

    #[test]
    fn ref_counter_counts_method_calls_and_field_access() {
        let counts =
            ref_counts("fn f(x: A) { x.method(); let _ = x.field; }");
        assert_eq!(counts.get("method").copied(), Some(1));
        assert_eq!(counts.get("field").copied(), Some(1));
    }

    #[test]
    fn ref_counter_does_not_double_count_decl_idents() {
        // `fn foo` decl ident is not a Path; `foo()` call is. So the
        // ref count for foo is 1 (the call), not 2.
        let counts = ref_counts("fn foo() {} fn caller() { foo(); }");
        assert_eq!(counts.get("foo").copied(), Some(1));
    }

    #[test]
    fn pub_use_chain_increments_reexport() {
        // A `pub use crate::inner::Bar` keeps Bar alive — the path
        // visit picks up the last segment.
        let counts = ref_counts("pub use crate::inner::Bar;");
        assert_eq!(counts.get("Bar").copied(), Some(1));
    }

    #[test]
    fn variant_pattern_is_a_reference() {
        let src = "fn f(v: E) { match v { E::A => {}, _ => {} } }";
        let counts = ref_counts(src);
        // `E::A` path has segments [E, A]; A is the last segment.
        // The type annotation `v: E` produces a separate Path with
        // segments [E]; that's the only one whose last segment is E.
        assert_eq!(counts.get("A").copied(), Some(1));
        assert_eq!(counts.get("E").copied(), Some(1));
    }

    #[test]
    fn detect_flags_unreferenced_pub_fn() {
        let tmp = tempdir();
        write_file(tmp.path(), "src/lib.rs", "pub fn alone() {}\n");
        let items = detect_files(&tmp).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["alone"]);
    }

    #[test]
    fn detect_keeps_referenced_pub_fn_alive() {
        let tmp = tempdir();
        write_file(
            tmp.path(),
            "src/lib.rs",
            "pub fn used() { used_in_b(); }\n",
        );
        write_file(tmp.path(), "src/b.rs", "pub fn used_in_b() {}\n");
        let items = detect_files(&tmp).unwrap();
        // `used_in_b` is referenced from lib.rs, so it stays alive.
        // `used` itself has no caller and *is* flagged.
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"used"));
        assert!(!names.contains(&"used_in_b"));
    }

    #[test]
    fn detect_keeps_main_alive_without_callers() {
        let tmp = tempdir();
        write_file(tmp.path(), "src/main.rs", "pub fn main() {}\n");
        let items = detect_files(&tmp).unwrap();
        assert!(items.is_empty(), "main was flagged: {items:?}");
    }

    #[test]
    fn detect_flags_unused_inherent_method() {
        let tmp = tempdir();
        write_file(
            tmp.path(),
            "src/lib.rs",
            "pub struct Foo; impl Foo { pub fn used(&self) {} pub fn unused(&self) {} }\n\
             pub fn caller(f: &Foo) { f.used(); }\n",
        );
        let items = detect_files(&tmp).unwrap();
        // `unused` has no caller; `used` is called via method-call;
        // `caller` itself has no caller.
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"unused"));
        assert!(names.contains(&"caller"));
        assert!(!names.contains(&"used"));
    }

    #[test]
    fn detect_flags_unused_enum_variant() {
        let tmp = tempdir();
        write_file(
            tmp.path(),
            "src/lib.rs",
            "pub enum E { A, B } \
             pub fn caller(e: E) { match e { E::A => {} _ => {} } }\n",
        );
        let items = detect_files(&tmp).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"B"));
        assert!(!names.contains(&"A"));
    }

    #[test]
    fn format_renders_parent_when_present() {
        let items = vec![
            UnusedItem {
                file: "src/a.rs".into(),
                line: 3,
                name: "method".into(),
                kind: "method",
                parent: Some("Foo".into()),
            },
            UnusedItem {
                file: "src/b.rs".into(),
                line: 9,
                name: "Solo".into(),
                kind: "fn",
                parent: None,
            },
        ];
        let s = format(&items);
        assert!(s.contains("method Foo::method — src/a.rs:3"));
        assert!(s.contains("fn Solo — src/b.rs:9"));
    }

    #[test]
    fn format_says_no_candidates_when_empty() {
        assert_eq!(format(&[]), "rustics unused: no candidates found.\n");
    }

    #[test]
    fn detect_skips_files_that_fail_to_parse() {
        let tmp = tempdir();
        write_file(tmp.path(), "src/lib.rs", "pub fn good() {}\n");
        write_file(tmp.path(), "src/broken.rs", "this is :: not :: rust\n");
        let items = detect_files(&tmp).unwrap();
        assert!(items.iter().any(|i| i.name == "good"));
    }

    #[test]
    fn detect_propagates_read_errors() {
        let files = vec![DiscoveredFile {
            absolute: std::path::PathBuf::from("/no/such/file_for_unused_test.rs"),
            relative: "missing.rs".to_string(),
        }];
        let err = detect(&files).unwrap_err();
        assert!(format!("{err:#}").contains("missing.rs"));
    }

    #[test]
    fn detect_at_walks_workspace_root() {
        let tmp = tempdir();
        write_file(
            tmp.path(),
            "Cargo.toml",
            "[workspace]\nmembers = []\nresolver = \"2\"\n",
        );
        write_file(tmp.path(), "src/lib.rs", "pub fn solitary() {}\n");
        let items = detect_at(tmp.path()).unwrap();
        assert!(items.iter().any(|i| i.name == "solitary"));
    }

    #[test]
    fn type_path_last_segment_returns_none_for_non_path_types() {
        // `impl (u8, u8)` is a tuple-type self; the helper falls back
        // to None and the methods still get collected without a
        // parent label.
        let ty: Type = syn::parse_str("(u8, u8)").unwrap();
        assert_eq!(type_path_last_segment(&ty), None);
        let ty: Type = syn::parse_str("Foo<u8>").unwrap();
        assert_eq!(type_path_last_segment(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn is_root_attr_recognises_known_forms() {
        let attr_test: Attribute = syn::parse_quote!(#[test]);
        let attr_no_mangle: Attribute = syn::parse_quote!(#[no_mangle]);
        let attr_ctor: Attribute = syn::parse_quote!(#[ctor::ctor]);
        let attr_other: Attribute = syn::parse_quote!(#[derive(Debug)]);
        assert!(is_root_attr(&attr_test));
        assert!(is_root_attr(&attr_no_mangle));
        assert!(is_root_attr(&attr_ctor));
        assert!(!is_root_attr(&attr_other));
    }

    fn make_item(name: &str, kind: &'static str) -> UnusedItem {
        UnusedItem {
            file: "src/lib.rs".into(),
            line: 1,
            name: name.into(),
            kind,
            parent: None,
        }
    }

    #[test]
    fn parse_kind_filter_returns_none_when_empty() {
        assert!(parse_kind_filter(&[]).unwrap().is_none());
    }

    #[test]
    fn parse_kind_filter_accepts_known_kinds() {
        let allowed = parse_kind_filter(&["fn".into(), "method".into()])
            .unwrap()
            .unwrap();
        assert_eq!(allowed.len(), 2);
        assert!(allowed.contains("fn"));
        assert!(allowed.contains("method"));
    }

    #[test]
    fn parse_kind_filter_splits_comma_separated_values() {
        // `--filter fn,struct,method` arrives as a single CSV string
        // when the user uses one flag with commas.
        let allowed = parse_kind_filter(&["fn,struct,method".into()])
            .unwrap()
            .unwrap();
        assert_eq!(allowed.len(), 3);
        assert!(allowed.contains("fn"));
        assert!(allowed.contains("struct"));
        assert!(allowed.contains("method"));
    }

    #[test]
    fn parse_kind_filter_trims_whitespace() {
        let allowed = parse_kind_filter(&[" fn , method ".into()])
            .unwrap()
            .unwrap();
        assert!(allowed.contains("fn"));
        assert!(allowed.contains("method"));
    }

    #[test]
    fn parse_kind_filter_skips_empty_chunks() {
        // `--filter fn,,struct` (a typo) should not panic; the empty
        // chunk between the commas is silently skipped.
        let allowed = parse_kind_filter(&["fn,,struct".into()])
            .unwrap()
            .unwrap();
        assert_eq!(allowed.len(), 2);
    }

    #[test]
    fn parse_kind_filter_rejects_unknown_kind() {
        let err = parse_kind_filter(&["functon".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown kind `functon`"));
        assert!(msg.contains("Valid kinds:"));
    }

    #[test]
    fn parse_kind_filter_rejects_first_unknown_in_csv() {
        let err = parse_kind_filter(&["fn,unknown,method".into()]).unwrap_err();
        assert!(format!("{err:#}").contains("unknown kind `unknown`"));
    }

    #[test]
    fn parse_kind_filter_only_whitespace_is_treated_as_empty() {
        // All-whitespace input never adds any kind → still effectively
        // "no filter". The CLI also short-circuits on empty Vec, but
        // mirroring the behaviour here keeps the contract consistent.
        let allowed = parse_kind_filter(&["  , ,".into()]).unwrap();
        assert!(allowed.is_none());
    }

    #[test]
    fn apply_kind_filter_no_op_when_allowed_is_none() {
        let items = vec![make_item("foo", "fn"), make_item("Bar", "struct")];
        let filtered = apply_kind_filter(items.clone(), None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn apply_kind_filter_keeps_only_allowed_kinds() {
        let items = vec![
            make_item("foo", "fn"),
            make_item("Bar", "struct"),
            make_item("baz", "method"),
        ];
        let mut allow = HashSet::new();
        allow.insert("fn".into());
        allow.insert("method".into());
        let filtered = apply_kind_filter(items, Some(&allow));
        let kinds: Vec<&str> = filtered.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, ["fn", "method"]);
    }

    #[test]
    fn known_kinds_covers_every_kind_collect_emits() {
        // Self-app sanity: every kind string the collectors hand to
        // `make_decl` must be in `KNOWN_KINDS`, otherwise the filter
        // would silently drop a valid record. Drives that invariant.
        for kind in [
            "fn",
            "struct",
            "enum",
            "trait",
            "type",
            "const",
            "static",
            "union",
            "variant",
            "method",
            "assoc-const",
        ] {
            assert!(KNOWN_KINDS.contains(&kind), "missing {kind}");
        }
    }

    // -----------------------------------------------------------------
    // Tempdir helpers — kept here so we don't add a dev dep.
    // -----------------------------------------------------------------

    fn write_file(dir: &Path, rel: &str, body: &str) {
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(abs, body).unwrap();
    }

    fn detect_files(tmp: &TempDir) -> Result<Vec<UnusedItem>> {
        let files: Vec<DiscoveredFile> = walk(tmp.path());
        detect(&files)
    }

    fn walk(root: &Path) -> Vec<DiscoveredFile> {
        let mut out = Vec::new();
        walk_inner(root, root, &mut out);
        out
    }

    fn walk_inner(root: &Path, dir: &Path, out: &mut Vec<DiscoveredFile>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_inner(root, &path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(DiscoveredFile {
                    absolute: path,
                    relative,
                });
            }
        }
    }

    fn tempdir() -> TempDir {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rustics-unused-test-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
