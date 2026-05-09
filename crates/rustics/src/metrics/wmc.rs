//! `wmc` — Weighted Methods per Class (CK 1994).
//!
//! Sum of cyclomatic complexity values across the methods of a single
//! class. In Rust the natural mapping for "class" is an `impl` block —
//! one block per type per role (inherent + each trait impl). The CK
//! suite was originally proposed for OO classes and has been validated
//! in many empirical studies as a defect-density predictor; the score
//! captures both *width* (many methods) and *depth* (each method
//! complex) under one number.
//!
//! Threshold convention: SonarSource and various industry papers use
//! 50 as the warning threshold (informally a "fat class"); CK
//! suggested no specific number, leaving it as project-defined. We
//! adopt the 50/100 split that's become broadly common.
//!
//! References:
//! * Chidamber & Kemerer (1994), "A Metrics Suite for Object Oriented
//!   Design", IEEE Trans. Softw. Eng. 20(6): 476-493 — original
//!   definition of WMC.
//! * Subramanyam & Krishnan (2003) and Basili et al. (1996) — empirical
//!   validation of WMC as defect / change-proneness predictor.

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_impls;

/// Weighted Methods per Class (CK 1994). Summed CC over an `impl` block.
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
            // 50 / 100 split is the SonarSource / Basili-tradition
            // threshold. CK 1994 deliberately leaves it project-set.
            default_warning: Some(Threshold::new(50.0)),
            default_error: Some(Threshold::new(100.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_impls(input.ast, |frame| {
            let mut total: u32 = 0;
            for item in &frame.item.items {
                if let syn::ImplItem::Fn(method) = item {
                    total += super::cyclomatic_complexity::compute_cc(&method.block.stmts);
                }
            }
            Some(f64::from(total))
        })
    }
}

const RATIONALE: &str = "\
Weighted Methods per Class (CK 1994) sums the cyclomatic complexity \
of every method in the class — a single number that captures both \
width (how many methods) and depth (how complex each is). High WMC \
correlates empirically with defect density and change-proneness in \
multiple validation studies (Basili et al. 1996, Subramanyam & \
Krishnan 2003). In Rust the natural unit is the `impl` block: one \
score per inherent or trait impl. Past 50 the type is usually \
carrying multiple roles; past 100 the load is rarely defensible.";

const REFACTOR_HINTS: &[&str] = &[
    "Split the impl block by role: separate `impl Foo { /* core */ }` \
from `impl Foo { /* serde */ }` so each block scores independently.",
    "Extract methods that delegate to a helper type; the type's \
constructor becomes one method and the helper carries the complexity.",
    "If the methods share a code structure (e.g. each is a `match` over \
the same variant), collapse the dispatch into a single method that \
takes the variant as a parameter.",
];

const REFERENCES: &[&str] = &[
    "Chidamber & Kemerer (1994). A Metrics Suite for Object Oriented \
Design. IEEE Trans. Softw. Eng. 20(6): 476-493.",
    "Basili, Briand & Melo (1996). A validation of object-oriented \
design metrics as quality indicators. IEEE TSE 22(10): 751-761.",
    "Subramanyam & Krishnan (2003). Empirical analysis of CK metrics \
for object-oriented design complexity. IEEE TSE 29(4): 297-310.",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        Wmc.measure(&input)
    }

    fn wmc_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_impl_is_zero() {
        let src = "struct Foo; impl Foo {}";
        assert_eq!(wmc_of(src, "Foo"), 0);
    }

    #[test]
    fn three_trivial_methods_sum_to_three() {
        // Each method has CC=1 (single straight-line path). WMC = 1+1+1 = 3.
        let src = "struct Foo; impl Foo { fn a(&self) {} fn b(&self) {} fn c(&self) {} }";
        assert_eq!(wmc_of(src, "Foo"), 3);
    }

    #[test]
    fn complex_method_pulls_score_higher_than_method_count() {
        // a: CC=1 (trivial). b: CC=4 (3 ifs + base). WMC = 1+4 = 5.
        // The old impl-method-count would have reported 2.
        let src = r#"
            struct Foo;
            impl Foo {
                fn a(&self) {}
                fn b(&self, x: i32) -> i32 {
                    let mut n = 0;
                    if x > 0 { n += 1; }
                    if x > 1 { n += 1; }
                    if x > 2 { n += 1; }
                    n
                }
            }
        "#;
        assert_eq!(wmc_of(src, "Foo"), 5);
    }

    #[test]
    fn associated_const_does_not_count() {
        // const items aren't methods; only fn items contribute.
        let src = "struct Foo; impl Foo { const N: i32 = 1; fn a(&self) {} }";
        assert_eq!(wmc_of(src, "Foo"), 1);
    }

    #[test]
    fn metadata_is_well_formed() {
        let md = Wmc.metadata();
        assert_eq!(md.id, "wmc");
        assert_eq!(md.default_warning.map(|t| t.value), Some(50.0));
        assert_eq!(md.default_error.map(|t| t.value), Some(100.0));
        assert!(!md.references.is_empty());
        assert!(md.references[0].contains("Chidamber"));
    }
}
