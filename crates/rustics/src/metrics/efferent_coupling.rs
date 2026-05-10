//! Efferent coupling — count of distinct module roots a file
//! imports via `use` statements.

use std::collections::HashSet;

use ra_ap_syntax::ast::{self, AstNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::scope::{ScopeKind, ScopeRef};

/// Efferent coupling calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EfferentCoupling;

impl MetricCalculator for EfferentCoupling {
    fn id(&self) -> &'static str {
        "efferent-coupling"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Efferent Coupling (per-file imports)",
            category: MetricCategory::Coupling,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(15.0)),
            default_error: Some(Threshold::new(30.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        let mut roots: HashSet<String> = HashSet::new();
        for desc in input.tree.syntax().descendants() {
            let Some(use_) = ast::Use::cast(desc) else {
                continue;
            };
            collect_use_roots(use_.use_tree(), &mut roots);
        }
        // `std` / `core` / `alloc` are stdlib: not project-internal
        // dependencies for the Martin sense of efferent coupling.
        roots.remove("std");
        roots.remove("core");
        roots.remove("alloc");
        roots.remove("self");
        roots.remove("super");
        let scope_path = input
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        vec![MetricMeasurement::new(
            ScopeRef::new(scope_path, ScopeKind::Module, 1),
            roots.len() as f64,
        )]
    }
}

fn collect_use_roots(tree: Option<ast::UseTree>, out: &mut HashSet<String>) {
    let Some(tree) = tree else { return };
    if let Some(path) = tree.path() {
        if let Some(seg) = first_segment(&path) {
            out.insert(seg);
        }
    }
    if let Some(group) = tree.use_tree_list() {
        for child in group.use_trees() {
            collect_use_roots(Some(child), out);
        }
    }
}

fn first_segment(path: &ast::Path) -> Option<String> {
    let mut p = Some(path.clone());
    let mut head = None;
    while let Some(cur) = p {
        if let Some(s) = cur.segment().and_then(|s| s.name_ref()) {
            head = Some(s.text().to_string());
        }
        p = cur.qualifier();
    }
    head
}

const RATIONALE: &str =
    "Efferent coupling counts the distinct external module roots a file imports.";

const REFACTOR_HINTS: &[&str] = &[
    "If a file imports many crates, see whether some imports could move to a sibling module that uses them more centrally.",
];

const REFERENCES: &[&str] =
    &["Martin, R. (1994). OO Design Quality Metrics: An Analysis of Dependencies."];
