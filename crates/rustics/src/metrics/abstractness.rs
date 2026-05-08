//! Abstractness (A) — Martin 1994.
//!
//! Plan §6.3. The fraction of a module's *type-defining* items that are
//! `trait` definitions: `A = trait_defs / type_defs`. Range `[0, 1]`.
//!
//! Type-defining items at M1: `trait`, `struct`, `enum`, `union`,
//! `type` aliases. `impl` blocks are not type definitions; `fn` items
//! are not type definitions. We exclude `use` / `mod` / `extern crate`
//! statements so the ratio reflects what kinds of *types* the module
//! produces.

use syn::Item;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::scope::{ScopeKind, ScopeRef};

/// Abstractness calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct Abstractness;

impl MetricCalculator for Abstractness {
    fn id(&self) -> &'static str {
        "abstractness"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Abstractness (A)",
            category: MetricCategory::Coupling,
            // Informational at M1 — Distance from Main Sequence (D = |A + I − 1|)
            // is the actionable derived metric, and it needs cross-file Ca to
            // compute I. We ship A standalone now so the input is recorded.
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        let (traits, types) = count_type_defs(input.ast);
        let value = if types == 0 {
            0.0
        } else {
            f64::from(traits) / f64::from(types)
        };
        let scope = ScopeRef::new(String::new(), ScopeKind::Module, 1);
        vec![MetricMeasurement::new(scope, value)]
    }
}

const RATIONALE: &str = "\
Abstractness names the proportion of type definitions that are *traits* \
(abstract contracts) versus concrete types (struct/enum/union). A library \
module typically sits high (lots of trait-driven design); a leaf \
implementation module sits low. The number is one of two inputs to \
Distance from Main Sequence (D = |A + I − 1|, plan §6.3); it is reported \
informationally at M1 because Instability needs cross-file aggregation \
that lands in M2.";

const REFACTOR_HINTS: &[&str] = &[
    "If a module mixes many traits with many concrete types, splitting it \
into a `*_traits` module and a `*_impl` module makes the role of each \
file obvious.",
    "Sealed traits (`pub trait Foo: sealed::Sealed {}`) often live alongside \
their implementations; that pattern legitimately lowers a module's \
Abstractness without changing its design.",
];

const REFERENCES: &[&str] = &[
    "Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of Dependencies.",
    "plan §6.3 — Abstractness (A).",
];

/// Returns `(trait_definitions, total_type_definitions)` for the file.
/// The total is across `trait`, `struct`, `enum`, `union`, `type` items.
fn count_type_defs(file: &syn::File) -> (u32, u32) {
    let mut traits = 0u32;
    let mut total = 0u32;
    for item in &file.items {
        match item {
            Item::Trait(_) => {
                traits += 1;
                total += 1;
            }
            Item::Struct(_) | Item::Enum(_) | Item::Union(_) | Item::Type(_) => total += 1,
            _ => {}
        }
    }
    (traits, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        Abstractness.measure(&input)
    }

    fn a_of(src: &str) -> f64 {
        measure(src)
            .first()
            .map(|m| m.value)
            .expect("one measurement per file")
    }

    #[test]
    fn no_type_defs_is_zero() {
        assert_eq!(a_of("fn f() {}"), 0.0);
    }

    #[test]
    fn all_traits_is_one() {
        let src = "trait A {} trait B {}";
        assert_eq!(a_of(src), 1.0);
    }

    #[test]
    fn all_concrete_is_zero() {
        let src = "struct A; struct B; enum C { X }";
        assert_eq!(a_of(src), 0.0);
    }

    #[test]
    fn half_and_half() {
        let src = "trait A {} struct B; trait C {} struct D;";
        assert_eq!(a_of(src), 0.5);
    }

    #[test]
    fn type_alias_counts_as_concrete() {
        let src = "trait T {} type X = i32;";
        assert_eq!(a_of(src), 0.5);
    }

    #[test]
    fn impl_blocks_do_not_count() {
        let src = "trait T {} struct S; impl T for S {} impl S {}";
        // Two type defs (T trait + S struct), two impls — A = 1/2.
        assert_eq!(a_of(src), 0.5);
    }
}
