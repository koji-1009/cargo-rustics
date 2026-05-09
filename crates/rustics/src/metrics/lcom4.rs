//! `lcom4` — Lack of Cohesion in Methods, version 4 (Hitz & Montazeri 1995).
//!
//! Counts the number of disjoint connected components in the method
//! graph of an `impl` block. Two methods are in the same component
//! when they share at least one `self`-field access, OR when one
//! calls the other. LCOM4 = 1 means "fully cohesive" (every method
//! reaches every other through field/call sharing); LCOM4 ≥ 2 means
//! the impl block has multiple disjoint method clusters that could
//! be split into separate types.
//!
//! Why LCOM4 (vs the original LCOM by CK 1994)? CK's LCOM was
//! defined as `|P| - |Q|` clamped to 0, where P = method pairs with
//! no shared field and Q = pairs that share at least one. That
//! version produces 0 for many cohesive *and* incohesive classes,
//! defeating the metric. Hitz & Montazeri's LCOM4 fixes this by
//! using component count directly — proven robust across multiple
//! validation studies.
//!
//! Rust mapping: an inherent `impl T { … }` block plays the role of
//! a class. Trait impls (`impl Trait for T { … }`) are *skipped* —
//! the method set there is dictated by the trait contract, not a
//! cohesion choice the author can refactor; flagging them would
//! report visitor / `Iterator` / `Display` impls as low-cohesion
//! when the multi-method shape is the trait's API surface, not a
//! code smell.
//! - Methods = `fn` items in the inherent block (including `&self`,
//!   `&mut self`, `self`, and associated functions).
//! - Field-share edge: methods both access `self.<name>` for some
//!   shared `<name>`. Associated functions (no `self`) cannot share
//!   fields, but still get connected via the call-edge rule below.
//! - Call edge: method `a` calls method `b` of the same impl
//!   (`self.b(...)`).
//!
//! ## Known limitations (AST-only)
//!
//! * **Aliased self**: `let s = self; s.field` is invisible — only
//!   the bare keyword `self` is recognised as the receiver.
//!   Aliasing through a binding requires name resolution that an
//!   AST-only lens does not have.
//! * **Qualified self paths**: `<Self as Trait>::method()`
//!   (`ExprPath { qself: Some(_), .. }`) is not counted as a call
//!   edge; the visitor matches only the bare `Self::method`
//!   form. False negative on the disambiguation idiom.
//! * **Macros**: tokens *inside* macro invocations
//!   (`vec![self.x; n]`, `format!("{}", self.field)`) are not
//!   walked by `syn::Visit`, so field accesses hidden in macro
//!   bodies don't connect methods.
//!
//! References:
//! * Hitz & Montazeri (1995). "Measuring coupling and cohesion in
//!   object-oriented systems". Proc. Int. Symp. on Applied Corporate
//!   Computing.
//! * Marinescu (2002). "Measurement and quality in object-oriented
//!   design" — LCOM4 validated as defect predictor.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{ExprCall, ExprField, ExprMethodCall, ExprPath, ExprStruct, ImplItem, ImplItemFn};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// `lcom4` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lcom4;

impl MetricCalculator for Lcom4 {
    fn id(&self) -> &'static str {
        "lcom4"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Lack of Cohesion in Methods, v4 (Hitz & Montazeri 1995)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            // 1 is fully cohesive. 2+ → at least one cluster is
            // separable. We warn at 2 (per Hitz & Montazeri's
            // "needs review") and error at 5 (effectively "the
            // impl block is several types in disguise").
            default_warning: Some(Threshold::new(2.0)),
            default_error: Some(Threshold::new(5.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.ast, |frame| {
            // Skip trait impls — the method set is dictated by the
            // trait contract, not a cohesion choice. See module-level
            // doc.
            if frame.item.trait_.is_some() {
                return None;
            }
            let methods = collect_method_signatures(frame.item);
            if methods.is_empty() {
                return Some(0.0);
            }
            let edges = build_method_graph(frame.item, &methods);
            let components = count_components(methods.len(), &edges);
            Some(components as f64)
        })
    }
}

const RATIONALE: &str = "\
LCOM4 (Hitz & Montazeri 1995) counts disjoint method clusters in an \
inherent impl block — methods that don't share any field access or \
call relationship. A cohesive impl scores 1: every method reaches \
every other through some chain of shared state or calls. Score ≥ 2 \
means the block has independent method clusters that could be split \
into separate types without losing anything. \
Trait impls are skipped: their method set is dictated by the trait \
contract, not a cohesion choice the author can refactor. The metric \
was validated by Marinescu (2002) as a defect-density predictor \
superior to the original CK LCOM definition.";

const REFACTOR_HINTS: &[&str] = &[
    "Group the disjoint clusters into separate types: each cluster \
becomes a struct that owns the fields its methods touch.",
    "If one cluster is a small constructor + helper pair, move it \
into a free function or an `impl T` block dedicated to that role.",
    "Methods that touch *no* fields and aren't called by other methods \
in the impl form their own singleton component. Consider whether \
they belong on the type at all — they might be better as free \
functions.",
];

const REFERENCES: &[&str] = &[
    "Hitz & Montazeri (1995). Measuring coupling and cohesion in \
object-oriented systems. Proc. Int. Symp. on Applied Corporate Computing.",
    "Marinescu (2002). Measurement and quality in object-oriented design \
— validation of LCOM4 as defect predictor.",
];

/// One method's identity. Receiver kind doesn't matter for LCOM4
/// component-counting: associated functions still get connected
/// via inbound `self.method(…)` call edges from other methods, and
/// they cannot field-share since they have no `self` of their own.
#[derive(Debug, Clone)]
struct MethodInfo {
    name: String,
}

/// Collects every `fn` item in the impl block in source order.
fn collect_method_signatures(item: &syn::ItemImpl) -> Vec<MethodInfo> {
    item.items
        .iter()
        .filter_map(|i| match i {
            ImplItem::Fn(f) => Some(MethodInfo {
                name: f.sig.ident.to_string(),
            }),
            _ => None,
        })
        .collect()
}

/// Set of `(field_name, method_index)` access events + method-call
/// graph; the union is what defines connectedness.
type Edges = Vec<(usize, usize)>;

fn build_method_graph(item: &syn::ItemImpl, methods: &[MethodInfo]) -> Edges {
    let mut accesses: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut calls: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let name_to_index: BTreeMap<&str, usize> = methods
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();
    let mut method_idx = 0usize;
    for item_inner in &item.items {
        let ImplItem::Fn(method) = item_inner else {
            continue;
        };
        scan_method_body(method, method_idx, &mut accesses, &mut calls, &name_to_index);
        method_idx += 1;
    }
    let mut edges: Edges = Vec::new();
    push_field_share_edges(&accesses, &mut edges);
    push_call_edges(&calls, &mut edges);
    edges
}

/// Records every `self.<field>` read and every same-impl method call
/// inside `method` (which has index `idx` in the methods vector).
fn scan_method_body(
    method: &ImplItemFn,
    idx: usize,
    accesses: &mut BTreeMap<String, BTreeSet<usize>>,
    calls: &mut BTreeMap<usize, BTreeSet<usize>>,
    name_to_index: &BTreeMap<&str, usize>,
) {
    let mut v = MethodWalker {
        idx,
        accesses,
        calls,
        name_to_index,
    };
    v.visit_block(&method.block);
}

struct MethodWalker<'a> {
    idx: usize,
    accesses: &'a mut BTreeMap<String, BTreeSet<usize>>,
    calls: &'a mut BTreeMap<usize, BTreeSet<usize>>,
    name_to_index: &'a BTreeMap<&'a str, usize>,
}

impl<'a, 'ast> Visit<'ast> for MethodWalker<'a> {
    // A nested `impl T { … }` introduces a *different* `Self`. Its
    // `self.<field>` / `Self { … }` / `Self::method(…)` references
    // belong to the inner type, not the outer impl we're measuring.
    // Stop recursion at the nested impl boundary so those don't leak
    // into the outer impl's accesses/calls maps.
    fn visit_item_impl(&mut self, _node: &'ast syn::ItemImpl) {}
    // Same for nested function items: an `fn helper() { let s = …; … }`
    // declared inside a method body has its own `self` binding (it
    // can't refer to the outer impl's `self`). Don't walk it.
    fn visit_item_fn(&mut self, _node: &'ast syn::ItemFn) {}

    fn visit_expr_field(&mut self, node: &'ast ExprField) {
        // self.<field> — base must be `self`. Both named (`self.x`)
        // and numeric (`self.0`) members count: tuple-struct field
        // sharing is the same connectivity signal as named-field
        // sharing.
        if is_self_path(&node.base) {
            self.accesses
                .entry(member_name(&node.member))
                .or_default()
                .insert(self.idx);
        }
        visit::visit_expr_field(self, node);
    }

    fn visit_expr_struct(&mut self, node: &'ast ExprStruct) {
        // `Self { x: …, y: … }` — every named-field initializer
        // counts as an access to that field. This is how `new()` /
        // `default()` constructors connect to accessor methods that
        // later read the same field via `self.x`.
        if path_is_self(&node.path) {
            for f in &node.fields {
                self.accesses
                    .entry(member_name(&f.member))
                    .or_default()
                    .insert(self.idx);
            }
        }
        visit::visit_expr_struct(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        // self.<method>(...) — connect to that method by index.
        if is_self_path(&node.receiver) {
            let target = node.method.to_string();
            if let Some(&j) = self.name_to_index.get(target.as_str()) {
                if j != self.idx {
                    self.calls.entry(self.idx).or_default().insert(j);
                }
            }
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        // Either `Self::method(…)` (path-call to an associated
        // function) or `Self(…)` (tuple-struct construction). The
        // tuple-struct case treats each positional argument as an
        // access to field "0", "1", …, mirroring the named-field
        // initializer rule above so a `new()` that returns
        // `Self(0, 0)` connects to `self.0` / `self.1` accessors.
        if let syn::Expr::Path(ExprPath { path, qself: None, .. }) = node.func.as_ref() {
            if path_is_self(path) {
                // Tuple-struct construction: positional field accesses.
                for i in 0..node.args.len() {
                    self.accesses
                        .entry(i.to_string())
                        .or_default()
                        .insert(self.idx);
                }
            } else if path.segments.len() == 2 && path.segments[0].ident == "Self" {
                // Self::method(…) — call edge into the impl.
                let target = path.segments[1].ident.to_string();
                if let Some(&j) = self.name_to_index.get(target.as_str()) {
                    if j != self.idx {
                        self.calls.entry(self.idx).or_default().insert(j);
                    }
                }
            }
        }
        visit::visit_expr_call(self, node);
    }
}

/// Stringifies a struct/tuple-struct member: named members keep the
/// identifier; tuple positions become "0", "1", …. The same key
/// space is used for both `self.<field>` reads and `Self { … }` /
/// `Self(…)` constructor field initialisers.
fn member_name(member: &syn::Member) -> String {
    match member {
        syn::Member::Named(ident) => ident.to_string(),
        syn::Member::Unnamed(index) => index.index.to_string(),
    }
}

/// True iff `expr` is the bare receiver `self`.
fn is_self_path(expr: &syn::Expr) -> bool {
    if let syn::Expr::Path(ExprPath { path, qself: None, .. }) = expr {
        if path.segments.len() == 1 && path.segments[0].ident == "self" {
            return true;
        }
    }
    false
}

/// True iff `path` is the bare type-path `Self` (used in struct
/// literals and tuple-construction calls).
fn path_is_self(path: &syn::Path) -> bool {
    path.segments.len() == 1 && path.segments[0].ident == "Self"
}

/// For each field touched by ≥ 2 methods, emit edges between every
/// pair of those methods.
fn push_field_share_edges(
    accesses: &BTreeMap<String, BTreeSet<usize>>,
    out: &mut Edges,
) {
    for methods in accesses.values() {
        let v: Vec<usize> = methods.iter().copied().collect();
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                out.push((v[i], v[j]));
            }
        }
    }
}

/// Method-call edges go both directions for the cohesion graph
/// (LCOM4 is undirected).
fn push_call_edges(calls: &BTreeMap<usize, BTreeSet<usize>>, out: &mut Edges) {
    for (&from, targets) in calls {
        for &to in targets {
            out.push((from, to));
        }
    }
}

/// Counts connected components via union-find.
fn count_components(node_count: usize, edges: &Edges) -> usize {
    if node_count == 0 {
        return 0;
    }
    let mut parent: Vec<usize> = (0..node_count).collect();
    for &(a, b) in edges {
        union(&mut parent, a, b);
    }
    let mut roots: BTreeSet<usize> = BTreeSet::new();
    for i in 0..node_count {
        roots.insert(find(&mut parent, i));
    }
    roots.len()
}

/// Iterative two-pass union-find lookup with path compression.
///
/// First pass climbs to the root; second pass re-walks the chain
/// and points every visited node directly at the root. Iterative
/// (not recursive) so a pathological method graph cannot blow the
/// stack — defensive even though path compression keeps real-world
/// depth at O(log n).
fn find(parent: &mut [usize], x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    let mut current = x;
    while parent[current] != root {
        let next = parent[current];
        parent[current] = root;
        current = next;
    }
    root
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let ra = find(parent, a);
    let rb = find(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        Lcom4.measure(&input)
    }

    fn lcom_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_impl_is_zero() {
        // Hitz & Montazeri define LCOM4 only for non-empty classes;
        // zero methods → score 0 (no work to do).
        let src = "struct Foo; impl Foo {}";
        assert_eq!(lcom_of(src, "Foo"), 0);
    }

    #[test]
    fn cohesive_class_scores_one() {
        // a touches `x`, b touches `x` → field share connects them.
        // c calls a → call edge connects to the cluster. All in
        // one component → LCOM4 = 1.
        let src = r#"
            struct Foo { x: i32 }
            impl Foo {
                fn a(&self) -> i32 { self.x }
                fn b(&self) -> i32 { self.x + 1 }
                fn c(&self) -> i32 { self.a() + 2 }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 1);
    }

    #[test]
    fn two_disjoint_clusters_score_two() {
        // a, b share `x`. c, d share `y`. No call edges. Two
        // disjoint components.
        let src = r#"
            struct Foo { x: i32, y: i32 }
            impl Foo {
                fn a(&self) -> i32 { self.x }
                fn b(&self) -> i32 { self.x * 2 }
                fn c(&self) -> i32 { self.y }
                fn d(&self) -> i32 { self.y + 1 }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 2);
    }

    #[test]
    fn singleton_method_no_state_is_its_own_component() {
        // a touches `x`, b touches `x` (cluster 1). c touches no
        // fields and is not called → isolated component.
        let src = r#"
            struct Foo { x: i32 }
            impl Foo {
                fn a(&self) -> i32 { self.x }
                fn b(&self) -> i32 { self.x + 1 }
                fn c() -> i32 { 0 }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 2);
    }

    #[test]
    fn call_edge_alone_connects_methods() {
        // No shared fields; b calls a. Still one component.
        let src = r#"
            struct Foo;
            impl Foo {
                fn a(&self) -> i32 { 1 }
                fn b(&self) -> i32 { self.a() }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 1);
    }

    #[test]
    fn self_struct_literal_counts_as_field_access() {
        // `new()` initializes `x` and `y` via `Self { x: …, y: … }`.
        // `get_x` reads `self.x`. `get_y` reads `self.y`. With the
        // Rust-aware extension, all three connect through `new` →
        // LCOM4 = 1.
        let src = r#"
            struct Foo { x: i32, y: i32 }
            impl Foo {
                fn new() -> Self { Self { x: 0, y: 0 } }
                fn get_x(&self) -> i32 { self.x }
                fn get_y(&self) -> i32 { self.y }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 1);
    }

    #[test]
    fn self_path_call_counts_as_call_edge() {
        // `outer` calls `Self::inner(...)`. With the Rust-aware
        // extension, that's a call edge → 1 component.
        let src = r#"
            struct Foo;
            impl Foo {
                fn inner() -> i32 { 0 }
                fn outer() -> i32 { Self::inner() + 1 }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 1);
    }

    #[test]
    fn trait_impls_are_skipped() {
        // The MetricCalculator-style trait impl below has 3 disjoint
        // methods (no shared fields, no internal calls). LCOM4 should
        // ignore it because the method set is dictated by the trait
        // contract, not a cohesion choice — so no measurement at all.
        let src = r#"
            struct S;
            trait T { fn id(&self); fn metadata(&self); fn measure(&self); }
            impl T for S {
                fn id(&self) { }
                fn metadata(&self) { }
                fn measure(&self) { }
            }
        "#;
        let measurements = measure(src);
        assert!(
            measurements.is_empty(),
            "trait impl unexpectedly measured: {measurements:?}"
        );
    }

    #[test]
    fn nested_impl_inside_method_body_does_not_leak() {
        // Pre-fix: the walker recursed into `impl Inner { fn h() { self.y } }`
        // declared inside `b`'s body, recording `self.y` as if `b`
        // touched outer Foo's `y`. That falsely connected `b` and
        // `c` (which legitimately reads self.y), giving LCOM4 = 1.
        // After the fix: the nested impl is opaque to the walker; b
        // and c remain in separate components → LCOM4 = 2.
        let src = r#"
            struct Foo { x: i32, y: i32 }
            impl Foo {
                fn a(&self) -> i32 { self.x }
                fn b(&self) {
                    struct Inner { y: i32 }
                    impl Inner { fn h(&self) -> i32 { self.y } }
                }
                fn c(&self) -> i32 { self.x + self.y }
            }
        "#;
        // a + c share `x` → cluster {a, c}. b is its own component.
        assert_eq!(lcom_of(src, "Foo"), 2);
    }

    #[test]
    fn tuple_struct_self_construction_connects_accessors() {
        // Pre-fix: `Self(0, 0)` (an ExprCall, not an ExprStruct) was
        // invisible to the constructor↔field-access connectivity
        // rule, and `self.0` / `self.1` (Member::Unnamed) were
        // dropped by visit_expr_field. So a tuple-struct with a
        // constructor + two accessors scored LCOM4 = 3 (three
        // singletons). After the fix: positional accesses connect
        // through `new`, score = 1.
        let src = r#"
            struct Foo(i32, i32);
            impl Foo {
                fn new() -> Self { Self(0, 0) }
                fn get0(&self) -> i32 { self.0 }
                fn get1(&self) -> i32 { self.1 }
            }
        "#;
        assert_eq!(lcom_of(src, "Foo"), 1);
    }

    #[test]
    fn metadata_cites_hitz_and_montazeri() {
        let md = Lcom4.metadata();
        assert!(md.references.iter().any(|r| r.contains("Hitz")));
        assert!(md.references.iter().any(|r| r.contains("1995")));
    }
}
