//! Halstead Volume — Halstead 1977.
//!
//! recovered metric from CS history.
//! Volume `V = N · log2(η)`, where:
//!
//! * `N1` = total operator occurrences, `n1` = distinct operators
//! * `N2` = total operand occurrences, `n2` = distinct operands
//! * `N`  = `N1 + N2` (length), `η` = `n1 + n2` (vocabulary)
//!
//! # Operator / operand split for Rust
//!
//! Operators (counted toward `N1`):
//!
//! * Every punctuation token (`+`, `=`, `;`, `,`, `?`, `::`, `&&`, …).
//! * Every keyword identifier (`if`, `let`, `match`, `fn`, …).
//! * Every grouping delimiter family (parens / braces / brackets — the
//!   delimiter pair counts once).
//!
//! Operands (counted toward `N2`):
//!
//! * Non-keyword identifiers.
//! * Literals (numbers, strings, characters, booleans).
//!
//! The split mirrors the conventional Halstead categorisation. We
//! deliberately keep it Layer 1 — token-level — so the metric runs on
//! the same `syn::File` every other lens uses, no separate lexer.

use std::collections::HashSet;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;

use crate::input::MetricInput;
use crate::measurement::MetricMeasurement;
use crate::metric::{MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, Threshold};
use crate::visitor::measure_functions;

/// Halstead Volume calculator.
#[derive(Debug, Default, Clone, Copy)]
pub struct HalsteadVolume;

impl MetricCalculator for HalsteadVolume {
    fn id(&self) -> &'static str {
        "halstead-volume"
    }

    fn metadata(&self) -> MetricMetadata {
        MetricMetadata {
            id: self.id(),
            display_name: "Halstead Volume",
            category: MetricCategory::Function,
            polarity: MetricPolarity::LowerIsBetter,
            // listed warning 1000 as a starting point; M1
            // self-application calibration shows ordinary Rust functions cluster ~700–1500 due
            // to verbose punctuation. We raise the warning to 1500 and
            // the error to 3000 so the lens flags genuine outliers.
            default_warning: Some(Threshold::new(1500.0)),
            default_error: Some(Threshold::new(3000.0)),
            rationale: RATIONALE,
            refactor_hints: REFACTOR_HINTS,
            references: REFERENCES,
        }
    }

    fn measure(&self, input: &MetricInput<'_>) -> Vec<MetricMeasurement> {
        measure_functions(input.ast, |frame| {
            // Tests carry fixture literals that inflate vocabulary
            // without reflecting production complexity. Skip them.
            if frame.is_test() {
                return None;
            }
            frame.body.map(|body| {
                // Each statement contributes its own token stream — we
                // never put the outer braces into the count, so the
                // grouping delimiter `{}` is *not* added by entering the
                // function (it would be added by inner blocks if any).
                let mut counts = HalsteadCounts::default();
                for stmt in &body.stmts {
                    counts.consume(&stmt.to_token_stream());
                }
                counts.volume()
            })
        })
    }
}

const RATIONALE: &str = "\
Halstead Volume measures the size of a program in *information-theoretic* \
units — the number of bits required to write down the operator and \
operand stream. It is sensitive to both length and vocabulary, so a \
function that uses many distinct names scores higher than one that reuses \
the same handful even when the line counts agree. The metric was \
proposed in 1977; it survived because the measurement target — \
'how big is this implementation, including its vocabulary' — is real.";

const REFACTOR_HINTS: &[&str] = &[
    "If the function uses many one-off names (`x_a`, `x_b`, `tmp1`, `tmp2`), \
collapse them into a struct or enum so the vocabulary shrinks.",
    "Long arithmetic / formatting expressions can often move to a helper. The \
helper's name replaces a stretch of operators and operands at the call \
site.",
    "Lift repeated literal constants to `const` definitions at module level. \
The function's operand vocabulary contracts.",
];

const REFERENCES: &[&str] = &[
    "Halstead, M. H. (1977). Elements of Software Science. Elsevier.",
];

/// Halstead counts — totals and distinct sets for both operators and operands.
#[derive(Default)]
struct HalsteadCounts {
    operator_total: u64,
    operand_total: u64,
    operators: HashSet<String>,
    operands: HashSet<String>,
}

impl HalsteadCounts {
    fn consume(&mut self, stream: &TokenStream) {
        for tt in stream.clone() {
            match tt {
                TokenTree::Ident(id) => {
                    let s = id.to_string();
                    if is_rust_keyword(&s) {
                        self.add_operator(s);
                    } else {
                        self.add_operand(s);
                    }
                }
                TokenTree::Punct(p) => self.add_operator(p.as_char().to_string()),
                TokenTree::Literal(lit) => self.add_operand(lit.to_string()),
                TokenTree::Group(g) => {
                    self.add_operator(delimiter_key(g.delimiter()).to_string());
                    self.consume(&g.stream());
                }
            }
        }
    }

    fn add_operator(&mut self, name: String) {
        self.operator_total += 1;
        self.operators.insert(name);
    }

    fn add_operand(&mut self, name: String) {
        self.operand_total += 1;
        self.operands.insert(name);
    }

    /// Halstead volume `V = N · log2(η)`. Returns 0 when the body is
    /// empty (vocabulary 0) — log2(0) is undefined; we treat the empty
    /// case as zero volume rather than NaN.
    fn volume(&self) -> f64 {
        let n = (self.operator_total + self.operand_total) as f64;
        let eta = (self.operators.len() + self.operands.len()) as f64;
        if n == 0.0 || eta <= 1.0 {
            return 0.0;
        }
        n * eta.log2()
    }
}

fn delimiter_key(d: Delimiter) -> &'static str {
    match d {
        Delimiter::Parenthesis => "()",
        Delimiter::Brace => "{}",
        Delimiter::Bracket => "[]",
        Delimiter::None => "<<inv>>",
    }
}

/// True iff `name` is a Rust strict or reserved keyword.
fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "Self"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn measure(src: &str) -> Vec<MetricMeasurement> {
        let ast = syn::parse_file(src).expect("parse");
        let input = MetricInput::new(Path::new("t.rs"), src, &ast);
        HalsteadVolume.measure(&input)
    }

    fn v_of(src: &str, scope: &str) -> f64 {
        measure(src)
            .into_iter()
            .find(|m| m.scope.path == scope)
            .map(|m| m.value)
            .unwrap_or_else(|| panic!("no scope `{scope}`"))
    }

    #[test]
    fn empty_body_is_zero() {
        assert_eq!(v_of("fn f() {}", "f"), 0.0);
    }

    #[test]
    fn vocab_one_is_zero_volume() {
        // body containing a single ident — vocab 1, log2(1) = 0.
        assert_eq!(v_of("fn f() { x }", "f"), 0.0);
    }

    #[test]
    fn volume_grows_with_length() {
        let small = v_of("fn f() { let x = 1; }", "f");
        let large = v_of(
            "fn f() { let x = 1; let y = 2; let z = 3; let w = x + y + z; }",
            "f",
        );
        assert!(large > small);
    }

    #[test]
    fn keyword_counts_as_operator() {
        // The two functions have the same operand vocabulary but the
        // first uses `let` (an operator); the second writes only the
        // expression. The first should have higher volume.
        let with_let = v_of("fn f() { let x = 1 + 2; }", "f");
        let without_let = v_of("fn f() { 1 + 2; }", "f");
        assert!(with_let > without_let);
    }
}
