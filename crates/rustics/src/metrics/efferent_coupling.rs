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

/// Collects the *outermost* path's leftmost segment into `out`.
///
/// `use foo::{A, B}` is one external dependency on `foo`, not three on
/// `foo` / `A` / `B` — Martin's Ce counts distinct external module
/// roots, and `A` / `B` are members of `foo`, not separate modules.
/// We therefore only recurse into `use_tree_list` when the outer tree
/// has *no* path (the top-level grouped form `use {foo, bar}`); when
/// the outer path is present, the children's identifiers are inside
/// that path and add nothing to the root set.
fn collect_use_roots(tree: Option<ast::UseTree>, out: &mut HashSet<String>) {
    let Some(tree) = tree else { return };
    if let Some(path) = tree.path() {
        if let Some(seg) = first_segment(&path) {
            out.insert(seg);
        }
        return;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ce(src: &str) -> f64 {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        EfferentCoupling.measure(&input)[0].value
    }

    #[test]
    fn single_use_is_one_root() {
        assert_eq!(ce("use foo::Bar;"), 1.0);
    }

    #[test]
    fn grouped_use_counts_outer_path_once() {
        // The crux of the lens: `use foo::{A, B, C}` is one external
        // dependency on `foo`. `A` / `B` / `C` are members of `foo`,
        // not separate modules.
        assert_eq!(ce("use foo::{A, B, C};"), 1.0);
    }

    #[test]
    fn nested_groups_still_one_root() {
        assert_eq!(ce("use foo::{bar::{X, Y}, baz};"), 1.0);
    }

    #[test]
    fn top_level_group_recurses() {
        // The only form where the outer tree has no path: every
        // child contributes its own root.
        assert_eq!(ce("use {foo, bar};"), 2.0);
    }

    #[test]
    fn stdlib_roots_are_dropped() {
        assert_eq!(ce("use std::io; use core::mem; use alloc::vec;"), 0.0);
    }

    #[test]
    fn self_and_super_roots_are_dropped() {
        assert_eq!(ce("use self::a; use super::b;"), 0.0);
    }

    #[test]
    fn multiple_roots_sum() {
        assert_eq!(ce("use foo::A; use bar::B; use baz::{C, D};"), 3.0);
    }

    #[test]
    fn duplicate_root_counted_once() {
        // `use foo::A` and `use foo::B` both depend on `foo` only.
        assert_eq!(ce("use foo::A; use foo::B;"), 1.0);
    }
}
