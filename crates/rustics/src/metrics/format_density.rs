//! Format density — count of format-style macro calls per fn:
//! `format!`, `print!`, `println!`, `eprint!`, `eprintln!`, `write!`,
//! `writeln!`.

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Format density calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct FormatDensity;

const FORMAT_MACROS: &[&str] = &[
    "format", "print", "println", "eprint", "eprintln", "write", "writeln",
];

impl MetricCalculator for FormatDensity {
    fn id(&self) -> &'static str {
        "format-density"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Format Density",
            category: MetricCategory::RustErgonomics,
            polarity: MetricPolarity::LowerIsBetter,
            default_warning: Some(Threshold::new(8.0)),
            default_error: Some(Threshold::new(15.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.tree, |frame| {
            if frame.is_test() {
                return None;
            }
            frame
                .item
                .body()
                .map(|body| f64::from(count_format_macros(body.syntax())))
        })
    }
}

fn count_format_macros(node: &SyntaxNode) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        let Some(mac) = ast::MacroCall::cast(desc) else {
            continue;
        };
        let name = mac
            .path()
            .and_then(|p| p.segment())
            .and_then(|s| s.name_ref())
            .map(|n| n.text().to_string())
            .unwrap_or_default();
        if FORMAT_MACROS.contains(&name.as_str()) {
            n += 1;
        }
    }
    n
}

const RATIONALE: &str = "\
Format density counts string-formatting macro calls in a function body. \
Dense use suggests the function is doing more reporting / logging than \
domain work and would benefit from a structured logger or a separate \
display helper.";

const REFACTOR_HINTS: &[&str] = &[
    "Move log lines to a structured logger (`tracing`, `log`) so they don't pile up in business code.",
    "Build user-facing strings with a dedicated `Display` impl rather than inline `format!` calls.",
];

const REFERENCES: &[&str] = &[];
