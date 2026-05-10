//! Abstractness — Martin's `A`: ratio of abstract to total types
//! in a file.

use ra_ap_syntax::{ast::AstNode, SyntaxKind};

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
            display_name: "Abstractness (Martin 1994)",
            category: MetricCategory::Coupling,
            polarity: MetricPolarity::Informational,
            default_warning: None,
            default_error: None,
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        let mut total = 0u32;
        let mut abstract_ = 0u32;
        for desc in input.tree.syntax().descendants() {
            match desc.kind() {
                SyntaxKind::STRUCT | SyntaxKind::ENUM | SyntaxKind::UNION => total += 1,
                SyntaxKind::TRAIT => {
                    total += 1;
                    abstract_ += 1;
                }
                _ => {}
            }
        }
        if total == 0 {
            return Vec::new();
        }
        let scope_path = input
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        vec![MetricMeasurement::new(
            ScopeRef::new(scope_path, ScopeKind::Module, 1),
            f64::from(abstract_) / f64::from(total),
        )]
    }
}

const RATIONALE: &str =
    "Abstractness (Martin 1994) — ratio of trait declarations to total type declarations in a file.";

const REFACTOR_HINTS: &[&str] = &[];

const REFERENCES: &[&str] =
    &["Martin, R. (1994). OO Design Quality Metrics: An Analysis of Dependencies."];
