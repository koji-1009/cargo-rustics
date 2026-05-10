//! Npath complexity (Nejmeh 1988) — multiplicative path count
//! through a function: each control-flow construct multiplies the
//! ambient npath by the number of branches it adds.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Npath complexity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct NpathComplexity;

impl MetricCalculator for NpathComplexity {
    fn id(&self) -> &'static str {
        "npath-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Npath Complexity (Nejmeh 1988)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(200.0)),
            default_error: Some(Threshold::new(1000.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            Some(npath(body.syntax()))
        })
    }
}

/// Npath approximation: 1 baseline times the product of (1 + branch
/// count) across siblings. We compute by walking children: the base
/// is 1; each child either adds branching multiplicatively or is
/// transparent (sequenced).
fn npath(node: &SyntaxNode) -> f64 {
    let mut acc = 1.0_f64;
    for child in node.children() {
        acc *= npath_factor(&child);
    }
    if acc > 1e9 {
        1e9
    } else {
        acc
    }
}

fn product_of_children(node: &SyntaxNode) -> f64 {
    let mut acc = 1.0_f64;
    for child in node.children() {
        acc *= npath_factor(&child);
    }
    if acc < 1.0 { 1.0 } else { acc }
}

fn npath_factor(node: &SyntaxNode) -> f64 {
    match node.kind() {
        SyntaxKind::IF_EXPR => {
            // if : 1 (no else) or 2 (else); recurse into bodies.
            let if_ = ast::IfExpr::cast(node.clone());
            let then_n = if_
                .as_ref()
                .and_then(|i| i.then_branch())
                .map(|b| npath(b.syntax()))
                .unwrap_or(1.0);
            let else_n = if_
                .as_ref()
                .and_then(|i| i.else_branch())
                .map(|e| match e {
                    ast::ElseBranch::Block(b) => npath(b.syntax()),
                    ast::ElseBranch::IfExpr(ie) => npath_factor(ie.syntax()),
                })
                .unwrap_or(1.0);
            then_n + else_n
        }
        SyntaxKind::MATCH_EXPR => {
            let m = ast::MatchExpr::cast(node.clone());
            let arms = m
                .as_ref()
                .and_then(|m| m.match_arm_list())
                .map(|al| al.arms().count())
                .unwrap_or(0)
                .max(1);
            arms as f64
        }
        SyntaxKind::WHILE_EXPR | SyntaxKind::FOR_EXPR | SyntaxKind::LOOP_EXPR => {
            product_of_children(node) + 1.0
        }
        _ => product_of_children(node),
    }
}

const RATIONALE: &str = "\
Npath complexity (Nejmeh 1988) is the product of branch counts through \
a function — multiplicative where CC is additive. A function with two \
sequential `if`s has CC 2 + (whatever nesting) but npath 4 (2 × 2). \
Past 200 the test surface is hard to enumerate.";

const REFACTOR_HINTS: &[&str] = &[
    "Lift independent decision blocks into named helpers — sequential `if`s in the host fn become a single call site each.",
    "Replace cascading early-return checks with `?` over a Result the caller decomposes.",
];

const REFERENCES: &[&str] = &[
    "Nejmeh, B. A. (1988). NPATH: A measure of execution path complexity. CACM.",
];
