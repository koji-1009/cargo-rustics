//! Iterator chain length — longest chained method-call sequence
//! per fn (`.iter().map(...).filter(...).collect()` is depth 4).

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Iterator chain length calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct IteratorChainLength;

impl MetricCalculator for IteratorChainLength {
    fn id(&self) -> &'static str {
        "iterator-chain-length"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Iterator Chain Length",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(7.0)),
            default_error: Some(Threshold::new(12.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            Some(f64::from(longest_chain(body.syntax())))
        })
    }
}

fn longest_chain(node: &SyntaxNode) -> u32 {
    let mut max = 0u32;
    for desc in node.descendants() {
        let Some(call) = ast::MethodCallExpr::cast(desc) else {
            continue;
        };
        // Only count chain *roots* — a method-call whose receiver
        // is not itself a method call. The chain length is then 1
        // + however many dots the chain extends.
        if let Some(ast::Expr::MethodCallExpr(_)) = call.receiver() {
            // not a root; covered by some outer iteration.
            continue;
        }
        let depth = chain_depth_starting_at(call);
        if depth > max {
            max = depth;
        }
    }
    max
}

fn chain_depth_starting_at(root: ast::MethodCallExpr) -> u32 {
    // Walk parents up the chain: the chain extends as long as the
    // current node's *parent* is a MethodCallExpr whose receiver is
    // the current node.
    let mut depth = 1u32;
    let mut cur = root;
    while let Some(parent_node) = cur.syntax().parent() {
        let Some(parent_call) = ast::MethodCallExpr::cast(parent_node) else {
            break;
        };
        let Some(receiver) = parent_call.receiver() else {
            break;
        };
        let receiver_node = receiver.syntax();
        if receiver_node != cur.syntax() {
            break;
        }
        depth += 1;
        cur = parent_call;
    }
    depth
}

const RATIONALE: &str = "\
Iterator chain length flags long `.iter().map().filter().collect()` \
sequences. Past 7 the chain is hard to read at a glance; splitting it at \
a meaningful intermediate (`let processed = source.iter().map(...).collect();`) \
restores readability without changing semantics.";

const REFACTOR_HINTS: &[&str] = &[
    "Bind a meaningful intermediate result (`let staged = ...;`) and start the next chain from it.",
    "If two of the methods are doing the same kind of transformation, fold them into a single helper.",
];

const REFERENCES: &[&str] = &[];
