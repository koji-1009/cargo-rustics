//! LCOM4 (Hitz & Montazeri 1995) — connected components in the
//! method graph of an inherent `impl` block. Two methods are
//! linked when they share at least one `self.<field>` access OR
//! when one calls the other.

use std::collections::{HashMap, HashSet};

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
fn connected_components(methods: &[MethodInfo]) -> u32 {
    let mut parent: Vec<usize> = (0..methods.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }
    let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    };
    let name_index: HashMap<&str, usize> = methods
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();
    for i in 0..methods.len() {
        for j in (i + 1)..methods.len() {
            let shared_field = methods[i]
                .fields
                .intersection(&methods[j].fields)
                .next()
                .is_some();
            let calls = methods[i].callees.contains(&methods[j].name)
                || methods[j].callees.contains(&methods[i].name);
            if shared_field || calls {
                union(&mut parent, i, j);
            }
        }
    }
    let _ = name_index;
    let mut roots: HashSet<usize> = HashSet::new();
    for i in 0..methods.len() {
        roots.insert(find(&mut parent, i));
    }
    roots.len() as u32
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
