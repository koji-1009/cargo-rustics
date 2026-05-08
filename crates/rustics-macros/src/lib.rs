//! `#[measured(...)]` — compile-time metric assertions.
//!
//! Plan §5.5. Apply to a function and the macro asserts at compile
//! time that every `<metric> <op> <n>` constraint holds; if any
//! crosses, the macro emits `compile_error!(...)` and the build fails
//! with a precise error location pointing at the function.
//!
//! ```ignore
//! use rustics_macros::measured;
//!
//! #[measured(cyclomatic_complexity < 10, lifetime_arity <= 2)]
//! fn parse(input: &str) -> Result<i32, ()> {
//!     // body
//! }
//! ```
//!
//! Recognised metrics (kebab-case in the lens catalogue, snake_case
//! here for Rust identifier syntax):
//!
//! * `cyclomatic_complexity` (`cyclomatic-complexity`)
//! * `cognitive_complexity` (`cognitive-complexity`)
//! * `maximum_nesting_level` (`maximum-nesting-level`)
//! * `number_of_parameters` (`number-of-parameters`)
//! * `lifetime_arity` (`lifetime-arity`)
//! * `generic_arity` (`generic-arity`)
//! * `clone_density` (`clone-density`)
//! * `unsafe_block_scope` (`unsafe-block-scope`)
//! * `panic_density` (`panic-density`)
//! * `result_chain_depth` (`result-chain-depth`)
//! * `await_depth` (`await-depth`)
//! * `halstead_volume` (`halstead-volume`)
//! * `method_length` (`method-length`)
//! * `source_lines_of_code` (`source-lines-of-code`)
//!
//! Operators: `<`, `<=`, `==`, `>=`, `>`, `!=`.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, ItemFn, LitInt, Token};

/// Attribute macro entry — see crate-level docs.
#[proc_macro_attribute]
pub fn measured(args: TokenStream, item: TokenStream) -> TokenStream {
    let constraints = parse_macro_input!(args as ConstraintList);
    let func = parse_macro_input!(item as ItemFn);

    if let Err(err) = check_constraints(&constraints.0, &func) {
        let msg = err.to_string();
        return quote!(compile_error!(#msg);).into();
    }
    quote!(#func).into()
}

struct ConstraintList(Punctuated<Constraint, Token![,]>);

struct Constraint {
    metric: syn::Ident,
    op: Op,
    threshold: u64,
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
    Ne,
}

impl Parse for ConstraintList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self(Punctuated::parse_terminated(input)?))
    }
}

impl Parse for Constraint {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let metric: syn::Ident = input.parse()?;
        let op = parse_op(input)?;
        let threshold: LitInt = input.parse()?;
        let threshold = threshold.base10_parse::<u64>()?;
        Ok(Constraint {
            metric,
            op,
            threshold,
        })
    }
}

fn parse_op(input: ParseStream) -> syn::Result<Op> {
    if let Some(op) = parse_two_char_op(input)? {
        return Ok(op);
    }
    parse_one_char_op(input)
}

fn parse_two_char_op(input: ParseStream) -> syn::Result<Option<Op>> {
    if input.peek(Token![<=]) {
        input.parse::<Token![<=]>()?;
        return Ok(Some(Op::Le));
    }
    if input.peek(Token![>=]) {
        input.parse::<Token![>=]>()?;
        return Ok(Some(Op::Ge));
    }
    if input.peek(Token![==]) {
        input.parse::<Token![==]>()?;
        return Ok(Some(Op::Eq));
    }
    if input.peek(Token![!=]) {
        input.parse::<Token![!=]>()?;
        return Ok(Some(Op::Ne));
    }
    Ok(None)
}

fn parse_one_char_op(input: ParseStream) -> syn::Result<Op> {
    if input.peek(Token![<]) {
        input.parse::<Token![<]>()?;
        return Ok(Op::Lt);
    }
    if input.peek(Token![>]) {
        input.parse::<Token![>]>()?;
        return Ok(Op::Gt);
    }
    Err(input.error("expected one of `<`, `<=`, `==`, `>=`, `>`, `!=`"))
}

#[derive(Debug)]
struct CheckError(String);

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn check_constraints(
    constraints: &Punctuated<Constraint, Token![,]>,
    func: &ItemFn,
) -> Result<(), CheckError> {
    let measurements = compute_measurements(func);
    for c in constraints {
        let metric_id = ident_to_metric_id(&c.metric)?;
        let value = measurements.get(metric_id).copied().unwrap_or(0.0);
        if !op_holds(c.op, value, c.threshold as f64) {
            return Err(CheckError(format!(
                "rustics::measured: `{name}` {op} {threshold} failed (actual {value})",
                name = c.metric,
                op = op_word(c.op),
                threshold = c.threshold,
                value = value,
            )));
        }
    }
    Ok(())
}

fn ident_to_metric_id(ident: &syn::Ident) -> Result<&'static str, CheckError> {
    let snake = ident.to_string();
    let id = match snake.as_str() {
        "cyclomatic_complexity" => "cyclomatic-complexity",
        "cognitive_complexity" => "cognitive-complexity",
        "maximum_nesting_level" => "maximum-nesting-level",
        "number_of_parameters" => "number-of-parameters",
        "lifetime_arity" => "lifetime-arity",
        "generic_arity" => "generic-arity",
        "clone_density" => "clone-density",
        "unsafe_block_scope" => "unsafe-block-scope",
        "panic_density" => "panic-density",
        "result_chain_depth" => "result-chain-depth",
        "await_depth" => "await-depth",
        "halstead_volume" => "halstead-volume",
        "method_length" => "method-length",
        "source_lines_of_code" => "source-lines-of-code",
        other => {
            return Err(CheckError(format!(
                "rustics::measured: unknown metric `{other}` — see #[measured] doc \
                 for the supported set"
            )));
        }
    };
    Ok(id)
}

fn op_word(op: Op) -> &'static str {
    match op {
        Op::Lt => "<",
        Op::Le => "<=",
        Op::Eq => "==",
        Op::Ge => ">=",
        Op::Gt => ">",
        Op::Ne => "!=",
    }
}

fn op_holds(op: Op, value: f64, threshold: f64) -> bool {
    match op {
        Op::Lt => value < threshold,
        Op::Le => value <= threshold,
        Op::Eq => (value - threshold).abs() < f64::EPSILON,
        Op::Ge => value >= threshold,
        Op::Gt => value > threshold,
        Op::Ne => (value - threshold).abs() >= f64::EPSILON,
    }
}

/// Wraps the `ItemFn` in a single-file `syn::File` so the rustics
/// library's per-file walker can run against it. Returns
/// `metric_id -> value` for every built-in lens that produces a
/// measurement on the function.
fn compute_measurements(func: &ItemFn) -> std::collections::HashMap<&'static str, f64> {
    let synthetic_file = syn::File {
        shebang: None,
        attrs: vec![],
        items: vec![syn::Item::Fn(func.clone())],
    };
    let mut out = std::collections::HashMap::new();
    let placeholder_path = std::path::PathBuf::from("__measured__.rs");
    let placeholder_source = String::new();
    let input = rustics::MetricInput::new(&placeholder_path, &placeholder_source, &synthetic_file);
    for metric in rustics::builtin_metrics() {
        let id = metric.id();
        let measurements = metric.measure(&input);
        if let Some(value) = measurements.first().map(|m| m.value) {
            out.insert(id, value);
        }
    }
    out
}
