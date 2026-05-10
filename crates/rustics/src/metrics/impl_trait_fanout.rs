//! Impl-trait fanout — count of distinct trait names that appear
//! as bounds inside an `impl` block (informational).

use std::collections::HashSet;

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxKind,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_impls;

/// Impl-trait fanout calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImplTraitFanout;

impl MetricCalculator for ImplTraitFanout {
    fn id(&self) -> &'static str {
        "impl-trait-fanout"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Impl Trait Fanout (impl Trait sites)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.tree, |frame| {
            let mut traits: HashSet<String> = HashSet::new();
            for desc in frame.item.syntax().descendants() {
                if desc.kind() != SyntaxKind::IMPL_TRAIT_TYPE {
                    continue;
                }
                collect_bound_names(&desc, &mut traits);
            }
            if traits.is_empty() { None } else { Some(traits.len() as f64) }
        })
    }
}

fn collect_bound_names(node: &ra_ap_syntax::SyntaxNode, traits: &mut HashSet<String>) {
    let Some(it) = ast::ImplTraitType::cast(node.clone()) else {
        return;
    };
    let Some(bounds) = it.type_bound_list() else {
        return;
    };
    for bound in bounds.bounds() {
        let Some(ast::Type::PathType(p)) = bound.ty() else {
            continue;
        };
        let Some(seg) = p.path().and_then(|p| p.segment()).and_then(|s| s.name_ref()) else {
            continue;
        };
        traits.insert(seg.text().to_string());
    }
}

const RATIONALE: &str = "\
Impl-trait fanout reports the count of distinct traits used as `impl \
Trait` bounds inside an `impl` block. Informational; high values \
sometimes indicate a method that's juggling many type-erased \
abstractions.";

const REFACTOR_HINTS: &[&str] = &[];

const REFERENCES: &[&str] = &[];
