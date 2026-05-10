//! Unsafe block scope — count of `unsafe { ... }` blocks per fn,
//! plus +1 if the fn itself is `unsafe fn`.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxKind, SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Unsafe block scope calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsafeBlockScope;

impl MetricCalculator for UnsafeBlockScope {
    fn id(&self) -> &'static str {
        "unsafe-block-scope"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Unsafe Block Scope",
            category: MetricCategory::RustSafety,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(2.0)),
            default_error: Some(Threshold::new(5.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            let envelope = if frame.item.unsafe_token().is_some() {
                1
            } else {
                0
            };
            Some(f64::from(envelope + count_unsafe_blocks(body.syntax())))
        })
    }
}

fn count_unsafe_blocks(node: &SyntaxNode) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        if desc.kind() != SyntaxKind::BLOCK_EXPR {
            continue;
        }
        if let Some(b) = ast::BlockExpr::cast(desc) {
            if b.unsafe_token().is_some() {
                n += 1;
            }
        }
    }
    n
}

const RATIONALE: &str = "\
Each `unsafe` block is an explicit promise that the contained operations \
uphold the invariants the borrow checker can't prove. A function whose \
body is dotted with `unsafe` blocks is asking the reader to verify each \
one independently — concentrating the unsafe code in a single audited \
helper is usually clearer.";

const REFACTOR_HINTS: &[&str] = &[
    "Bundle several `unsafe` operations into one block with a single safety comment that covers all of them.",
    "If most of the function is unsafe, mark the function itself `unsafe fn` and let callers shoulder the audit.",
];

const REFERENCES: &[&str] = &[];
