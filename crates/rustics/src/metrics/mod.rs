//! Built-in metric implementations.
//!
//! Each module owns one metric. Metrics never depend on each other — the
//! independence principle (plan §3.2) is the precondition that lets the CLI
//! parallelise metric runs and that lets new lenses be added without
//! touching old ones.
//!
//! M1 ships exactly one metric (`cyclomatic_complexity`); the
//! [`crate::builtin_metrics`] enumeration is the public seam new lenses
//! plug into.

pub mod await_depth;
pub mod clone_density;
pub mod cyclomatic_complexity;
pub mod generic_arity;
pub mod lifetime_arity;
pub mod maximum_nesting_level;
pub mod method_length;
pub mod number_of_parameters;
pub mod panic_density;
pub mod result_chain_depth;
pub mod source_lines_of_code;
pub mod unsafe_block_scope;
