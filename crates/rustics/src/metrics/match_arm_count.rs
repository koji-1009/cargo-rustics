//! Match arm count — maximum number of arms across all `match`
//! expressions in a function body. Sealed-aware.

use ra_ap_syntax::ast::{self, AstNode, Pat};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Match arm count calculator.
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
            default_error: Some(Threshold::new(15.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            let body = frame.item.body()?;
            let mut max = 0u32;
            for desc in body.syntax().descendants() {
                let Some(m) = ast::MatchExpr::cast(desc) else {
                    continue;
                };
                let arms = match_arms_count(&m);
                if arms > max {
                    max = arms;
                }
            }
            if max == 0 {
                None
            } else {
                Some(f64::from(max))
            }
        })
    }
}

/// Returns the arm count, or 0 when the match is sealed-exhaustive
/// (no `_` arm) since the compiler enforces exhaustiveness for it.
fn match_arms_count(m: &ast::MatchExpr) -> u32 {
    let Some(arm_list) = m.match_arm_list() else {
        return 0;
    };
    let arms: Vec<_> = arm_list.arms().collect();
    let has_wildcard = arms
        .iter()
        .any(|a| a.pat().is_some_and(|p| matches!(p, Pat::WildcardPat(_))));
    if has_wildcard {
        arms.len() as u32
    } else {
        0
    }
}

const RATIONALE: &str = "\
A `match` with many arms encodes a state machine the reader has to keep \
in mind. Sealed `match` (no `_` arm) is exempt — the compiler enforces \
exhaustiveness, so the cognitive risk a wide arm-count was meant to flag \
does not exist.";

const REFACTOR_HINTS: &[&str] = &[
    "Group related arms into a helper that takes the variant and returns the result.",
    "If the match is dispatching on a string / number, consider a `HashMap`-backed lookup table.",
    "Replace `_` catch-alls with named variants so the compiler enforces exhaustiveness.",
];

const REFERENCES: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let parsed = ra_ap_syntax::SourceFile::parse(src, ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let input = MetricInput::new(Path::new("t.rs"), src, &tree);
        MatchArmCount.measure(&input)
    }

    #[test]
    fn no_match_emits_nothing() {
        assert!(measure("fn f() {}").is_empty());
    }

    #[test]
    fn sealed_match_emits_zero_and_no_record() {
        let src = "enum E { A, B } fn f(e: E) { match e { E::A => {}, E::B => {} } }";
        // No wildcard → contributes 0 → no record at all.
        let m = measure(src);
        assert!(m.iter().all(|x| x.scope.path != "f"));
    }

    #[test]
    fn wildcard_match_uses_arm_count() {
        let src = "fn f(x: i32) { match x { 1 => {}, 2 => {}, _ => {} } }";
        assert_eq!(measure(src)[0].value, 3.0);
    }
}
