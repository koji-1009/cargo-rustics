//! Efferent Coupling (Ce) — Martin 1994.
//!
//! The number of distinct *outgoing* dependencies a module has
//! — for each `use <root>::…` statement we count `<root>` once. The
//! count includes external crates (`std`, `serde`, …) and internal
//! modules (`crate`, `super`, `self`, …) alike — both contribute to the
//! reading load when you open the file.
//!
//! # Caveats
//!
//! * cargo-rustics walks one file at a time at Layer 1, so a module
//!   spanning multiple files (rare in Rust) measures each file
//!   independently. The CLI's file-derived module prefix means each
//!   measurement is anchored at the file's path.
//! * The Afferent Coupling (Ca) lens — and the derived Instability (I)
//!   = Ce / (Ca + Ce), Distance from Main Sequence (D) = |A + I - 1| —
//!   require cross-file aggregation. They land in M2 alongside the
//!   regression command's two-snapshot loader.

use syn::{Item, UseTree};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::scope::{ScopeKind, ScopeRef};

/// Efferent Coupling calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EfferentCoupling;

impl MetricCalculator for EfferentCoupling {
    fn id(&self) -> &'static str {
        "efferent-coupling"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Efferent Coupling (Ce)",
            category: MetricCategory::Coupling,
            polarity: MetricPolarity::LowerIsBetter,
            // 20+ outgoing roots in a single module is the "this file
            // is doing too much" signal. The threshold is generous —
            // facade modules legitimately use many things.
            default_warning: Some(Threshold::new(20.0)),
            default_error: Some(Threshold::new(40.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        let count = count_efferent(input.ast);
        // Empty scope path — the CLI prepends the file-derived module
        // prefix (e.g. `reporters::ai`) to anchor the measurement at
        // the module level.
        let scope = ScopeRef::new(String::new(), ScopeKind::Module, 1);
        vec![MetricMeasurement::new(scope, f64::from(count))]
    }
}

const RATIONALE: &str = "\
Efferent Coupling counts the things a module reaches outward to. A high \
Ce means the module ties many other things together — sometimes that is \
the right design (a facade), more often it means responsibilities have \
landed here that belong elsewhere. Pair with Afferent Coupling for \
the Instability ratio I = Ce / (Ca + Ce).";

const REFACTOR_HINTS: &[&str] = &[
    "Pull `use` statements that only one function uses inside that function — \
the module-level Ce drops without touching the function's behaviour.",
    "If most outgoing edges go to one larger system (auth, persistence), \
extract a small adapter module and have the rest of the file talk through \
the adapter only.",
    "Re-exports from a `prelude` module can collapse many `use` lines into \
one, lowering Ce while keeping the same names available.",
];

const REFERENCES: &[&str] = &[
    "Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of Dependencies.",
];

/// Counts the number of distinct top-level path roots in `use` items.
/// `use std::a; use std::b;` contributes 1; `use std; use serde;` is 2.
fn count_efferent(file: &syn::File) -> u32 {
    let mut roots = std::collections::HashSet::new();
    for item in &file.items {
        if let Item::Use(u) = item {
            collect_roots(&u.tree, &mut roots);
        }
    }
    roots.len() as u32
}

/// Walks a `UseTree` and collects every leftmost path segment we
/// encounter. `use {std::a, serde::b};` contributes both `std` and
/// `serde`.
fn collect_roots(tree: &UseTree, out: &mut std::collections::HashSet<String>) {
    match tree {
        UseTree::Path(p) => {
            out.insert(p.ident.to_string());
        }
        UseTree::Name(n) => {
            out.insert(n.ident.to_string());
        }
        UseTree::Rename(r) => {
            out.insert(r.ident.to_string());
        }
        UseTree::Glob(_) => {
            // `use foo::*;` — the parent walker already pushed `foo`.
        }
        UseTree::Group(g) => {
            for item in &g.items {
                collect_roots(item, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        EfferentCoupling.measure(&input)
    }

    fn n_of(src: &str) -> u32 {
        measure(src)
            .first()
            .map(|m| m.value as u32)
            .expect("one measurement per file")
    }

    #[test]
    fn no_uses_is_zero() {
        assert_eq!(n_of("fn f() {}"), 0);
    }

    #[test]
    fn single_use_root() {
        assert_eq!(n_of("use std::collections::HashMap;"), 1);
    }

    #[test]
    fn two_distinct_roots() {
        let src = "use std::collections::HashMap; use serde::Serialize;";
        assert_eq!(n_of(src), 2);
    }

    #[test]
    fn duplicate_root_counted_once() {
        let src = "use std::a; use std::b; use std::c;";
        assert_eq!(n_of(src), 1);
    }

    #[test]
    fn group_use_distributes() {
        let src = "use {std::a, serde::b, anyhow::Result};";
        assert_eq!(n_of(src), 3);
    }

    #[test]
    fn crate_super_self_count() {
        // Internal coupling targets count too — they are still reading
        // load when opening the file.
        let src = "use crate::a; use super::b; use self::c;";
        assert_eq!(n_of(src), 3);
    }

    #[test]
    fn metadata_is_well_formed() {
        let md = EfferentCoupling.metadata();
        assert_eq!(md.id, "efferent-coupling");
        assert!(md.default_warning.is_some());
    }
}
