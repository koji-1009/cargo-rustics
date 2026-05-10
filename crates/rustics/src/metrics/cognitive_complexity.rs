//! Cognitive Complexity (SonarSource 2018) — control-flow penalty
//! plus a nesting penalty plus a logical-op-sequence penalty.

use ra_ap_syntax::{
    ast::{self, AstNode, BinaryOp, LogicOp},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Cognitive complexity calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct CognitiveComplexity;

impl MetricCalculator for CognitiveComplexity {
    fn id(&self) -> &'static str {
        "cognitive-complexity"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Cognitive Complexity (SonarSource 2018)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(15.0)),
            default_error: Some(Threshold::new(25.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            Some(f64::from(walk_cog(body.syntax(), 0)))
        })
    }
}

fn walk_cog(node: &SyntaxNode, depth: u32) -> u32 {
    let mut total = 0u32;
    for child in node.children() {
        let (penalty, next_depth) = node_penalty(&child, depth);
        total += penalty;
        total += walk_cog(&child, next_depth);
    }
    total
}

fn node_penalty(node: &SyntaxNode, depth: u32) -> (u32, u32) {
    match node.kind() {
        SyntaxKind::IF_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::FOR_EXPR
        | SyntaxKind::LOOP_EXPR
        | SyntaxKind::MATCH_EXPR => {
            // B1: +1 control-flow + B2 nesting penalty.
            (1 + depth, depth + 1)
        }
        SyntaxKind::BIN_EXPR => {
            // B3: +1 per logical-op sequence boundary. Approximate
            // as +1 per &&/|| binary expr.
            let p = ast::BinExpr::cast(node.clone())
                .map(|b| {
                    matches!(
                        b.op_kind(),
                        Some(BinaryOp::LogicOp(LogicOp::And))
                            | Some(BinaryOp::LogicOp(LogicOp::Or))
                    ) as u32
                })
                .unwrap_or(0);
            (p, depth)
        }
        // Closures open a new fn-like scope — depth resets.
        SyntaxKind::CLOSURE_EXPR => (0, 0),
        _ => (0, depth),
    }
}

const RATIONALE: &str = "\
Cognitive Complexity (SonarSource 2018) penalises control-flow plus \
nesting plus logical-op sequences. Unlike CC, deeply-nested code \
contributes more than the sum of its parts, matching how working memory \
strain actually scales with depth.";

const REFACTOR_HINTS: &[&str] = &[
    "Lift the deepest nested block into a named helper.",
    "Replace deep `if`/`else` chains with a `match` on a small enum.",
];

const REFERENCES: &[&str] =
    &["Campbell, G. A. (2018). Cognitive Complexity — A new way of measuring understandability."];
