//! Halstead Volume (Halstead 1977) — `N · log2(η)` where N is the
//! total token count and η the distinct-token count of a function
//! body.

use std::collections::HashSet;

use ra_ap_syntax::{ast::AstNode, SyntaxKind, SyntaxNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Halstead Volume calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct HalsteadVolume;

impl MetricCalculator for HalsteadVolume {
    fn id(&self) -> &'static str {
        "halstead-volume"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Halstead Volume",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(1500.0)),
            default_error: Some(Threshold::new(3000.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            if frame.is_test() {
                return None;
            }
            let body = frame.item.body()?;
            Some(volume(body.syntax()))
        })
    }
}

fn volume(node: &SyntaxNode) -> f64 {
    let mut total = 0u64;
    let mut distinct: HashSet<(SyntaxKind, String)> = HashSet::new();
    for token in node.descendants_with_tokens().filter_map(|e| e.into_token()) {
        if matches!(token.kind(), SyntaxKind::WHITESPACE | SyntaxKind::COMMENT) {
            continue;
        }
        total += 1;
        distinct.insert((token.kind(), token.text().to_string()));
    }
    if total == 0 || distinct.is_empty() {
        return 0.0;
    }
    let n = total as f64;
    let eta = distinct.len() as f64;
    n * eta.log2()
}

const RATIONALE: &str = "\
Halstead Volume counts non-trivia tokens in the body and weights by \
log2(η) where η is the distinct-token vocabulary. The result tracks \
'how much information the reader has to process'. Off-by-default lenses \
elsewhere can pair with Halstead for a fuller picture; this one is the \
classical 1977 formula.";

const REFACTOR_HINTS: &[&str] = &[
    "Extract token-dense computations (binary-op chains, deeply-nested expressions) into named bindings.",
    "Replace local helper functions inlined into the body with proper fns elsewhere.",
];

const REFERENCES: &[&str] = &[
    "Halstead, M. H. (1977). Elements of Software Science. Elsevier.",
];
