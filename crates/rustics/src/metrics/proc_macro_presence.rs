//! Proc-macro presence — count of attribute / derive macro
//! invocations on items in the file.

use ra_ap_syntax::{ast::{self, AstNode, HasAttrs}, SyntaxNode};

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity};
use crate::scope::{ScopeKind, ScopeRef};

/// Proc-macro presence calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcMacroPresence;

impl MetricCalculator for ProcMacroPresence {
    fn id(&self) -> &'static str {
        "proc-macro-presence"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Proc-Macro Presence (file-level)",
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
        let n = count_proc_attrs(input.tree.syntax());
        if n == 0 {
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
            f64::from(n),
        )]
    }
}

fn count_proc_attrs(node: &SyntaxNode) -> u32 {
    let mut n = 0u32;
    for desc in node.descendants() {
        if let Some(item) = ast::Item::cast(desc.clone()) {
            n += count_attrs_on_item(&item);
        }
    }
    n
}

fn count_attrs_on_item(item: &ast::Item) -> u32 {
    item.attrs()
        .filter(|a| attr_looks_like_proc_macro(a))
        .count() as u32
}

fn attr_looks_like_proc_macro(a: &ast::Attr) -> bool {
    let Some(path) = a.path() else {
        return false;
    };
    let segments: Vec<String> = path
        .segments()
        .filter_map(|s| s.name_ref().map(|n| n.text().to_string()))
        .collect();
    if segments.is_empty() {
        return false;
    }
    // Built-in attributes we don't count.
    const BUILTIN: &[&str] = &[
        "allow", "warn", "deny", "forbid", "cfg", "test", "bench", "doc",
        "inline", "must_use", "deprecated", "non_exhaustive", "no_mangle",
        "export_name", "repr", "macro_use", "macro_export",
        "automatically_derived", "rustfmt",
    ];
    if segments.len() == 1 && BUILTIN.contains(&segments[0].as_str()) {
        return false;
    }
    true
}

const RATIONALE: &str = "\
Proc-macro presence reports how many proc-macro invocations the file \
carries. Informational only — heavy proc-macro use can hide compile-time \
costs and obscure the file's plain-source meaning, but is not by itself \
a defect.";

const REFACTOR_HINTS: &[&str] = &[
    "Verify each derive's correctness with cargo expand if compile times grow.",
    "Group multiple derives that all need the same dependent crate behind a single attribute when possible.",
];

const REFERENCES: &[&str] = &[];
