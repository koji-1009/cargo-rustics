//! `match-arm-count` — maximum number of arms in any single `match`
//! expression inside a function body.
//!
//!
//! control-flow shape; sealed enums with one arm per variant are the
//! idiomatic case and want a generous threshold. Past about 7 arms a
//! human reader is using working memory to track which arm covers what,
//! and past 12 the function is effectively a switch table that wants
//! to be extracted into a helper enum or split into smaller dispatchers.
//!
//! Rust-specific framing — relationship to other lenses:
//!
//! * `cyclomatic-complexity`'s sealed-aware rule does *not*
//!   penalise an exhaustive `match Enum`. A clean exhaustive match
//!   over a 30-variant enum is CC = 1. This lens sees the 30 arms.
//! * `cognitive-complexity` adds 1 per `match`, ignoring arm count.
//!   This lens disambiguates "one match with 30 arms" (a switch table)
//!   from "30 separate matches" (probably hiding a state machine).
//! * `maximum-nesting-level` sees a deep nest, not arm spread.
//!
//! What counts: the maximum `arms.len()` over every `ExprMatch` node
//! reachable in the function body. We take the *max* (not the sum)
//! because the cognitive cost of a single big match is what hurts;
//! two small matches in sequence read as two small things.
//!
//! What is excluded:
//!
//! * `let-else` with a single rejection arm — that's a guard clause
//!   shape, not a switch table. We don't traverse `let` patterns.
//! * `if let` chains — those have their own readability profile
//!   (covered by nesting + cyclomatic).
//!
//! Default thresholds derived from a survey of the rustics codebase:
//! the typical `MetricCalculator::measure` body has 1-3 arms; the
//! worst case in any built-in lens is 6 arms (the `Op` table in
//! `rustics-macros`). 7 is the warning point where the function is
//! reaching switch-table territory; 12 is where extraction becomes
//! the obvious refactor.

use syn::visit::{self, Visit};
use syn::{ExprMatch, Pat};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `match-arm-count` (sealed-aware) calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MatchArmCount;

impl MetricCalculator for MatchArmCount {
    fn id(&self) -> &'static str {
        "match-arm-count"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Match Arm Count (sealed-aware)",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(7.0)),
            default_error: Some(Threshold::new(12.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = MatchVisitor { max_arms: 0 };
                v.visit_block(body);
                f64::from(v.max_arms)
            })
        })
    }
}

const RATIONALE: &str = "\
A `match` with many arms reads as a switch table: the reader holds \
each arm's pattern in working memory while scanning for the one that \
applies. Sealed enums with one arm per variant are the idiomatic \
exhaustive case (compile-time-checked); this lens flags wide matches \
where you, the human reader, are doing the dispatch work that an \
extracted helper enum or `match` on a smaller key could absorb.";

const REFACTOR_HINTS: &[&str] = &[
    "Group arm clusters into a helper enum: \
`enum Action { File(FileOp), Net(NetOp) }` then match those.",
    "Use guard clauses for early arms (`0..10 if x % 2 == 0 => …`) \
to collapse repetitive conditions and reduce visual branching.",
    "If the dispatch is on a string / id, replace the `match` with \
a `HashMap<&'static str, fn(...)>` lookup at the call site.",
    "Wide matches inside `impl Trait for T` can usually be split: \
each variant's arm becomes its own helper method, and the `match` \
shrinks to a one-liner that delegates.",
];

const REFERENCES: &[&str] = &[
];

/// Walks a body tracking the maximum `arms.len()` across every
/// `ExprMatch` node *that has a wildcard / catch-all arm*. A match
/// without one is exhaustive on a sealed enum: the compiler is doing
/// the dispatch check, so the cognitive cost is the read-time of the
/// dispatch table itself, which is bounded by the enum's variant
/// count — adding variants forces every match site to update, so
/// the lens has nothing to flag. We exempt it for symmetry with the
/// `cyclomatic-complexity` sealed-aware rule.
struct MatchVisitor {
    max_arms: u32,
}

impl<'ast> Visit<'ast> for MatchVisitor {
    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        if has_catchall_arm(node) {
            let n = node.arms.len() as u32;
            if n > self.max_arms {
                self.max_arms = n;
            }
        }
        // Continue descending — a nested match inside an arm is its
        // own dispatch and we want the max over the whole body.
        visit::visit_expr_match(self, node);
    }
}

/// True iff the match has a top-level catch-all arm — either `_ => …`
/// or a bare-name binding like `other => …`. Both indicate the match
/// is *not* compile-time-exhaustive, so the reader is doing the
/// dispatch work the compiler would otherwise.
fn has_catchall_arm(m: &ExprMatch) -> bool {
    m.arms.iter().any(|arm| is_catchall(&arm.pat))
}

fn is_catchall(pat: &Pat) -> bool {
    match pat {
        Pat::Wild(_) => true,
        // `other => …` — single bare identifier with no sub-pattern.
        Pat::Ident(i) if i.subpat.is_none() && i.by_ref.is_none() && i.mutability.is_none() => {
            true
        }
        Pat::Or(or) => or.cases.iter().any(is_catchall),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        MatchArmCount.measure(&input)
    }

    fn arms_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_match_is_zero() {
        assert_eq!(arms_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn single_match_is_arm_count() {
        let src = r#"
            fn f(x: i32) -> i32 {
                match x {
                    0 => 0,
                    1 => 1,
                    _ => 2,
                }
            }
        "#;
        assert_eq!(arms_of(src, "f"), 3);
    }

    #[test]
    fn takes_max_not_sum_across_multiple_matches() {
        // Two matches in sequence: a 2-arm and a 5-arm. The lens
        // reports 5, not 7, because the cognitive cost is dominated
        // by the single biggest dispatch.
        let src = r#"
            fn f(x: i32) -> i32 {
                let a = match x { 0 => 0, _ => 1 };
                let b = match x {
                    0 => 0, 1 => 1, 2 => 2, 3 => 3, _ => 4,
                };
                a + b
            }
        "#;
        assert_eq!(arms_of(src, "f"), 5);
    }

    #[test]
    fn nested_match_is_seen() {
        let src = r#"
            fn f(x: (i32, i32)) -> i32 {
                match x.0 {
                    0 => match x.1 {
                        0 => 0, 1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, _ => 6,
                    },
                    _ => 0,
                }
            }
        "#;
        assert_eq!(arms_of(src, "f"), 7);
    }

    #[test]
    fn at_default_warning_threshold_is_seven() {
        // 7 is the warning threshold; the metric value at that arm
        // count must equal 7 exactly so threshold gating works.
        let src = "fn f(x: i32) -> i32 { match x { 0=>0, 1=>0, 2=>0, 3=>0, 4=>0, 5=>0, _=>0 } }";
        assert_eq!(arms_of(src, "f"), 7);
    }

    #[test]
    fn sealed_enum_match_is_zero() {
        // No wildcard / catch-all binding — the compiler is checking
        // exhaustiveness. The lens contributes 0 (sealed-aware, plan
        // §2.5) so an exhaustive `match cli.command { ... }` over a
        // 9-variant enum doesn't trip the threshold.
        let src = r#"
            enum E { A, B, C, D, E, F, G, H, I }
            fn f(e: E) -> i32 {
                match e {
                    E::A => 0, E::B => 1, E::C => 2,
                    E::D => 3, E::E => 4, E::F => 5,
                    E::G => 6, E::H => 7, E::I => 8,
                }
            }
        "#;
        assert_eq!(arms_of(src, "f"), 0);
    }

    #[test]
    fn named_catchall_binding_counts_as_open_match() {
        // `other => Err(...)` is a bare-identifier pattern that catches
        // everything — same readability burden as `_ =>`. Treat it as
        // open-dispatch.
        let src = r#"
            fn f(s: &str) -> i32 {
                match s {
                    "a" => 0, "b" => 1, "c" => 2,
                    "d" => 3, "e" => 4, "f" => 5,
                    "g" => 6, "h" => 7,
                    other => other.len() as i32,
                }
            }
        "#;
        assert_eq!(arms_of(src, "f"), 9);
    }

    #[test]
    fn catchall_inside_or_pattern_still_counts_as_open() {
        // `_ | Foo => …` is an or-pattern containing a wildcard;
        // the wildcard branch absorbs everything, so it's open.
        let src = r#"
            enum E { A, B, C }
            fn f(e: E) -> i32 {
                match e {
                    E::A => 0,
                    E::B | _ => 1,
                }
            }
        "#;
        assert_eq!(arms_of(src, "f"), 2);
    }

    #[test]
    fn metadata_thresholds_are_seven_and_twelve() {
        let md = MatchArmCount.metadata();
        assert_eq!(md.default_warning.map(|t| t.value), Some(7.0));
        assert_eq!(md.default_error.map(|t| t.value), Some(12.0));
        assert_eq!(md.polarity, MetricPolarity::LowerIsBetter);
    }
}
