//! LCOM4 (Hitz & Montazeri 1995) — connected components in the
//! method graph of an inherent `impl` block. Two methods are
//! linked when they share at least one `self.<field>` access OR
//! when one calls the other.

use std::collections::HashSet;

use ra_ap_syntax::{
    ast::{self, AstNode, HasName},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// LCOM4 calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lcom4;

impl MetricCalculator for Lcom4 {
    fn id(&self) -> &'static str {
        "lcom4"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "LCOM4 (Hitz & Montazeri 1995)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(2.0)),
            default_error: Some(Threshold::new(4.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.tree, |frame| {
            if frame.item.trait_().is_some() {
                return None;
            }
            let al = frame.item.assoc_item_list()?;
            let methods = collect_methods(&al);
            if methods.len() < 2 {
                return None;
            }
            Some(connected_components(&methods) as f64)
        })
    }
}

struct MethodInfo {
    name: String,
    fields: HashSet<String>,
    callees: HashSet<String>,
}

fn collect_methods(al: &ast::AssocItemList) -> Vec<MethodInfo> {
    let mut out = Vec::new();
    for item in al.assoc_items() {
        let ast::AssocItem::Fn(f) = item else {
            continue;
        };
        let Some(name_node) = f.name() else { continue };
        let mut fields = HashSet::new();
        let mut callees = HashSet::new();
        if let Some(body) = f.body() {
            collect_self_refs(body.syntax(), &mut fields, &mut callees);
        }
        out.push(MethodInfo {
            name: name_node.text().to_string(),
            fields,
            callees,
        });
    }
    out
}

fn collect_self_refs(
    node: &SyntaxNode,
    fields: &mut HashSet<String>,
    callees: &mut HashSet<String>,
) {
    for desc in node.descendants() {
        match desc.kind() {
            SyntaxKind::FIELD_EXPR => record_self_field(&desc, fields),
            SyntaxKind::METHOD_CALL_EXPR => record_self_method(&desc, callees),
            _ => {}
        }
    }
}

fn record_self_field(node: &SyntaxNode, fields: &mut HashSet<String>) {
    let Some(f) = ast::FieldExpr::cast(node.clone()) else {
        return;
    };
    if !is_self_receiver(&f.expr()) {
        return;
    }
    if let Some(name) = f.name_ref() {
        fields.insert(name.text().to_string());
    }
}

fn record_self_method(node: &SyntaxNode, callees: &mut HashSet<String>) {
    let Some(c) = ast::MethodCallExpr::cast(node.clone()) else {
        return;
    };
    if !is_self_receiver(&c.receiver()) {
        return;
    }
    if let Some(name) = c.name_ref() {
        callees.insert(name.text().to_string());
    }
}

fn is_self_receiver(expr: &Option<ast::Expr>) -> bool {
    match expr {
        Some(ast::Expr::PathExpr(p)) => p
            .path()
            .and_then(|p| p.segment())
            .and_then(|s| s.name_ref())
            .is_some_and(|n| n.text() == "self"),
        _ => false,
    }
}

/// Returns the number of disjoint connected components in the
/// method graph. Edges: shared self-field access OR method calls.
/// Path-compressed union-find (Tarjan 1975); see `find_root` /
/// `union` for the per-step primitives.
fn connected_components(methods: &[MethodInfo]) -> u32 {
    let mut parent: Vec<usize> = (0..methods.len()).collect();
    for i in 0..methods.len() {
        for j in (i + 1)..methods.len() {
            if shares_state(&methods[i], &methods[j]) {
                union(&mut parent, i, j);
            }
        }
    }
    let mut roots: HashSet<usize> = HashSet::new();
    for i in 0..methods.len() {
        roots.insert(find_root(&mut parent, i));
    }
    roots.len() as u32
}

/// Two methods share state when they touch at least one common
/// `self.<field>` *or* one calls the other through `self`. Either
/// edge type unions them into the same component.
fn shares_state(a: &MethodInfo, b: &MethodInfo) -> bool {
    a.fields.intersection(&b.fields).next().is_some()
        || a.callees.contains(&b.name)
        || b.callees.contains(&a.name)
}

/// Path-compressed `find` over the union-find parent vector.
fn find_root(parent: &mut [usize], i: usize) -> usize {
    if parent[i] != i {
        parent[i] = find_root(parent, parent[i]);
    }
    parent[i]
}

/// Union by replacement — point `a`'s root at `b`'s root.
fn union(parent: &mut [usize], a: usize, b: usize) {
    let ra = find_root(parent, a);
    let rb = find_root(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

const RATIONALE: &str = "\
LCOM4 (Hitz & Montazeri 1995) reports the number of disjoint method \
clusters in an inherent `impl` block. LCOM4 = 1 means every method \
reaches every other through field/call sharing; LCOM4 ≥ 2 suggests \
the impl could split into separate types.";

const REFACTOR_HINTS: &[&str] = &[
    "If methods cluster around two disjoint field sets, split the impl into two struct + impl pairs.",
    "If a cluster of methods shares no state with the rest, move them onto a sibling type the original delegates to.",
];

const REFERENCES: &[&str] = &[
    "Hitz, M., & Montazeri, B. (1995). Measuring Coupling and Cohesion In Object-Oriented Systems.",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        Lcom4.measure(&input)
    }

    #[test]
    fn empty_impl_yields_no_measurement() {
        let m = measure("struct S; impl S {}");
        assert!(m.is_empty(), "no methods should yield no measurement");
    }

    #[test]
    fn single_method_impl_yields_no_measurement() {
        // LCOM4 needs ≥ 2 methods to make a connected-components claim.
        let m = measure("struct S; impl S { fn a(&self) {} }");
        assert!(m.is_empty());
    }

    #[test]
    fn cohesive_impl_is_lcom_one() {
        // Both methods touch `self.x` — one connected component.
        let src = "struct S { x: i32 } \
                   impl S { \
                       fn a(&self) -> i32 { self.x } \
                       fn b(&self) -> i32 { self.x + 1 } \
                   }";
        let m = measure(src);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].value, 1.0);
    }

    #[test]
    fn disjoint_methods_are_lcom_two() {
        // `a` touches `x`, `b` touches `y`, no calls — two components.
        let src = "struct S { x: i32, y: i32 } \
                   impl S { \
                       fn a(&self) -> i32 { self.x } \
                       fn b(&self) -> i32 { self.y } \
                   }";
        let m = measure(src);
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn three_disjoint_methods_are_lcom_three() {
        let src = "struct S { x: i32, y: i32, z: i32 } \
                   impl S { \
                       fn a(&self) -> i32 { self.x } \
                       fn b(&self) -> i32 { self.y } \
                       fn c(&self) -> i32 { self.z } \
                   }";
        let m = measure(src);
        assert_eq!(m[0].value, 3.0);
    }

    #[test]
    fn shared_call_unites_clusters() {
        // `a` and `c` don't touch the same field, but `a` calls `c`,
        // so they're one component. With `b` separate, LCOM4 = 2.
        let src = "struct S { x: i32, y: i32 } \
                   impl S { \
                       fn a(&self) -> i32 { self.c() } \
                       fn b(&self) -> i32 { self.y } \
                       fn c(&self) -> i32 { self.x } \
                   }";
        let m = measure(src);
        assert_eq!(m[0].value, 2.0);
    }

    #[test]
    fn trait_impl_is_skipped() {
        // Trait `impl` blocks don't contribute — the cohesion graph
        // there reflects the trait shape, not the type's own design.
        let src = "trait T { fn a(&self); fn b(&self); } \
                   struct S { x: i32, y: i32 } \
                   impl T for S { \
                       fn a(&self) { let _ = self.x; } \
                       fn b(&self) { let _ = self.y; } \
                   }";
        let m = measure(src);
        assert!(
            m.is_empty(),
            "trait impl should not contribute LCOM4 measurements"
        );
    }

    #[test]
    fn non_self_field_access_does_not_link() {
        // `a` accesses `other.x`, `b` accesses `self.y`. Only `self.<f>`
        // is recorded, so they don't share a field — two components.
        let src = "struct S { y: i32 } \
                   struct Other { x: i32 } \
                   impl S { \
                       fn a(&self, o: &Other) -> i32 { o.x } \
                       fn b(&self) -> i32 { self.y } \
                   }";
        let m = measure(src);
        assert_eq!(m[0].value, 2.0);
    }
}
