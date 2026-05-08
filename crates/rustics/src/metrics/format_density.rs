//! `format-density` — count of `format!`-class macros in a function
//! body.
//!
//! Plan §M4. Format-class macros (`format!`, `println!`, `eprintln!`,
//! `write!`, `writeln!`, `print!`, `eprint!`) all build a `String`
//! through the formatting machinery; in hot paths, a dense cluster
//! of them is an allocation/I/O signal worth surfacing alongside
//! `clone-density`.
//!
//! What counts: macro invocations whose path's last segment is one
//! of the recognised names. False positives on user-defined macros
//! sharing those names are rare in practice.

use syn::visit::{self, Visit};
use syn::Macro;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// `format-density` calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct FormatDensity;

impl MetricCalculator for FormatDensity {
    fn id(&self) -> &'static str {
        "format-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Format Density",
            category: MetricCategory::RustPerformance,
            polarity: MetricPolarity::LowerIsBetter,
            // Five `println!` calls in one function is unusual outside
            // a CLI driver; ten is loud.
            default_warning: Some(Threshold::new(5.0)),
            default_error: Some(Threshold::new(10.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            frame.body.map(|body| {
                let mut v = FormatVisitor { count: 0 };
                v.visit_block(body);
                f64::from(v.count)
            })
        })
    }
}

const RATIONALE: &str = "\
Each format-class macro builds a `String` through the formatting \
machinery — fine in setup / display code, expensive in hot loops. The \
metric is a companion to clone-density: format calls are *another* \
allocation site that escapes the borrow story.";

const REFACTOR_HINTS: &[&str] = &[
    "Pre-format strings outside a hot loop into a `&str` and reuse them \
inside.",
    "Replace `format!` + `push_str` chains with `write!` on a re-used \
`String`/`Vec<u8>` buffer.",
    "If most calls are `println!`/`eprintln!`, consider whether the function \
should return a value the caller logs at one site instead.",
];

const REFERENCES: &[&str] = &["plan §M4 — continuous lens proliferation."];

struct FormatVisitor {
    count: u32,
}

impl<'ast> Visit<'ast> for FormatVisitor {
    fn visit_macro(&mut self, node: &'ast Macro) {
        if is_format_class(node) {
            self.count += 1;
        }
        visit::visit_macro(self, node);
    }
}

fn is_format_class(m: &Macro) -> bool {
    let last = m.path.segments.last().map(|s| s.ident.to_string());
    matches!(
        last.as_deref(),
        Some("format")
            | Some("println")
            | Some("eprintln")
            | Some("print")
            | Some("eprint")
            | Some("write")
            | Some("writeln")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        FormatDensity.measure(&input)
    }

    fn n_of(src: &str, scope: &str) -> u32 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value as u32)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn no_format_macros_is_zero() {
        assert_eq!(n_of("fn f() { let _x = 1; }", "f"), 0);
    }

    #[test]
    fn each_format_macro_counts() {
        let src = r#"
            fn f() {
                let _ = format!("a");
                println!("b");
                eprintln!("c");
                let mut s = String::new();
                let _ = write!(s, "d");
                let _ = writeln!(s, "e");
                print!("f");
                eprint!("g");
            }
        "#;
        assert_eq!(n_of(src, "f"), 7);
    }

    #[test]
    fn unrelated_macros_do_not_count() {
        let src = r#"
            fn f() {
                let v = vec![1, 2, 3];
                assert_eq!(v.len(), 3);
                let _ = v;
            }
        "#;
        assert_eq!(n_of(src, "f"), 0);
    }
}
