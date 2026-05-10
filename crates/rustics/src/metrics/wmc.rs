//! Weighted Methods per Class (WMC, CK 1994) — sum of cyclomatic
//! complexity across methods in an inherent `impl` block.

use ra_ap_syntax::{
    ast::{self, AstNode, BinaryOp, LogicOp, Pat},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// WMC calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Wmc;

impl MetricCalculator for Wmc {
    fn id(&self) -> &'static str {
        "wmc"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Weighted Methods per Class (CK 1994)",
            category: MetricCategory::ImplShape,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(50.0)),
            default_error: Some(Threshold::new(100.0)),
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
            let mut total = 0u32;
            for item in al.assoc_items() {
                if let ast::AssocItem::Fn(f) = item {
                    if let Some(body) = f.body() {
                        total += count_cc(body.syntax());
                    }
                }
            }
            Some(f64::from(total))
        })
    }
}

fn count_cc(node: &SyntaxNode) -> u32 {
    let mut cc = 1;
    for desc in node.descendants() {
        cc += node_contribution(desc);
    }
    cc
}

fn node_contribution(node: SyntaxNode) -> u32 {
    if matches!(
        node.kind(),
        SyntaxKind::IF_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::FOR_EXPR
            | SyntaxKind::LOOP_EXPR
            | SyntaxKind::TRY_EXPR
    ) {
        return 1;
    }
    if node.kind() == SyntaxKind::MATCH_EXPR {
        return ast::MatchExpr::cast(node).map(match_arms_contribution).unwrap_or(0);
    }
    if node.kind() == SyntaxKind::BIN_EXPR {
        return ast::BinExpr::cast(node).map(bin_logic).unwrap_or(0);
    }
    0
}

fn bin_logic(b: ast::BinExpr) -> u32 {
    matches!(
        b.op_kind(),
        Some(BinaryOp::LogicOp(LogicOp::And)) | Some(BinaryOp::LogicOp(LogicOp::Or))
    ) as u32
}

fn match_arms_contribution(m: ast::MatchExpr) -> u32 {
    let Some(arm_list) = m.match_arm_list() else {
        return 0;
    };
    let arms: Vec<_> = arm_list.arms().collect();
    let has_wildcard = arms
        .iter()
        .any(|a| a.pat().is_some_and(|p| matches!(p, Pat::WildcardPat(_))));
    if has_wildcard {
        (arms.len() as u32).saturating_sub(1)
    } else {
        0
    }
}

const RATIONALE: &str = "\
WMC sums per-method CC across an inherent impl block. CK established \
WMC > 50 as a tester's-burden indicator: the more weighted methods, \
the more the class earns its testing budget. Trait impls are skipped \
— their method set is the trait's contract, not a class-shape choice.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the impl block into helper types when the methods cluster around different concerns.",
    "Replace deep `match`-in-method patterns with strategy-style trait dispatch.",
];

const REFERENCES: &[&str] = &[
    "Chidamber, S. R., & Kemerer, C. F. (1994). A metrics suite for object oriented design. IEEE TSE.",
];
