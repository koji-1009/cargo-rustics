//! Dyn density — count of `dyn Trait` types appearing in a fn
//! signature or body.

use ra_ap_syntax::{ast::AstNode, SyntaxKind, SyntaxNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_functions;

/// Dyn density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct DynDensity;

impl MetricCalculator for DynDensity {
    fn id(&self) -> &'static str {
        "dyn-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Dyn Density (dyn Trait sites)",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            Some(f64::from(count_dyn(frame.item.syntax())))
        })
    }
}

fn count_dyn(node: &SyntaxNode) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        if desc.kind() == SyntaxKind::DYN_TRAIT_TYPE {
            n += 1;
        }
    }
    n
}

const RATIONALE: &str = "\
Dyn density counts `dyn Trait` use sites in a function. Each is a \
runtime dispatch point — fine in moderation, but pervasive `dyn` use is \
worth surfacing so the cost surface is visible.";

const REFACTOR_HINTS: &[&str] = &[
    "Replace `&dyn Trait` parameters with `impl Trait` where the trait object isn't needed for storage.",
    "Collapse a `Vec<Box<dyn Trait>>` into an enum dispatch when the variant set is small and known.",
];

const REFERENCES: &[&str] = &[];
