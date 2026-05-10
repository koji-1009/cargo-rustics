//! `macro_rules!` arm count — counts the number of arms in each
//! `macro_rules!` definition.

use ra_ap_syntax::{
    ast::{self, AstNode, HasName},
    SyntaxKind,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::scope::{ScopeKind, ScopeRef};

/// `macro_rules!` arm count calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacroRulesArmCount;

impl MetricCalculator for MacroRulesArmCount {
    fn id(&self) -> &'static str {
        "macro-rules-arm-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "macro_rules! Arm Count",
            category: MetricCategory::Macro,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(8.0)),
            default_error: Some(Threshold::new(15.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        let mut out = Vec::new();
        for desc in input.tree.syntax().descendants() {
            let Some(rules) = ast::MacroRules::cast(desc) else {
                continue;
            };
            let Some(name) = rules.name() else {
                continue;
            };
            let arm_count = count_arms(rules.syntax());
            if arm_count == 0 {
                continue;
            }
            // Approximate line by counting newlines before the
            // macro's offset.
            let offset: usize = rules.syntax().text_range().start().into();
            let prefix = input.source.get(..offset).unwrap_or("");
            let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
            out.push(MetricMeasurement::new(
                ScopeRef::new(name.text().to_string(), ScopeKind::Module, line),
                f64::from(arm_count),
            ));
        }
        out
    }
}

fn count_arms(node: &ra_ap_syntax::SyntaxNode) -> u32 {
    // Each arm is delimited by `=>` followed by a token tree. We
    // approximate by counting `=>` tokens at depth 1 of the macro
    // body.
    let Some(token_tree) = node.children().find(|c| c.kind() == SyntaxKind::TOKEN_TREE) else {
        return 0;
    };
    let mut n = 0u32;
    for tok in token_tree.children_with_tokens() {
        if let Some(t) = tok.into_token() {
            if t.kind() == SyntaxKind::FAT_ARROW {
                n += 1;
            }
        }
    }
    n
}

const RATIONALE: &str = "\
macro_rules! arm count flags declarative macros with many distinct match \
arms — past 8 arms the macro is doing several jobs and would benefit \
from being split into multiple smaller macros.";

const REFACTOR_HINTS: &[&str] = &[
    "Split a multi-purpose macro into one macro per kind of expansion.",
    "Move declarative macros that need many arms to proc macros, where the dispatch can use real Rust matching.",
];

const REFERENCES: &[&str] = &[];
