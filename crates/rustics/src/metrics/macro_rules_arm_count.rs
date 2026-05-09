//! `macro-rules-arm-count` — number of arms in a `macro_rules!` definition.
//!
//! macro lens. A `macro_rules!` with many arms is the
//! `match` of macro-land — each arm is one rule the expander tries in
//! order, and the cognitive load mirrors the cognitive load of a long
//! `match` body.

use proc_macro2::TokenTree;
use syn::visit::Visit;
use syn::{Item, ItemMacro};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::scope::{ScopeKind, ScopeRef};

/// `macro-rules-arm-count` calculator.
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
        let mut visitor = MacroVisitor {
            measurements: Vec::new(),
            module_path: Vec::new(),
        };
        visitor.visit_file(input.ast);
        visitor.measurements
    }
}

const RATIONALE: &str = "\
Each arm of a `macro_rules!` definition is one rule the expander tries; \
many arms scale the cognitive load of reading the macro the way many \
`match` arms scale the cognitive load of reading a function. Past eight \
arms, the order-dependence and overlap between rules become hard to keep \
straight.";

const REFACTOR_HINTS: &[&str] = &[
    "If the rules dispatch on a small set of categories, push the categories \
into a helper macro and call it from the main macro's arms.",
    "Procedural macros (`#[proc_macro]`) replace declarative macros for the \
complex cases — when the arm count grows past a dozen, it is usually time \
to convert.",
    "Some `macro_rules!` arms are added defensively (`($($any:tt)*) => {}`); \
make sure those are necessary rather than vestigial.",
];

const REFERENCES: &[&str] = &[];

/// Walks the file collecting `macro_rules!` definitions.
struct MacroVisitor {
    measurements: Vec<MetricMeasurement>,
    module_path: Vec<String>,
}

impl<'ast> Visit<'ast> for MacroVisitor {
    fn visit_item(&mut self, node: &'ast Item) {
        match node {
            Item::Mod(m) => {
                self.module_path.push(m.ident.to_string());
                syn::visit::visit_item_mod(self, m);
                self.module_path.pop();
            }
            Item::Macro(m) => self.handle_macro(m),
            _ => syn::visit::visit_item(self, node),
        }
    }
}

impl MacroVisitor {
    fn handle_macro(&mut self, item: &ItemMacro) {
        if !item.mac.path.is_ident("macro_rules") {
            return;
        }
        let Some(name) = item.ident.as_ref().map(|i| i.to_string()) else {
            return;
        };
        let arm_count = count_arms(item);
        let mut path = self.module_path.clone();
        path.push(name);
        let scope = ScopeRef::new(
            path.join("::"),
            ScopeKind::Module,
            item.mac.bang_token.span.start().line,
        );
        self.measurements
            .push(MetricMeasurement::new(scope, f64::from(arm_count)));
    }
}

/// Returns the number of arms in a `macro_rules!` body. Each arm is
/// terminated by exactly one `=>`, so we count the `=>` punctuation
/// pairs in the body group.
fn count_arms(item: &ItemMacro) -> u32 {
    let mut count = 0u32;
    let mut prev_eq = false;
    for tt in item.mac.tokens.clone() {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == '=' => {
                prev_eq = true;
                continue;
            }
            TokenTree::Punct(p) if p.as_char() == '>' && prev_eq => {
                count += 1;
                prev_eq = false;
                continue;
            }
            _ => prev_eq = false,
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        MacroRulesArmCount.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_macro_rules_no_measurements() {
        assert!(measure("fn f() {}").is_empty());
    }

    #[test]
    fn single_arm_is_one() {
        let src = r#"
            macro_rules! one {
                ($x:expr) => { $x };
            }
        "#;
        assert_eq!(n_of(src, "one"), 1);
    }

    #[test]
    fn three_arms_is_three() {
        let src = r#"
            macro_rules! three {
                () => { 0 };
                ($x:expr) => { $x };
                ($x:expr, $y:expr) => { $x + $y };
            }
        "#;
        assert_eq!(n_of(src, "three"), 3);
    }

    #[test]
    fn module_nesting_prefixes_scope() {
        let src = r#"
            mod outer {
                macro_rules! foo {
                    () => { };
                }
            }
        "#;
        assert_eq!(n_of(src, "outer::foo"), 1);
    }

    #[test]
    fn ordinary_macro_invocation_is_ignored() {
        // `vec![1,2,3]` is a macro *invocation*, not a `macro_rules!`
        // definition — should not produce a measurement.
        let src = "fn f() { let _v = vec![1, 2, 3]; }";
        assert!(measure(src).is_empty());
    }
}
