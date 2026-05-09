//! `proc-macro-presence` — informational lens flagging functions whose
//! attribute set contains a likely proc-macro.
//!
//! + §6.4. The metric counts function-level attributes that
//! are *not* in the small whitelist of well-known built-in attributes
//! (`cfg`, `allow`, `derive`, …). Multi-segment paths (`tokio::main`,
//! `axum::handler`, `serde::Serialize`) almost always come from a
//! proc-macro crate; single-segment unknown attributes (`my_codegen`)
//! are counted too because the registry of built-ins is small.
//!
//! Informational — never crosses a threshold. The signal feeds
//! the `rustContext` block once that ships.

use syn::Attribute;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::visitor::measure_functions;

/// `proc-macro-presence` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcMacroPresence;

impl MetricCalculator for ProcMacroPresence {
    fn id(&self) -> &'static str {
        "proc-macro-presence"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Proc-Macro Presence",
            category: MetricCategory::Macro,
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            let n = frame.attrs.iter().filter(|a| !is_builtin_attr(a)).count();
            Some(n as f64)
        })
    }
}

const RATIONALE: &str = "\
Functions decorated with proc-macro attributes (e.g. `#[tokio::main]`, \
`#[axum::handler]`, `#[serde::Serialize]`) execute code from another \
crate at compile time. The expanded body can be larger than the source \
suggests; reading the source alone misses the actual control flow. The \
metric flags such functions so the AI report can hint that the lens \
output is incomplete when the proc-macro is doing a lot of work.";

const REFACTOR_HINTS: &[&str] = &[
    "If the proc-macro is expanding into substantial logic, run \
`cargo rustics analyze --expanded-macros` to measure the \
post-expansion AST.",
    "Consider whether the proc-macro is essential or merely convenient — \
some attribute macros can be replaced by a plain function for code \
that the team has to read often.",
];

const REFERENCES: &[&str] = &[
];

/// True iff `attr` is one of the small set of attribute paths every
/// Rust source file uses (cfg / allow / derive / inline / repr / …).
/// Path-segmented attributes (e.g. `tokio::main`) always return false
/// — they are conservatively treated as proc-macro.
fn is_builtin_attr(attr: &Attribute) -> bool {
    let segs: Vec<_> = attr.path().segments.iter().collect();
    if segs.len() != 1 {
        return false;
    }
    let name = segs[0].ident.to_string();
    matches!(
        name.as_str(),
        "allow"
            | "deny"
            | "warn"
            | "forbid"
            | "must_use"
            | "inline"
            | "no_mangle"
            | "repr"
            | "derive"
            | "doc"
            | "cfg"
            | "cfg_attr"
            | "test"
            | "ignore"
            | "should_panic"
            | "bench"
            | "automatically_derived"
            | "panic_handler"
            | "alloc_error_handler"
            | "no_std"
            | "no_implicit_prelude"
            | "track_caller"
            | "cold"
            | "naked"
            | "deprecated"
            | "non_exhaustive"
            | "link"
            | "link_name"
            | "link_section"
            | "used"
            | "global_allocator"
            | "export_name"
            | "rustc_diagnostic_item"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        ProcMacroPresence.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_attributes_is_zero() {
        assert_eq!(n_of("fn f() {}", "f"), 0);
    }

    #[test]
    fn cfg_and_allow_do_not_count() {
        let src = "#[cfg(test)] #[allow(dead_code)] fn f() {}";
        assert_eq!(n_of(src, "f"), 0);
    }

    #[test]
    fn tokio_main_counts() {
        let src = "#[tokio::main] async fn f() {}";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn unknown_single_segment_counts() {
        let src = "#[my_codegen_attr] fn f() {}";
        assert_eq!(n_of(src, "f"), 1);
    }

    #[test]
    fn multiple_proc_macros_sum() {
        let src = "#[serde::Serialize] #[my_other::Trace] fn f() {}";
        assert_eq!(n_of(src, "f"), 2);
    }
}
